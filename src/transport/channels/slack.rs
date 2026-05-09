//! Slack channel adapter: polls `conversations.history` via the Web API,
//! sends replies, and handles media attachments and user allowlisting.
use std::future::Future;
use std::pin::Pin;

use anyhow::Context;
use futures_util::StreamExt;
use reqwest::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use super::attachments::media_attachment_url;
use super::traits::{
    Channel, ChannelCapabilities, ChannelEvent, ChannelMessage, MediaAttachment, MediaContent,
};
use crate::transport::channels::policy::{AllowlistMatch, is_allowed_user};

const SLACK_MEDIA_MAX_BYTES: u64 = 10 * 1024 * 1024;
const SLACK_MEDIA_MAX_REDIRECTS: usize = 5;

/// Slack channel — polls conversations.history via Web API
pub struct SlackChannel {
    bot_token: String,
    channel_id: Option<String>,
    allowed_users: Vec<String>,
    client: reqwest::Client,
}

impl SlackChannel {
    #[must_use]
    pub fn new(bot_token: String, channel_id: Option<String>, allowed_users: Vec<String>) -> Self {
        Self {
            bot_token,
            channel_id,
            allowed_users,
            client: crate::utils::http::build_http_client(),
        }
    }

    /// Check if a Slack user ID is in the allowlist.
    /// Empty list means deny everyone until explicitly configured.
    /// `"*"` means allow everyone.
    fn is_user_allowed(&self, user_id: &str) -> bool {
        is_allowed_user(&self.allowed_users, user_id, AllowlistMatch::Exact)
    }

    /// Get the bot's own user ID so we can ignore our own messages
    async fn get_bot_user_id(&self) -> Option<String> {
        let resp: serde_json::Value = self
            .client
            .get("https://slack.com/api/auth.test")
            .bearer_auth(&self.bot_token)
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;

        resp.get("user_id")
            .and_then(|u| u.as_str())
            .map(String::from)
    }

    fn parse_files(msg: &Value) -> Vec<MediaAttachment> {
        msg.get("files")
            .and_then(Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .filter_map(|file| {
                        let url = file.get("url_private").and_then(Value::as_str)?;
                        let mime_type = file.get("mimetype").and_then(Value::as_str);
                        let filename = file
                            .get("name")
                            .and_then(Value::as_str)
                            .map(ToString::to_string);
                        Some(media_attachment_url(url.to_string(), mime_type, filename))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn is_trusted_slack_media_url(url: &str) -> bool {
        let Ok(parsed) = reqwest::Url::parse(url) else {
            return false;
        };

        if parsed.scheme() != "https" {
            return false;
        }

        parsed.host_str().is_some_and(|host| {
            host == "slack.com" || host == "files.slack.com" || host.ends_with(".slack.com")
        })
    }

    async fn read_limited_media_response(resp: reqwest::Response) -> anyhow::Result<Vec<u8>> {
        if let Some(length) = resp.content_length()
            && length > SLACK_MEDIA_MAX_BYTES
        {
            anyhow::bail!(
                "Slack media response exceeds size limit: {length} bytes > {SLACK_MEDIA_MAX_BYTES}"
            );
        }

        let mut bytes = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("read Slack media chunk")?;
            let next_len = bytes.len().saturating_add(chunk.len());
            if u64::try_from(next_len).unwrap_or(u64::MAX) > SLACK_MEDIA_MAX_BYTES {
                anyhow::bail!(
                    "Slack media response exceeds size limit: > {SLACK_MEDIA_MAX_BYTES} bytes"
                );
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    async fn download_media_url(&self, media_url: &str) -> anyhow::Result<Vec<u8>> {
        let mut current_url = crate::security::validate_fetch_url(media_url, false)
            .await
            .context("validate Slack media URL")?;

        for redirect_count in 0..=SLACK_MEDIA_MAX_REDIRECTS {
            let host = current_url
                .host_str()
                .ok_or_else(|| anyhow::anyhow!("Slack media URL has no host"))?
                .to_string();
            let pinned_addrs = crate::security::resolve_public_fetch_addrs(&current_url).await?;
            let client = crate::utils::http::try_build_direct_http_client_with(
                reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .resolve_to_addrs(&host, &pinned_addrs),
            )
            .context("build pinned Slack media client")?;

            let request = client.get(current_url.clone());
            let request = if Self::is_trusted_slack_media_url(current_url.as_str()) {
                request.bearer_auth(&self.bot_token)
            } else {
                request
            };
            let resp = request
                .send()
                .await
                .context("download Slack media before upload")?;

            if is_redirect_status(resp.status()) {
                if redirect_count == SLACK_MEDIA_MAX_REDIRECTS {
                    anyhow::bail!("Slack media redirect limit exceeded");
                }
                current_url = validated_slack_media_redirect_target(&current_url, &resp).await?;
                continue;
            }

            let resp = resp
                .error_for_status()
                .context("download Slack media before upload")?;
            return Self::read_limited_media_response(resp).await;
        }

        anyhow::bail!("Slack media redirect limit exceeded")
    }
}

fn is_redirect_status(status: StatusCode) -> bool {
    status.is_redirection()
}

async fn validated_slack_media_redirect_target(
    current_url: &reqwest::Url,
    response: &reqwest::Response,
) -> anyhow::Result<reqwest::Url> {
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .ok_or_else(|| anyhow::anyhow!("Slack media redirect missing Location header"))?
        .to_str()
        .context("Slack media redirect Location is not valid UTF-8")?;
    let target = current_url
        .join(location)
        .context("Slack media redirect Location is not a valid URL")?;
    crate::security::validate_fetch_url(target.as_str(), false)
        .await
        .context("validate Slack media redirect URL")
}

impl Channel for SlackChannel {
    fn name(&self) -> &'static str {
        "slack"
    }

    fn max_message_length(&self) -> usize {
        3000
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            can_edit_message: true,
            can_delete_message: true,
            can_send_media: true,
            can_send_embed: true,
            can_send_typing: true,
            max_message_length: 3000,
            can_create_thread: true,
            can_add_reaction: true,
            can_receive_reactions: true,
            ..ChannelCapabilities::default()
        }
    }

    fn send<'a>(
        &'a self,
        message: &'a str,
        channel: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "channel": channel,
                "text": message
            });

            let resp = self
                .client
                .post("https://slack.com/api/chat.postMessage")
                .bearer_auth(&self.bot_token)
                .json(&body)
                .send()
                .await?;

            let status = resp.status();
            let body = resp
                .text()
                .await
                .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));

