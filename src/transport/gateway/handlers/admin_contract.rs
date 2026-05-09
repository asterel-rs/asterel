use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

use super::super::AppState;
use super::super::admin_contract::admin_openapi_contract_json;
use super::super::problem_details::problem_response;
use super::require_management_principal;

pub(crate) async fn handle_admin_openapi_contract(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;
    admin_openapi_contract_json().map(Json).map_err(|error| {
        tracing::error!(%error, "failed to load embedded admin OpenAPI contract");
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "admin_openapi_contract_unavailable",
            "Internal Server Error",
            "Failed to load admin OpenAPI contract",
        )
    })
}
