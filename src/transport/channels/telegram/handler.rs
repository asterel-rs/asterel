//! `Channel` trait implementation for Telegram: long-poll listener,
//! update parsing, file-download URL construction, and capability flags.
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use serde_json::Value;
use uuid::Uuid;

use super::TelegramChannel;
use crate::transport::channels::api_request::{
    CHANNEL_API_MAX_RATE_LIMIT_RETRIES, channel_api_error_message, wait_for_rate_limit,
};
use crate::transport::channels::attachments::collect_attachment_response_body;
use crate::transport::channels::traits::{
    Channel, ChannelCapabilities, ChannelEvent, ChannelMessage, MediaAttachment, MediaContent,
};

fn telegram_sender_identity(username: &str, user_id: Option<&str>) -> String {
    user_id
        .filter(|id| !id.trim().is_empty())
        .map_or_else(|| username.to_string(), ToString::to_string)
}

fn telegram_message_id(message: &serde_json::Value) -> Option<String> {
    message
        .get("message_id")
        .and_then(serde_json::Value::as_i64)
        .map(|id| id.to_string())
}

impl TelegramChannel {
    pub(crate) fn telegram_file_url(&self, file_path: &str) -> String {
        format!(
            "https://api.telegram.org/file/bot{}/{}",
            self.bot_token, file_path
        )
    }

    async fn get_file_download_url(&self, file_id: &str) -> Option<String> {
        #[cfg(test)]
        if let Some(file_path) = file_id.strip_prefix("test_file_path:") {
            return Some(self.telegram_file_url(file_path));
        }

        let resp = self
            .client
            .post(self.api_url("getFile"))
            .json(&serde_json::json!({ "file_id": file_id }))
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            return None;
        }

        let data: Value = resp.json().await.ok()?;
        let file_path = data
            .get("result")
            .and_then(|result| result.get("file_path"))
            .and_then(Value::as_str)?;

        Some(self.telegram_file_url(file_path))
    }

    async fn get_file_download_bytes(&self, file_id: &str) -> Option<Vec<u8>> {
        #[cfg(test)]
        if let Some(file_path) = file_id.strip_prefix("test_file_path:") {
            return Some(file_path.as_bytes().to_vec());
        }

        let url = self.get_file_download_url(file_id).await?;
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?;
        collect_attachment_response_body(resp).await.ok()
    }

    pub async fn parse_tg_attachments(&self, message: &Value) -> Vec<MediaAttachment> {
        let mut attachments = Vec::new();

        if let Some(photo_sizes) = message.get("photo").and_then(Value::as_array)
            && let Some(largest) = photo_sizes.last()
            && let Some(file_id) = largest.get("file_id").and_then(Value::as_str)
            && let Some(bytes) = self.get_file_download_bytes(file_id).await
        {
            attachments.push(MediaAttachment {
                mime_type: "image/jpeg".to_string(),
                data: MediaContent::Bytes(bytes),
                filename: None,
            });
        }

        if let Some(document) = message.get("document")
            && let Some(file_id) = document.get("file_id").and_then(Value::as_str)
            && let Some(bytes) = self.get_file_download_bytes(file_id).await
        {
            let mime_type = document
                .get("mime_type")
                .and_then(Value::as_str)
                .unwrap_or("application/octet-stream")
                .to_string();
            let filename = document
                .get("file_name")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            attachments.push(MediaAttachment {
                mime_type,
                data: MediaContent::Bytes(bytes),
                filename,
            });
        }

        if let Some(audio) = message.get("audio")
            && let Some(file_id) = audio.get("file_id").and_then(Value::as_str)
            && let Some(bytes) = self.get_file_download_bytes(file_id).await
        {
            let mime_type = audio
                .get("mime_type")
                .and_then(Value::as_str)
                .unwrap_or("audio/mpeg")
                .to_string();
            let filename = audio
                .get("file_name")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            attachments.push(MediaAttachment {
                mime_type,
                data: MediaContent::Bytes(bytes),
                filename,
            });
        }

        if let Some(voice) = message.get("voice")
            && let Some(file_id) = voice.get("file_id").and_then(Value::as_str)
            && let Some(bytes) = self.get_file_download_bytes(file_id).await
        {
            let mime_type = voice
                .get("mime_type")
                .and_then(Value::as_str)
                .unwrap_or("audio/ogg")
                .to_string();
            attachments.push(MediaAttachment {
                mime_type,
                data: MediaContent::Bytes(bytes),
                filename: Some("voice.ogg".to_string()),
            });
        }

        if let Some(video) = message.get("video")
            && let Some(file_id) = video.get("file_id").and_then(Value::as_str)
            && let Some(bytes) = self.get_file_download_bytes(file_id).await
        {
            let mime_type = video
                .get("mime_type")
                .and_then(Value::as_str)
                .unwrap_or("video/mp4")
                .to_string();
            let filename = video
                .get("file_name")
                .and_then(Value::as_str)
                .map(ToString::to_string);
            attachments.push(MediaAttachment {
                mime_type,
                data: MediaContent::Bytes(bytes),
                filename,
            });
        }

        attachments
    }
}

