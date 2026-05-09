//! Gateway handler inventory, grouped route builders, and transport-local
//! payload types shared across companion handlers.
mod a2a;
mod a2a_read_models;
pub(super) mod admin_activity;
pub(super) mod admin_agents;
pub(super) mod admin_auth;
pub(super) mod admin_channels;
pub(super) mod admin_companion;
pub(super) mod admin_contract;
pub(super) mod admin_cron;
pub(super) mod admin_governance;
pub mod admin_memory;
pub(super) mod admin_paths;
pub(super) mod admin_runtime;
pub(super) mod admin_sessions;
pub(super) mod admin_settings;
pub(super) mod admin_skills;
pub(super) mod admin_tenants;
pub(super) mod admin_uploads;
pub(super) mod admin_usage;
mod auth_context;
mod companion;
mod companion_helpers;
mod companion_surface;
mod tenant_scope;
pub(super) mod turn_bridge;
mod webhook;
#[cfg(feature = "whatsapp")]
mod whatsapp;

use axum::Router;
use axum::routing::{get, patch, post};
use serde::Deserialize;
use serde_json::Value;

use super::AppState;
use super::companion_bridge::{
    CompanionCaptionChannel, CompanionContextKind, CompanionEmotionalImpact, CompanionMediaKind,
};
use super::websocket::ws_handler;

#[cfg(test)]
pub(super) use a2a::a2a_text_message;
pub(super) use a2a::{
    handle_a2a_message, handle_a2a_task_cancel, handle_a2a_task_get, handle_a2a_tasks_get,
    handle_pair,
};
pub(super) use auth_context::{
    bearer_token, enforce_json_content_type, enforce_json_request_guards, enforce_request_auth,
    external_trust_source_from_headers, hashed_auth_principal, require_management_principal,
    require_paired_bearer_principal, resolve_webhook_source_identifier,
    verified_source_identifier_from_headers,
};
pub(super) use companion::{handle_companion_context_ingest, handle_companion_multimodal_ingest};
pub(super) use companion_surface::{
    handle_companion_surface_caption_emit, handle_companion_surface_request_window_cancel,
    handle_companion_surface_request_window_confirm, handle_companion_surface_request_window_get,
    handle_companion_surface_request_window_open, handle_companion_surface_widget_command,
};
pub(super) use tenant_scope::{
    paired_bearer_principal, request_management_policy_context, request_policy_context,
    request_tenant_id,
};
pub(super) use turn_bridge::{
    enforce_entity_rate_limit, enforce_entity_rate_limit_for_source, gateway_delivery_text,
    gateway_entity_id, gateway_workspace_dir, log_tool_loop_stop, run_tool_loop,
    webhook_replay_ack_response,
};
pub(super) use webhook::{
    handle_agent_card, handle_health, handle_openapi_contract, handle_ready, handle_webhook,
};
#[cfg(feature = "whatsapp")]
pub(super) use whatsapp::{handle_whatsapp_message, handle_whatsapp_verify};

pub(super) fn build_public_routes() -> Router<AppState> {
    Router::new()
        .route("/health", get(handle_health))
        .route("/healthz", get(handle_health))
        .route("/ready", get(handle_ready))
        .route("/readyz", get(handle_ready))
        .route("/openapi/v1.json", get(handle_openapi_contract))
        .route("/.well-known/agent.json", get(handle_agent_card))
        .route("/pair", post(handle_pair))
        .route("/a2a/v1/messages", post(handle_a2a_message))
        .route("/a2a/v1/tasks", get(handle_a2a_tasks_get))
        .route("/a2a/v1/tasks/{task_id}", get(handle_a2a_task_get))
        .route(
            "/a2a/v1/tasks/{task_id}/cancel",
            post(handle_a2a_task_cancel),
        )
        .route("/webhook", post(handle_webhook))
        .route(
            "/companion/context/ingest",
            post(handle_companion_context_ingest),
        )
        .route(
            "/companion/multimodal/ingest",
            post(handle_companion_multimodal_ingest),
        )
        .route(
            "/companion/surface/caption",
            post(handle_companion_surface_caption_emit),
        )
        .route(
            "/companion/surface/widget",
            post(handle_companion_surface_widget_command),
        )
        .route(
            "/companion/surface/request-window/open",
            post(handle_companion_surface_request_window_open),
        )
        .route(
            "/companion/surface/request-window/{window_id}",
            get(handle_companion_surface_request_window_get),
        )
        .route(
            "/companion/surface/request-window/{window_id}/confirm",
            post(handle_companion_surface_request_window_confirm),
        )
        .route(
            "/companion/surface/request-window/{window_id}/cancel",
            post(handle_companion_surface_request_window_cancel),
        )
        .route("/ws", get(ws_handler))
}

