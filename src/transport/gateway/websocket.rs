//! WebSocket upgrade handler: authenticates the connection, enforces
//! concurrency limits, and runs bidirectional message/event streaming.
use std::path::PathBuf;
use std::pin::pin;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use uuid::Uuid;

use super::AppState;
use super::autosave::{tenant_scoped_entity_id, tenant_workspace_dir};
use super::defense::{
    PolicyViolation, kill_switch_response, must_enforce_auth_violation, policy_violation_response,
};
use super::events::{ClientMessage, GatewayAgentStateNotifier, ServerMessage};
use super::handlers::turn_bridge::resolve_gateway_turn_session;
use super::ws_stream_sink::WebSocketStreamSink;
use crate::contracts::channels::SurfaceRealizationPolicy;
use crate::contracts::ids::{MessageId, SessionId};
use crate::core::agent::{LoopStopReason, ToolLoopResult};
use crate::core::persona::person_identity::channel_entity_id;
use crate::core::tools::middleware::ExecutionContext;
use crate::runtime::services::{
    CompanionTransportTurnRequest, CompanionTurnRuntimeDeps, run_transport_companion_turn,
};
use crate::security::pairing::constant_time_eq;
use crate::security::policy::TenantPolicyContext;
use crate::security::scrub::sanitize_api_error;

struct WebsocketTurnRunRequest<'a> {
    state: &'a AppState,
    tenant_context: &'a TenantPolicyContext,
    session_id: Option<&'a str>,
    session_owner_scope: Option<&'a str>,
    message: &'a str,
    image_blocks: &'a [crate::core::providers::response::ContentBlock],
    system_prompt: &'a str,
    temperature: f64,
    ctx: &'a ExecutionContext,
    source_identifier: &'a str,
    tenant_id: Option<&'a str>,
}

struct ChatClientMessageRequest<'a> {
    session_id: Option<SessionId>,
    message: &'a str,
    attachments: Option<&'a [super::events::ClientAttachment]>,
    connection_source_identifier: &'a str,
    connection_session_principal: Option<&'a str>,
    tenant_id: Option<&'a str>,
}

/// Query parameters for WebSocket upgrade (token-based auth for browsers).
#[derive(serde::Deserialize, Default)]
pub(super) struct WsQuery {
    #[serde(default)]
    token: Option<String>,
}

/// Axum handler that upgrades an HTTP request to a WebSocket
/// connection after authentication and concurrency checks.
pub(super) async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<WsQuery>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Response {
    // Allow token via query parameter for browser WebSocket clients
    // that cannot set Authorization headers.
    let headers = if query.token.is_some() && !headers.contains_key(header::AUTHORIZATION) {
        let mut headers = headers;
        if let Some(ref token) = query.token
            && let Ok(value) = HeaderValue::from_str(&format!("Bearer {token}"))
        {
            headers.insert(header::AUTHORIZATION, value);
        }
        headers
    } else {
        headers
    };
    if let Some(response) = enforce_ws_upgrade_auth(&state, &headers) {
        return response.into_response();
    }
    let tenant_context = match super::handlers::request_policy_context(&state, &headers) {
        Ok(context) => context,
        Err(response) => return response.into_response(),
    };
    let connection_source_identifier = websocket_connection_source_identifier(&state, &headers);
    let connection_session_principal = super::handlers::paired_bearer_principal(&state, &headers);

    // Enforce concurrent WebSocket connection limit
    let current = state
        .connections
        .active_ws_connections
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if current >= super::types::MAX_WS_CONNECTIONS {
        state
            .connections
            .active_ws_connections
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        tracing::warn!(
            active = current,
            limit = super::types::MAX_WS_CONNECTIONS,
            "WebSocket upgrade rejected: connection limit reached"
        );
        return (StatusCode::SERVICE_UNAVAILABLE, "too many connections").into_response();
    }

    let ws_counter = Arc::clone(&state.connections.active_ws_connections);
    ws.on_upgrade(move |socket| async move {
        handle_socket(
            socket,
            state,
            tenant_context,
            connection_source_identifier,
            connection_session_principal,
        )
        .await;
        ws_counter.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    })
    .into_response()
}

