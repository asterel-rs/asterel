//! `WhatsApp` Cloud API webhook handler: signature verification, message
//! extraction, tool-loop execution, and reply delivery.
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};

use super::super::autosave::{
    gateway_autosave_entity_id, gateway_runtime_policy_context, gateway_whatsapp_autosave_event,
    tenant_scoped_entity_id,
};
use super::super::defense::{ExternalIngressPolicyOutcome, apply_external_ingress_policy};
use super::super::problem_details::problem_response;
use super::super::signature::verify_wa_signature;
use super::super::{AppState, WhatsAppVerifyQuery};
use super::{enforce_entity_rate_limit, log_tool_loop_stop, run_tool_loop};
use crate::security::pairing::constant_time_eq;
use crate::security::scrub::sanitize_api_error;
use crate::security::writeback_guard::enforce_external_autosave_write_policy;
use crate::transport::channels::{Channel, WhatsAppChannel};
use crate::utils::text::truncate_ellipsis;

fn whatsapp_not_configured_response() -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::NOT_FOUND,
        "whatsapp_not_configured",
        "Not Found",
        "WhatsApp not configured",
    )
}

fn invalid_whatsapp_signature_response() -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::UNAUTHORIZED,
        "invalid_whatsapp_signature",
        "Unauthorized",
        "Invalid signature",
    )
}

fn missing_whatsapp_app_secret_response() -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "whatsapp_app_secret_required",
        "Service Unavailable",
        "WhatsApp webhook app_secret is required for signature verification",
    )
}

fn invalid_whatsapp_payload_response() -> (StatusCode, Json<serde_json::Value>) {
    problem_response(
        StatusCode::BAD_REQUEST,
        "invalid_whatsapp_payload",
        "Bad Request",
        "Invalid JSON payload",
    )
}

fn whatsapp_ack_response() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::OK, Json(serde_json::json!({"status": "ok"})))
}

async fn send_whatsapp_reply_or_log(wa: &WhatsAppChannel, sender: &str, message: &str) {
    if let Err(error) = wa.send_chunked(message, sender).await {
        tracing::error!("Failed to send WhatsApp reply: {error}");
    }
}

fn sanitized_whatsapp_llm_error(error: &anyhow::Error) -> String {
    sanitize_api_error(&format!("{error:#}"))
}

async fn process_whatsapp_message(
    state: &AppState,
    wa: &WhatsAppChannel,
    headers: &HeaderMap,
    sender: &str,
    content: &str,
) {
    let source = "gateway:whatsapp:signature=verified:first_party";
    let ingress =
        apply_external_ingress_policy(source, content, &state.runtime.external_knowledge_trust);
    let policy_context = gateway_runtime_policy_context(None);

    if ingress.blocked {
        tracing::warn!(
            source,
            "blocked high-risk external content at whatsapp ingress"
        );
        if let Err(error) = wa
            .send_chunked("I could not process that external content safely.", sender)
            .await
        {
            tracing::warn!(%error, "failed to send whatsapp safety block reply");
        }
        return;
    }

    if state.runtime.auto_save && should_autosave_whatsapp_ingress(&ingress) {
        let autosave_entity_id =
            tenant_scoped_entity_id(gateway_autosave_entity_id(sender), &policy_context);
        if let Err(error) = policy_context.enforce_recall_scope(autosave_entity_id.as_str()) {
            tracing::warn!(
                error,
                "gateway whatsapp autosave skipped due to policy context"
            );
        } else {
            let event = gateway_whatsapp_autosave_event(
                autosave_entity_id.as_str(),
                sender,
                ingress.persisted_summary.clone(),
            );
            if let Err(error) = enforce_external_autosave_write_policy(&event) {
                tracing::warn!(%error, "gateway whatsapp autosave rejected by write policy");
            } else if let Err(error) = state.runtime.mem.append_event(event).await {
                tracing::warn!(%error, "failed to autosave whatsapp event");
            }
        }
    }

    if let Err(policy_error) = state.runtime.security.consume_action_cost(0) {
        if let Err(error) = wa
            .send_chunked("I cannot respond right now due to policy limits.", sender)
            .await
        {
            tracing::warn!(%error, "failed to send whatsapp policy limit reply");
        }
        tracing::warn!("{policy_error}");
        return;
    }
    if enforce_entity_rate_limit(state, headers, &policy_context).is_some() {
        if let Err(error) = wa
            .send_chunked("I cannot respond right now due to rate limits.", sender)
            .await
        {
            tracing::warn!(%error, "failed to send whatsapp rate limit reply");
        }
        return;
    }

    match run_tool_loop(
        state,
        None,
        &ingress.model_input,
        &state.runtime.model,
        state.runtime.temperature,
        sender,
        None,
        policy_context,
    )
    .await
    {
        Ok(result) => {
            log_tool_loop_stop("gateway:whatsapp", &result.stop_reason, result.iterations);
            let response = super::gateway_delivery_text(&result.final_text);
            send_whatsapp_reply_or_log(wa, sender, &response).await;
        }
        Err(error) => {
            let sanitized = sanitized_whatsapp_llm_error(&error);
            tracing::error!(error = %sanitized, "LLM error for WhatsApp message");
            if let Err(error) = wa
                .send_chunked("Sorry, I couldn't process your message right now.", sender)
                .await
            {
                tracing::warn!(%error, "failed to send whatsapp error reply");
            }
        }
    }
}

