//! Companion surface handlers: caption emission, request-window lifecycle
//! (open/confirm/cancel/get), and widget runtime commands.
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};
use tokio::sync::Mutex;

use super::super::AppState;
use super::super::companion_bridge::{
    CompanionCaptionEvt, CompanionWidgetCommand, CompanionWindow,
};
use super::super::defense::policy_accounting_response;
use super::super::events::ServerMessage;
use super::super::problem_details::problem_response;
use super::companion_helpers::{
    COMPANION_REQUEST_WINDOW_DEFAULT_TTL_SECS, caption_sequence_or_next, companion_replay_scope,
    companion_request_window_replay_scope, companion_scope_key,
    companion_surface_caption_invalid_payload, companion_surface_request_window_invalid_payload,
    companion_surface_request_window_not_found, companion_surface_widget_invalid_payload,
    prune_companion_request_windows, publish_gateway_event,
};
use super::{
    CompanionSurfaceCaptionPayload, CompanionSurfaceRequestWindowOpenPayload,
    enforce_entity_rate_limit, enforce_json_content_type, enforce_request_auth,
    request_policy_context, webhook_replay_ack_response,
};

type CompanionSurfaceJsonResponse = (StatusCode, Json<serde_json::Value>);
type RequestWindowHandle = Arc<Mutex<HashMap<String, CompanionWindow>>>;

/// POST /companion/surface/caption — validate/store caption event for companion surface
pub(in super::super) async fn handle_companion_surface_caption_emit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Some(response) = enforce_request_auth(&state, &headers) {
        return response;
    }
    if let Some(response) = enforce_json_content_type(&headers) {
        return response;
    }

    let policy_context = match request_policy_context(&state, &headers) {
        Ok(context) => context,
        Err(response) => return response,
    };
    let scope_key = companion_scope_key(&policy_context);
    let replay_scope = companion_replay_scope("companion_surface_caption_emit", &scope_key);

    if !state
        .companion
        .replay_guard
        .check_and_record_hash(&replay_scope, &body)
    {
        tracing::warn!("Companion surface caption replay detected");
        return webhook_replay_ack_response();
    }

    let payload = match serde_json::from_slice::<CompanionSurfaceCaptionPayload>(&body) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::debug!(%error, "companion surface caption: JSON parse failed");
            return companion_surface_caption_invalid_payload(
                "Invalid JSON payload. Expected companion surface caption format".to_string(),
            );
        }
    };

    if let Err(policy_error) = state.runtime.security.consume_action_cost(0) {
        return policy_accounting_response(policy_error);
    }
    if let Some(response) = enforce_entity_rate_limit(&state, &headers, &policy_context) {
        return response;
    }

    let Some(log_handle) = state
        .companion
        .companion_caption_logs
        .get_or_insert_with(
            &scope_key,
            super::super::COMPANION_MAX_SCOPES,
            VecDeque::new,
        )
        .await
    else {
        return problem_response(
            StatusCode::TOO_MANY_REQUESTS,
            "capacity_exceeded",
            "Too Many Requests",
            "Companion caption scope limit exceeded. Try again later.",
        );
    };

    let caption_limit = state
        .companion
        .settings
        .read()
        .await
        .caption_retention_limit;

    let (event, stored_captions) = {
        let mut log = log_handle.lock().await;
        let sequence = caption_sequence_or_next(payload.sequence, &log);
        let event = match CompanionCaptionEvt::new(payload.channel, sequence, payload.text) {
            Ok(event) => event,
            Err(error) => return companion_surface_caption_invalid_payload(error.to_string()),
        };
        log.push_back(event.clone());
        while log.len() > caption_limit {
            log.pop_front();
        }
        (event, log.len())
    };

    publish_gateway_event(
        &state,
        ServerMessage::companion_caption(scope_key.clone(), event.clone()),
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "scope": scope_key,
            "event": event,
            "stored_captions": stored_captions,
        })),
    )
}