fn companion_surface_scope(tenant_context: &TenantPolicyContext) -> String {
    if tenant_context.tenant_mode_enabled
        && let Some(tenant_id) = tenant_context.tenant_id.as_deref()
        && !tenant_id.is_empty()
    {
        return format!("tenant:{tenant_id}");
    }
    "global".to_string()
}

fn should_forward_gateway_event(message: &ServerMessage, companion_scope: &str) -> bool {
    match message.companion_scope() {
        Some(scope) => scope == companion_scope,
        None => true,
    }
}

/// Validates bearer token and webhook secret headers for a WebSocket
/// upgrade request, returning an error response on failure.
pub(super) fn enforce_ws_upgrade_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Option<(StatusCode, Json<serde_json::Value>)> {
    if let Some(response) = kill_switch_response(state) {
        return Some(response);
    }

    if state.access.pairing.is_paired() {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("");
        let token = auth.strip_prefix("Bearer ").unwrap_or("");
        if !state.access.pairing.is_authenticated(token) {
            let violation = PolicyViolation::MissingOrInvalidBearer;
            if must_enforce_auth_violation(state, violation) {
                return Some(violation.enforce_response());
            }
        }
    }

    if let Some(ref secret) = state.access.webhook_secret {
        let header_val = headers
            .get("X-Webhook-Secret")
            .and_then(|value| value.to_str().ok());
        match header_val {
            Some(value) if constant_time_eq(value, secret.as_ref()) => {}
            _ => {
                let violation = PolicyViolation::MissingOrInvalidWebhookSecret;
                if must_enforce_auth_violation(state, violation) {
                    return Some(violation.enforce_response());
                }
            }
        }
    }

    if !state.access.pairing.is_paired()
        && state.access.webhook_secret.is_none()
        && let Some(response) = policy_violation_response(state, PolicyViolation::NoAuthConfigured)
    {
        return Some(response);
    }

    None
}

