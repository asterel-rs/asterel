//! Matrix Client-Server API channel adapter: `/sync` polling, room
//! event parsing, media download, and message sending.
mod media;
mod models;

#[cfg(test)]
mod tests;

use std::future::Future;
use std::pin::Pin;

use anyhow::Context;
use reqwest::Client;
use tokio::sync::mpsc;

use self::models::{EventContent, SyncResponse, TimelineEvent, WhoAmIResponse};
use crate::contracts::ids::UserId;
use crate::security::scrub::sanitize_api_error;
use crate::transport::channels::attachments::load_attachment_bytes;
use crate::transport::channels::policy::{AllowlistMatch, is_allowed_user};
use crate::transport::channels::traits::{Channel, ChannelEvent, ChannelMessage, MediaAttachment};

/// Matrix channel using the Client-Server API (no SDK needed).
/// Connects to any Matrix homeserver (Element, Synapse, etc.).
#[derive(Clone)]
pub struct MatrixChannel {
    homeserver: String,
    access_token: String,
    room_id: String,
    allowed_users: Vec<String>,
    client: Client,
}

impl MatrixChannel {
    #[must_use]
    pub fn new(
        homeserver: String,
        access_token: String,
        room_id: String,
        allowed_users: Vec<String>,
    ) -> Self {
        let homeserver = if homeserver.ends_with('/') {
            homeserver[..homeserver.len() - 1].to_string()
        } else {
            homeserver
        };
        Self {
            homeserver,
            access_token,
            room_id,
            allowed_users,
            client: crate::utils::http::build_http_client(),
        }
    }

    fn bearer_header(&self) -> String {
        format!("Bearer {}", self.access_token)
    }

    fn is_user_allowed(&self, sender: &str) -> bool {
        is_allowed_user(
            &self.allowed_users,
            sender,
            AllowlistMatch::AsciiCaseInsensitive,
        )
    }

    async fn get_my_user_id(&self) -> anyhow::Result<UserId> {
        let url = format!("{}/_matrix/client/v3/account/whoami", self.homeserver);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", self.bearer_header())
            .send()
            .await
            .context("send Matrix whoami request")?;

        if !resp.status().is_success() {
            let err = resp.text().await?;
            anyhow::bail!("Matrix whoami failed: {err}");
        }

        let who: WhoAmIResponse = resp.json().await.context("parse Matrix whoami response")?;
        Ok(who.user_id)
    }

    #[cfg(test)]
    fn mxc_to_http(&self, mxc_url: &str) -> Option<String> {
        media::mxc_to_http(&self.homeserver, mxc_url)
    }

    fn parse_media_attachments(&self, content: &EventContent) -> Vec<MediaAttachment> {
        media::parse_media_attachments(&self.homeserver, content)
    }

    fn process_event(&self, event: &TimelineEvent, my_user_id: &UserId) -> Option<ChannelMessage> {
        if event.sender == my_user_id.as_str() {
            return None;
        }
        if event.event_type != "m.room.message" {
            return None;
        }
        let msgtype = event.content.msgtype.as_deref().unwrap_or("");
        if !matches!(
            msgtype,
            "m.text" | "m.image" | "m.audio" | "m.video" | "m.file"
        ) {
            return None;
        }
        if !self.is_user_allowed(&event.sender) {
            return None;
        }
        let attachments = self.parse_media_attachments(&event.content);
        let body = event.content.body.clone().unwrap_or_default();
        if body.is_empty() && attachments.is_empty() {
            return None;
        }
        Some(ChannelMessage {
            id: event
                .event_id
                .clone()
                .unwrap_or_else(|| format!("mx_{}", chrono::Utc::now().timestamp_millis())),
            sender: event.sender.clone(),
            content: body,
            channel: "matrix".to_string(),
            context_hint: None,
            conversation_id: Some(self.room_id.clone()),
            thread_id: None,
            reply_to: None,
            message_id: event.event_id.clone(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            attachments,
        })
    }
}

impl Channel for MatrixChannel {
    fn name(&self) -> &'static str {
        "matrix"
    }

    fn max_message_length(&self) -> usize {
        60_000
    }

    fn send<'a>(
        &'a self,
        message: &'a str,
        _target: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            // Each send gets a unique txn_id so that identical messages can
            // be delivered more than once (avoiding content-based dedup).
            let txn_id = unique_txn_id(&self.room_id, message);
            let url = format!(
                "{}/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
                self.homeserver, self.room_id, txn_id
            );

            let body = serde_json::json!({
                "msgtype": "m.text",
                "body": message
            });

            let resp = self
                .client
                .put(&url)
                .header("Authorization", self.bearer_header())
                .json(&body)
                .send()
                .await
                .context("send Matrix room message")?;

            if !resp.status().is_success() {
                let err = resp.text().await?;
                anyhow::bail!("Matrix send failed: {err}");
            }