/// POST /companion/surface/widget — apply widget runtime command (spawn/update/remove/clear/open)
pub(in super::super) async fn handle_companion_surface_widget_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Some(response) = enforce_request_auth(&state, &headers) {
        return response;
    }
    if let Some(response) = enforce_json_content_type(&headers) {
        return response;
    }

    let policy_context = match request_policy_context(&state, &headers) {
        Ok(context) => context,
        Err(response) => return response,
    };
    let scope_key = companion_scope_key(&policy_context);
    let replay_scope = companion_replay_scope("companion_surface_widget_command", &scope_key);

    if !state
        .companion
        .replay_guard
        .check_and_record_hash(&replay_scope, &body)
    {
        tracing::warn!("Companion surface widget replay detected");
        return webhook_replay_ack_response();
    }

    let command = match serde_json::from_slice::<CompanionWidgetCommand>(&body) {
        Ok(command) => command,
        Err(error) => {
            tracing::debug!(%error, "companion widget command: JSON parse failed");
            return companion_surface_widget_invalid_payload(
                "Invalid JSON payload. Expected companion widget command format".to_string(),
            );
        }
    };

    if let Err(policy_error) = state.runtime.security.consume_action_cost(0) {
        return policy_accounting_response(policy_error);
    }
    if let Some(response) = enforce_entity_rate_limit(&state, &headers, &policy_context) {
        return response;
    }

    let now = chrono::Utc::now();

    let Some(runtime_handle) = state
        .companion
        .companion_widget_runtimes
        .get_or_insert_default(&scope_key, super::super::COMPANION_MAX_SCOPES)
        .await
    else {
        return problem_response(
            StatusCode::TOO_MANY_REQUESTS,
            "capacity_exceeded",
            "Too Many Requests",
            "Companion widget scope limit exceeded. Try again later.",
        );
    };
    let (result, widgets) = {
        let mut runtime = runtime_handle.lock().await;
        runtime.expire(now);
        let result = match runtime.apply(command, now) {
            Ok(result) => result,
            Err(error) => return companion_surface_widget_invalid_payload(error.to_string()),
        };
        let widgets = runtime.snapshot();
        (result, widgets)
    };

    publish_gateway_event(
        &state,
        ServerMessage::companion_widget(scope_key.clone(), result.clone(), widgets.clone()),
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "scope": scope_key,
            "result": result,
            "widgets": widgets,
        })),
    )
}

async fn get_or_create_request_window_handle(
    state: &AppState,
    scope_key: &str,
) -> Result<RequestWindowHandle, CompanionSurfaceJsonResponse> {
    state
        .companion
        .companion_request_windows
        .get_or_insert_with(scope_key, super::super::COMPANION_MAX_SCOPES, HashMap::new)
        .await
        .ok_or_else(|| {
            problem_response(
                StatusCode::TOO_MANY_REQUESTS,
                "capacity_exceeded",
                "Too Many Requests",
                "Companion request-window scope limit exceeded. Try again later.",
            )
        })
}

async fn get_request_window_handle(
    state: &AppState,
    scope_key: &str,
    window_id: &str,
) -> Result<RequestWindowHandle, CompanionSurfaceJsonResponse> {
    state
        .companion
        .companion_request_windows
        .get_scope(scope_key)
        .await
        .ok_or_else(|| companion_surface_request_window_not_found(window_id))
}

async fn store_request_window(
    windows_handle: &RequestWindowHandle,
    now: chrono::DateTime<chrono::Utc>,
    window: CompanionWindow,
) {
    let mut windows = windows_handle.lock().await;
    prune_companion_request_windows(&mut windows, now);
    windows.insert(window.window_id.clone(), window);
    prune_companion_request_windows(&mut windows, now);
}

async fn mutate_request_window<F>(
    windows_handle: &RequestWindowHandle,
    window_id: &str,
    now: chrono::DateTime<chrono::Utc>,
    mutate: F,
) -> Result<CompanionWindow, CompanionSurfaceJsonResponse>
where
    F: FnOnce(&mut CompanionWindow, chrono::DateTime<chrono::Utc>) -> Result<(), String>,
{
    let mut windows = windows_handle.lock().await;
    prune_companion_request_windows(&mut windows, now);
    let Some(window) = windows.get_mut(window_id) else {
        return Err(companion_surface_request_window_not_found(window_id));
    };
    mutate(window, now).map_err(companion_surface_request_window_invalid_payload)?;
    Ok(window.clone())
}

/// POST /companion/surface/request-window/open — open operator-safe request-window
pub(in super::super) async fn handle_companion_surface_request_window_open(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Some(response) = enforce_request_auth(&state, &headers) {
        return response;
    }
    if let Some(response) = enforce_json_content_type(&headers) {
        return response;
    }

    let policy_context = match request_policy_context(&state, &headers) {
        Ok(context) => context,
        Err(response) => return response,
    };
    let scope_key = companion_scope_key(&policy_context);
    let replay_scope = companion_replay_scope("companion_surface_request_window_open", &scope_key);

    if !state
        .companion
        .replay_guard
        .check_and_record_hash(&replay_scope, &body)
    {
        tracing::warn!("Companion request-window open replay detected");
        return webhook_replay_ack_response();
    }

    let payload = match serde_json::from_slice::<CompanionSurfaceRequestWindowOpenPayload>(&body) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::debug!(%error, "companion request-window: JSON parse failed");
            return companion_surface_request_window_invalid_payload(
                "Invalid JSON payload. Expected companion request-window format".to_string(),
            );
        }
    };

    if let Err(policy_error) = state.runtime.security.consume_action_cost(0) {
        return policy_accounting_response(policy_error);
    }
    if let Some(response) = enforce_entity_rate_limit(&state, &headers, &policy_context) {
        return response;
    }

    let now = chrono::Utc::now();
    let ttl_secs = payload
        .ttl_secs
        .unwrap_or(COMPANION_REQUEST_WINDOW_DEFAULT_TTL_SECS);

    let window = match CompanionWindow::new(payload.requested_action, now, ttl_secs) {
        Ok(window) => window,
        Err(error) => return companion_surface_request_window_invalid_payload(error.to_string()),
    };

    let windows_handle = match get_or_create_request_window_handle(&state, &scope_key).await {
        Ok(handle) => handle,
        Err(response) => return response,
    };
    store_request_window(&windows_handle, now, window.clone()).await;

    publish_gateway_event(
        &state,
        ServerMessage::companion_request_window(scope_key.clone(), "opened", window.clone()),
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "scope": scope_key,
            "window": window,
        })),
    )
}