async fn handle_socket(
    mut socket: WebSocket,
    state: AppState,
    tenant_context: TenantPolicyContext,
    connection_source_identifier: String,
    connection_session_principal: Option<String>,
) {
    let companion_scope = companion_surface_scope(&tenant_context);
    let tenant_id = tenant_context.tenant_id.as_deref();
    let mut event_rx = state.companion.gateway_events.subscribe();
    let connected = ServerMessage::connected();
    if send_message(&mut socket, &connected, tenant_id)
        .await
        .is_err()
    {
        return;
    }

    // CANCEL-SAFE: spawned tool-loop tasks run independently via tokio::spawn;
    // socket close causes graceful completion, no resource leaks.
    loop {
        tokio::select! {
            socket_result = socket.recv() => {
                let Some(result) = socket_result else {
                    break;
                };
                let message = match result {
                    Ok(message) => message,
                    Err(error) => {
                        tracing::debug!("websocket receive error: {error}");
                        break;
                    }
                };

                match message {
                    Message::Text(text) => match serde_json::from_str::<ClientMessage>(&text) {
                        Ok(client_message) => {
                            if handle_client_message(
                                &mut socket,
                                &state,
                                client_message,
                                &tenant_context,
                                &connection_source_identifier,
                                connection_session_principal.as_deref(),
                                tenant_id,
                            )
                            .await
                            .is_err()
                            {
                                break;
                            }
                        }
                        Err(error) => {
                            let server_message = sanitized_error_message(format!(
                                "invalid message: {error}"
                            ));
                            if send_message(&mut socket, &server_message, tenant_id).await.is_err() {
                                break;
                            }
                        }
                    },
                    Message::Close(_) => break,
                    Message::Ping(data) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            event = event_rx.recv() => {
                match event {
                    Ok(server_message) => {
                        if !should_forward_gateway_event(&server_message, &companion_scope) {
                            continue;
                        }
                        if send_message(&mut socket, &server_message, tenant_id).await.is_err() {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "websocket gateway event stream lagged");
                    }
                    Err(RecvError::Closed) => {
                        tracing::debug!("websocket gateway event stream closed");
                        break;
                    }
                }
            }
        }
    }
}

async fn handle_client_message(
    socket: &mut WebSocket,
    state: &AppState,
    message: ClientMessage,
    tenant_context: &TenantPolicyContext,
    connection_source_identifier: &str,
    connection_session_principal: Option<&str>,
    tenant_id: Option<&str>,
) -> Result<(), axum::Error> {
    match message {
        ClientMessage::Chat {
            session_id,
            message,
            attachments,
        } => {
            handle_chat_client_message(
                socket,
                state,
                tenant_context,
                ChatClientMessageRequest {
                    session_id,
                    message: &message,
                    attachments: attachments.as_deref(),
                    connection_source_identifier,
                    connection_session_principal,
                    tenant_id,
                },
            )
            .await?;
        }
        ClientMessage::Typing { .. } => {}
        ClientMessage::Ping => {
            send_message(socket, &ServerMessage::Pong, tenant_id).await?;
        }
    }

    Ok(())
}

async fn handle_chat_client_message(
    socket: &mut WebSocket,
    state: &AppState,
    tenant_context: &TenantPolicyContext,
    request: ChatClientMessageRequest<'_>,
) -> Result<(), axum::Error> {
    if let Err(policy_error) = state.runtime.security.consume_action_cost(0) {
        let server_message = sanitized_error_message(policy_error);
        send_message(socket, &server_message, request.tenant_id).await?;
        return Ok(());
    }

    if !send_typing_indicator(socket, request.tenant_id).await? {
        return Ok(());
    }

    let image_blocks = resolve_attachment_content_blocks(
        request.attachments,
        &state.runtime.config.workspace_dir,
        tenant_context.tenant_id.as_deref(),
    )
    .await;

    let workspace_dir = match websocket_workspace_dir(state, tenant_context).await {
        Ok(workspace_dir) => workspace_dir,
        Err(error) => {
            tracing::error!(%error, "failed to resolve websocket workspace");
            let server_message = sanitized_error_message("Failed to resolve tenant workspace");
            send_message(socket, &server_message, request.tenant_id).await?;
            return Ok(());
        }
    };
    let base_prompt =
        crate::transport::channels::gateway_base_prompt(Some(workspace_dir.as_path()));
    let session_binding = resolve_gateway_turn_session(
        state.runtime.session_manager.as_deref(),
        "gateway_ws",
        request.session_id.as_ref().map(SessionId::as_str),
        tenant_context,
        request.connection_session_principal,
        true,
        request.connection_source_identifier,
    )
    .await
    .map_err(axum::Error::new)?;
    let source_identifier = session_binding.source_identifier();

    let ctx = build_websocket_execution_context(
        state,
        tenant_context,
        source_identifier,
        session_binding.context_session_id(),
        workspace_dir.clone(),
    );
    let outcome = execute_websocket_turn_with_streaming(
        socket,
        WebsocketTurnRunRequest {
            state,
            tenant_context,
            session_id: session_binding.context_session_id(),
            session_owner_scope: session_binding.session_owner_scope(),
            message: request.message,
            image_blocks: &image_blocks,
            system_prompt: &base_prompt,
            temperature: state.runtime.temperature,
            ctx: &ctx,
            source_identifier,
            tenant_id: request.tenant_id,
        },
    )
    .await;

    match outcome {
        Ok(outcome) => {
            handle_websocket_turn_success(socket, outcome, request.tenant_id, (), request.message)
                .await
        }
        Err(error) => {
            let server_message = sanitized_error_message(error.to_string());
            send_message(socket, &server_message, request.tenant_id).await
        }
    }
}

async fn execute_websocket_turn_with_streaming(
    socket: &mut WebSocket,
    request: WebsocketTurnRunRequest<'_>,
) -> Result<crate::core::agent::TurnExecutionOutcome, axum::Error> {
    let stream_session_id = request
        .session_id
        .map_or_else(|| SessionId::new("anonymous"), SessionId::new);
    let stream_message_id = MessageId::new(format!("wsmsg_{}", Uuid::new_v4()));
    let (stream_tx, mut stream_rx) = mpsc::channel::<String>(128);
    let stream_sink = Arc::new(WebSocketStreamSink::new(
        stream_tx,
        stream_session_id,
        stream_message_id,
        request.tenant_id.map(str::to_owned),
    ));
    let entity_id = request.ctx.entity_id.to_string();
    let person_id = format!("ws.{}", request.source_identifier);
    let surface_realization_policy = SurfaceRealizationPolicy::gateway_ws();
    let execution = run_transport_companion_turn(CompanionTransportTurnRequest {
        runtime: CompanionTurnRuntimeDeps {
            mem: Arc::clone(&request.state.runtime.mem),
            persona_config: &request.state.runtime.config.persona,
            session_manager: request.state.runtime.session_manager.as_deref(),
            working_memory_capacity: request.state.runtime.config.memory.working_memory_capacity,
            registry: Arc::clone(&request.state.runtime.registry),
            max_tool_iterations: request.state.runtime.max_tool_loop_iterations,
            loop_detection: request.state.runtime.loop_detection.clone(),
            response_finalization_enabled: request
                .state
                .runtime
                .config
                .persona
                .enable_response_finalization,
            naturalness_gate_enabled: request.state.runtime.config.persona.enable_naturalness_gate,
            self_amendment_candidate_sink: Some(Arc::new(
                request
                    .state
                    .runtime
                    .self_amendment_candidate_review
                    .clone(),
            )),
        },
        workspace_dir: request.ctx.workspace_dir.as_path(),
        base_prompt: request.system_prompt,
        user_message: request.message,
        entity_id: &entity_id,
        person_id: &person_id,
        base_temperature: request.temperature,
        policy_context: request.tenant_context,
        session_surface: Some("gateway_ws"),
        channel_context_hint: None,
        surface_realization_policy: Some(&surface_realization_policy),
        session_owner_scope: request.session_owner_scope,
        working_memory_session_id: request.ctx.session_id.as_deref().unwrap_or("anonymous"),
        history_channel_name: "gateway_ws",
        history_session_key: request.ctx.session_id.as_deref(),
        history_tenant_id: request.tenant_context.tenant_id.as_deref(),
        history_max_tokens: request.state.runtime.session_history_max_tokens,
        fallback_history: &[],
        provider: request.state.runtime.provider.as_ref(),
        image_content: request.image_blocks,
        model: &request.state.runtime.model,
        inference_options: None,
        ctx: request.ctx,
        stream_sink: Some(stream_sink),
        state_notifier: Some(Arc::new(GatewayAgentStateNotifier::new(
            request.source_identifier,
            request.state.companion.gateway_events.clone(),
        ))),
        transcript_log_target: "transport::gateway::websocket",
    });
    let mut execution = pin!(execution);
    let mut stream_open = true;

    let outcome = loop {
        tokio::select! {
            maybe_json = stream_rx.recv(), if stream_open => {
                if let Some(json) = maybe_json {
                    send_json_message(socket, &json).await?;
                } else {
                    stream_open = false;
                }
            }
            result = &mut execution => {
                break result;
            }
        }
    };

    while let Ok(json) = stream_rx.try_recv() {
        send_json_message(socket, &json).await?;
    }

    outcome.map_err(axum::Error::new)
}

async fn handle_websocket_turn_success(
    socket: &mut WebSocket,
    outcome: crate::core::agent::TurnExecutionOutcome,
    tenant_id: Option<&str>,
    _post_turn_context: (),
    _user_message: &str,
) -> Result<(), axum::Error> {
    if should_send_websocket_chat_response(&outcome.result) {
        send_message(
            socket,
            &ServerMessage::chat_response(
                outcome.session_id,
                outcome.result.final_text.clone(),
                None,
                None,
            ),
            tenant_id,
        )
        .await?;
    }
    finalize_websocket_stream(socket, outcome.result, tenant_id).await
}

fn should_send_websocket_chat_response(result: &ToolLoopResult) -> bool {
    !result.streaming_delivered
        && !result.final_text.is_empty()
        && !matches!(result.stop_reason, LoopStopReason::Error(_))
}

async fn send_typing_indicator(
    socket: &mut WebSocket,
    tenant_id: Option<&str>,
) -> Result<bool, axum::Error> {
    let typing = ServerMessage::Typing { agent: true };
    if let Err(error) = send_message(socket, &typing, tenant_id).await {
        tracing::warn!(%error, "failed to send typing indicator; WebSocket may be closed");
        return Ok(false);
    }

    Ok(true)
}

fn build_websocket_execution_context(
    state: &AppState,
    tenant_context: &TenantPolicyContext,
    source_identifier: &str,
    session_id: Option<&str>,
    workspace_dir: PathBuf,
) -> ExecutionContext {
    let entity_id = ws_entity_id(source_identifier, tenant_context);
    let mut ctx = ExecutionContext::runtime_root(
        Arc::clone(&state.runtime.security),
        workspace_dir,
        Arc::clone(&state.runtime.rate_limiter),
        Some(Arc::clone(&state.runtime.permission_store)),
        tenant_context.clone(),
    );
    ctx.entity_id = entity_id;
    ctx.memory = Some(Arc::clone(&state.runtime.mem));
    ctx.observer = Arc::clone(&state.runtime.observer);
    ctx.session_id = session_id.map(std::string::ToString::to_string);
    ctx.subagent_manager = Some(Arc::clone(&state.runtime.subagent_manager));
    ctx
}

async fn finalize_websocket_stream(
    socket: &mut WebSocket,
    result: ToolLoopResult,
    tenant_id: Option<&str>,
) -> Result<(), axum::Error> {
    if let LoopStopReason::Error(error) = &result.stop_reason {
        let server_message = sanitized_error_message(error);
        send_message(socket, &server_message, tenant_id).await?;
        return Ok(());
    }
    if matches!(result.stop_reason, LoopStopReason::MaxIterations) {
        tracing::warn!("websocket tool loop hit max iterations");
    }

    Ok(())
}

fn sanitized_error_message(message: impl AsRef<str>) -> ServerMessage {
    ServerMessage::error(sanitize_api_error(message.as_ref()))
}

async fn websocket_workspace_dir(
    state: &AppState,
    tenant_context: &TenantPolicyContext,
) -> anyhow::Result<PathBuf> {
    tenant_workspace_dir(
        &state.runtime.security.workspace_dir,
        tenant_context,
        "websocket",
    )
    .await
}

async fn send_message(
    socket: &mut WebSocket,
    message: &ServerMessage,
    tenant_id: Option<&str>,
) -> Result<(), axum::Error> {
    let json = message.to_envelope_json(tenant_id);
    send_json_message(socket, &json).await
}

async fn send_json_message(socket: &mut WebSocket, json: &str) -> Result<(), axum::Error> {
    socket.send(Message::Text(json.to_owned().into())).await
}

/// Resolve client attachments into `ContentBlock::Image` entries.
///
/// Reads uploaded files from the persisted gateway upload store and converts
/// image files to base64 content blocks that providers understand.
async fn resolve_attachment_content_blocks(
    attachments: Option<&[super::events::ClientAttachment]>,
    workspace_dir: &std::path::Path,
    tenant_id: Option<&str>,
) -> Vec<crate::core::providers::response::ContentBlock> {
    use crate::core::providers::response::{ContentBlock, ImageSource};
    use base64::Engine as _;

    let Some(attachments) = attachments else {
        return Vec::new();
    };

    let mut blocks = Vec::new();
    let Some(tenant_id) = tenant_id else {
        return blocks;
    };
    for attachment in attachments {
        if !attachment.content_type.starts_with("image/") {
            continue;
        }

        if !attachment.upload_id.starts_with("upl_")
            || attachment.upload_id.len() > 48
            || attachment.upload_id.contains('/')
            || attachment.upload_id.contains('\\')
            || attachment.upload_id.contains("..")
        {
            tracing::warn!(
                upload_id = %attachment.upload_id,
                "rejected upload_id: invalid format"
            );
            continue;
        }

        let Some((upload_dir, upload_path, stored_content_type)) =
            super::handlers::admin_uploads::resolve_stored_upload_path(
                workspace_dir,
                tenant_id,
                &attachment.upload_id,
            )
        else {
            tracing::warn!(
                upload_id = %attachment.upload_id,
                "failed to resolve upload metadata"
            );
            continue;
        };
        let media_type = stored_content_type.unwrap_or_else(|| attachment.content_type.clone());
        if !media_type.starts_with("image/") {
            tracing::warn!(
                upload_id = %attachment.upload_id,
                content_type = %media_type,
                "rejected upload: stored content type is not an image"
            );
            continue;
        }

        let canonical = match tokio::fs::canonicalize(&upload_path).await {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(
                    upload_id = %attachment.upload_id,
                    %error,
                    "failed to read uploaded attachment"
                );
                continue;
            }
        };
        let canonical_uploads = match tokio::fs::canonicalize(&upload_dir).await {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(
                    upload_id = %attachment.upload_id,
                    %error,
                    "failed to resolve uploads directory"
                );
                continue;
            }
        };
        if !canonical.starts_with(&canonical_uploads) {
            tracing::warn!(
                upload_id = %attachment.upload_id,
                "upload path escaped uploads directory"
            );
            continue;
        }

        let data = match tokio::fs::read(&canonical).await {
            Ok(data) => data,
            Err(error) => {
                tracing::warn!(
                    upload_id = %attachment.upload_id,
                    %error,
                    "failed to read uploaded attachment"
                );
                continue;
            }
        };

        let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
        blocks.push(ContentBlock::Image {
            source: ImageSource::Base64 {
                media_type,
                data: encoded,
            },
        });
    }

    blocks
}

