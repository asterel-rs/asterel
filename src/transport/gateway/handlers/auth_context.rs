//! Gateway edge auth helpers: bearer parsing, request guards, and
//! webhook/source identity normalization.
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Json;

use super::super::AppState;
use super::super::defense::{
    PolicyViolation, kill_switch_response, must_enforce_auth_violation, policy_violation_response,
};
use super::super::problem_details::problem_response;
use super::tenant_scope;
use crate::core::persona::person_identity::sanitize_person_id;
use crate::security::pairing::{constant_time_eq, hash_token};

pub(in super::super) const WEBHOOK_SOURCE_HEADER: &str = "x-asterel-source";
const MAX_WEBHOOK_SOURCE_LEN: usize = 128;

pub(in super::super) fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.strip_prefix("Bearer "))
        .filter(|token| !token.is_empty())
}

pub(in super::super) fn hashed_auth_principal(token: &str) -> String {
    let digest = hash_token(token);
    let short = &digest[..16];
    format!("auth-{short}")
}

pub(in super::super) fn verified_source_identifier_from_headers(
    state: &AppState,
    headers: &HeaderMap,
    fallback: &str,
) -> String {
    if state.access.pairing.is_paired()
        && let Some(token) = bearer_token(headers)
        && state.access.pairing.is_authenticated(token)
    {
        return hashed_auth_principal(token);
    }

    fallback.to_string()
}

pub(in super::super) fn resolve_webhook_source_identifier(
    headers: &HeaderMap,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    if let Some(token) = bearer_token(headers) {
        return Ok(hashed_auth_principal(token));
    }

    let Some(raw_source) = headers
        .get(WEBHOOK_SOURCE_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "missing_webhook_source",
            "Bad Request",
            "Shared-secret webhook requests must include x-asterel-source.",
        ));
    };

    if raw_source.len() > MAX_WEBHOOK_SOURCE_LEN {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "invalid_webhook_source",
            "Bad Request",
            format!("x-asterel-source must be {MAX_WEBHOOK_SOURCE_LEN} characters or fewer."),
        ));
    }

    let sanitized = sanitize_person_id(raw_source);
    if sanitized.is_empty() {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "invalid_webhook_source",
            "Bad Request",
            "x-asterel-source must contain at least one ASCII letter or digit.",
        ));
    }

    Ok(sanitized)
}

/// Build a trust-scoring source identifier from the request context.
///
/// Security note: this intentionally ignores client-provided headers
/// such as `X-Signature-Verified` or `X-Forwarded-Proto`. An untrusted
/// caller could inject these headers to artificially boost their trust
/// score, so gateway trust sources stay limited to signals the gateway
/// can verify on its own.
pub(in super::super) fn external_trust_source_from_headers(
    headers: &HeaderMap,
    base: &str,
) -> String {
    let _ = headers;
    base.to_string()
}

pub(in super::super) fn enforce_json_content_type(
    headers: &HeaderMap,
) -> Option<(StatusCode, Json<serde_json::Value>)> {
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| {
            let lower = content_type.to_ascii_lowercase();
            lower.starts_with("application/json")
        });
    if is_json {
        None
    } else {
        Some(problem_response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_content_type",
            "Unsupported Media Type",
            "Content-Type must be application/json".to_string(),
        ))
    }
}

pub(in super::super) fn enforce_json_request_guards(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<(StatusCode, Json<serde_json::Value>)> {
    enforce_request_auth(state, headers).or_else(|| enforce_json_content_type(headers))
}

pub(in super::super) fn enforce_request_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<(StatusCode, Json<serde_json::Value>)> {
    if let Some(response) = kill_switch_response(state) {
        return Some(response);
    }

    if state.access.pairing.is_paired() {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let token = auth.strip_prefix("Bearer ").unwrap_or("");
        if !state.access.pairing.is_authenticated(token) {
            let violation = PolicyViolation::MissingOrInvalidBearer;
            if must_enforce_auth_violation(state, violation) {
                return Some(violation.enforce_response());
            }
        }
    }

    if let Some(secret) = &state.access.webhook_secret {
        let header_value = headers
            .get("X-Webhook-Secret")
            .and_then(|value| value.to_str().ok());
        match header_value {
            Some(value) if constant_time_eq(value, secret.as_ref()) => {}
            _ => {
                let violation = PolicyViolation::MissingOrInvalidWebhookSecret;
                if must_enforce_auth_violation(state, violation) {
                    return Some(violation.enforce_response());
                }
            }
        }
    }

    if !state.access.pairing.is_paired()
        && state.access.webhook_secret.is_none()
        && let Some(response) = policy_violation_response(state, PolicyViolation::NoAuthConfigured)
    {
        return Some(response);
    }

    None
}

pub(in super::super) fn require_management_principal(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    let principal = require_paired_bearer_principal(state, headers)?;

    let policy_context = tenant_scope::request_management_policy_context(state, headers)?;
    if policy_context.tenant_id.is_none() {
        return Err(problem_response(
            StatusCode::FORBIDDEN,
            "tenant_scope_required",
            "Forbidden",
            "Admin endpoints require a tenant-scoped paired bearer token.",
        ));
    }

    Ok(principal)
}

pub(in super::super) fn require_paired_bearer_principal(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    if let Some(response) = kill_switch_response(state) {
        return Err(response);
    }

    if !state.access.pairing.is_paired() {
        return Err(problem_response(
            StatusCode::FORBIDDEN,
            "paired_bearer_required",
            "Forbidden",
            "This endpoint requires an authenticated paired bearer token.",
        ));
    }

    tenant_scope::paired_bearer_principal(state, headers)
        .ok_or_else(|| PolicyViolation::MissingOrInvalidBearer.enforce_response())
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, header};

    use super::{WEBHOOK_SOURCE_HEADER, hash_token, resolve_webhook_source_identifier};

    #[test]
    fn resolve_webhook_source_identifier_uses_hashed_bearer_principal_when_present() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer source-token".parse().expect("header"),
        );
        headers.insert(WEBHOOK_SOURCE_HEADER, "producer-a".parse().expect("header"));

        let resolved =
            resolve_webhook_source_identifier(&headers).expect("bearer identity should win");

        assert_eq!(
            resolved,
            format!("auth-{}", &hash_token("source-token")[..16])
        );
    }

    #[test]
    fn resolve_webhook_source_identifier_requires_shared_secret_caller_header() {
        let headers = HeaderMap::new();

        let (status, body) = resolve_webhook_source_identifier(&headers)
            .expect_err("shared-secret callers should require an explicit caller header");

        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(body.0["code"], "missing_webhook_source");
    }

    #[test]
    fn resolve_webhook_source_identifier_sanitizes_shared_secret_caller_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            WEBHOOK_SOURCE_HEADER,
            "Desk Lamp:Reader".parse().expect("header"),
        );

        let resolved =
            resolve_webhook_source_identifier(&headers).expect("source header should sanitize");

        assert_eq!(resolved, "Desk_Lamp_Reader__hdaca0d413151");
    }
}
