use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;

use super::super::AppState;
use super::super::events::ServerMessage;
use super::super::problem_details::problem_response;
use super::super::ws_events::RuntimeUpdatedPayload;
use super::companion_helpers::publish_gateway_event;
use super::require_management_principal;
use crate::runtime::services::{
    AdminStateError, load_admin_runtime_config_snapshot, save_admin_runtime_config,
    set_admin_provider_auth_profile, set_admin_provider_default_model, set_admin_provider_enabled,
    update_admin_active_provider_selection,
};
use crate::security::auth::canonical_provider_name;
use crate::utils::http::sync_runtime_http_proxy;

type AdminProblem = (StatusCode, Json<serde_json::Value>);

fn config_load_failed(error: &AdminStateError) -> AdminProblem {
    tracing::error!(%error, "admin: failed to load config");
    problem_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "config_load_failed",
        "Internal Server Error",
        "Failed to load persisted configuration.",
    )
}

fn config_save_failed(error: &AdminStateError) -> AdminProblem {
    tracing::error!(%error, "admin: failed to save config");
    problem_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "config_save_failed",
        "Internal Server Error",
        "Failed to save configuration.",
    )
}

fn auth_profiles_load_failed(error: &AdminStateError) -> AdminProblem {
    tracing::error!(%error, "admin: failed to load auth profiles");
    problem_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "auth_profiles_load_failed",
        "Internal Server Error",
        "Failed to load auth profiles.",
    )
}

fn auth_profiles_save_failed(error: &AdminStateError) -> AdminProblem {
    tracing::error!(%error, "admin: failed to save auth profiles");
    problem_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        "auth_profiles_save_failed",
        "Internal Server Error",
        "Failed to save auth profiles.",
    )
}

fn config_persistence_failed(error: &AdminStateError) -> AdminProblem {
    match error {
        AdminStateError::ConfigLoad { .. } => config_load_failed(error),
        _ => config_save_failed(error),
    }
}

fn auth_profile_persistence_failed(error: &AdminStateError) -> AdminProblem {
    match error {
        AdminStateError::AuthProfilesLoad { .. } => auth_profiles_load_failed(error),
        _ => auth_profiles_save_failed(error),
    }
}

pub(crate) async fn handle_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let config = load_admin_runtime_config_snapshot(&state.runtime.config)
        .map_err(|error| config_load_failed(&error))?;

    Ok(Json(serde_json::json!({
        "workspace_dir": config.workspace_dir.display().to_string(),
        "memory": {
            "backend": config.memory.backend,
            "auto_save": config.memory.auto_save,
        },
        "autonomy": {
            "max_tool_loop_iterations": config.autonomy.max_tool_loop_iterations,
        },
        "gateway": {
            "host": config.gateway.host,
            "port": config.gateway.port,
            "require_pairing": config.gateway.require_pairing,
            "defense_mode": format!(
                "{defense_mode:?}",
                defense_mode = config.gateway.defense_mode
            ),
            "cors_origins": config.gateway.cors_origins,
            "max_body_size_bytes": config.gateway.max_body_size_bytes,
        },
        "session": {
            "parent_fork_max_tokens": config.session.parent_fork_max_tokens,
        },
        "network": {
            "proxy": config.network.proxy,
        },
    })))
}

pub(crate) async fn handle_providers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let config = load_admin_runtime_config_snapshot(&state.runtime.config)
        .map_err(|error| config_load_failed(&error))?;
    let active_provider = config.default_provider.as_deref().unwrap_or("unknown");
    let active_model = config.default_model.as_deref().unwrap_or("unknown");

    Ok(Json(serde_json::json!({
        "active_provider": active_provider,
        "active_model": active_model,
        "temperature": config.default_temperature,
    })))
}

