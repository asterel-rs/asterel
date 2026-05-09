//! Agent listing handler.
//!
//! Routes:
//! - `GET /admin/v1/agents` — list available agents from runtime config

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

use super::super::AppState;
use super::require_management_principal;

pub(crate) async fn handle_agents_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let mut agents = Vec::<serde_json::Value>::new();

    // Primary agent from runtime config
    agents.push(serde_json::json!({
        "id": "primary",
        "name": "Asterel",
        "model": state.runtime.model,
        "is_default": true,
        "status": "active",
    }));

    // Active subagent runs
    let subagent_runs = state.runtime.subagent_manager.list();
    for run in &subagent_runs {
        agents.push(serde_json::json!({
            "id": run.run_id.to_string(),
            "name": run.label.as_deref().unwrap_or("subagent"),
            "model": run.model,
            "is_default": false,
            "status": format!("{:?}", run.status).to_lowercase(),
            "task": run.task,
        }));
    }

    Ok(Json(serde_json::json!({
        "items": agents,
        "count": agents.len(),
    })))
}

pub(crate) use handle_agents_list as handle_admin_agents_list;
