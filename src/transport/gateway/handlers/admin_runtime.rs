use std::sync::atomic::Ordering;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

use crate::runtime::diagnostics::control_plane_read_models::{
    RuntimeCapabilitiesReadModel, RuntimeCapabilityDetailReadModel, RuntimeStatusReadModel,
};
use crate::runtime::services::{
    RuntimeStatusSnapshot, load_admin_mood, load_admin_runtime_status,
    load_runtime_operational_snapshot, request_gateway_restart,
};

use super::super::{AppState, MAX_WS_CONNECTIONS};
use super::{request_management_policy_context, require_management_principal};

pub(crate) async fn handle_runtime(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<RuntimeStatusReadModel>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;
    let policy_context = request_management_policy_context(&state, &headers)?;

    let operational = load_runtime_operational_snapshot(state.runtime.config.as_ref());
    let session_db_available = state.runtime.session_manager.is_some();
    let memory_backend = state.runtime.mem.name().to_string();
    let memory_review_available =
        operational.memory_review.is_supported() && state.runtime.mem.list_entities().await.is_ok();

    let ws_connections = state
        .connections
        .active_ws_connections
        .load(Ordering::Relaxed);

    Ok(Json(load_admin_runtime_status(RuntimeStatusSnapshot {
        status: if operational.session_persistence.is_runtime_required() && !session_db_available {
            "degraded".to_string()
        } else {
            "ok".to_string()
        },
        memory_backend,
        persistence_status: if session_db_available {
            "connected".to_string()
        } else {
            operational.session_persistence.status.as_str().to_string()
        },
        model: state.runtime.model.clone(),
        ws_connections,
        max_ws_connections: MAX_WS_CONNECTIONS,
        capabilities: RuntimeCapabilitiesReadModel {
            companion: true,
            governance: true,
            memory_review: memory_review_available,
            channel_posture: true,
            session_review: operational.session_persistence.is_supported() && session_db_available,
            a2a: true,
            multi_tenant: policy_context.tenant_mode_enabled,
        },
        capability_details: runtime_capability_details(
            &operational,
            memory_review_available,
            session_db_available,
            policy_context.tenant_mode_enabled,
        ),
    })))
}

fn runtime_capability_details(
    operational: &crate::runtime::services::RuntimeOperationalSnapshot,
    memory_review_available: bool,
    session_db_available: bool,
    multi_tenant_enabled: bool,
) -> Vec<RuntimeCapabilityDetailReadModel> {
    let mut details = vec![
        capability_detail("observability", &operational.observability),
        capability_detail("session_review", &operational.session_persistence),
        capability_detail("memory_review", &operational.memory_review),
        capability_detail("memory_signal_metrics", &operational.memory_signal_metrics),
        capability_detail("persona_state_metrics", &operational.persona_state_metrics),
        capability_detail("cron", &operational.cron),
        RuntimeCapabilityDetailReadModel {
            name: "multi_tenant".to_string(),
            status: if multi_tenant_enabled {
                "supported".to_string()
            } else {
                "unsupported".to_string()
            },
            reason: (!multi_tenant_enabled)
                .then(|| "tenant mode disabled for this request".to_string()),
        },
    ];
    if !session_db_available && operational.session_persistence.is_runtime_required() {
        details.push(RuntimeCapabilityDetailReadModel {
            name: "session_persistence".to_string(),
            status: "degraded".to_string(),
            reason: Some("session manager is unavailable".to_string()),
        });
    }
    if !memory_review_available && operational.memory_review.is_supported() {
        details.push(RuntimeCapabilityDetailReadModel {
            name: "memory_review".to_string(),
            status: "degraded".to_string(),
            reason: Some("memory backend entity listing failed".to_string()),
        });
    }
    details
}

fn capability_detail(
    name: &str,
    state: &crate::runtime::services::RuntimeCapabilityState,
) -> RuntimeCapabilityDetailReadModel {
    RuntimeCapabilityDetailReadModel {
        name: name.to_string(),
        status: state.status.as_str().to_string(),
        reason: state.reason.clone(),
    }
}

/// `POST /admin/v1/gateway/restart` — request a gateway restart.
pub(crate) async fn handle_gateway_restart(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let response = request_gateway_restart();
    tracing::info!(status = %response.status, "admin: gateway restart requested");

    serde_json::to_value(response).map(Json).map_err(|error| {
        tracing::error!(%error, "failed to serialize gateway restart response");
        super::super::problem_details::problem_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialization_error",
            "Internal Server Error",
            "Failed to serialize response.".to_string(),
        )
    })
}

/// `GET /admin/v1/mood` — current agent affect/mood state.
pub(crate) async fn handle_admin_mood(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    serde_json::to_value(load_admin_mood())
        .map(Json)
        .map_err(|error| {
            tracing::error!(%error, "failed to serialize mood response");
            super::super::problem_details::problem_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "serialization_error",
                "Internal Server Error",
                "Failed to serialize response.".to_string(),
            )
        })
}

pub(crate) use handle_gateway_restart as handle_admin_gateway_restart;
pub(crate) use handle_runtime as handle_admin_runtime;
