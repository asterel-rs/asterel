//! `WhatsApp` Business Cloud API channel — **[PARTIAL]** capability.
//!
//! # Capability Status: PARTIAL
//!
//! | Surface              | Status    | Notes                                               |
//! |----------------------|-----------|-----------------------------------------------------|
//! | Webhook ingest       | Partial   | Text messages only; non-text (image/audio/etc) skip |
//! | Send API             | Working   | Text messages via Cloud API v21.0                   |
//! | `Channel::listen`    | No-op     | Webhook push-based; listen just keeps task alive    |
//! | Non-text receive     | Skipped   | Images, audio, stickers, etc. are silently dropped  |
//! | Phone allowlist      | Working   | E.164 format; wildcard `"*"` allows all numbers     |
//! | Approval broker      | Missing   | No `WhatsAppApprovalBroker`; falls to CLI fallback  |
//!
//! # Architecture: Ingest vs Send
//!
//! `WhatsApp` operates in push mode (webhooks from Meta), not polling mode.
//! The two concerns are kept separate in this file:
//!
//! - **Webhook ingest** (`parse_webhook`): converts Meta JSON payloads to
//!   `ChannelMessage` values. Called by the gateway's `/whatsapp` route.
//!   Only text messages (`msg.type == "text"`) produce a `ChannelMessage`;
//!   all other message types are logged at `DEBUG` and skipped.
//!
//! - **Send API** (`Channel::send`): posts outbound text messages to the
//!   `WhatsApp` Cloud API (`/{version}/{phone_number_id}/messages`).
//!
//! `Channel::listen` is intentionally a no-op loop — it keeps the channel
//! task alive so the runtime does not treat this channel as crashed, but
//! message delivery happens entirely through the gateway webhook path.
//!
//! # Known Gaps (Watchlist)
//!
//! - Non-text receive (images, audio, video, documents, stickers, reactions)
//!   is not implemented; messages are dropped with a debug log.
//! - No read-receipt or delivery-receipt handling.
//! - No `WhatsAppApprovalBroker`; tool approval requests on this channel
//!   fall through to `CliApprovalBroker` (operator terminal) or auto-deny.
//! - Outbound media (image/document send) not implemented.
//! - No status webhook processing (delivered/read callbacks from Meta).

use std::future::Future;
use std::pin::Pin;

use uuid::Uuid;

use super::super::traits::{Channel, ChannelEvent, ChannelMessage};
use crate::transport::channels::api_request::{
    CHANNEL_API_MAX_RATE_LIMIT_RETRIES, channel_api_error_message, wait_for_rate_limit,
};

/// `WhatsApp` Cloud API version. Update when Meta deprecates older versions.
const WHATSAPP_API_VERSION: &str = "v21.0";

/// `WhatsApp` channel — uses `WhatsApp` Business Cloud API.
///
/// **[PARTIAL]**: webhook ingest (text-only) and outbound text send are
/// functional. Non-text receive, media send, read receipts, and an
/// interactive approval broker are not yet implemented. See module doc for
/// the full capability status table.
pub struct WhatsAppChannel {
    access_token: String,
    phone_number_id: String,
    verify_token: String,
    allowed_numbers: Vec<String>,
    client: reqwest::Client,
    api_base_url: String,
}