fn ws_entity_id(
    source_identifier: &str,
    tenant_context: &TenantPolicyContext,
) -> crate::contracts::ids::EntityId {
    tenant_scoped_entity_id(
        crate::contracts::ids::EntityId::new(channel_entity_id("gateway", source_identifier)),
        tenant_context,
    )
}

fn websocket_connection_source_identifier(state: &AppState, headers: &HeaderMap) -> String {
    super::handlers::paired_bearer_principal(state, headers)
        .unwrap_or_else(|| format!("websocket-{}", Uuid::new_v4().simple()))
}

#[cfg(test)]
mod tests {
    use super::{
        companion_surface_scope, resolve_attachment_content_blocks, sanitized_error_message,
        should_forward_gateway_event, should_send_websocket_chat_response,
    };
    use crate::contracts::ids::SessionId;
    use crate::core::agent::{LoopStopReason, ToolLoopResult};
    use crate::core::providers::response::{ContentBlock, ImageSource};
    use crate::security::policy::TenantPolicyContext;
    use crate::transport::gateway::companion_bridge::{
        CompanionCaptionChannel, CompanionCaptionEvt,
    };
    use crate::transport::gateway::events::{
        ClientAttachment, CompanionContextIngressEvent, ServerMessage,
    };
    use tempfile::TempDir;

