//! General webhook, health-check, `OpenAPI` contract, and A2A agent-card handlers.
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};

use super::super::autosave::{
    gateway_autosave_entity_id, gateway_webhook_autosave_event, tenant_scoped_entity_id,
};
use super::super::contract::openapi_contract_json;
use super::super::defense::{apply_external_ingress_policy, policy_accounting_response};
use super::super::problem_details::problem_response;
use super::super::{AppState, WebhookBody};
use super::a2a_read_models::build_a2a_agent_card;
use super::{
    enforce_entity_rate_limit_for_source, enforce_json_content_type, enforce_request_auth,
    external_trust_source_from_headers, gateway_workspace_dir, log_tool_loop_stop,
    request_policy_context, resolve_webhook_source_identifier, run_tool_loop,
    webhook_replay_ack_response,
};
use crate::contracts::strings::verdicts::EXTERNAL_CONTENT_BLOCKED_BY_SAFETY_POLICY;
use crate::core::providers;
use crate::runtime::services::load_gateway_readiness_assessment;
use crate::security::policy::TenantPolicyContext;
use crate::security::writeback_guard::enforce_external_autosave_write_policy;

type GatewayJsonResponse = (StatusCode, Json<serde_json::Value>);

fn parse_webhook_body(body: &Bytes) -> Result<WebhookBody, GatewayJsonResponse> {
    serde_json::from_slice::<WebhookBody>(body).map_err(|error| {
        tracing::debug!(%error, "webhook: JSON parse failed");
        problem_response(
            StatusCode::BAD_REQUEST,
            "invalid_json_payload",
            "Bad Request",
            "Invalid JSON payload. Expected: {\"message\": \"...\"}".to_string(),
        )
    })
}

fn replay_scope_for_request(
    base: &str,
    tenant_id: Option<&str>,
    source_identifier: &str,
) -> String {
    format!(
        "{base}:tenant={}:source={}",
        tenant_id.unwrap_or("_"),
        source_identifier
    )
}

/// GET /health — always public (no secrets leaked)
pub(in super::super) async fn handle_health(State(state): State<AppState>) -> impl IntoResponse {
    let body = serde_json::json!({
        "status": "ok",
        "paired": state.access.pairing.is_paired(),
        "runtime": crate::runtime::diagnostics::health::snapshot_json(),
    });
    Json(body)
}

/// GET /ready — gateway readiness probe based on required component status.
pub(in super::super) async fn handle_ready(State(state): State<AppState>) -> impl IntoResponse {
    let readiness = load_gateway_readiness_assessment(
        state.runtime.config.as_ref(),
        state.runtime.readiness_profile,
        state.runtime.session_manager.is_some(),
    );
    let status = if readiness.ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let body = serde_json::json!({
        "status": if readiness.ready { "ready" } else { "not_ready" },
        "paired": state.access.pairing.is_paired(),
        "runtime": readiness.runtime,
        "required_components": readiness.required_components,
        "failing_components": readiness.failing_components,
    });
    (status, Json(body))
}

/// GET /openapi/v1.json — machine-readable API contract baseline
pub(in super::super) async fn handle_openapi_contract() -> impl IntoResponse {
    match openapi_contract_json() {
        Ok(contract) => (StatusCode::OK, Json(contract)),
        Err(error) => {
            tracing::error!(%error, "failed to load embedded OpenAPI contract");
            problem_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "openapi_contract_unavailable",
                "Internal Server Error",
                "Failed to load OpenAPI contract",
            )
        }
    }
}

pub(in super::super) async fn handle_agent_card(
    State(state): State<AppState>,
) -> impl IntoResponse {
    let auth_mode =
        if state.access.pairing.require_pairing() || state.access.webhook_secret.is_some() {
            "bearer"
        } else {
            "none"
        };
    (StatusCode::OK, Json(build_a2a_agent_card(auth_mode)))
}

