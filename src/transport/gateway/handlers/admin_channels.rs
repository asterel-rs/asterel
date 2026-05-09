use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

use super::super::AppState;
use super::super::events::ServerMessage;
use super::super::problem_details::problem_response;
use super::super::ws_events::ChannelUpdatedPayload;
use super::companion_helpers::publish_gateway_event;
use super::require_management_principal;
use crate::contracts::ids::ChannelId;
use crate::runtime::services::{
    create_admin_channel, list_admin_channels, run_admin_channel_action, update_admin_channel,
};

fn channel_problem(
    code: &'static str,
    error: &anyhow::Error,
) -> (StatusCode, Json<serde_json::Value>) {
    let detail = error.to_string();
    let status = if detail.contains("not configured") {
        StatusCode::NOT_FOUND
    } else if detail.contains("already configured") {
        StatusCode::CONFLICT
    } else if detail.contains("not supported by this runtime build") {
        StatusCode::NOT_IMPLEMENTED
    } else if detail.contains("unknown channel")
        || detail.contains("requires a config payload")
        || detail.contains("unsupported channel action")
        || detail.contains("does not accept config payloads")
        || detail.contains("is not managed through admin channel mutations")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    problem_response(
        status,
        code,
        status.canonical_reason().unwrap_or("Error"),
        detail,
    )
}

fn channel_json(record: &crate::runtime::services::ManagedChannelRecord) -> serde_json::Value {
    serde_json::json!({
        "id": record.id,
        "name": record.display_name,
        "type": record.id,
        "configured": record.configured,
        "enabled": record.enabled,
        "supported": record.supported,
        "runtime_owner": record.owner.as_str(),
    })
}

pub(crate) async fn handle_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let inventory = list_admin_channels(&state.runtime.config).map_err(|error| {
        tracing::error!(%error, "admin: failed to load channel inventory");
        channel_problem("admin_channel_list_failed", &error)
    })?;
    let items = inventory.items.iter().map(channel_json).collect::<Vec<_>>();

    Ok(Json(serde_json::json!({
        "items": items,
        "active_names": inventory.active_names,
        "high_freedom": inventory.high_freedom,
    })))
}

#[derive(serde::Deserialize)]
pub(crate) struct ChannelCreateBody {
    #[serde(rename = "type")]
    pub channel_type: String,
    pub name: String,
    pub config: Option<serde_json::Value>,
}

pub(crate) async fn handle_create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ChannelCreateBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let ChannelCreateBody {
        channel_type,
        name,
        config,
    } = body;
    let result = create_admin_channel(&state.runtime.config, &channel_type, config).map_err(
        |error| {
            tracing::error!(%error, channel_type = %channel_type, "admin: failed to create channel");
            channel_problem("admin_channel_create_failed", &error)
        },
    )?;
    publish_gateway_event(
        &state,
        ServerMessage::channel_updated(ChannelUpdatedPayload {
            channel_id: ChannelId::new(result.record.id.clone()),
            channel_type: result.record.id.clone(),
            status: "updated".to_string(),
            detail: Some(format!(
                "configured '{}' via admin; apply mode: {}; reload_requested={}",
                name,
                result.apply_mode.as_str(),
                result.reload_requested
            )),
        }),
    );

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "status": "created",
            "channel": channel_json(&result.record),
            "changes": result.changes,
            "apply_mode": result.apply_mode.as_str(),
            "reload_requested": result.reload_requested,
        })),
    ))
}

#[derive(serde::Deserialize)]
pub(crate) struct ChannelUpdateBody {
    pub enabled: Option<bool>,
    pub config: Option<serde_json::Value>,
}

pub(crate) async fn handle_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<String>,
    Json(body): Json<ChannelUpdateBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let ChannelUpdateBody { enabled, config } = body;
    let result = update_admin_channel(&state.runtime.config, &channel_id, enabled, config)
        .map_err(|error| {
            tracing::error!(%error, %channel_id, "admin: failed to update channel");
            channel_problem("admin_channel_update_failed", &error)
        })?;
    publish_gateway_event(
        &state,
        ServerMessage::channel_updated(ChannelUpdatedPayload {
            channel_id: ChannelId::new(result.record.id.clone()),
            channel_type: result.record.id.clone(),
            status: "updated".to_string(),
            detail: Some(format!(
                "apply mode: {}; reload_requested={}",
                result.apply_mode.as_str(),
                result.reload_requested
            )),
        }),
    );

    Ok(Json(serde_json::json!({
        "status": "updated",
        "channel": channel_json(&result.record),
        "changes": result.changes,
        "apply_mode": result.apply_mode.as_str(),
        "reload_requested": result.reload_requested,
    })))
}

#[derive(serde::Deserialize)]
pub(crate) struct ChannelActionBody {
    pub action: String,
}

pub(crate) async fn handle_action(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(channel_id): Path<String>,
    Json(body): Json<ChannelActionBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let ChannelActionBody { action } = body;
    let result = run_admin_channel_action(&state.runtime.config, &channel_id, &action)
        .await
        .map_err(|error| {
            tracing::error!(%error, %channel_id, action = %action, "admin: failed to run channel action");
            channel_problem("admin_channel_action_failed", &error)
        })?;
    publish_gateway_event(
        &state,
        ServerMessage::channel_updated(ChannelUpdatedPayload {
            channel_id: ChannelId::new(result.record.id.clone()),
            channel_type: result.record.id.clone(),
            status: result.status.clone(),
            detail: result.detail.clone(),
        }),
    );

    Ok(Json(serde_json::json!({
        "status": result.status,
        "action": result.action,
        "channel": channel_json(&result.record),
        "detail": result.detail,
        "apply_mode": result.apply_mode.map(crate::runtime::services::RuntimeApplyMode::as_str),
        "reload_requested": result.reload_requested,
    })))
}

pub(crate) use handle_action as handle_admin_channels_action;
pub(crate) use handle_create as handle_admin_channels_create;
pub(crate) use handle_list as handle_admin_channels_list;
pub(crate) use handle_update as handle_admin_channels_update;