    #[test]
    fn companion_scope_defaults_to_global_when_tenant_mode_disabled() {
        let context = TenantPolicyContext::disabled();

        assert_eq!(companion_surface_scope(&context), "global");
    }

    #[test]
    fn companion_scope_uses_tenant_prefix_when_enabled() {
        let context = TenantPolicyContext::enabled("tenant-alpha");

        assert_eq!(companion_surface_scope(&context), "tenant:tenant-alpha");
    }

    #[test]
    fn forward_filter_allows_only_matching_companion_scope() {
        let event = CompanionCaptionEvt::new(CompanionCaptionChannel::Assistant, 1, "hello")
            .expect("caption event should build");
        let message = ServerMessage::companion_caption("tenant:alpha", event);

        assert!(should_forward_gateway_event(&message, "tenant:alpha"));
        assert!(!should_forward_gateway_event(&message, "global"));
    }

    #[test]
    fn forward_filter_applies_to_companion_context_ingress() {
        let event = CompanionContextIngressEvent {
            session_id: SessionId::new("session-1"),
            tab_id: "tab-a".to_string(),
            kind: "page".to_string(),
            topic: "page_snapshot".to_string(),
            source: "extension".to_string(),
            accepted: true,
            reason: "accepted".to_string(),
            dedupe_key: "k1".to_string(),
            slot_key: Some("external.document.slot".to_string()),
            signal_tier: Some("raw".to_string()),
        };
        let message = ServerMessage::companion_context_ingress("tenant:alpha", event);

        assert!(should_forward_gateway_event(&message, "tenant:alpha"));
        assert!(!should_forward_gateway_event(&message, "tenant:beta"));
    }