/// Persist an autosave event for the webhook request if `auto_save` is enabled.
async fn webhook_autosave(
    state: &AppState,
    source_identifier: &str,
    policy_context: &TenantPolicyContext,
    persisted_summary: String,
) {
    let autosave_entity_id = tenant_scoped_entity_id(
        gateway_autosave_entity_id(source_identifier),
        policy_context,
    );
    if let Err(error) = policy_context.enforce_recall_scope(autosave_entity_id.as_str()) {
        tracing::warn!(
            error,
            "gateway webhook autosave skipped due to policy context"
        );
        return;
    }
    let event = gateway_webhook_autosave_event(autosave_entity_id.as_str(), persisted_summary);
    if let Err(error) = enforce_external_autosave_write_policy(&event) {
        tracing::warn!(%error, "gateway webhook autosave rejected by write policy");
    } else if let Err(error) = state.runtime.mem.append_event(event).await {
        tracing::warn!(%error, "failed to autosave webhook event");
    }
}

/// POST /webhook — main webhook endpoint
#[allow(clippy::too_many_lines)] // Auth, replay defense, ingress safety, autosave, and tool execution are validated in one flow.
pub(in super::super) async fn handle_webhook(
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

    let webhook_body = match parse_webhook_body(&body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let policy_context = match request_policy_context(&state, &headers) {
        Ok(context) => context,
        Err(response) => return response,
    };
    let source_identifier = match resolve_webhook_source_identifier(&headers) {
        Ok(source_identifier) => source_identifier,
        Err(response) => return response,
    };
    let replay_scope = replay_scope_for_request(
        "webhook",
        policy_context.tenant_id.as_deref(),
        &source_identifier,
    );
    if !state
        .companion
        .replay_guard
        .check_and_record_hash(&replay_scope, &body)
    {
        tracing::warn!(%replay_scope, "Webhook replay detected");
        return webhook_replay_ack_response();
    }

    let source = external_trust_source_from_headers(&headers, "gateway:webhook");
    let ingress = apply_external_ingress_policy(
        &source,
        &webhook_body.message,
        &state.runtime.external_knowledge_trust,
    );

    if ingress.blocked {
        tracing::warn!(
            source,
            "blocked high-risk external content at gateway webhook ingress"
        );
        return problem_response(
            StatusCode::BAD_REQUEST,
            "external_content_blocked",
            "Bad Request",
            EXTERNAL_CONTENT_BLOCKED_BY_SAFETY_POLICY,
        );
    }

    if state.runtime.auto_save {
        webhook_autosave(
            &state,
            &source_identifier,
            &policy_context,
            ingress.persisted_summary.clone(),
        )
        .await;
    }

    if let Err(policy_error) = state.runtime.security.consume_action_cost(0) {
        return policy_accounting_response(policy_error);
    }
    if let Some(response) =
        enforce_entity_rate_limit_for_source(&state, &source_identifier, &policy_context)
    {
        return response;
    }

    let workspace_dir = match gateway_workspace_dir(&state, &policy_context).await {
        Ok(workspace_dir) => workspace_dir,
        Err(error) => {
            tracing::error!(%error, "failed to resolve gateway workspace");
            return problem_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "tenant_workspace_unavailable",
                "Internal Server Error",
                "Failed to resolve tenant workspace",
            );
        }
    };
    let base_prompt =
        crate::transport::channels::gateway_base_prompt(Some(workspace_dir.as_path()));
    let session_principal = super::paired_bearer_principal(&state, &headers);
    match run_tool_loop(
        &state,
        Some(&base_prompt),
        &ingress.model_input,
        &state.runtime.model,
        state.runtime.temperature,
        &source_identifier,
        session_principal.as_deref(),
        policy_context,
    )
    .await
    {
        Ok(result) => {
            log_tool_loop_stop("gateway:webhook", &result.stop_reason, result.iterations);
            let response = super::gateway_delivery_text(&result.final_text);
            let body = serde_json::json!({
                "response": response,
                "model": state.runtime.model
            });
            (StatusCode::OK, Json(body))
        }
        Err(e) => {
            state
                .companion
                .replay_guard
                .forget_scoped(&replay_scope, &body);
            tracing::error!(
                "Webhook provider error: {}",
                providers::sanitize_api_error(&e.to_string())
            );
            problem_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "llm_request_failed",
                "Internal Server Error",
                "LLM request failed",
            )
        }
    }
}