pub(super) fn build_admin_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/v1/openapi.json",
            get(admin_contract::handle_admin_openapi_contract),
        )
        .route(
            "/admin/v1/runtime",
            get(admin_runtime::handle_admin_runtime),
        )
        .route("/admin/v1/usage", get(admin_usage::handle_admin_usage))
        .route("/admin/v1/mood", get(admin_runtime::handle_admin_mood))
        .route(
            "/admin/v1/activity",
            get(admin_activity::handle_admin_activity_timeline),
        )
        .route(
            "/admin/v1/agents",
            get(admin_agents::handle_admin_agents_list),
        )
        .route(
            "/admin/v1/gateway/restart",
            post(admin_runtime::handle_admin_gateway_restart),
        )
        .merge(build_admin_session_routes())
        .merge(build_admin_governance_routes())
        .merge(build_admin_auth_routes())
        .merge(build_admin_settings_routes())
        .merge(build_admin_channel_routes())
        .merge(build_admin_skill_routes())
        .merge(build_admin_cron_routes())
        .merge(build_admin_memory_routes())
        .merge(build_admin_companion_routes())
        .merge(build_admin_tenant_routes())
}

fn build_admin_session_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/v1/sessions",
            get(admin_sessions::handle_admin_sessions_list)
                .post(admin_sessions::handle_admin_session_create),
        )
        .route(
            "/admin/v1/sessions/{session_id}",
            get(admin_sessions::handle_admin_session_get)
                .delete(admin_sessions::handle_admin_session_delete),
        )
        .route(
            "/admin/v1/sessions/{session_id}/messages",
            get(admin_sessions::handle_admin_session_messages)
                .post(admin_sessions::handle_admin_session_message_create),
        )
}

fn build_admin_governance_routes() -> Router<AppState> {
    Router::new().route(
        "/admin/v1/governance",
        get(admin_governance::handle_admin_governance_summary),
    )
}

fn build_admin_auth_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/v1/auth/profiles",
            get(admin_auth::handle_admin_auth_profiles),
        )
        .route(
            "/admin/v1/auth/profiles/{id}",
            patch(admin_auth::handle_admin_auth_profile_patch),
        )
}

fn build_admin_settings_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/v1/providers",
            get(admin_settings::handle_admin_providers)
                .patch(admin_settings::handle_admin_providers_update),
        )
        .route(
            "/admin/v1/providers/{id}",
            patch(admin_settings::handle_admin_provider_update),
        )
        .route(
            "/admin/v1/settings",
            get(admin_settings::handle_admin_settings)
                .patch(admin_settings::handle_admin_settings_update),
        )
}

fn build_admin_channel_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/v1/channels",
            get(admin_channels::handle_admin_channels_list)
                .post(admin_channels::handle_admin_channels_create),
        )
        .route(
            "/admin/v1/channels/{channel_id}",
            patch(admin_channels::handle_admin_channels_update),
        )
        .route(
            "/admin/v1/channels/{channel_id}/actions",
            post(admin_channels::handle_admin_channels_action),
        )
}

fn build_admin_skill_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/v1/skills",
            get(admin_skills::handle_admin_skills_list),
        )
        .route(
            "/admin/v1/skills/install",
            post(admin_skills::handle_admin_skills_install),
        )
        .route(
            "/admin/v1/skills/{skill_id}",
            patch(admin_skills::handle_admin_skills_update)
                .delete(admin_skills::handle_admin_skills_remove),
        )
}

fn build_admin_cron_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/v1/cron/jobs",
            get(admin_cron::handle_admin_cron_list).post(admin_cron::handle_admin_cron_create),
        )
        .route(
            "/admin/v1/cron/jobs/{job_id}",
            patch(admin_cron::handle_admin_cron_update)
                .delete(admin_cron::handle_admin_cron_remove),
        )
        .route(
            "/admin/v1/cron/jobs/{job_id}/run",
            post(admin_cron::handle_admin_cron_run),
        )
}