fn should_autosave_whatsapp_ingress(ingress: &ExternalIngressPolicyOutcome) -> bool {
    !ingress.blocked
}

/// GET /whatsapp — Meta webhook verification
pub(in super::super) async fn handle_whatsapp_verify(
    State(state): State<AppState>,
    Query(params): Query<WhatsAppVerifyQuery>,
) -> impl IntoResponse {
    let Some(ref wa) = state.whatsapp.channel else {
        return (StatusCode::NOT_FOUND, "WhatsApp not configured".to_string());
    };

    // Verify the token matches (constant-time comparison to prevent timing attacks)
    let token_matches = params
        .verify_token
        .as_deref()
        .is_some_and(|t| constant_time_eq(t, wa.verify_token()));
    if params.mode.as_deref() == Some("subscribe") && token_matches {
        if let Some(ch) = params.challenge {
            tracing::info!("WhatsApp webhook verified successfully");
            return (StatusCode::OK, ch);
        }
        return (StatusCode::BAD_REQUEST, "Missing hub.challenge".to_string());
    }

    tracing::warn!("WhatsApp webhook verification failed — token mismatch");
    (StatusCode::FORBIDDEN, "Forbidden".to_string())
}

/// POST /whatsapp — incoming message webhook
pub(in super::super) async fn handle_whatsapp_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let Some(ref wa) = state.whatsapp.channel else {
        return whatsapp_not_configured_response();
    };

    let Some(ref app_secret) = state.whatsapp.app_secret else {
        tracing::error!("WhatsApp webhook rejected: app_secret is not configured");
        return missing_whatsapp_app_secret_response();
    };

    let signature = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !verify_wa_signature(app_secret, &body, signature) {
        tracing::warn!(
            "WhatsApp webhook signature verification failed (signature: {})",
            if signature.is_empty() {
                "missing"
            } else {
                "invalid"
            }
        );
        return invalid_whatsapp_signature_response();
    }

    // ── Replay protection: check if we've seen this body before ──
    if !state
        .companion
        .replay_guard
        .check_and_record_hash("whatsapp", &body)
    {
        tracing::warn!("WhatsApp webhook replay detected");
        return whatsapp_ack_response();
    }

    let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return invalid_whatsapp_payload_response();
    };

    let messages = wa.parse_webhook(&payload);

    if messages.is_empty() {
        // Acknowledge the webhook even if no messages (could be status updates)
        return whatsapp_ack_response();
    }

    // Acknowledge Meta immediately (15-second response deadline), then
    // process messages asynchronously to avoid replay on slow LLM calls.
    let state_clone = state.clone();
    let wa_clone = std::sync::Arc::clone(wa);
    let headers_clone = headers.clone();
    tokio::spawn(async move {
        for msg in &messages {
            tracing::info!(
                "WhatsApp message from {}: {}",
                msg.sender,
                truncate_ellipsis(&msg.content, 50)
            );
            process_whatsapp_message(
                &state_clone,
                &wa_clone,
                &headers_clone,
                &msg.sender,
                &msg.content,
            )
            .await;
        }
    });

    whatsapp_ack_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_whatsapp_ingress_is_not_autosaveable() {
        let blocked = ExternalIngressPolicyOutcome {
            model_input: "blocked".to_string(),
            persisted_summary: "content_omitted".to_string(),
            blocked: true,
        };
        let allowed = ExternalIngressPolicyOutcome {
            model_input: "allowed".to_string(),
            persisted_summary: "content_omitted".to_string(),
            blocked: false,
        };

        assert!(!should_autosave_whatsapp_ingress(&blocked));
        assert!(should_autosave_whatsapp_ingress(&allowed));
    }

    #[test]
    fn whatsapp_llm_error_log_message_is_sanitized() {
        let error = anyhow::anyhow!("provider echoed sk-leaked-secret-token in body");
        let sanitized = sanitized_whatsapp_llm_error(&error);

        assert!(!sanitized.contains("sk-leaked-secret-token"));
        assert!(sanitized.contains("[REDACTED]"));
    }
}