/// `PATCH /admin/v1/providers/{id}` — update provider enabled state, default model, or default auth profile.
///
/// # Errors
///
/// Returns an error when the caller is unauthorized, the provider id is
/// invalid, the referenced auth profile does not exist, or persisted admin
/// state cannot be loaded or saved.
#[derive(serde::Deserialize)]
pub(crate) struct UpdateProviderBody {
    pub enabled: Option<bool>,
    pub default_model: Option<String>,
    pub auth_profile_id: Option<String>,
}

pub(crate) async fn handle_update_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider_id): Path<String>,
    Json(body): Json<UpdateProviderBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let provider_id = validate_provider_id(&provider_id)?;
    let canonical_id = canonical_provider_name(&provider_id);
    let changes = apply_provider_update_changes(&state, &provider_id, &canonical_id, &body)?;

    Ok(build_provider_update_response(&provider_id, &changes))
}

fn validate_provider_id(
    provider_id: &str,
) -> Result<String, (StatusCode, Json<serde_json::Value>)> {
    let provider_id = provider_id.trim().to_string();
    if provider_id.is_empty() {
        return Err(problem_response(
            StatusCode::BAD_REQUEST,
            "missing_provider",
            "Bad Request",
            "Provider id cannot be empty.".to_string(),
        ));
    }
    Ok(provider_id)
}

fn apply_provider_update_changes(
    state: &AppState,
    provider_id: &str,
    canonical_id: &str,
    body: &UpdateProviderBody,
) -> Result<Vec<&'static str>, (StatusCode, Json<serde_json::Value>)> {
    let mut changes = Vec::new();

    if let Some(enabled) = body.enabled {
        apply_provider_enabled_change(state, canonical_id, enabled)?;
        changes.push("enabled");
    }

    if let Some(model) = body.default_model.as_deref()
        && apply_provider_default_model_change(state, provider_id, model)?
    {
        changes.push("default_model");
    }

    if let Some(profile_id) = body.auth_profile_id.as_deref()
        && apply_provider_auth_profile_change(state, profile_id)?
    {
        changes.push("auth_profile_id");
    }

    Ok(changes)
}

fn build_provider_update_response(
    provider_id: &str,
    changes: &[&'static str],
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "provider": provider_id,
        "changes": changes,
        "status": "updated",
    }))
}

fn apply_provider_enabled_change(
    state: &AppState,
    canonical_id: &str,
    enabled: bool,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    set_admin_provider_enabled(&state.runtime.config, canonical_id, enabled)
        .map_err(|error| auth_profile_persistence_failed(&error))
}

fn apply_provider_default_model_change(
    state: &AppState,
    canonical_id: &str,
    model: &str,
) -> Result<bool, (StatusCode, Json<serde_json::Value>)> {
    set_admin_provider_default_model(&state.runtime.config, canonical_id, model)
        .map_err(|error| config_persistence_failed(&error))
}

fn apply_provider_auth_profile_change(
    state: &AppState,
    profile_id: &str,
) -> Result<bool, (StatusCode, Json<serde_json::Value>)> {
    let updated = set_admin_provider_auth_profile(&state.runtime.config, profile_id)
        .map_err(|error| auth_profile_persistence_failed(&error))?;

    if !updated {
        return Err(problem_response(
            StatusCode::NOT_FOUND,
            "auth_profile_not_found",
            "Not Found",
            format!("Auth profile '{profile_id}' not found."),
        ));
    }

    Ok(true)
}

/// `PATCH /admin/v1/settings` — update persisted gateway or network configuration.
///
/// # Errors
///
/// Returns an error when the caller is unauthorized or the updated runtime
/// configuration cannot be saved.
#[derive(serde::Deserialize)]
pub(crate) struct UpdateSettingsBody {
    pub network: Option<NetworkPatch>,
    pub gateway: Option<GatewayPatch>,
}

#[derive(serde::Deserialize)]
pub(crate) struct NetworkPatch {
    pub proxy: Option<String>,
}