    #[test]
    fn sanitized_error_message_scrubs_secret_text() {
        let message = sanitized_error_message("provider failed with sk-test-secret-token");
        let value = serde_json::to_value(message).expect("serialize server error");

        assert_eq!(value["type"], "error");
        assert!(
            !value["message"]
                .as_str()
                .expect("message string")
                .contains("sk-test-secret-token")
        );
    }

    fn websocket_result(final_text: &str, streaming_delivered: bool) -> ToolLoopResult {
        ToolLoopResult {
            final_text: final_text.to_string(),
            tool_calls: Vec::new(),
            attachments: Vec::new(),
            loop_detection_events: Vec::new(),
            iterations: 1,
            tokens_used: None,
            stop_reason: LoopStopReason::Completed,
            logprobs: None,
            streaming_delivered,
        }
    }

    #[test]
    fn websocket_chat_response_depends_on_actual_stream_delivery() {
        assert!(should_send_websocket_chat_response(&websocket_result(
            "finalized answer",
            false
        )));
        assert!(!should_send_websocket_chat_response(&websocket_result(
            "already streamed",
            true
        )));
        assert!(!should_send_websocket_chat_response(&websocket_result(
            "", false
        )));
    }

    #[tokio::test]
    async fn attachment_resolution_reads_from_gateway_upload_store_metadata() {
        let temp = TempDir::new().expect("temp dir");
        let uploads_dir = temp.path().join(".asterel/gateway/uploads/tenant-a");
        std::fs::create_dir_all(&uploads_dir).expect("create uploads dir");

        let upload_id = "upl_test_attachment";
        let stored_name = format!("{upload_id}-diagram.png");
        std::fs::write(uploads_dir.join(&stored_name), b"png-bytes").expect("write upload");
        std::fs::write(
            uploads_dir.join(format!("{upload_id}.json")),
            serde_json::to_vec_pretty(&serde_json::json!({
                "upload_id": upload_id,
                "tenant_id": "tenant-a",
                "field_name": "file",
                "original_name": "diagram.png",
                "stored_name": stored_name,
                "content_type": "image/png",
                "size_bytes": 9,
                "stored_path": ".asterel/gateway/uploads/tenant-a/upl_test_attachment-diagram.png",
                "source_ref": format!("admin-upload:{upload_id}"),
            }))
            .expect("serialize metadata"),
        )
        .expect("write metadata");

        let blocks = resolve_attachment_content_blocks(
            Some(&[ClientAttachment {
                upload_id: upload_id.to_string(),
                filename: "diagram.png".to_string(),
                content_type: "image/png".to_string(),
            }]),
            temp.path(),
            Some("tenant-a"),
        )
        .await;

        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::Image {
                source: ImageSource::Base64 { media_type, data },
            } => {
                assert_eq!(media_type, "image/png");
                assert_eq!(data, "cG5nLWJ5dGVz");
            }
            other => panic!("expected image content block, got {other:?}"),
        }

        let foreign_blocks = resolve_attachment_content_blocks(
            Some(&[ClientAttachment {
                upload_id: upload_id.to_string(),
                filename: "diagram.png".to_string(),
                content_type: "image/png".to_string(),
            }]),
            temp.path(),
            Some("tenant-b"),
        )
        .await;
        assert!(foreign_blocks.is_empty());
    }
}
