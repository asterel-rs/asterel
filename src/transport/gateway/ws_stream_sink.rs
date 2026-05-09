use std::future::Future;
use std::pin::Pin;

use tokio::sync::{Mutex, mpsc};

use super::ws_events::{
    MessageCompletedPayload, MessageCreatedPayload, MessageDeltaPayload, ToolCallUpdatedPayload,
};
use crate::contracts::ids::{MessageId, SessionId};
use crate::core::providers::streaming::{StreamEvent, StreamSink};

pub struct WebSocketStreamSink {
    tx: mpsc::Sender<String>,
    session_id: SessionId,
    message_id: MessageId,
    tenant_id: Option<String>,
    state: Mutex<WebSocketStreamSinkState>,
}

#[derive(Default)]
struct WebSocketStreamSinkState {
    created_sent: bool,
    next_delta_index: u64,
}

impl WebSocketStreamSink {
    #[must_use]
    pub fn new(
        tx: mpsc::Sender<String>,
        session_id: SessionId,
        message_id: MessageId,
        tenant_id: Option<String>,
    ) -> Self {
        Self {
            tx,
            session_id,
            message_id,
            tenant_id,
            state: Mutex::new(WebSocketStreamSinkState::default()),
        }
    }

    fn build_created_payload(&self) -> MessageCreatedPayload {
        MessageCreatedPayload {
            session_id: self.session_id.clone(),
            message_id: self.message_id.clone(),
            role: "assistant".to_string(),
            content: Some(String::new()),
        }
    }

    async fn send_created_if_needed(&self) {
        let should_send = {
            let mut state = self.state.lock().await;
            if state.created_sent {
                false
            } else {
                state.created_sent = true;
                true
            }
        };

        if should_send {
            let payload = self.build_created_payload();
            self.send_payload("message_created", &payload).await;
        }
    }

    async fn next_delta_index(&self) -> u64 {
        let mut state = self.state.lock().await;
        let index = state.next_delta_index;
        state.next_delta_index = state.next_delta_index.saturating_add(1);
        index
    }

    async fn send_payload<T>(&self, event_type: &str, payload: &T)
    where
        T: serde::Serialize,
    {
        #[derive(serde::Serialize)]
        struct BorrowedEnvelope<'a, T> {
            #[serde(rename = "type")]
            event_type: &'a str,
            tenant_id: Option<&'a str>,
            ts: String,
            payload: &'a T,
        }

        let envelope = BorrowedEnvelope {
            event_type,
            tenant_id: self.tenant_id.as_deref(),
            ts: chrono::Utc::now().to_rfc3339(),
            payload,
        };
        let raw = match serde_json::to_string(&envelope) {
            Ok(raw) => raw,
            Err(error) => {
                tracing::warn!(%error, event_type, "failed to serialize websocket stream payload");
                return;
            }
        };
        if self.tx.send(raw).await.is_err() {
            tracing::warn!(event_type, "websocket stream sink receiver closed");
        }
    }
}

impl StreamSink for WebSocketStreamSink {
    fn on_event<'a>(
        &'a self,
        event: &'a StreamEvent,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            match event {
                StreamEvent::ResponseStart { .. } => {
                    self.send_created_if_needed().await;
                }
                StreamEvent::TextDelta { text } => {
                    self.send_created_if_needed().await;
                    let payload = MessageDeltaPayload {
                        session_id: self.session_id.clone(),
                        message_id: self.message_id.clone(),
                        delta: text.clone(),
                        index: self.next_delta_index().await,
                    };
                    self.send_payload("message_delta", &payload).await;
                }
                StreamEvent::ToolCallComplete { id, name, .. } => {
                    let payload = ToolCallUpdatedPayload {
                        session_id: self.session_id.clone(),
                        tool_call_id: id.clone(),
                        tool_name: name.clone(),
                        status: "completed".to_string(),
                        detail: None,
                    };
                    self.send_payload("tool_call_updated", &payload).await;
                }
                StreamEvent::Done {
                    input_tokens,
                    output_tokens,
                    ..
                } => {
                    self.send_created_if_needed().await;
                    let payload = MessageCompletedPayload {
                        session_id: self.session_id.clone(),
                        message_id: self.message_id.clone(),
                        content: None,
                        input_tokens: *input_tokens,
                        output_tokens: *output_tokens,
                    };
                    self.send_payload("message_completed", &payload).await;
                }
                StreamEvent::ToolCallDelta { .. } => {}
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::providers::response::StopReason;

    fn parse_envelope(raw: &str) -> serde_json::Value {
        serde_json::from_str(raw).expect("envelope should be valid JSON")
    }

    fn make_sink(tx: mpsc::Sender<String>) -> WebSocketStreamSink {
        WebSocketStreamSink::new(
            tx,
            SessionId::new("session-1"),
            MessageId::new("message-1"),
            Some("tenant-a".to_string()),
        )
    }

