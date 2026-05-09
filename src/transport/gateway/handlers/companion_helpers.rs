//! Shared helpers for companion handlers: payload validation, error
//! constructors, replay-scope keys, and gateway event publishing.
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as FmtWrite;

use axum::http::StatusCode;
use axum::response::Json;
use serde_json::Value;

use super::super::AppState;
use super::super::companion_bridge::{
    CompanionCaptionEvt, CompanionContextIngressReason, CompanionContextKind, CompanionCtxEvent,
    CompanionWindow,
};
use super::super::events::ServerMessage;
use super::super::problem_details::problem_response;
use crate::core::memory::SourceKind;
use crate::security::policy::TenantPolicyContext;

/// Returns a 400 error response for invalid companion context payloads.
pub(super) fn companion_context_ingest_invalid_payload(
    detail: impl Into<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::BAD_REQUEST,
        "invalid_companion_context_ingest_payload",
        "Bad Request",
        detail.into(),
    )
}

/// Returns a 400 error response when companion context ingestion fails.
pub(super) fn companion_context_ingest_failed(
    detail: impl Into<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::BAD_REQUEST,
        "companion_context_ingest_failed",
        "Bad Request",
        detail.into(),
    )
}

/// Returns a 400 error response for invalid multimodal ingest payloads.
pub(super) fn companion_multimodal_ingest_invalid_payload(
    detail: impl Into<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::BAD_REQUEST,
        "invalid_companion_multimodal_ingest_payload",
        "Bad Request",
        detail.into(),
    )
}

/// Returns a 400 error response for invalid caption payloads.
pub(super) fn companion_surface_caption_invalid_payload(
    detail: impl Into<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::BAD_REQUEST,
        "invalid_companion_surface_caption_payload",
        "Bad Request",
        detail.into(),
    )
}

/// Returns a 400 error response for invalid widget command payloads.
pub(super) fn companion_surface_widget_invalid_payload(
    detail: impl Into<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::BAD_REQUEST,
        "invalid_companion_surface_widget_payload",
        "Bad Request",
        detail.into(),
    )
}

/// Returns a 400 error response for invalid request-window payloads.
pub(super) fn companion_surface_request_window_invalid_payload(
    detail: impl Into<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::BAD_REQUEST,
        "invalid_companion_surface_request_window_payload",
        "Bad Request",
        detail.into(),
    )
}

/// Returns a 404 error response when a request window is not found.
pub(super) fn companion_surface_request_window_not_found(
    window_id: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::NOT_FOUND,
        "companion_request_window_not_found",
        "Not Found",
        format!("request-window '{window_id}' not found"),
    )
}

/// Default time-to-live in seconds for companion request windows.
pub(super) const COMPANION_REQUEST_WINDOW_DEFAULT_TTL_SECS: u64 = 60;
/// Maximum number of request windows stored per scope before pruning.
pub(super) const COMPANION_REQUEST_WINDOW_MAX_ENTRIES: usize = 1024;

/// Derives a scope key from the tenant policy context (tenant-scoped
/// or `"global"`).
pub(super) fn companion_scope_key(policy_context: &TenantPolicyContext) -> String {
    if policy_context.tenant_mode_enabled
        && let Some(tenant_id) = policy_context.tenant_id.as_deref()
        && !tenant_id.is_empty()
    {
        return format!("tenant:{tenant_id}");
    }
    "global".to_string()
}

/// Derives the only companion admin scope visible to a tenant-scoped
/// management request.
pub(super) fn companion_admin_scope_key(
    policy_context: &TenantPolicyContext,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    if policy_context.tenant_mode_enabled
        && policy_context
            .tenant_id
            .as_deref()
            .is_none_or(|tenant_id| tenant_id.trim().is_empty())
    {
        return Err(problem_response(
            StatusCode::FORBIDDEN,
            "tenant_scope_required",
            "Forbidden",
            "Tenant-scoped companion admin requests require tenant context.".to_string(),
        ));
    }
    Ok(companion_scope_key(policy_context))
}

/// Rejects caller-selected companion admin scopes that do not match the
/// authenticated management tenant context.
pub(super) fn enforce_companion_admin_scope(
    requested_scope: &str,
    policy_context: &TenantPolicyContext,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let allowed_scope = companion_admin_scope_key(policy_context)?;
    if requested_scope == allowed_scope {
        return Ok(());
    }
    Err(problem_response(
        StatusCode::FORBIDDEN,
        "companion_scope_mismatch",
        "Forbidden",
        format!(
            "Requested companion scope '{requested_scope}' is outside the authenticated management scope."
        ),
    ))
}

