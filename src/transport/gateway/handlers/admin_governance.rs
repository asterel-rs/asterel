//! Admin governance read models for companion-safe operator oversight.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

use crate::plugins::companion::surface::CompanionRequestWindowState;
use crate::runtime::diagnostics::control_plane_read_models::GovernanceSummaryReadModel;
use crate::runtime::services::{
    PendingWindowSnapshot, load_admin_governance_summary_with_runtime_trust,
};

use super::super::AppState;
use super::companion_helpers::companion_admin_scope_key;
use super::{request_management_policy_context, require_management_principal};

pub(crate) async fn handle_admin_governance_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<GovernanceSummaryReadModel>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;
    let policy_context = request_management_policy_context(&state, &headers)?;
    let allowed_scope = companion_admin_scope_key(&policy_context)?;
    let memory_review_available = state.runtime.mem.list_entities().await.is_ok();

    let scope_keys = state
        .companion
        .companion_request_windows
        .scope_keys()
        .await
        .into_iter()
        .filter(|scope| scope == &allowed_scope)
        .collect::<Vec<_>>();
    let mut pending_window_items = Vec::new();
    for scope in &scope_keys {
        if let Some(handle) = state
            .companion
            .companion_request_windows
            .get_scope(scope)
            .await
        {
            let windows = handle.lock().await;
            for window in windows
                .values()
                .filter(|window| window.state == CompanionRequestWindowState::Pending)
            {
                pending_window_items.push(PendingWindowSnapshot {
                    scope: scope.clone(),
                    window_id: window.window_id.clone(),
                    requested_action: window.requested_action.clone(),
                    created_at: window.created_at.clone(),
                    expires_at: window.expires_at.clone(),
                });
            }
        }
    }

    Ok(Json(load_admin_governance_summary_with_runtime_trust(
        state.runtime.mem.name().to_string(),
        memory_review_available,
        scope_keys.len(),
        pending_window_items,
    )))
}