#[derive(serde::Deserialize)]
pub(crate) struct GatewayPatch {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub max_body_size_bytes: Option<usize>,
}

pub(crate) async fn handle_update_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpdateSettingsBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let mut config = load_admin_runtime_config_snapshot(&state.runtime.config)
        .map_err(|error| config_load_failed(&error))?;
    let mut changes: Vec<&'static str> = Vec::new();

    if let Some(gateway) = &body.gateway {
        if let Some(host) = &gateway.host {
            let host = host.trim();
            if !host.is_empty() {
                config.gateway.host = host.to_string();
                changes.push("gateway.host");
            }
        }
        if let Some(port) = gateway.port {
            config.gateway.port = port;
            changes.push("gateway.port");
        }
        if let Some(max_body_size_bytes) = gateway.max_body_size_bytes {
            if max_body_size_bytes == 0 {
                return Err(problem_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_max_body_size",
                    "Bad Request",
                    "max_body_size_bytes must be greater than 0".to_string(),
                ));
            }
            config.gateway.max_body_size_bytes = max_body_size_bytes;
            changes.push("gateway.max_body_size_bytes");
        }
    }

    if let Some(network) = &body.network {
        let proxy = network
            .proxy
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        if config.network.proxy != proxy {
            config.network.proxy = proxy;
            changes.push("network.proxy");
        }
    }

    config.validate_autonomy_controls().map_err(|error| {
        problem_response(
            StatusCode::BAD_REQUEST,
            "invalid_settings_patch",
            "Bad Request",
            error.to_string(),
        )
    })?;

    if !changes.is_empty() {
        save_admin_runtime_config(&config).map_err(|error| config_save_failed(&error))?;
    }

    if body.network.is_some() {
        sync_runtime_http_proxy(config.network.proxy.as_deref()).map_err(|error| {
            problem_response(
                StatusCode::BAD_REQUEST,
                "invalid_network_proxy",
                "Bad Request",
                format!("Failed to apply network.proxy: {error}"),
            )
        })?;
    }

    let apply_mode = if changes.iter().any(|change| change.starts_with("network.")) {
        if state.runtime.config.runtime.enable_live_settings_reload {
            publish_gateway_event(
                &state,
                ServerMessage::runtime_updated(RuntimeUpdatedPayload {
                    component: "network".to_string(),
                    status: "updated".to_string(),
                    detail: Some(
                        "Network client policy updated for new outbound requests; daemon live-reload will roll long-lived services."
                            .to_string(),
                    ),
                }),
            );
            "daemon_live_reload"
        } else {
            "restart_required"
        }
    } else if state.runtime.config.runtime.enable_live_settings_reload {
        "daemon_live_reload"
    } else {
        "restart_required"
    };

    Ok(Json(serde_json::json!({
        "changes": changes,
        "status": "updated",
        "apply_mode": apply_mode,
    })))
}

/// `PATCH /admin/v1/providers` — update active provider and model (no provider ID in URL).
#[derive(serde::Deserialize)]
pub(crate) struct UpdateActiveProviderBody {
    pub active_provider: Option<String>,
    pub active_model: Option<String>,
    pub temperature: Option<f64>,
}

pub(crate) async fn handle_update_active_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpdateActiveProviderBody>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    require_management_principal(&state, &headers)?;

    let changes = update_admin_active_provider_selection(
        &state.runtime.config,
        body.active_provider.as_deref(),
        body.active_model.as_deref(),
        body.temperature,
    )
    .map_err(|error| config_persistence_failed(&error))?;

    Ok(Json(serde_json::json!({
        "changes": changes,
        "status": "updated",
    })))
}

pub(crate) use handle_providers as handle_admin_providers;
pub(crate) use handle_settings as handle_admin_settings;
pub(crate) use handle_update_active_provider as handle_admin_providers_update;
pub(crate) use handle_update_provider as handle_admin_provider_update;
pub(crate) use handle_update_settings as handle_admin_settings_update;