    #[tokio::test]
    async fn emits_full_streaming_event_sequence() {
        let (tx, mut rx) = mpsc::channel(16);
        let sink = make_sink(tx);

        sink.on_event(&StreamEvent::ResponseStart { model: None })
            .await;
        sink.on_event(&StreamEvent::TextDelta {
            text: "hello".to_string(),
        })
        .await;
        sink.on_event(&StreamEvent::ToolCallComplete {
            id: "tool-1".to_string(),
            name: "bash".to_string(),
            input: serde_json::json!({"command": "pwd"}),
        })
        .await;
        sink.on_event(&StreamEvent::Done {
            stop_reason: Some(StopReason::EndTurn),
            input_tokens: Some(1),
            output_tokens: Some(2),
        })
        .await;

        let created = parse_envelope(&rx.recv().await.expect("created"));
        let delta = parse_envelope(&rx.recv().await.expect("delta"));
        let tool = parse_envelope(&rx.recv().await.expect("tool"));
        let completed = parse_envelope(&rx.recv().await.expect("completed"));

        assert_eq!(created["type"], "message_created");
        assert_eq!(created["tenant_id"], "tenant-a");
        assert_eq!(created["payload"]["role"], "assistant");
        assert_eq!(created["payload"]["session_id"], "session-1");
        assert_eq!(created["payload"]["message_id"], "message-1");

        assert_eq!(delta["type"], "message_delta");
        assert_eq!(delta["payload"]["delta"], "hello");
        assert_eq!(delta["payload"]["index"], 0);

        assert_eq!(tool["type"], "tool_call_updated");
        assert_eq!(tool["payload"]["tool_call_id"], "tool-1");
        assert_eq!(tool["payload"]["tool_name"], "bash");
        assert_eq!(tool["payload"]["status"], "completed");

        assert_eq!(completed["type"], "message_completed");
        assert_eq!(completed["payload"]["input_tokens"], 1);
        assert_eq!(completed["payload"]["output_tokens"], 2);
    }

    #[tokio::test]
    async fn message_created_sent_exactly_once_across_multiple_deltas() {
        let (tx, mut rx) = mpsc::channel(16);
        let sink = make_sink(tx);

        sink.on_event(&StreamEvent::TextDelta {
            text: "one".to_string(),
        })
        .await;
        sink.on_event(&StreamEvent::TextDelta {
            text: "two".to_string(),
        })
        .await;
        sink.on_event(&StreamEvent::TextDelta {
            text: "three".to_string(),
        })
        .await;
        sink.on_event(&StreamEvent::Done {
            stop_reason: Some(StopReason::EndTurn),
            input_tokens: None,
            output_tokens: None,
        })
        .await;

        let mut types = Vec::new();
        while let Ok(json) = rx.try_recv() {
            types.push(parse_envelope(&json)["type"].as_str().unwrap().to_string());
        }

        assert_eq!(
            types,
            vec![
                "message_created",
                "message_delta",
                "message_delta",
                "message_delta",
                "message_completed",
            ]
        );
    }

    #[tokio::test]
    async fn delta_indices_increment_sequentially() {
        let (tx, mut rx) = mpsc::channel(16);
        let sink = make_sink(tx);

        for _ in 0..4 {
            sink.on_event(&StreamEvent::TextDelta {
                text: "x".to_string(),
            })
            .await;
        }
        sink.on_event(&StreamEvent::Done {
            stop_reason: Some(StopReason::EndTurn),
            input_tokens: None,
            output_tokens: None,
        })
        .await;

        let _ = rx.recv().await;

        let mut indices = Vec::new();
        for _ in 0..4 {
            let env = parse_envelope(&rx.recv().await.expect("delta"));
            indices.push(env["payload"]["index"].as_u64().expect("index"));
        }
        assert_eq!(indices, vec![0, 1, 2, 3]);
    }

    #[tokio::test]
    async fn null_tenant_id_serializes_as_json_null() {
        let (tx, mut rx) = mpsc::channel(8);
        let sink = WebSocketStreamSink::new(tx, SessionId::new("s1"), MessageId::new("m1"), None);

        sink.on_event(&StreamEvent::ResponseStart { model: None })
            .await;

        let env = parse_envelope(&rx.recv().await.expect("created"));
        assert!(env["tenant_id"].is_null());
    }

    #[tokio::test]
    async fn tool_call_delta_produces_no_output() {
        let (tx, mut rx) = mpsc::channel(8);
        let sink = make_sink(tx);

        sink.on_event(&StreamEvent::ToolCallDelta {
            index: 0,
            id: Some("tc-1".to_string()),
            name: Some("shell".to_string()),
            input_json_delta: r#"{"cmd":"ls"}"#.to_string(),
        })
        .await;
        sink.on_event(&StreamEvent::Done {
            stop_reason: Some(StopReason::EndTurn),
            input_tokens: None,
            output_tokens: None,
        })
        .await;

        let first = parse_envelope(&rx.recv().await.expect("created"));
        let second = parse_envelope(&rx.recv().await.expect("completed"));
        assert_eq!(first["type"], "message_created");
        assert_eq!(second["type"], "message_completed");
        assert!(rx.try_recv().is_err(), "no extra messages");
    }

    #[tokio::test]
    async fn closed_receiver_does_not_panic() {
        let (tx, rx) = mpsc::channel(1);
        let sink = make_sink(tx);
        drop(rx);

        sink.on_event(&StreamEvent::TextDelta {
            text: "orphaned".to_string(),
        })
        .await;
        sink.on_event(&StreamEvent::Done {
            stop_reason: Some(StopReason::EndTurn),
            input_tokens: None,
            output_tokens: None,
        })
        .await;
    }

    #[tokio::test]
    async fn done_without_tokens_serializes_null_usage() {
        let (tx, mut rx) = mpsc::channel(8);
        let sink = make_sink(tx);

        sink.on_event(&StreamEvent::Done {
            stop_reason: Some(StopReason::EndTurn),
            input_tokens: None,
            output_tokens: None,
        })
        .await;

        let _ = rx.recv().await;
        let completed = parse_envelope(&rx.recv().await.expect("completed"));
        assert!(completed["payload"].get("input_tokens").is_none());
        assert!(completed["payload"].get("output_tokens").is_none());
    }
}
