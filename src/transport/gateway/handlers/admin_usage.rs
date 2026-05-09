//! Token usage tracking handler.
//!
//! Routes:
//! - `GET /admin/v1/usage` — aggregate token counts across sessions

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

use super::super::AppState;
use super::super::problem_details::problem_response;
use super::{request_management_policy_context, require_management_principal};
use crate::contracts::ids::SessionId;
use crate::runtime::services::{
    OperatorScope, OperatorSessionAccess, list_operator_session_messages, list_operator_sessions,
};

pub(crate) async fn handle_usage(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let principal = require_management_principal(&state, &headers)?;
    let policy_context = request_management_policy_context(&state, &headers)?;
    let scope = OperatorScope::from_management_context(principal, policy_context);

    let Some(session_manager) = state.runtime.session_manager.as_deref() else {
        return Ok(Json(serde_json::json!({
            "total_input_tokens": 0,
            "total_output_tokens": 0,
            "total_tokens": 0,
            "session_count": 0,
            "message_count": 0,
        })));
    };

    let mut total_input: u64 = 0;
    let mut total_output: u64 = 0;
    let mut message_count: u64 = 0;
    let mut session_count: usize = 0;
    let mut session_cursor: Option<String> = None;

    loop {
        let sessions = list_operator_sessions(
            session_manager,
            &scope,
            session_cursor.as_deref(),
            Some(200),
        )
        .await
        .map_err(|error| {
            tracing::error!(%error, "admin: failed to list scoped sessions for usage");
            problem_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "usage_list_failed",
                "Internal Server Error",
                "Failed to list sessions for usage aggregation.".to_string(),
            )
        })?;

        session_count = session_count.saturating_add(sessions.items.len());
        for session in &sessions.items {
            let session_id = SessionId::new(session.id.clone());
            let mut message_cursor: Option<String> = None;
            loop {
                let Ok(OperatorSessionAccess::Visible(messages)) = list_operator_session_messages(
                    session_manager,
                    &scope,
                    &session_id,
                    message_cursor.as_deref(),
                    Some(200),
                )
                .await
                else {
                    break;
                };
                for msg in &messages.items {
                    total_input += msg.input_tokens.unwrap_or(0);
                    total_output += msg.output_tokens.unwrap_or(0);
                    message_count += 1;
                }
                message_cursor = messages.next_cursor;
                if message_cursor.is_none() {
                    break;
                }
            }
        }
        session_cursor = sessions.next_cursor;
        if session_cursor.is_none() {
            break;
        }
    }

    Ok(Json(serde_json::json!({
        "total_input_tokens": total_input,
        "total_output_tokens": total_output,
        "total_tokens": total_input + total_output,
        "session_count": session_count,
        "message_count": message_count,
        "model": state.runtime.model,
    })))
}

pub(crate) use handle_usage as handle_admin_usage;