fn build_admin_memory_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/v1/memory/entities",
            get(admin_memory::list_memory_entities),
        )
        .route(
            "/admin/v1/memory/consolidation",
            get(admin_memory::list_memory_consolidation_statuses),
        )
        .route(
            "/admin/v1/memory/exposure",
            get(admin_memory::get_memory_exposure_status),
        )
        .route(
            "/admin/v1/memory/self-amendments",
            get(admin_memory::list_self_amendment_candidates),
        )
        .route(
            "/admin/v1/memory/self-amendments/approve",
            post(admin_memory::approve_self_amendment_candidate_handler),
        )
        .route(
            "/admin/v1/memory/entities/{entity_id}/slots",
            get(admin_memory::list_memory_entity_slots),
        )
        .route(
            "/admin/v1/memory/correct",
            post(admin_memory::correct_memory_slot),
        )
        .route(
            "/admin/v1/memory/forget",
            post(admin_memory::forget_memory_slot),
        )
        .route(
            "/admin/v1/memory/checkpoint",
            post(admin_memory::create_checkpoint),
        )
}

fn build_admin_companion_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/v1/companions",
            get(admin_companion::handle_admin_companions_list)
                .patch(admin_companion::handle_admin_companions_update),
        )
        .route(
            "/admin/v1/companions/{scope}/captions",
            get(admin_companion::handle_admin_companion_captions),
        )
        .route(
            "/admin/v1/companions/{scope}/widgets",
            get(admin_companion::handle_admin_companion_widgets),
        )
        .route(
            "/admin/v1/companions/{scope}/windows",
            get(admin_companion::handle_admin_companion_windows),
        )
        .route(
            "/admin/v1/companions/{scope}/windows/{window_id}/confirm",
            post(admin_companion::handle_admin_companion_window_confirm),
        )
        .route(
            "/admin/v1/companions/{scope}/windows/{window_id}/cancel",
            post(admin_companion::handle_admin_companion_window_cancel),
        )
        .route(
            "/admin/v1/companions/{scope}/ingress",
            post(admin_companion::handle_admin_companion_ingress),
        )
}

fn build_admin_tenant_routes() -> Router<AppState> {
    Router::new()
        .route(
            "/admin/v1/tenants",
            get(admin_tenants::handle_admin_tenants_list),
        )
        .route(
            "/admin/v1/tenant-context",
            get(admin_tenants::handle_admin_tenant_context)
                .post(admin_tenants::handle_admin_set_tenant_context),
        )
}

pub(super) fn build_admin_upload_routes() -> Router<AppState> {
    Router::new().route(
        "/admin/v1/uploads",
        post(admin_uploads::handle_admin_upload),
    )
}

#[cfg(feature = "whatsapp")]
pub(super) fn build_whatsapp_routes(app: Router<AppState>) -> Router<AppState> {
    app.route("/whatsapp", get(handle_whatsapp_verify))
        .route("/whatsapp", post(handle_whatsapp_message))
}

fn default_json_object() -> Value {
    serde_json::json!({})
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CompanionContextIngestPayload {
    pub session_id: crate::contracts::ids::SessionId,
    pub tab_id: String,
    pub kind: CompanionContextKind,
    pub topic: String,
    pub source: String,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub media_ref: Option<String>,
    #[serde(default = "default_json_object")]
    pub payload: Value,
    #[serde(default)]
    pub entity_id: Option<crate::contracts::ids::EntityId>,
    #[serde(default)]
    pub signal_tier: Option<crate::core::memory::SignalTier>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CompanionMultimodalIngestPayload {
    pub source_ref: String,
    pub media_kind: CompanionMediaKind,
    pub descriptors: Vec<String>,
    #[serde(default)]
    pub transcript: Option<String>,
    #[serde(default)]
    pub emotional_impact: Option<CompanionEmotionalImpact>,
    #[serde(default)]
    pub entity_id: Option<crate::contracts::ids::EntityId>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CompanionSurfaceCaptionPayload {
    pub channel: CompanionCaptionChannel,
    #[serde(default)]
    pub sequence: Option<u64>,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct CompanionSurfaceRequestWindowOpenPayload {
    pub requested_action: String,
    #[serde(default)]
    pub ttl_secs: Option<u64>,
}
