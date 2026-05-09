//! Admin session management handlers.
//!
//! Routes:
//! - `GET    /admin/v1/sessions`               — list with cursor + tenant filter
//! - `POST   /admin/v1/sessions`               — create session
//! - `GET    /admin/v1/sessions/{id}`          — session detail
//! - `DELETE /admin/v1/sessions/{id}`          — delete session
//! - `GET    /admin/v1/sessions/{id}/messages` — messages with cursor pagination
//! - `POST   /admin/v1/sessions/{id}/messages` — append a message

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

use crate::contracts::ids::SessionId;
use crate::core::sessions::types::MessageRole;
use crate::runtime::services::{
    OperatorScope, OperatorSessionAccess, append_operator_session_message,
    build_operator_session_message_read_model, create_operator_session, delete_operator_session,
    list_operator_session_messages, list_operator_sessions, load_operator_session,
};

use super::super::AppState;
use super::super::problem_details::problem_response;
use super::{request_management_policy_context, require_management_principal};

#[derive(serde::Deserialize)]
pub(crate) struct SessionListQuery {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
    pub tenant_id: Option<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct SessionMessagesQuery {
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

#[derive(serde::Deserialize)]
pub(crate) struct CreateSessionBody {
    pub title: Option<String>,
    pub tenant_id: Option<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct MessagePart {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub text: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct CreateMessageBody {
    pub parts: Vec<MessagePart>,
}

pub(crate) async fn handle_sessions_list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SessionListQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let scope = require_management_scope(&state, &headers)?;
    let tenant_scope = require_scoped_tenant(&scope)?;

    if query
        .tenant_id
        .as_deref()
        .is_some_and(|tenant_id| tenant_id != tenant_scope)
    {
        return Err(tenant_scope_mismatch_response());
    }

    let session_manager = require_session_manager(&state)?;
    let response = list_operator_sessions(
        session_manager,
        &scope,
        query.cursor.as_deref(),
        query.limit,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "admin: failed to list sessions");
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "session_list_failed",
            "Internal Server Error",
            "Failed to list sessions.".to_string(),
        )
    })?;

    Ok(Json(serde_json::to_value(response).map_err(|error| {
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialization_error",
            "Internal Server Error",
            error.to_string(),
        )
    })?))
}

pub(crate) async fn handle_session_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<CreateSessionBody>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let scope = require_management_scope(&state, &headers)?;
    let tenant_scope = require_scoped_tenant(&scope)?;

    let body = body.map_or(
        CreateSessionBody {
            title: None,
            tenant_id: None,
        },
        |Json(b)| b,
    );

    if body
        .tenant_id
        .as_deref()
        .is_some_and(|tenant_id| tenant_id != tenant_scope)
    {
        return Err(tenant_scope_mismatch_response());
    }

    let session_manager = require_session_manager(&state)?;

    let session = create_operator_session(session_manager, &scope, body.title.as_deref())
        .await
        .map_err(|error| {
            tracing::error!(%error, "admin: failed to create session");
            problem_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "session_create_failed",
                "Internal Server Error",
                "Failed to create session.".to_string(),
            )
        })?;

    Ok(Json(serde_json::to_value(session).map_err(|error| {
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialization_error",
            "Internal Server Error",
            error.to_string(),
        )
    })?))
}

pub(crate) async fn handle_session_get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let scope = require_management_scope(&state, &headers)?;

    let session_manager = require_session_manager(&state)?;
    let session = match load_operator_session(session_manager, &scope, &SessionId::new(&session_id))
        .await
        .map_err(|error| {
            tracing::error!(%error, %session_id, "admin: failed to fetch session");
            problem_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "session_fetch_failed",
                "Internal Server Error",
                "Failed to fetch session.".to_string(),
            )
        })? {
        OperatorSessionAccess::Visible(session) => session,
        OperatorSessionAccess::NotFound => return Err(session_not_found_response(&session_id)),
        OperatorSessionAccess::ScopeDenied => {
            return Err(session_scope_denied_response(&session_id));
        }
    };

    Ok(Json(serde_json::to_value(session).map_err(|error| {
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialization_error",
            "Internal Server Error",
            error.to_string(),
        )
    })?))
}

pub(crate) async fn handle_session_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<serde_json::Value>)> {
    let scope = require_management_scope(&state, &headers)?;

    let session_manager = require_session_manager(&state)?;
    let deleted = delete_operator_session(session_manager, &scope, &session_id)
        .await
        .map_err(|error| {
            tracing::error!(%error, %session_id, "admin: failed to delete session");
            problem_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "session_delete_failed",
                "Internal Server Error",
                "Failed to delete session.".to_string(),
            )
        })?;

    match deleted {
        OperatorSessionAccess::Visible(()) => Ok(StatusCode::NO_CONTENT),
        OperatorSessionAccess::NotFound => Err(session_not_found_response(&session_id)),
        OperatorSessionAccess::ScopeDenied => Err(session_scope_denied_response(&session_id)),
    }
}

