//! Gateway defense layer: external-content ingress policy, bearer-token
//! validation, rate-limit accounting, and per-defense-mode enforcement.
use axum::http::StatusCode;
use axum::response::Json;

use super::AppState;
use super::problem_details::problem_response;
use crate::config::ExternalKnowledgeTrustConfig;
use crate::config::GatewayDefenseMode;
use crate::security::external_content::{ExternalAction, prepare_content_with_trust};

/// Result of applying the external ingress content policy to incoming
/// text.
#[derive(Debug, Clone)]
pub(super) struct ExternalIngressPolicyOutcome {
    pub(super) model_input: String,
    pub(super) persisted_summary: String,
    pub(super) blocked: bool,
}

/// Wraps incoming external text with safety boundaries and determines
/// whether the content should be blocked.
pub(super) fn apply_external_ingress_policy(
    source: &str,
    text: &str,
    trust: &ExternalKnowledgeTrustConfig,
) -> ExternalIngressPolicyOutcome {
    let prepared = prepare_content_with_trust(source, text, trust);

    ExternalIngressPolicyOutcome {
        model_input: prepared.model_input,
        persisted_summary: prepared.persisted_summary.as_memory_value(),
        blocked: matches!(prepared.action, ExternalAction::Block),
    }
}

/// Categories of gateway authentication and authorization failures.
#[derive(Debug, Clone, Copy)]
pub(super) enum PolicyViolation {
    KillSwitchEnabled,
    MissingOrInvalidBearer,
    MissingOrInvalidWebhookSecret,
    NoAuthConfigured,
}

impl PolicyViolation {
    /// Returns `true` if this violation is an authentication failure.
    pub(super) fn is_auth_violation(self) -> bool {
        matches!(
            self,
            Self::MissingOrInvalidBearer | Self::MissingOrInvalidWebhookSecret
        )
    }

    /// Returns a machine-readable reason string for this violation.
    pub(super) fn reason(self) -> &'static str {
        match self {
            Self::KillSwitchEnabled => "kill_switch_enabled",
            Self::MissingOrInvalidBearer => "missing_or_invalid_bearer",
            Self::MissingOrInvalidWebhookSecret => "missing_or_invalid_webhook_secret",
            Self::NoAuthConfigured => "no_auth_configured",
        }
    }

    /// Builds an HTTP error response for this policy violation.
    pub(super) fn enforce_response(self) -> (StatusCode, Json<serde_json::Value>) {
        match self {
            Self::KillSwitchEnabled => problem_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "kill_switch_enabled",
                "Service Unavailable",
                "Service unavailable — gateway emergency kill switch is enabled.",
            ),
            Self::MissingOrInvalidBearer => problem_response(
                StatusCode::UNAUTHORIZED,
                "missing_or_invalid_bearer",
                "Unauthorized",
                "Unauthorized — pair first via POST /pair, then send Authorization: Bearer <token>",
            ),
            Self::MissingOrInvalidWebhookSecret => problem_response(
                StatusCode::UNAUTHORIZED,
                "missing_or_invalid_webhook_secret",
                "Unauthorized",
                "Unauthorized — invalid or missing X-Webhook-Secret header",
            ),
            Self::NoAuthConfigured => problem_response(
                StatusCode::FORBIDDEN,
                "no_auth_configured",
                "Forbidden",
                "Forbidden — no authentication configured. Pair first via POST /pair or configure a webhook secret.",
            ),
        }
    }
}

/// Returns the active defense mode.
pub(super) fn effective_defense_mode(state: &AppState) -> GatewayDefenseMode {
    state.access.defense_mode
}

/// Applies the defense mode to a non-auth policy violation, returning
/// an error response for Warn/Enforce modes or `None` for Audit.
pub(super) fn policy_violation_response(
    state: &AppState,
    violation: PolicyViolation,
) -> Option<(StatusCode, Json<serde_json::Value>)> {
    debug_assert!(
        !violation.is_auth_violation(),
        "auth violations must be handled by must_enforce_auth_violation"
    );

    let mode = effective_defense_mode(state);
    let reason = violation.reason();
    match mode {
        GatewayDefenseMode::Audit => {
            tracing::warn!(
                mode = "audit",
                violation = reason,
                "Webhook policy violation recorded"
            );
            None
        }
        GatewayDefenseMode::Warn => {
            tracing::warn!(
                mode = "warn",
                violation = reason,
                "Webhook policy violation warning"
            );
            Some((
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "mode": "warn",
                    "warning": reason,
                    "blocked": false
                })),
            ))
        }
        GatewayDefenseMode::Enforce => {
            tracing::warn!(
                mode = "enforce",
                violation = reason,
                "Webhook policy violation blocked"
            );
            Some(violation.enforce_response())
        }
    }
}

/// Returns `true` if an authentication violation must be enforced
/// (always true regardless of defense mode).
pub(super) fn must_enforce_auth_violation(state: &AppState, violation: PolicyViolation) -> bool {
    debug_assert!(violation.is_auth_violation());

    let mode = effective_defense_mode(state);
    let reason = violation.reason();

    match mode {
        GatewayDefenseMode::Audit => {
            tracing::warn!(
                mode = "audit",
                violation = reason,
                "Authentication violation blocked (audit mode keeps auth enforcement)"
            );
            true
        }
        GatewayDefenseMode::Warn => {
            tracing::warn!(
                mode = "warn",
                violation = reason,
                "Authentication violation blocked (warn mode)"
            );
            true
        }
        GatewayDefenseMode::Enforce => {
            tracing::warn!(
                mode = "enforce",
                violation = reason,
                "Authentication violation blocked"
            );
            true
        }
    }
}

/// Returns a 429 Too Many Requests response for rate-limit violations.
pub(super) fn policy_accounting_response(
    policy_error: &'static str,
) -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::TOO_MANY_REQUESTS,
        "policy_limit_exceeded",
        "Too Many Requests",
        policy_error,
    )
}