            Ok(())
        })
    }

    fn listen<'a>(
        &'a self,
        tx: mpsc::Sender<ChannelEvent>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            tracing::info!("Matrix channel listening on room {}...", self.room_id);

            let my_user_id = self
                .get_my_user_id()
                .await
                .context("get Matrix user identity")?;

            // Initial sync to get the since token
            let url = format!(
                "{}/_matrix/client/v3/sync?timeout=30000&filter={{\"room\":{{\"timeline\":{{\"limit\":1}}}}}}",
                self.homeserver
            );

            let resp = self
                .client
                .get(&url)
                .header("Authorization", self.bearer_header())
                .send()
                .await
                .context("send Matrix initial sync request")?;

            if !resp.status().is_success() {
                let err = resp.text().await?;
                anyhow::bail!("Matrix initial sync failed: {err}");
            }

            let sync: SyncResponse = resp
                .json()
                .await
                .context("parse Matrix initial sync response")?;
            let mut since = sync.next_batch;

            // Long-poll loop
            loop {
                let url = format!(
                    "{}/_matrix/client/v3/sync?since={}&timeout=30000",
                    self.homeserver, since
                );

                let resp = self
                    .client
                    .get(&url)
                    .header("Authorization", self.bearer_header())
                    .send()
                    .await;

                let resp = match resp {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("Matrix sync error: {e}, retrying...");
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        continue;
                    }
                };

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "<failed to read body>".into());
                    tracing::warn!(
                        %status,
                        body = body.chars().take(500).collect::<String>(),
                        "Matrix sync returned non-success status, retrying..."
                    );
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    continue;
                }

                let sync: SyncResponse = match resp.json().await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("Matrix sync JSON parse failed: {e}, retrying...");
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                        continue;
                    }
                };
                since = sync.next_batch;

                // Process events from our room
                if let Some(room) = sync.rooms.join.get(&self.room_id) {
                    for event in &room.timeline.events {
                        if let Some(msg) = self.process_event(event, &my_user_id)
                            && tx.send(ChannelEvent::Message(msg)).await.is_err()
                        {
                            return Ok(());
                        }
                    }
                }
            }
        })
    }

    fn health_check<'a>(&'a self) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
        Box::pin(async move {
            let url = format!("{}/_matrix/client/v3/account/whoami", self.homeserver);
            let Ok(resp) = self
                .client
                .get(&url)
                .header("Authorization", self.bearer_header())
                .send()
                .await
            else {
                return false;
            };

            resp.status().is_success()
        })
    }

    fn send_media<'a>(
        &'a self,
        attachment: &'a MediaAttachment,
        _recipient: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let bytes = load_attachment_bytes(attachment)
                .await
                .context("load Matrix media before upload")?;

            let upload_url = format!("{}/_matrix/media/v3/upload", self.homeserver);
            let upload_resp = self
                .client
                .post(&upload_url)
                .header("Authorization", self.bearer_header())
                .header("Content-Type", attachment.mime_type.clone())
                .body(bytes.clone())
                .send()
                .await
                .context("send Matrix upload request")?;

            if !upload_resp.status().is_success() {
                let err = sanitize_api_error(&upload_resp.text().await?);
                anyhow::bail!("Matrix media upload failed: {err}");
            }

            let upload_data: serde_json::Value = upload_resp
                .json()
                .await
                .context("parse Matrix upload response")?;
            let Some(content_uri) = upload_data
                .get("content_uri")
                .and_then(serde_json::Value::as_str)
            else {
                anyhow::bail!("Matrix upload response missing content_uri");
            };

            let txn_id = unique_txn_id(&self.room_id, content_uri);
            let send_url = format!(
                "{}/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
                self.homeserver, self.room_id, txn_id
            );
            let filename = attachment
                .filename
                .clone()
                .unwrap_or_else(|| "attachment".to_string());
            let msgtype = if attachment.mime_type.starts_with("image/") {
                "m.image"
            } else if attachment.mime_type.starts_with("audio/") {
                "m.audio"
            } else if attachment.mime_type.starts_with("video/") {
                "m.video"
            } else {
                "m.file"
            };

            let body = serde_json::json!({
                "msgtype": msgtype,
                "body": filename,
                "url": content_uri,
                "info": {
                    "mimetype": attachment.mime_type,
                    "size": bytes.len()
                }
            });

            let send_resp = self
                .client
                .put(&send_url)
                .header("Authorization", self.bearer_header())
                .json(&body)
                .send()
                .await
                .context("send Matrix media message")?;

            if !send_resp.status().is_success() {
                let err = sanitize_api_error(&send_resp.text().await?);
                anyhow::bail!("Matrix send media failed: {err}");
            }

            Ok(())
        })
    }
}

/// Generate a unique `txn_id` for each message send attempt.
///
/// Includes a monotonic counter and wall-clock timestamp alongside the
/// content hash so that identical messages can be sent more than once
/// (each send gets a distinct `txn_id`), while retries within the same
/// process reuse the same ID only when they share the same counter value.
fn unique_txn_id(room_id: &str, content: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};

    use sha2::{Digest, Sha256};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let mut hasher = Sha256::new();
    hasher.update(room_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(content.as_bytes());
    hasher.update(b"\0");
    hasher.update(ts.to_le_bytes());
    hasher.update(seq.to_le_bytes());
    let hash = hasher.finalize();
    format!("zc_{}", hex::encode(&hash[..16]))
}