/// GET /companion/surface/request-window/{window_id} — inspect request-window state
pub(in super::super) async fn handle_companion_surface_request_window_get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(window_id): Path<String>,
) -> impl IntoResponse {
    if let Some(response) = enforce_request_auth(&state, &headers) {
        return response;
    }

    let policy_context = match request_policy_context(&state, &headers) {
        Ok(context) => context,
        Err(response) => return response,
    };
    let scope_key = companion_scope_key(&policy_context);
    let windows_handle = match get_request_window_handle(&state, &scope_key, &window_id).await {
        Ok(handle) => handle,
        Err(response) => return response,
    };

    let now = chrono::Utc::now();
    let window = match mutate_request_window(&windows_handle, &window_id, now, |window, now| {
        window
            .refresh_expiry(now)
            .map_err(|error| error.to_string())
    })
    .await
    {
        Ok(window) => window,
        Err(response) => return response,
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "scope": scope_key,
            "window": window,
        })),
    )
}

/// POST /companion/surface/request-window/{window_id}/confirm — confirm request-window
pub(in super::super) async fn handle_companion_surface_request_window_confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(window_id): Path<String>,
    body: Bytes,
) -> impl IntoResponse {
    if let Some(response) = enforce_request_auth(&state, &headers) {
        return response;
    }
    if let Some(response) = enforce_json_content_type(&headers) {
        return response;
    }

    let policy_context = match request_policy_context(&state, &headers) {
        Ok(context) => context,
        Err(response) => return response,
    };
    let scope_key = companion_scope_key(&policy_context);
    let replay_scope = companion_request_window_replay_scope(
        "companion_surface_request_window_confirm",
        &scope_key,
        &window_id,
    );
    let windows_handle = match get_request_window_handle(&state, &scope_key, &window_id).await {
        Ok(handle) => handle,
        Err(response) => return response,
    };

    if !state
        .companion
        .replay_guard
        .check_and_record_hash(&replay_scope, &body)
    {
        tracing::warn!("Companion request-window confirm replay detected");
        return webhook_replay_ack_response();
    }

    if let Err(policy_error) = state.runtime.security.consume_action_cost(0) {
        return policy_accounting_response(policy_error);
    }
    if let Some(response) = enforce_entity_rate_limit(&state, &headers, &policy_context) {
        return response;
    }

    let now = chrono::Utc::now();
    let window = match mutate_request_window(&windows_handle, &window_id, now, |window, now| {
        window.confirm(now).map_err(|error| error.to_string())
    })
    .await
    {
        Ok(window) => window,
        Err(response) => return response,
    };

    publish_gateway_event(
        &state,
        ServerMessage::companion_request_window(scope_key.clone(), "confirmed", window.clone()),
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "scope": scope_key,
            "window": window,
        })),
    )
}

/// POST /companion/surface/request-window/{window_id}/cancel — cancel request-window
pub(in super::super) async fn handle_companion_surface_request_window_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(window_id): Path<String>,
    body: Bytes,
) -> impl IntoResponse {
    if let Some(response) = enforce_request_auth(&state, &headers) {
        return response;
    }
    if let Some(response) = enforce_json_content_type(&headers) {
        return response;
    }

    let policy_context = match request_policy_context(&state, &headers) {
        Ok(context) => context,
        Err(response) => return response,
    };
    let scope_key = companion_scope_key(&policy_context);
    let replay_scope = companion_request_window_replay_scope(
        "companion_surface_request_window_cancel",
        &scope_key,
        &window_id,
    );
    let windows_handle = match get_request_window_handle(&state, &scope_key, &window_id).await {
        Ok(handle) => handle,
        Err(response) => return response,
    };

    if !state
        .companion
        .replay_guard
        .check_and_record_hash(&replay_scope, &body)
    {
        tracing::warn!("Companion request-window cancel replay detected");
        return webhook_replay_ack_response();
    }

    if let Err(policy_error) = state.runtime.security.consume_action_cost(0) {
        return policy_accounting_response(policy_error);
    }
    if let Some(response) = enforce_entity_rate_limit(&state, &headers, &policy_context) {
        return response;
    }

    let now = chrono::Utc::now();
    let window = match mutate_request_window(&windows_handle, &window_id, now, |window, _| {
        window.cancel().map_err(|error| error.to_string())
    })
    .await
    {
        Ok(window) => window,
        Err(response) => return response,
    };

    publish_gateway_event(
        &state,
        ServerMessage::companion_request_window(scope_key.clone(), "cancelled", window.clone()),
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "scope": scope_key,
            "window": window,
        })),
    )
}
