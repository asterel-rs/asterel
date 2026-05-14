//! Gateway tenant-scope helpers: bearer-to-principal resolution, public
//! request tenant scoping, and operator-selected tenant context.
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

use super::super::AppState;
use super::super::autosave::{gateway_runtime_policy_context, sanitize_tenant_id};
use super::super::problem_details::problem_response;
use super::auth_context::{bearer_token, hashed_auth_principal};
use crate::runtime::services;
use crate::security::policy::TenantPolicyContext;

pub(in super::super) fn request_tenant_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-asterel-tenant")
        .and_then(|value| value.to_str().ok())
        .and_then(sanitize_tenant_id)
}

fn selected_tenant_for_principal(state: &AppState, principal: &str) -> Option<String> {
    services::resolve_selected_tenant_for_principal(
        &state.connections.tenant_bindings,
        &state.runtime.config,
        principal,
    )
}

fn tenant_scope_for_header(
    state: &AppState,
    principal: &str,
    tenant_id: &str,
) -> Result<TenantPolicyContext, (StatusCode, Json<serde_json::Value>)> {
    if let Some(selected_tenant) = selected_tenant_for_principal(state, principal)
        && selected_tenant != tenant_id
    {
        return Err(problem_response(
            StatusCode::FORBIDDEN,
            "tenant_scope_mismatch",
            "Forbidden",
            "Requested tenant scope does not match the paired bearer tenant context.",
        ));
    }

    Ok(gateway_runtime_policy_context(Some(tenant_id)))
}

pub(in super::super) fn paired_bearer_principal(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<String> {
    if !state.access.pairing.is_paired() {
        return None;
    }
    bearer_token(headers)
        .filter(|token| state.access.pairing.is_accepted_token(token))
        .map(hashed_auth_principal)
}

pub(in super::super) fn request_policy_context(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<TenantPolicyContext, (StatusCode, Json<serde_json::Value>)> {
    let principal = paired_bearer_principal(state, headers);
    let tenant_id = request_tenant_id(headers);
    if let Some(tenant_id) = tenant_id.as_deref() {
        let Some(principal) = principal.as_deref() else {
            return Err(problem_response(
                StatusCode::FORBIDDEN,
                "tenant_scope_requires_paired_bearer",
                "Forbidden",
                "Tenant-scoped gateway requests require an authenticated paired bearer token.",
            ));
        };
        return tenant_scope_for_header(state, principal, tenant_id);
    }

    Ok(gateway_runtime_policy_context(None))
}

pub(in super::super) fn request_management_policy_context(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<TenantPolicyContext, (StatusCode, Json<serde_json::Value>)> {
    let principal = paired_bearer_principal(state, headers);
    let tenant_id = request_tenant_id(headers);
    if let Some(tenant_id) = tenant_id.as_deref() {
        let Some(principal) = principal.as_deref() else {
            return Err(problem_response(
                StatusCode::FORBIDDEN,
                "tenant_scope_requires_paired_bearer",
                "Forbidden",
                "Tenant-scoped gateway requests require an authenticated paired bearer token.",
            ));
        };
        return tenant_scope_for_header(state, principal, tenant_id);
    }

    if let Some(principal) = principal.as_deref()
        && let Some(selected_tenant) = selected_tenant_for_principal(state, principal)
    {
        return Ok(gateway_runtime_policy_context(Some(&selected_tenant)));
    }

    Ok(gateway_runtime_policy_context(None))
}