impl Channel for TelegramChannel {
    fn name(&self) -> &'static str {
        "telegram"
    }

    fn max_message_length(&self) -> usize {
        4096
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            can_send_media: true,
            can_send_typing: true,
            can_edit_message: true,
            can_delete_message: true,
            max_message_length: 4096,
            ..ChannelCapabilities::default()
        }
    }

    fn send<'a>(
        &'a self,
        message: &'a str,
        chat_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "chat_id": chat_id,
                "text": message,
            });

            for attempt in 0..=CHANNEL_API_MAX_RATE_LIMIT_RETRIES {
                let resp = self
                    .client
                    .post(self.api_url("sendMessage"))
                    .json(&body)
                    .send()
                    .await?;

                if resp.status().as_u16() == 429 && attempt < CHANNEL_API_MAX_RATE_LIMIT_RETRIES {
                    wait_for_rate_limit(resp.headers()).await;
                    continue;
                }

                if !resp.status().is_success() {
                    let err = channel_api_error_message("Telegram", "sendMessage", resp).await;
                    anyhow::bail!(err);
                }

                return Ok(());
            }

            anyhow::bail!("Telegram sendMessage failed due to rate limiting")
        })
    }

    #[allow(clippy::too_many_lines)]
    fn listen<'a>(
        &'a self,
        tx: tokio::sync::mpsc::Sender<ChannelEvent>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let mut offset: i64 = 0;

            tracing::info!("Telegram channel listening for messages...");

            loop {
                let url = self.api_url("getUpdates");
                let body = serde_json::json!({
                    "offset": offset,
                    "timeout": 30,
                    "allowed_updates": ["message"]
                });

                let resp = match self.client.post(&url).json(&body).send().await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("Telegram poll error: {e}");
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        continue;
                    }
                };

                let data: serde_json::Value = match resp.json().await {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::warn!("Telegram parse error: {e}");
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        continue;
                    }
                };

                if let Some(results) = data.get("result").and_then(serde_json::Value::as_array) {
                    for update in results {
                        // Advance offset past this update
                        if let Some(uid) =
                            update.get("update_id").and_then(serde_json::Value::as_i64)
                        {
                            offset = uid + 1;
                        }

                        let Some(message) = update.get("message") else {
                            continue;
                        };

                        let username_opt = message
                            .get("from")
                            .and_then(|f| f.get("username"))
                            .and_then(|u| u.as_str());
                        let username = username_opt.unwrap_or("unknown");

                        let user_id = message
                            .get("from")
                            .and_then(|f| f.get("id"))
                            .and_then(serde_json::Value::as_i64);
                        let user_id_str = user_id.map(|id| id.to_string());

                        let mut identities = vec![username];
                        if let Some(ref id) = user_id_str {
                            identities.push(id.as_str());
                        }

                        if !self.is_any_user_allowed(identities.iter().copied()) {
                            tracing::warn!(
                                "Telegram: ignoring message from unauthorized user: username={username}, user_id={}. \
 Allowlist Telegram @username or numeric user ID, then run `asterel onboard --channels-only`.",
                                user_id_str.as_deref().unwrap_or("unknown")
                            );
                            continue;
                        }

                        let text = message
                            .get("text")
                            .and_then(serde_json::Value::as_str)
                            .or_else(|| message.get("caption").and_then(serde_json::Value::as_str))
                            .unwrap_or("");

                        let attachments = self.parse_tg_attachments(message).await;
                        if text.is_empty() && attachments.is_empty() {
                            continue;
                        }

                        let chat_id = message
                            .get("chat")
                            .and_then(|c| c.get("id"))
                            .and_then(serde_json::Value::as_i64)
                            .map(|id| id.to_string())
                            .unwrap_or_default();
                        let message_id = telegram_message_id(message);
                        let sender_identity =
                            telegram_sender_identity(username, user_id_str.as_deref());

                        let msg = ChannelMessage {
                            id: message_id
                                .clone()
                                .unwrap_or_else(|| Uuid::new_v4().to_string()),
                            sender: sender_identity,
                            content: text.to_string(),
                            channel: "telegram".to_string(),
                            context_hint: None,
                            conversation_id: (!chat_id.is_empty()).then_some(chat_id),
                            thread_id: None,
                            reply_to: None,
                            message_id,
                            timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                            attachments,
                        };

                        if tx.send(ChannelEvent::Message(msg)).await.is_err() {
                            return Ok(());
                        }
                    }
                }
            }
        })
    }

    fn health_check<'a>(&'a self) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
        Box::pin(async move {
            self.client
                .get(self.api_url("getMe"))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
        })
    }

    fn send_media<'a>(
        &'a self,
        attachment: &'a MediaAttachment,
        recipient: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let mime = attachment.mime_type.as_str();
            let filename = attachment.filename.as_deref().unwrap_or("attachment");
            let extension = Path::new(filename).extension().and_then(|ext| ext.to_str());
            let is_voice = mime == "audio/ogg"
                || mime == "audio/opus"
                || extension.is_some_and(|ext| ext.eq_ignore_ascii_case("ogg"))
                || extension.is_some_and(|ext| ext.eq_ignore_ascii_case("opus"));

            match &attachment.data {
                MediaContent::Url(url) => {
                    if mime.starts_with("image/") {
                        self.send_photo_by_url(recipient, url, None).await
                    } else if mime.starts_with("audio/") {
                        if is_voice {
                            self.send_voice_by_url(recipient, url, None).await
                        } else {
                            self.send_audio_by_url(recipient, url, None).await
                        }
                    } else if mime.starts_with("video/") {
                        self.send_video_by_url(recipient, url, None).await
                    } else {
                        self.send_document_by_url(recipient, url, None).await
                    }
                }
                MediaContent::Bytes(bytes) => {
                    if mime.starts_with("image/") {
                        self.send_photo_bytes(recipient, bytes.clone(), filename, None)
                            .await
                    } else if mime.starts_with("audio/") {
                        if is_voice {
                            self.send_voice_bytes(recipient, bytes.clone(), filename, None)
                                .await
                        } else {
                            self.send_audio_bytes(recipient, bytes.clone(), filename, None)
                                .await
                        }
                    } else if mime.starts_with("video/") {
                        self.send_video_bytes(recipient, bytes.clone(), filename, None)
                            .await
                    } else {
                        self.send_document_bytes(recipient, bytes.clone(), filename, None)
                            .await
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod identity_tests {
    use super::{telegram_message_id, telegram_sender_identity};

    #[test]
    fn telegram_sender_identity_prefers_numeric_user_id() {
        assert_eq!(telegram_sender_identity("alice", Some("12345")), "12345");
    }

    #[test]
    fn telegram_sender_identity_falls_back_to_username() {
        assert_eq!(telegram_sender_identity("alice", None), "alice");
    }

    #[test]
    fn telegram_message_id_reads_platform_message_id() {
        let message = serde_json::json!({"message_id": 77});
        assert_eq!(telegram_message_id(&message).as_deref(), Some("77"));
    }
}