            if !status.is_success() {
                anyhow::bail!("Slack chat.postMessage failed ({status}): {body}");
            }

            // Slack returns 200 for most app-level errors; check JSON "ok" field
            let parsed: serde_json::Value = match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(error) => {
                    tracing::warn!(%error, "Slack response is not valid JSON");
                    anyhow::bail!("Slack chat.postMessage returned non-JSON response: {body}");
                }
            };
            if parsed.get("ok") == Some(&serde_json::Value::Bool(false)) {
                let err = parsed
                    .get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("unknown");
                anyhow::bail!("Slack chat.postMessage failed: {err}");
            }

            Ok(())
        })
    }

    fn listen<'a>(
        &'a self,
        tx: tokio::sync::mpsc::Sender<ChannelEvent>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let channel_id = self
                .channel_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Slack channel_id required for listening"))?;

            let bot_user_id = self.get_bot_user_id().await.unwrap_or_default();
            let mut last_ts = String::new();

            tracing::info!("Slack channel listening on #{channel_id}...");

            loop {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;

                let mut params = vec![("channel", channel_id.clone()), ("limit", "10".to_string())];
                if !last_ts.is_empty() {
                    params.push(("oldest", last_ts.clone()));
                }

                let resp = match self
                    .client
                    .get("https://slack.com/api/conversations.history")
                    .bearer_auth(&self.bot_token)
                    .query(&params)
                    .send()
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("Slack poll error: {e}");
                        continue;
                    }
                };

                let data: serde_json::Value = match resp.json().await {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::warn!("Slack parse error: {e}");
                        continue;
                    }
                };

                if let Some(messages) = data.get("messages").and_then(|m| m.as_array()) {
                    // Messages come newest-first, reverse to process oldest first
                    for msg in messages.iter().rev() {
                        let ts = msg.get("ts").and_then(|t| t.as_str()).unwrap_or("");
                        let user = msg
                            .get("user")
                            .and_then(|u| u.as_str())
                            .unwrap_or("unknown");
                        let text = msg.get("text").and_then(|t| t.as_str()).unwrap_or("");
                        let attachments = Self::parse_files(msg);

                        // Skip bot's own messages
                        if user == bot_user_id {
                            continue;
                        }

                        if !self.is_user_allowed(user) {
                            tracing::warn!(
                                "Slack: ignoring message from unauthorized user: {user}"
                            );
                            continue;
                        }

                        // Skip empty or already-seen
                        if (text.is_empty() && attachments.is_empty()) || ts <= last_ts.as_str() {
                            continue;
                        }

                        last_ts = ts.to_string();

                        let channel_msg = ChannelMessage {
                            id: Uuid::new_v4().to_string(),
                            sender: user.to_string(),
                            content: text.to_string(),
                            channel: "slack".to_string(),
                            context_hint: None,
                            conversation_id: Some(channel_id.clone()),
                            thread_id: None,
                            reply_to: None,
                            message_id: None,
                            timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0),
                            attachments,
                        };

                        if tx.send(ChannelEvent::Message(channel_msg)).await.is_err() {
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
                .get("https://slack.com/api/auth.test")
                .bearer_auth(&self.bot_token)
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
            let bytes = match &attachment.data {
                MediaContent::Url(media_url) => self.download_media_url(media_url).await?,
                MediaContent::Bytes(raw_bytes) => raw_bytes.clone(),
            };

            let filename = attachment
                .filename
                .clone()
                .unwrap_or_else(|| "attachment".to_string());
            let file_part = reqwest::multipart::Part::bytes(bytes).file_name(filename);
            let form = reqwest::multipart::Form::new()
                .text("channels", recipient.to_string())
                .part("file", file_part);

            let resp = self
                .client
                .post("https://slack.com/api/files.upload")
                .bearer_auth(&self.bot_token)
                .multipart(form)
                .send()
                .await
                .context("send Slack files.upload request")?;

            let status = resp.status();
            let body = resp
                .text()
                .await
                .unwrap_or_else(|e| format!("<failed to read response body: {e}>"));

            if !status.is_success() {
                anyhow::bail!("Slack files.upload failed ({status}): {body}");
            }

            let parsed: Value = match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(error) => {
                    tracing::warn!(%error, "Slack response is not valid JSON");
                    anyhow::bail!("Slack files.upload returned non-JSON response: {body}");
                }
            };
            if parsed.get("ok") == Some(&Value::Bool(false)) {
                let err = parsed
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                anyhow::bail!("Slack files.upload failed: {err}");
            }

            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn slack_channel_name() {
        let ch = SlackChannel::new("xoxb-fake".into(), None, vec![]);
        assert_eq!(ch.name(), "slack");
    }

    #[test]
    fn slack_channel_with_channel_id() {
        let ch = SlackChannel::new("xoxb-fake".into(), Some("C12345".into()), vec![]);
        assert_eq!(ch.channel_id, Some("C12345".to_string()));
    }

    #[test]
    fn empty_allowlist_denies_everyone() {
        let ch = SlackChannel::new("xoxb-fake".into(), None, vec![]);
        assert!(!ch.is_user_allowed("U12345"));
        assert!(!ch.is_user_allowed("anyone"));
    }

    #[test]
    fn wildcard_allows_everyone() {
        let ch = SlackChannel::new("xoxb-fake".into(), None, vec!["*".into()]);
        assert!(ch.is_user_allowed("U12345"));
    }

    #[test]
    fn specific_allowlist_filters() {
        let ch = SlackChannel::new("xoxb-fake".into(), None, vec!["U111".into(), "U222".into()]);
        assert!(ch.is_user_allowed("U111"));
        assert!(ch.is_user_allowed("U222"));
        assert!(!ch.is_user_allowed("U333"));
    }

    #[test]
    fn allowlist_exact_match_not_substring() {
        let ch = SlackChannel::new("xoxb-fake".into(), None, vec!["U111".into()]);
        assert!(!ch.is_user_allowed("U1111"));
        assert!(!ch.is_user_allowed("U11"));
    }

    #[test]
    fn allowlist_empty_user_id() {
        let ch = SlackChannel::new("xoxb-fake".into(), None, vec!["U111".into()]);
        assert!(!ch.is_user_allowed(""));
    }

    #[test]
    fn allowlist_case_sensitive() {
        let ch = SlackChannel::new("xoxb-fake".into(), None, vec!["U111".into()]);
        assert!(ch.is_user_allowed("U111"));
        assert!(!ch.is_user_allowed("u111"));
    }

    #[test]
    fn allowlist_wildcard_and_specific() {
        let ch = SlackChannel::new("xoxb-fake".into(), None, vec!["U111".into(), "*".into()]);
        assert!(ch.is_user_allowed("U111"));
        assert!(ch.is_user_allowed("anyone"));
    }

    #[test]
    fn trusted_slack_media_url_accepts_slack_hosts() {
        assert!(SlackChannel::is_trusted_slack_media_url(
            "https://files.slack.com/files-pri/T/F/file.txt"
        ));
        assert!(SlackChannel::is_trusted_slack_media_url(
            "https://foo.slack.com/files"
        ));
    }

    #[test]
    fn trusted_slack_media_url_rejects_non_slack_hosts() {
        assert!(!SlackChannel::is_trusted_slack_media_url(
            "https://example.com/file.txt"
        ));
        assert!(!SlackChannel::is_trusted_slack_media_url(
            "http://files.slack.com/file.txt"
        ));
    }

    #[test]
    fn parse_files_extracts_media_attachment() {
        let msg = serde_json::json!({
            "files": [
                {
                    "id": "F123",
                    "name": "report.pdf",
                    "mimetype": "application/pdf",
                    "url_private": "https://files.slack.com/files-pri/T/F/report.pdf"
                }
            ]
        });

        let attachments = SlackChannel::parse_files(&msg);
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].mime_type, "application/pdf");
        assert_eq!(attachments[0].filename.as_deref(), Some("report.pdf"));
        assert!(matches!(
            &attachments[0].data,
            MediaContent::Url(url) if url.contains("files-pri")
        ));
    }

    #[test]
    fn parse_files_empty_array_returns_none() {
        let msg = serde_json::json!({ "files": [] });
        assert!(SlackChannel::parse_files(&msg).is_empty());
    }

    #[test]
    fn parse_files_missing_field_returns_none() {
        let msg = serde_json::json!({ "text": "hello" });
        assert!(SlackChannel::parse_files(&msg).is_empty());
    }

    #[test]
    fn parse_files_skips_entries_without_private_url() {
        let msg = serde_json::json!({
            "files": [
                {"id": "F1", "name": "no_url.txt", "mimetype": "text/plain"},
                {
                    "id": "F2",
                    "name": "with_url.txt",
                    "mimetype": "text/plain",
                    "url_private": "https://files.slack.com/files-pri/T/F/with_url.txt"
                }
            ]
        });

        let attachments = SlackChannel::parse_files(&msg);
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].filename.as_deref(), Some("with_url.txt"));
    }

    #[test]
    fn parse_files_defaults_mime_type() {
        let msg = serde_json::json!({
            "files": [
                {
                    "id": "F123",
                    "name": "blob.bin",
                    "url_private": "https://files.slack.com/files-pri/T/F/blob.bin"
                }
            ]
        });

        let attachments = SlackChannel::parse_files(&msg);
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].mime_type, "application/octet-stream");
    }

    #[tokio::test]
    async fn limited_media_response_rejects_oversized_content_length() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/large.bin"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![
                0_u8;
                (SLACK_MEDIA_MAX_BYTES + 1)
                    as usize
            ]))
            .mount(&server)
            .await;

        let response = reqwest::Client::new()
            .get(format!("{}/large.bin", server.uri()))
            .send()
            .await
            .expect("mock response should be returned");
        let err = SlackChannel::read_limited_media_response(response)
            .await
            .expect_err("oversized declared Slack media should be rejected")
            .to_string();

        assert!(err.contains("exceeds size limit"));
    }

    #[tokio::test]
    async fn slack_media_redirect_target_is_validated_before_follow() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/redirect"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", "http://127.0.0.1/private-media"),
            )
            .mount(&server)
            .await;

        let response = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("client")
            .get(format!("{}/redirect", server.uri()))
            .send()
            .await
            .expect("mock redirect should be returned");
        let current_url = reqwest::Url::parse("http://8.8.8.8/redirect").unwrap();

        let err = validated_slack_media_redirect_target(&current_url, &response)
            .await
            .expect_err("private redirect target should be rejected before follow");
        let err = format!("{err:?}");

        assert!(err.contains("SSRF blocked"));
    }

    #[tokio::test]
    async fn send_media_uses_files_upload_endpoint() {
        let ch = SlackChannel::new("xoxb-fake".into(), Some("C12345".into()), vec!["*".into()]);
        let attachment = MediaAttachment {
            mime_type: "text/plain".to_string(),
            data: MediaContent::Bytes(b"hello".to_vec()),
            filename: Some("note.txt".to_string()),
        };

        let err = ch
            .send_media(&attachment, "C12345")
            .await
            .expect_err("network failure expected")
            .to_string();
        assert!(err.contains("files.upload") || err.contains("Slack"));
    }
}
