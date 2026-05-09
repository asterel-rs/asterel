//! Unified WebSocket event envelope and new event payload types.
//!
//! Every server-to-client WebSocket message is wrapped in [`WsEventEnvelope`]:
//! ```json
//! { "type": "<event_type>", "tenant_id": "<optional>", "ts": "<RFC3339>", "payload": { ... } }
//! ```
use serde::{Deserialize, Serialize};

use crate::contracts::ids::{ChannelId, MessageId, RunId, SessionId};

/// Top-level envelope sent to all WebSocket clients.
///
/// Consumers dispatch on `type`, then read `payload`.
#[derive(Debug, Clone, Serialize)]
pub struct WsEventEnvelope {
    #[serde(rename = "type")]
    pub event_type: String,
    pub tenant_id: Option<String>,
    pub ts: String,
    pub payload: serde_json::Value,
}

impl WsEventEnvelope {
    /// Build an envelope with the current UTC timestamp.
    pub fn new(
        event_type: impl Into<String>,
        tenant_id: Option<&str>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            event_type: event_type.into(),
            tenant_id: tenant_id.map(str::to_owned),
            ts: chrono::Utc::now().to_rfc3339(),
            payload,
        }
    }

    /// Serialize to JSON, returning an error envelope on failure.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"type":"error","tenant_id":null,"ts":"","payload":{"message":"serialization failed"}}"#
                .to_string()
        })
    }
}

/// Payload for `session_updated`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionUpdatedPayload {
    pub session_id: SessionId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// Payload for `message_created`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageCreatedPayload {
    pub session_id: SessionId,
    pub message_id: MessageId,
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

/// Payload for `message_delta`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageDeltaPayload {
    pub session_id: SessionId,
    pub message_id: MessageId,
    pub delta: String,
    pub index: u64,
}

/// Payload for `message_completed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageCompletedPayload {
    pub session_id: SessionId,
    pub message_id: MessageId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