impl WhatsAppChannel {
    #[must_use]
    pub fn new(
        access_token: String,
        phone_number_id: String,
        verify_token: String,
        allowed_numbers: Vec<String>,
    ) -> Self {
        Self {
            access_token,
            phone_number_id,
            verify_token,
            allowed_numbers,
            client: crate::utils::http::build_http_client(),
            api_base_url: "https://graph.facebook.com".to_string(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_api_base_url(
        access_token: String,
        phone_number_id: String,
        verify_token: String,
        allowed_numbers: Vec<String>,
        api_base_url: String,
    ) -> Self {
        Self {
            access_token,
            phone_number_id,
            verify_token,
            allowed_numbers,
            client: crate::utils::http::build_http_client(),
            api_base_url,
        }
    }

    fn api_url(&self, suffix: &str) -> String {
        format!(
            "{}/{}",
            self.api_base_url.trim_end_matches('/'),
            suffix.trim_start_matches('/')
        )
    }

    /// Check if a phone number is allowed (E.164 format: +1234567890)
    pub(super) fn is_number_allowed(&self, phone: &str) -> bool {
        self.allowed_numbers.iter().any(|n| n == "*" || n == phone)
    }

    /// Get the verify token for webhook verification
    #[must_use]
    pub fn verify_token(&self) -> &str {
        &self.verify_token
    }

    /// **[Webhook Ingest]** Parse an incoming Meta webhook payload into `ChannelMessage` values.
    ///
    /// Only `type == "text"` messages produce output. Images, audio, video,
    /// stickers, reactions, and other non-text types are dropped with a debug log.
    /// Called by the gateway `/whatsapp` route — not by `Channel::listen`.
    pub fn parse_webhook(&self, payload: &serde_json::Value) -> Vec<ChannelMessage> {
        let mut messages = Vec::new();

        // WhatsApp Cloud API webhook structure:
        // { "object": "whatsapp_business_account", "entry": [...] }
        let Some(entries) = payload.get("entry").and_then(|e| e.as_array()) else {
            return messages;
        };

        for entry in entries {
            let Some(changes) = entry.get("changes").and_then(|c| c.as_array()) else {
                continue;
            };

            for change in changes {
                let Some(value) = change.get("value") else {
                    continue;
                };

                let Some(msgs) = value.get("messages").and_then(|m| m.as_array()) else {
                    continue;
                };

                for msg in msgs {
                    let Some(from) = msg.get("from").and_then(|f| f.as_str()) else {
                        continue;
                    };

                    let normalized_from = if from.starts_with('+') {
                        from.to_string()
                    } else {
                        format!("+{from}")
                    };

                    if !self.is_number_allowed(&normalized_from) {
                        tracing::warn!(
                            "WhatsApp: ignoring message from unauthorized number: {normalized_from}. \
    Add to allowed_numbers in config.toml, then run `asterel onboard --channels-only`."
                        );
                        continue;
                    }

                    // Extract text content (support text messages only for now)
                    let content = if let Some(text_obj) = msg.get("text") {
                        text_obj
                            .get("body")
                            .and_then(|b| b.as_str())
                            .unwrap_or("")
                            .to_string()
                    } else {
                        // Could be image, audio, etc. — skip for now
                        tracing::debug!("WhatsApp: skipping non-text message from {from}");
                        continue;
                    };

                    if content.is_empty() {
                        continue;
                    }

                    let timestamp = msg
                        .get("timestamp")
                        .and_then(|t| t.as_str())
                        .and_then(|t| t.parse::<u64>().ok())
                        .unwrap_or_else(|| {
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs()
                        });
                    let message_id = msg.get("id").and_then(|id| id.as_str()).map(str::to_string);

                    messages.push(ChannelMessage {
                        id: message_id
                            .clone()
                            .unwrap_or_else(|| Uuid::new_v4().to_string()),
                        sender: normalized_from.clone(),
                        content,
                        channel: "whatsapp".to_string(),
                        context_hint: None,
                        conversation_id: Some(normalized_from),
                        thread_id: None,
                        reply_to: None,
                        message_id,
                        timestamp,
                        attachments: Vec::new(),
                    });
                }
            }
        }

        messages
    }
}

impl Channel for WhatsAppChannel {
    fn name(&self) -> &'static str {
        "whatsapp"
    }

    fn max_message_length(&self) -> usize {
        4096
    }

    fn send<'a>(
        &'a self,
        message: &'a str,
        recipient: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            // [Send API] POST text message to WhatsApp Cloud API.
            // Only plain text is supported; media/template send is not implemented.
            let url = self.api_url(&format!(
                "{WHATSAPP_API_VERSION}/{}/messages",
                self.phone_number_id
            ));

            // Normalize recipient (remove leading + if present for API)
            let to = recipient.strip_prefix('+').unwrap_or(recipient);

            let body = serde_json::json!({
                "messaging_product": "whatsapp",
                "recipient_type": "individual",
                "to": to,
                "type": "text",
                "text": {
                    "preview_url": false,
                    "body": message
                }
            });

            for attempt in 0..=CHANNEL_API_MAX_RATE_LIMIT_RETRIES {
                let resp = self
                    .client
                    .post(&url)
                    .header("Authorization", format!("Bearer {}", self.access_token))
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .await?;

                if resp.status().as_u16() == 429 && attempt < CHANNEL_API_MAX_RATE_LIMIT_RETRIES {
                    wait_for_rate_limit(resp.headers()).await;
                    continue;
                }

                if !resp.status().is_success() {
                    let err = channel_api_error_message("WhatsApp", "send message", resp).await;
                    tracing::error!("{err}");
                    anyhow::bail!(err);
                }

                return Ok(());
            }

            anyhow::bail!("WhatsApp send message failed due to rate limiting")
        })
    }

    fn listen<'a>(
        &'a self,
        _tx: tokio::sync::mpsc::Sender<ChannelEvent>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            // [PARTIAL — No-op listener] WhatsApp is push-based (Meta webhook).
            // Ingest goes through `parse_webhook` called by the gateway /whatsapp route.
            // This loop keeps the channel task alive without polling.
            tracing::info!(
                "WhatsApp channel active (webhook mode — no-op listen). \
                Ingest: gateway POST /whatsapp -> parse_webhook. \
                Configure Meta webhook to POST to your gateway's /whatsapp endpoint."
            );

            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            }
        })
    }

    fn health_check<'a>(&'a self) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
        Box::pin(async move {
            // Check if we can reach the WhatsApp API
            let url = self.api_url(&format!("{WHATSAPP_API_VERSION}/{}", self.phone_number_id));

            self.client
                .get(&url)
                .header("Authorization", format!("Bearer {}", self.access_token))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
        })
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    async fn spawn_http_sequence(responses: Vec<String>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test HTTP server");
        let addr = listener.local_addr().expect("server addr");

        tokio::spawn(async move {
            for response in responses {
                let (mut stream, _) = listener.accept().await.expect("accept request");
                let mut buffer = [0_u8; 2048];
                let _ = stream.read(&mut buffer).await;
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write response");
            }
        });

        format!("http://{addr}")
    }

    fn http_json_response(status: &str, extra_headers: &[(&str, &str)], body: &str) -> String {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in extra_headers {
            response.push_str(name);
            response.push_str(": ");
            response.push_str(value);
            response.push_str("\r\n");
        }
        response.push_str("\r\n");
        response.push_str(body);
        response
    }

    #[tokio::test]
    async fn whatsapp_send_retries_429_retry_after_before_success() {
        let base_url = spawn_http_sequence(vec![
            http_json_response(
                "429 Too Many Requests",
                &[("Retry-After", "0")],
                r#"{"error":{"message":"wait"}}"#,
            ),
            http_json_response("200 OK", &[], r#"{"messages":[{"id":"wamid.1"}]}"#),
        ])
        .await;
        let ch = WhatsAppChannel::new_with_api_base_url(
            "token".into(),
            "phone".into(),
            "verify".into(),
            vec!["*".into()],
            base_url,
        );

        ch.send("hello", "+15551234567")
            .await
            .expect("second send attempt should succeed");
    }

    #[tokio::test]
    async fn whatsapp_send_error_sanitizes_provider_body() {
        let base_url = spawn_http_sequence(vec![http_json_response(
            "400 Bad Request",
            &[],
            r#"{"error":{"message":"bad token sk-whatsapp-secret-value"}}"#,
        )])
        .await;
        let ch = WhatsAppChannel::new_with_api_base_url(
            "token".into(),
            "phone".into(),
            "verify".into(),
            vec!["*".into()],
            base_url,
        );

        let err = ch
            .send("hello", "+15551234567")
            .await
            .expect_err("provider failure should be returned");
        let message = err.to_string();

        assert!(message.contains("WhatsApp send message failed"));
        assert!(!message.contains("sk-whatsapp-secret-value"));
        assert!(message.contains("[REDACTED]"));
    }
}