/// Builds a replay-guard scope from a base scope and scope key.
pub(super) fn companion_replay_scope(base_scope: &str, scope_key: &str) -> String {
    format!("{base_scope}:{scope_key}")
}

/// Builds a replay-guard scope for a specific request window.
pub(super) fn companion_request_window_replay_scope(
    base_scope: &str,
    scope_key: &str,
    window_id: &str,
) -> String {
    format!("{base_scope}:{scope_key}:{window_id}")
}

/// Parses an RFC 3339 timestamp string into a UTC `DateTime`.
pub(super) fn parse_rfc3339_utc(raw: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    crate::platform::cron::parse_rfc3339(raw).ok()
}

/// Removes expired and excess request windows, keeping the newest.
pub(super) fn prune_companion_request_windows(
    windows: &mut HashMap<String, CompanionWindow>,
    now: chrono::DateTime<chrono::Utc>,
) {
    windows.retain(|_, window| {
        parse_rfc3339_utc(&window.expires_at).is_some_and(|expires_at| now <= expires_at)
    });

    if windows.len() <= COMPANION_REQUEST_WINDOW_MAX_ENTRIES {
        return;
    }

    let overflow = windows
        .len()
        .saturating_sub(COMPANION_REQUEST_WINDOW_MAX_ENTRIES);
    let mut by_expiry = windows
        .iter()
        .filter_map(|(store_key, window)| {
            parse_rfc3339_utc(&window.expires_at).map(|expires_at| (store_key.clone(), expires_at))
        })
        .collect::<Vec<_>>();
    by_expiry.sort_by_key(|(_, expires_at)| *expires_at);

    for (store_key, _) in by_expiry.into_iter().take(overflow) {
        windows.remove(&store_key);
    }
}

/// Broadcasts a gateway event to all active WebSocket subscribers.
pub(crate) fn publish_gateway_event(state: &AppState, message: ServerMessage) {
    if let Err(error) = state.companion.gateway_events.send(message) {
        tracing::trace!(%error, "gateway event dropped because no websocket subscribers are active");
    }
}

/// Converts an ingress reason enum variant to its string label.
pub(super) fn companion_context_ingress_reason_label(
    reason: CompanionContextIngressReason,
) -> &'static str {
    match reason {
        CompanionContextIngressReason::Accepted => "accepted",
        CompanionContextIngressReason::DuplicateSuppressed => "duplicate_suppressed",
    }
}

/// Returns the requested sequence number, or the next sequential
/// value derived from the existing caption log.
pub(super) fn caption_sequence_or_next(
    requested: Option<u64>,
    existing_log: &VecDeque<CompanionCaptionEvt>,
) -> u64 {
    requested.unwrap_or_else(|| {
        existing_log
            .back()
            .map_or(1, |event| event.sequence.saturating_add(1))
    })
}

/// Maps a companion context kind to its corresponding memory source
/// kind.
pub(super) fn companion_context_source_kind(kind: CompanionContextKind) -> SourceKind {
    match kind {
        CompanionContextKind::Page | CompanionContextKind::Video => SourceKind::Document,
        CompanionContextKind::Subtitle => SourceKind::Conversation,
        CompanionContextKind::VisionFrame => SourceKind::Api,
    }
}

/// Trims and filters an optional entity ID, returning `None` for
/// empty or whitespace-only values.
pub(super) fn normalize_optional_entity_id(
    raw: Option<&str>,
) -> Option<crate::contracts::ids::EntityId> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(crate::contracts::ids::EntityId::new)
}

/// Serializes a JSON value to a truncated summary string for logging.
pub(super) fn payload_summary(payload: &Value, max_chars: usize) -> String {
    if payload.is_null() {
        return String::new();
    }
    let serialized = serde_json::to_string(payload).unwrap_or_else(|_| String::new());
    if serialized.len() <= max_chars || serialized.chars().count() <= max_chars {
        return serialized;
    }

    let mut shortened = serialized.chars().take(max_chars).collect::<String>();
    shortened.push_str("...");
    shortened
}

/// Builds a pipe-delimited content summary string from a companion
/// context event for memory storage.
pub(super) fn companion_context_content(event: &CompanionCtxEvent) -> String {
    let mut out = format!(
        "kind={} topic={} source={}",
        event.kind.as_str(),
        event.topic,
        event.source
    );
    if let Some(source_url) = event.source_url.as_deref() {
        let _ = write!(out, " | source_url={source_url}");
    }
    if let Some(media_ref) = event.media_ref.as_deref() {
        let _ = write!(out, " | media_ref={media_ref}");
    }
    let summary = payload_summary(&event.payload, 512);
    if !summary.is_empty() {
        let _ = write!(out, " | payload={summary}");
    }
    out
}