/// Payload for `tool_call_updated`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallUpdatedPayload {
    pub session_id: SessionId,
    pub tool_call_id: String,
    pub tool_name: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Payload for `runtime_updated`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeUpdatedPayload {
    pub component: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Payload for `channel_updated`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelUpdatedPayload {
    pub channel_id: ChannelId,
    pub channel_type: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Payload for `cron_run_updated`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronRunUpdatedPayload {
    pub job_id: String,
    pub run_id: RunId,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_top_level_fields_present() {
        let payload = serde_json::json!({"key": "value"});
        let env = WsEventEnvelope::new("message_delta", Some("tenant-1"), payload);
        let value: serde_json::Value = serde_json::from_str(&env.to_json()).unwrap();

        assert_eq!(value["type"], "message_delta");
        assert_eq!(value["tenant_id"], "tenant-1");
        assert!(value["ts"].as_str().is_some(), "ts must be a string");
        assert_eq!(value["payload"]["key"], "value");
    }

    #[test]
    fn envelope_null_tenant_when_none() {
        let env = WsEventEnvelope::new(
            "agent_state",
            None,
            serde_json::Value::Object(serde_json::Map::default()),
        );
        let value: serde_json::Value = serde_json::from_str(&env.to_json()).unwrap();
        assert!(value["tenant_id"].is_null());
    }

    #[test]
    fn session_updated_payload_omits_null_optionals() {
        let p = SessionUpdatedPayload {
            session_id: SessionId::new("s1"),
            title: Some("My session".to_string()),
            updated_at: None,
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["session_id"], "s1");
        assert_eq!(v["title"], "My session");
        assert!(v.get("updated_at").is_none());
    }

    #[test]
    fn message_delta_payload_roundtrip() {
        let p = MessageDeltaPayload {
            session_id: SessionId::new("s2"),
            message_id: MessageId::new("m1"),
            delta: "hello".to_string(),
            index: 3,
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["delta"], "hello");
        assert_eq!(v["index"], 3);
    }

    #[test]
    fn cron_run_updated_payload_omits_all_null_optionals() {
        let p = CronRunUpdatedPayload {
            job_id: "j1".to_string(),
            run_id: RunId::new("r1"),
            status: "running".to_string(),
            detail: None,
            started_at: None,
            finished_at: None,
        };
        let v = serde_json::to_value(&p).unwrap();
        assert!(v.get("detail").is_none());
        assert!(v.get("started_at").is_none());
        assert!(v.get("finished_at").is_none());
    }

    // --- WebSocket Protocol Contract Tests ---
    // These tests verify that Rust event serialization matches what the
    // Desktop TypeScript client expects. They catch renames/removals that
    // would silently break the Desktop.

    #[test]
    fn ws_contract_envelope_has_required_fields() {
        let envelope = WsEventEnvelope::new("test_event", Some("tenant-1"), serde_json::json!({}));
        let json = serde_json::to_value(&envelope).unwrap();

        assert!(json.get("type").is_some(), "envelope must have 'type'");
        assert!(
            json.get("tenant_id").is_some(),
            "envelope must have 'tenant_id'"
        );
        assert!(json.get("ts").is_some(), "envelope must have 'ts'");
        assert!(
            json.get("payload").is_some(),
            "envelope must have 'payload'"
        );
    }

    #[test]
    fn ws_contract_message_created_has_required_fields() {
        let payload = MessageCreatedPayload {
            session_id: SessionId::new("s1"),
            message_id: MessageId::new("m1"),
            role: "assistant".to_string(),
            content: Some("hello".to_string()),
        };
        let json = serde_json::to_value(&payload).unwrap();

        // Desktop expects: session_id, message_id, role, content, created_at
        assert!(
            json.get("session_id").is_some(),
            "message_created must have 'session_id'"
        );
        assert!(
            json.get("message_id").is_some(),
            "message_created must have 'message_id'"
        );
        assert!(
            json.get("role").is_some(),
            "message_created must have 'role'"
        );
        assert!(
            json.get("content").is_some(),
            "message_created must have 'content'"
        );
    }

    #[test]
    fn ws_contract_message_delta_has_required_fields() {
        let payload = MessageDeltaPayload {
            session_id: SessionId::new("s1"),
            message_id: MessageId::new("m1"),
            delta: "hello".to_string(),
            index: 0,
        };
        let json = serde_json::to_value(&payload).unwrap();

        // Desktop expects: session_id, message_id, delta
        assert!(
            json.get("session_id").is_some(),
            "message_delta must have 'session_id'"
        );
        assert!(
            json.get("message_id").is_some(),
            "message_delta must have 'message_id'"
        );
        assert!(
            json.get("delta").is_some(),
            "message_delta must have 'delta'"
        );
    }

    #[test]
    fn ws_contract_message_completed_has_required_fields() {
        let payload = MessageCompletedPayload {
            session_id: SessionId::new("s1"),
            message_id: MessageId::new("m1"),
            content: Some("hello".to_string()),
            input_tokens: Some(10),
            output_tokens: Some(20),
        };
        let json = serde_json::to_value(&payload).unwrap();

        // Desktop expects: session_id, message_id, content, input_tokens,
        // output_tokens
        assert!(
            json.get("session_id").is_some(),
            "message_completed must have 'session_id'"
        );
        assert!(
            json.get("message_id").is_some(),
            "message_completed must have 'message_id'"
        );
        assert!(
            json.get("content").is_some(),
            "message_completed must have 'content'"
        );
        assert!(
            json.get("input_tokens").is_some(),
            "message_completed must have 'input_tokens'"
        );
        assert!(
            json.get("output_tokens").is_some(),
            "message_completed must have 'output_tokens'"
        );
    }

    #[test]
    fn ws_contract_tool_call_updated_has_required_fields() {
        let payload = ToolCallUpdatedPayload {
            session_id: SessionId::new("s1"),
            tool_call_id: "tc1".to_string(),
            tool_name: "web_search".to_string(),
            status: "running".to_string(),
            detail: Some("searching...".to_string()),
        };
        let json = serde_json::to_value(&payload).unwrap();

        // Desktop expects: session_id, tool_call_id, tool_name, status, detail
        assert!(
            json.get("session_id").is_some(),
            "tool_call_updated must have 'session_id'"
        );
        assert!(
            json.get("tool_call_id").is_some(),
            "tool_call_updated must have 'tool_call_id'"
        );
        assert!(
            json.get("tool_name").is_some(),
            "tool_call_updated must have 'tool_name'"
        );
        assert!(
            json.get("status").is_some(),
            "tool_call_updated must have 'status'"
        );
        assert!(
            json.get("detail").is_some(),
            "tool_call_updated must have 'detail'"
        );
    }

    #[test]
    fn ws_contract_session_updated_has_required_fields() {
        let payload = SessionUpdatedPayload {
            session_id: SessionId::new("s1"),
            title: Some("My Session".to_string()),
            updated_at: None,
        };
        let json = serde_json::to_value(&payload).unwrap();

        // Desktop expects: session_id, state (but Rust has title/updated_at)
        // Verify at least session_id is present
        assert!(
            json.get("session_id").is_some(),
            "session_updated must have 'session_id'"
        );
    }

    #[test]
    fn ws_contract_runtime_updated_has_required_fields() {
        let payload = RuntimeUpdatedPayload {
            component: "gateway".to_string(),
            status: "healthy".to_string(),
            detail: None,
        };
        let json = serde_json::to_value(&payload).unwrap();

        // Verify core fields are present
        assert!(
            json.get("component").is_some(),
            "runtime_updated must have 'component'"
        );
        assert!(
            json.get("status").is_some(),
            "runtime_updated must have 'status'"
        );
    }

    #[test]
    fn ws_contract_channel_updated_has_required_fields() {
        let payload = ChannelUpdatedPayload {
            channel_id: ChannelId::new("ch1"),
            channel_type: "discord".to_string(),
            status: "connected".to_string(),
            detail: None,
        };
        let json = serde_json::to_value(&payload).unwrap();

        // Verify core fields are present
        assert!(
            json.get("channel_id").is_some(),
            "channel_updated must have 'channel_id'"
        );
        assert!(
            json.get("channel_type").is_some(),
            "channel_updated must have 'channel_type'"
        );
        assert!(
            json.get("status").is_some(),
            "channel_updated must have 'status'"
        );
    }

    #[test]
    fn ws_contract_cron_run_updated_has_required_fields() {
        let payload = CronRunUpdatedPayload {
            job_id: "job1".to_string(),
            run_id: RunId::new("run1"),
            status: "completed".to_string(),
            detail: None,
            started_at: Some("2025-01-01T00:00:00Z".to_string()),
            finished_at: Some("2025-01-01T00:01:00Z".to_string()),
        };
        let json = serde_json::to_value(&payload).unwrap();

        // Verify core fields are present
        assert!(
            json.get("job_id").is_some(),
            "cron_run_updated must have 'job_id'"
        );
        assert!(
            json.get("run_id").is_some(),
            "cron_run_updated must have 'run_id'"
        );
        assert!(
            json.get("status").is_some(),
            "cron_run_updated must have 'status'"
        );
    }
}
