//! Server-sent and client-received event types exchanged over the gateway
//! WebSocket and SSE streams (companion context, multimodal, captions, etc.).
use serde::{Deserialize, Serialize};

use super::companion_bridge::{
    CompanionCaptionEvt, CompanionWidgetRuntimeResult, CompanionWidgetState, CompanionWindow,
};
use super::ws_events::{
    ChannelUpdatedPayload, CronRunUpdatedPayload, MessageCompletedPayload, MessageCreatedPayload,
    MessageDeltaPayload, RuntimeUpdatedPayload, SessionUpdatedPayload, ToolCallUpdatedPayload,
    WsEventEnvelope,
};
use crate::contracts::ids::{SessionId, SlotKey};

/// Notification emitted when a companion context payload is ingested.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompanionContextIngressEvent {
    pub session_id: SessionId,
    pub tab_id: String,
    pub kind: String,
    pub topic: String,
    pub source: String,
    pub accepted: bool,
    pub reason: String,
    pub dedupe_key: String,
    #[serde(default)]
    pub slot_key: Option<String>,
    #[serde(default)]
    pub signal_tier: Option<String>,
}

/// Notification emitted when a multimodal media record is ingested.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompanionMultimodalIngressEvent {
    pub record_id: String,
    pub media_kind: String,
    pub source_ref: String,
    pub accepted: bool,
    pub slot_key: SlotKey,
    pub signal_tier: String,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Inbound message types received from WebSocket clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ClientMessage {
    Chat {
        session_id: Option<SessionId>,
        message: String,
        #[serde(default)]
        attachments: Option<Vec<ClientAttachment>>,
    },
    Typing {
        session_id: Option<SessionId>,
    },
    Ping,
}

/// Client-side attachment reference sent with a chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ClientAttachment {
    pub upload_id: String,
    pub filename: String,
    pub content_type: String,
}

/// Outbound message types sent to WebSocket and SSE clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    ChatResponse {
        session_id: Option<SessionId>,
        content: String,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    Typing {
        agent: bool,
    },
    Error {
        message: String,
    },
    Pong,
    Connected {
        version: String,
    },
    CompanionCaption {
        scope: String,
        event: CompanionCaptionEvt,
    },
    CompanionWidget {
        scope: String,
        result: CompanionWidgetRuntimeResult,
        widgets: Vec<CompanionWidgetState>,
    },
    #[serde(rename = "companion_request_window")]
    CompanionWindow {
        scope: String,
        action: String,
        window: CompanionWindow,
    },
    CompanionContextIngress {
        scope: String,
        event: CompanionContextIngressEvent,
    },
    CompanionMultimodalIngress {
        scope: String,
        event: CompanionMultimodalIngressEvent,
    },
    AgentState {
        agent_id: String,
        state: String,
        detail: Option<String>,
        timestamp: String,
    },
    #[serde(rename = "session_updated")]
    SessionUpdated(SessionUpdatedPayload),
    #[serde(rename = "message_created")]
    MessageCreated(MessageCreatedPayload),
    #[serde(rename = "message_delta")]
    MessageDelta(MessageDeltaPayload),
    #[serde(rename = "message_completed")]
    MessageCompleted(MessageCompletedPayload),
    #[serde(rename = "tool_call_updated")]
    ToolCallUpdated(ToolCallUpdatedPayload),
    #[serde(rename = "runtime_updated")]
    RuntimeUpdated(RuntimeUpdatedPayload),
    #[serde(rename = "channel_updated")]
    ChannelUpdated(ChannelUpdatedPayload),
    #[serde(rename = "cron_run_updated")]
    CronRunUpdated(CronRunUpdatedPayload),
}

impl ServerMessage {
    /// Creates a chat response message with optional token usage stats.
    pub fn chat_response(
        session_id: Option<SessionId>,
        content: String,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> Self {
        Self::ChatResponse {
            session_id,
            content,
            input_tokens,
            output_tokens,
        }
    }

    /// Creates an error message to send to the client.
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }

    /// Creates a connected message containing the server version.
    pub fn connected() -> Self {
        Self::Connected {
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    /// Creates a companion caption event scoped to a tenant or global.
    pub fn companion_caption(scope: impl Into<String>, event: CompanionCaptionEvt) -> Self {
        Self::CompanionCaption {
            scope: scope.into(),
            event,
        }
    }

    /// Creates a companion widget event with runtime result and state.
    pub fn companion_widget(
        scope: impl Into<String>,
        result: CompanionWidgetRuntimeResult,
        widgets: Vec<CompanionWidgetState>,
    ) -> Self {
        Self::CompanionWidget {
            scope: scope.into(),
            result,
            widgets,
        }
    }

    /// Creates a request window lifecycle event (opened/confirmed/etc.).
    pub fn companion_request_window(
        scope: impl Into<String>,
        action: impl Into<String>,
        window: CompanionWindow,
    ) -> Self {
        Self::CompanionWindow {
            scope: scope.into(),
            action: action.into(),
            window,
        }
    }

    /// Creates a context ingress notification for a companion event.
    pub fn companion_context_ingress(
        scope: impl Into<String>,
        event: CompanionContextIngressEvent,
    ) -> Self {
        Self::CompanionContextIngress {
            scope: scope.into(),
            event,
        }
    }

    /// Creates a multimodal ingress notification for a media record.
    pub fn companion_multimodal_ingress(
        scope: impl Into<String>,
        event: CompanionMultimodalIngressEvent,
    ) -> Self {
        Self::CompanionMultimodalIngress {
            scope: scope.into(),
            event,
        }
    }

    /// Creates an agent state update broadcast for the Star Office.
    pub fn agent_state(
        agent_id: impl Into<String>,
        state: impl Into<String>,
        detail: Option<String>,
    ) -> Self {
        Self::AgentState {
            agent_id: agent_id.into(),
            state: state.into(),
            detail,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Creates a `session_updated` event.
    pub fn session_updated(payload: SessionUpdatedPayload) -> Self {
        Self::SessionUpdated(payload)
    }

    /// Creates a `message_created` event.
    pub fn message_created(payload: MessageCreatedPayload) -> Self {
        Self::MessageCreated(payload)
    }

    /// Creates a `message_delta` event.
    pub fn message_delta(payload: MessageDeltaPayload) -> Self {
        Self::MessageDelta(payload)
    }

    /// Creates a `message_completed` event.
    pub fn message_completed(payload: MessageCompletedPayload) -> Self {
        Self::MessageCompleted(payload)
    }

    /// Creates a `tool_call_updated` event.
    pub fn tool_call_updated(payload: ToolCallUpdatedPayload) -> Self {
        Self::ToolCallUpdated(payload)
    }

    /// Creates a `runtime_updated` event.
    pub fn runtime_updated(payload: RuntimeUpdatedPayload) -> Self {
        Self::RuntimeUpdated(payload)
    }

    /// Creates a `channel_updated` event.
    pub fn channel_updated(payload: ChannelUpdatedPayload) -> Self {
        Self::ChannelUpdated(payload)
    }

    /// Creates a `cron_run_updated` event.
    pub fn cron_run_updated(payload: CronRunUpdatedPayload) -> Self {
        Self::CronRunUpdated(payload)
    }

    /// Returns the companion scope string if this is a companion event.
    #[must_use]
    pub fn companion_scope(&self) -> Option<&str> {
        match self {
            Self::CompanionCaption { scope, .. }
            | Self::CompanionWidget { scope, .. }
            | Self::CompanionWindow { scope, .. }
            | Self::CompanionContextIngress { scope, .. }
            | Self::CompanionMultimodalIngress { scope, .. } => Some(scope),
            _ => None,
        }
    }

    /// Serializes this message to a JSON string, falling back to an
    /// error payload on serialization failure.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self)
            .unwrap_or_else(|_| r#"{"type":"error","message":"serialization failed"}"#.to_string())
    }

    /// Serializes this message wrapped in the unified WebSocket event envelope.
    ///
    /// The `type` field is lifted to the top level; remaining fields become `payload`.
    pub fn to_envelope_json(&self, tenant_id: Option<&str>) -> String {
        let Ok(inner) = serde_json::to_value(self) else {
            return WsEventEnvelope::new(
                "error",
                tenant_id,
                serde_json::json!({"message": "serialization failed"}),
            )
            .to_json();
        };
        let event_type = inner
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("unknown")
            .to_string();
        let mut payload = inner;
        if let serde_json::Value::Object(ref mut map) = payload {
            map.remove("type");
        }
        WsEventEnvelope::new(event_type, tenant_id, payload).to_json()
    }
}

/// Bridges tool loop state transitions to gateway WebSocket broadcasts.
pub struct GatewayAgentStateNotifier {
    agent_id: String,
    sender: tokio::sync::broadcast::Sender<ServerMessage>,
}

impl GatewayAgentStateNotifier {
    /// Creates a new notifier that broadcasts state changes for the given agent.
    pub fn new(
        agent_id: impl Into<String>,
        sender: tokio::sync::broadcast::Sender<ServerMessage>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            sender,
        }
    }
}

impl crate::core::agent::tool_loop::AgentStateNotifier for GatewayAgentStateNotifier {
    fn notify_state(&self, state: &str, detail: Option<&str>) {
        let _ = self.sender.send(ServerMessage::agent_state(
            &self.agent_id,
            state,
            detail.map(String::from),
        ));
    }

    fn notify_tool_call(
        &self,
        tool_call_id: &str,
        tool_name: &str,
        status: &str,
        detail: Option<&str>,
    ) {
        let _ = self
            .sender
            .send(ServerMessage::tool_call_updated(ToolCallUpdatedPayload {
                session_id: SessionId::new(String::new()),
                tool_call_id: tool_call_id.to_string(),
                tool_name: tool_name.to_string(),
                status: status.to_string(),
                detail: detail.map(String::from),
            }));
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::{
        ClientMessage, CompanionContextIngressEvent, CompanionMultimodalIngressEvent, ServerMessage,
    };
    use crate::contracts::ids::SessionId;
    use crate::transport::gateway::companion_bridge::{
        CompanionAction, CompanionCaptionChannel, CompanionCaptionEvt,
        CompanionWidgetRuntimeResult, CompanionWidgetState, CompanionWindow,
    };

    #[test]
    fn client_message_chat_roundtrip() {
        let original = ClientMessage::Chat {
            session_id: Some(SessionId::new("session-1")),
            message: "hello".to_string(),
            attachments: None,
        };

        let json = serde_json::to_string(&original).unwrap();
        let decoded: ClientMessage = serde_json::from_str(&json).unwrap();

        assert!(matches!(
            decoded,
            ClientMessage::Chat {
                session_id: Some(session_id),
                message,
                attachments: _,
            } if session_id == SessionId::new("session-1") && message == "hello"
        ));
    }

    #[test]
    fn client_message_ping_roundtrip() {
        let original = ClientMessage::Ping;

        let json = serde_json::to_string(&original).unwrap();
        let decoded: ClientMessage = serde_json::from_str(&json).unwrap();

        assert!(matches!(decoded, ClientMessage::Ping));
    }

    #[test]
    fn server_message_chat_response_serializes() {
        let message = ServerMessage::chat_response(
            Some(SessionId::new("session-2")),
            "world".to_string(),
            Some(10),
            Some(20),
        );
        let value = serde_json::to_value(message).unwrap();

        assert_eq!(value["type"], "chat_response");
        assert_eq!(value["session_id"], "session-2");
        assert_eq!(value["content"], "world");
        assert_eq!(value["input_tokens"], 10);
        assert_eq!(value["output_tokens"], 20);
    }

    #[test]
    fn server_message_error_serializes() {
        let message = ServerMessage::error("boom");
        let value = serde_json::to_value(message).unwrap();

        assert_eq!(value["type"], "error");
        assert_eq!(value["message"], "boom");
    }

    #[test]
    fn server_message_connected_includes_version() {
        let message = ServerMessage::connected();
        let value = serde_json::to_value(message).unwrap();

        assert_eq!(value["type"], "connected");
        assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn server_message_to_json_produces_valid_json() {
        let json = ServerMessage::Pong.to_json();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(value["type"], "pong");
    }

    #[test]
    fn companion_caption_message_serializes() {
        let event = CompanionCaptionEvt::new(CompanionCaptionChannel::Assistant, 1, "hello")
            .expect("caption event should build");
        let message = ServerMessage::companion_caption("global", event);
        let value = serde_json::to_value(message).unwrap();

        assert_eq!(value["type"], "companion_caption");
        assert_eq!(value["scope"], "global");
        assert_eq!(value["event"]["sequence"], 1);
    }

    #[test]
    fn companion_widget_message_serializes() {
        let result = CompanionWidgetRuntimeResult {
            action: CompanionAction::Spawn,
            affected_widget_id: Some("weather.panel".to_string()),
            opened_url: None,
            active_widgets: 1,
        };
        let widgets = vec![CompanionWidgetState {
            widget_id: "weather.panel".to_string(),
            payload: json!({"title":"Weather"}),
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
            expires_at: None,
        }];
        let message = ServerMessage::companion_widget("global", result, widgets);
        let value = serde_json::to_value(message).unwrap();

        assert_eq!(value["type"], "companion_widget");
        assert_eq!(value["scope"], "global");
        assert_eq!(value["result"]["action"], "spawn");
        assert_eq!(value["widgets"][0]["widget_id"], "weather.panel");
    }

    #[test]
    fn companion_request_window_message_exposes_scope_accessor() {
        let window = CompanionWindow::new("dangerous_action", Utc::now(), 30)
            .expect("request window should build");
        let message = ServerMessage::companion_request_window("tenant:alpha", "opened", window);
        let value = serde_json::to_value(&message).unwrap();

        assert_eq!(message.companion_scope(), Some("tenant:alpha"));
        assert_eq!(value["type"], "companion_request_window");
        assert_eq!(value["action"], "opened");
    }

    #[test]
    fn companion_context_ingress_message_serializes() {
        let event = CompanionContextIngressEvent {
            session_id: SessionId::new("s1"),
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
        let message = ServerMessage::companion_context_ingress("global", event);
        let value = serde_json::to_value(&message).unwrap();

        assert_eq!(value["type"], "companion_context_ingress");
        assert_eq!(value["scope"], "global");
        assert_eq!(value["event"]["session_id"], "s1");
        assert_eq!(value["event"]["accepted"], true);
    }

    #[test]
    fn companion_multimodal_ingress_message_serializes() {
        let event = CompanionMultimodalIngressEvent {
            record_id: "rec-1".to_string(),
            media_kind: "photo".to_string(),
            source_ref: "camera/frame_1".to_string(),
            accepted: true,
            slot_key: "external.conversation.slot".into(),
            signal_tier: "belief".to_string(),
            reason: None,
        };
        let message = ServerMessage::companion_multimodal_ingress("tenant:alpha", event);
        let value = serde_json::to_value(&message).unwrap();

        assert_eq!(message.companion_scope(), Some("tenant:alpha"));
        assert_eq!(value["type"], "companion_multimodal_ingress");
        assert_eq!(value["event"]["record_id"], "rec-1");
        assert_eq!(value["event"]["media_kind"], "photo");
    }

    #[test]
    fn agent_state_message_serializes() {
        let message = ServerMessage::agent_state("main", "executing", Some("running tool".into()));
        let value = serde_json::to_value(&message).unwrap();

        assert_eq!(value["type"], "agent_state");
        assert_eq!(value["agent_id"], "main");
        assert_eq!(value["state"], "executing");
        assert_eq!(value["detail"], "running tool");
        assert!(value["timestamp"].as_str().is_some());
    }

    #[test]
    fn agent_state_has_no_companion_scope() {
        let message = ServerMessage::agent_state("main", "idle", None);

        assert_eq!(message.companion_scope(), None);
    }
}