pub(crate) async fn handle_session_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Query(query): Query<SessionMessagesQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let scope = require_management_scope(&state, &headers)?;

    let session_manager = require_session_manager(&state)?;
    let sid = SessionId::new(session_id);
    let response = match list_operator_session_messages(
        session_manager,
        &scope,
        &sid,
        query.cursor.as_deref(),
        query.limit,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, session_id = %sid, "admin: failed to get session messages");
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "session_messages_failed",
            "Internal Server Error",
            "Failed to retrieve session messages.".to_string(),
        )
    })? {
        OperatorSessionAccess::Visible(response) => response,
        OperatorSessionAccess::NotFound => return Err(session_not_found_response(sid.as_str())),
        OperatorSessionAccess::ScopeDenied => {
            return Err(session_scope_denied_response(sid.as_str()));
        }
    };

    Ok(Json(serde_json::to_value(response).map_err(|error| {
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialization_error",
            "Internal Server Error",
            error.to_string(),
        )
    })?))
}

pub(crate) async fn handle_session_message_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
    Json(body): Json<CreateMessageBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    let scope = require_management_scope(&state, &headers)?;

    if body.parts.is_empty() {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "empty_parts",
            "Bad Request",
            "Message must contain at least one part.".to_string(),
        ));
    }

    if let Some(kind) = body
        .parts
        .iter()
        .find_map(|part| part.kind.as_deref().filter(|kind| *kind != "text"))
    {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "unsupported_message_part",
            "Bad Request",
            format!("Unsupported message part type '{kind}'. Only text parts are accepted."),
        ));
    }

    let session_manager = require_session_manager(&state)?;
    let mut content = String::new();
    for part in &body.parts {
        if !content.is_empty() {
            content.push('\n');
        }
        content.push_str(&part.text);
    }

    let message = match append_operator_session_message(
        session_manager,
        &scope,
        &session_id,
        MessageRole::User,
        &content,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, session_id = %session_id, "admin: failed to append message");
        problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "message_append_failed",
            "Internal Server Error",
            "Failed to append message.".to_string(),
        )
    })? {
        OperatorSessionAccess::Visible(message) => message,
        OperatorSessionAccess::NotFound => return Err(session_not_found_response(&session_id)),
        OperatorSessionAccess::ScopeDenied => {
            return Err(session_scope_denied_response(&session_id));
        }
    };

    Ok(Json(
        serde_json::to_value(build_operator_session_message_read_model(&message)).map_err(
            |error| {
                problem_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "serialization_error",
                    "Internal Server Error",
                    error.to_string(),
                )
            },
        )?,
    ))
}

pub(crate) use handle_session_create as handle_admin_session_create;
pub(crate) use handle_session_delete as handle_admin_session_delete;
pub(crate) use handle_session_get as handle_admin_session_get;
pub(crate) use handle_session_message_create as handle_admin_session_message_create;
pub(crate) use handle_session_messages as handle_admin_session_messages;
pub(crate) use handle_sessions_list as handle_admin_sessions_list;

fn require_management_scope(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<OperatorScope, (StatusCode, Json<serde_json::Value>)> {
    let principal = require_management_principal(state, headers)?;
    let policy_context = request_management_policy_context(state, headers)?;
    if policy_context.tenant_id.is_none() {
        return Err(problem_response(
            StatusCode::FORBIDDEN,
            "tenant_scope_required",
            "Forbidden",
            "Admin endpoints require a tenant-scoped paired bearer token.",
        ));
    }

    Ok(OperatorScope::from_management_context(
        principal,
        policy_context,
    ))
}

fn tenant_scope_mismatch_response() -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::FORBIDDEN,
        "tenant_scope_mismatch",
        "Forbidden",
        "Requested tenant does not match the caller tenant scope.",
    )
}

fn session_not_found_response(session_id: &str) -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::NOT_FOUND,
        "session_not_found",
        "Not Found",
        format!("Session {session_id} not found."),
    )
}

fn session_scope_denied_response(session_id: &str) -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::FORBIDDEN,
        "session_scope_denied",
        "Forbidden",
        format!("Session {session_id} is outside the current operator scope."),
    )
}

fn require_scoped_tenant(
    scope: &OperatorScope,
) -> Result<&str, (StatusCode, Json<serde_json::Value>)> {
    scope.require_tenant_id().ok_or_else(|| {
        problem_response(
            StatusCode::FORBIDDEN,
            "tenant_scope_required",
            "Forbidden",
            "Admin endpoints require a tenant-scoped paired bearer token.",
        )
    })
}

fn require_session_manager(
    state: &AppState,
) -> Result<&crate::core::sessions::SessionOrchestrator, (StatusCode, Json<serde_json::Value>)> {
    state.runtime.session_manager.as_deref().ok_or_else(|| {
        problem_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "session_store_unavailable",
            "Service Unavailable",
            "Session store is not initialized.".to_string(),
        )
    })
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::{session_not_found_response, session_scope_denied_response};

    #[test]
    fn admin_session_problem_responses_distinguish_missing_from_scope_denied() {
        let missing = session_not_found_response("session-1");
        let denied = session_scope_denied_response("session-1");

        assert_eq!(missing.0, StatusCode::NOT_FOUND);
        assert_eq!(missing.1.0["code"], "session_not_found");
        assert_eq!(denied.0, StatusCode::FORBIDDEN);
        assert_eq!(denied.1.0["code"], "session_scope_denied");
        assert_ne!(missing.1.0["code"], denied.1.0["code"]);
    }
}
