//! Activity timeline handler — returns recent activity across subsystems.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

use super::super::AppState;
use super::{request_management_policy_context, require_management_principal};
use crate::runtime::services::{OperatorScope, list_operator_sessions};

pub(crate) async fn handle_activity_timeline(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let principal = require_management_principal(&state, &headers)?;
    let policy_context = request_management_policy_context(&state, &headers)?;
    let scope = OperatorScope::from_management_context(principal, policy_context);

    let mut events = Vec::<serde_json::Value>::new();

    // Gather recent sessions
    if let Some(session_manager) = state.runtime.session_manager.as_deref()
        && let Ok(sessions) = list_operator_sessions(session_manager, &scope, None, Some(10)).await
    {
        for session in sessions.items.iter().take(10) {
            events.push(serde_json::json!({
                "kind": "session",
                "id": session.id,
                "label": format!("{} session", session.surface),
                "state": session.state,
                "timestamp": session.updated_at,
            }));
        }
    }

    // Sort by timestamp descending
    events.sort_by(|a, b| {
        let a_ts = a.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
        let b_ts = b.get("timestamp").and_then(|v| v.as_str()).unwrap_or("");
        b_ts.cmp(a_ts)
    });

    // Keep only the most recent 20
    events.truncate(20);

    Ok(Json(serde_json::json!({
        "events": events,
        "count": events.len(),
    })))
}

pub(crate) use handle_activity_timeline as handle_admin_activity_timeline;
