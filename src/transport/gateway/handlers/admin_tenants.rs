use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

use super::super::AppState;
use super::super::problem_details::problem_response;
use super::require_paired_bearer_principal;
use crate::runtime::services::{
    OperatorScope, load_operator_tenant_context_read_model, load_operator_tenant_context_view,
    load_operator_tenant_inventory_read_model, update_operator_tenant_context_view,
};

pub(crate) async fn handle_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let principal = require_paired_bearer_principal(&state, &headers)?;
    let caller_tenant = load_operator_tenant_context_view(
        &state.connections.tenant_bindings,
        &state.runtime.config,
        &principal,
        None,
    );
    let scope = OperatorScope {
        principal,
        tenant_id: caller_tenant.active_tenant.clone(),
        tenant_mode_available: caller_tenant.tenant_mode_available,
    };

    let inventory = load_operator_tenant_inventory_read_model(
        &state.connections.tenant_bindings,
        &state.runtime.config,
        &scope,
    )
    .await
    .map_err(|error| {
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "tenant_bindings_load_failed",
            "Internal Server Error",
            format!("Failed to load tenant bindings: {error}"),
        )
    })?;

    Ok(Json(serde_json::to_value(inventory).map_err(|error| {
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialization_error",
            "Internal Server Error",
            error.to_string(),
        )
    })?))
}

pub(crate) async fn handle_context(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let principal = require_paired_bearer_principal(&state, &headers)?;

    let view = load_operator_tenant_context_read_model(
        &state.connections.tenant_bindings,
        &state.runtime.config,
        &principal,
        headers
            .get("x-asterel-tenant")
            .and_then(|v| v.to_str().ok()),
    );

    Ok(Json(serde_json::to_value(view).map_err(|error| {
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialization_error",
            "Internal Server Error",
            error.to_string(),
        )
    })?))
}

#[derive(serde::Deserialize)]
pub(crate) struct TenantContextSetBody {
    pub tenant_id: Option<String>,
}

pub(crate) async fn handle_set_context(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<TenantContextSetBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let principal = require_paired_bearer_principal(&state, &headers)?;

    let tenant_id = body.tenant_id;

    if tenant_id
        .as_deref()
        .is_some_and(|value| super::super::autosave::sanitize_tenant_id(value).is_none())
    {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "invalid_tenant_id",
            "Bad Request",
            "tenant_id must contain at least one ASCII letter or digit.".to_string(),
        ));
    }

    let update = update_operator_tenant_context_view(
        &state.connections.tenant_bindings,
        &state.runtime.config,
        &principal,
        tenant_id.as_deref(),
    )
    .map_err(|error| {
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "tenant_bindings_save_failed",
            "Internal Server Error",
            format!("Failed to save tenant bindings: {error}"),
        )
    })?;

    Ok(Json(serde_json::json!({
        "status": "updated",
        "tenant_id": update.tenant_id,
    })))
}

pub(crate) use handle_context as handle_admin_tenant_context;
pub(crate) use handle_list as handle_admin_tenants_list;
pub(crate) use handle_set_context as handle_admin_set_tenant_context;
