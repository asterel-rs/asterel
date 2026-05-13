//! Rate-limited HTTP client for the Discord REST API. Handles per-route
//! bucket tracking, 429 retries, and multipart file uploads.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::body::Bytes;
use reqwest::header::HeaderMap;
use reqwest::{Method, Response};
use serde_json::json;
use tokio::sync::Mutex;
use tokio::time::sleep;

use super::types::{API_BASE, InteractionCallbackType};
use crate::security::scrub::sanitize_api_error;

const MAX_RATE_LIMIT_RETRIES: u8 = 3;

/// Percent-encode a string for use in URI path segments (RFC 3986).
/// Encodes all characters except unreserved characters (A-Z, a-z, 0-9, `-`, `_`, `.`, `~`)
/// and `:` which Discord uses in custom emoji identifiers (e.g., `name:id`).
fn encode_uri_component(s: &str) -> String {
    let mut encoded = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' => {
                encoded.push(byte as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

fn is_discord_interaction_token_segment(segments: &[&str], index: usize) -> bool {
    let is_callback_token = index >= 2
        && segments.get(index - 2) == Some(&"interactions")
        && segments.get(index + 1) == Some(&"callback");
    let is_webhook_token = index >= 2 && segments.get(index - 2) == Some(&"webhooks");
    is_callback_token || is_webhook_token
}

fn sanitize_discord_error_body(body: &str) -> String {
    sanitize_api_error(body)
}

#[derive(Debug, Clone)]
struct RateLimitBucket {
    remaining: u32,
    reset_at: f64,
}

/// Rate-limited HTTP client for the Discord REST API.
pub struct DiscordHttpClient {
    client: reqwest::Client,
    bot_token: String,
    buckets: Arc<Mutex<HashMap<String, RateLimitBucket>>>,
    global_reset_at: Arc<Mutex<Option<f64>>>,
}

impl DiscordHttpClient {
    /// Create a new HTTP client authenticated with the given bot token.
    #[must_use]
    pub fn new(bot_token: impl Into<String>) -> Self {
        Self {
            client: crate::utils::http::build_http_client(),
            bot_token: bot_token.into(),
            buckets: Arc::new(Mutex::new(HashMap::new())),
            global_reset_at: Arc::new(Mutex::new(None)),
        }
    }

    /// Return a reference to the underlying HTTP client for non-API requests.
    #[must_use]
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Send a text message to a channel.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails, response
    /// payload parsing fails, or the API returns a non-success status.
    pub async fn send_message(&self, channel_id: &str, content: &str) -> Result<serde_json::Value> {
        let url = format!("{API_BASE}/channels/{channel_id}/messages");
        let response = self
            .request(
                Method::POST,
                &url,
                Some(json!({ "content": content, "allowed_mentions": { "parse": [] } })),
            )
            .await
            .context("send Discord message")?;
        response
            .json()
            .await
            .context("parse Discord send message response JSON")
    }

    /// Send a message with interactive components to a channel.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails, response
    /// payload parsing fails, or the API returns a non-success status.
    pub async fn send_message_with_components(
        &self,
        channel_id: &str,
        content: Option<&str>,
        components: Vec<serde_json::Value>,
    ) -> Result<serde_json::Value> {
        let url = format!("{API_BASE}/channels/{channel_id}/messages");
        let mut body = json!({ "components": components, "allowed_mentions": { "parse": [] } });
        if let Some(text) = content {
            body["content"] = json!(text);
        }
        let response = self
            .request(Method::POST, &url, Some(body))
            .await
            .context("send Discord message with components")?;
        response
            .json()
            .await
            .context("parse Discord component message response JSON")
    }

    /// Send a message with text content and rich components.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails, response
    /// payload parsing fails, or the API returns a non-success status.
    pub async fn send_rich_message(
        &self,
        channel_id: &str,
        content: &str,
        components: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let url = format!("{API_BASE}/channels/{channel_id}/messages");
        let mut body = if components.is_array() {
            json!({ "components": components, "allowed_mentions": { "parse": [] } })
        } else if components.is_object() {
            let mut b = components;
            b["allowed_mentions"] = json!({ "parse": [] });
            b
        } else {
            anyhow::bail!("Discord rich message components must be an object or array")
        };
        body["content"] = json!(content);
        let response = self
            .request(Method::POST, &url, Some(body))
            .await
            .context("send Discord rich message")?;
        response
            .json()
            .await
            .context("parse Discord rich message response JSON")
    }

    /// Send an embed message to a channel (discards the message ID).
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails, response
    /// payload parsing fails, or the API returns a non-success status.
    pub async fn send_embed(
        &self,
        channel_id: &str,
        title: Option<&str>,
        description: &str,
        color: Option<u32>,
    ) -> Result<()> {
        let _message_id = self
            .send_embed_message(channel_id, title, description, color)
            .await?;
        Ok(())
    }

    /// Send an embed message and return the created message ID.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails, response
    /// payload parsing fails, or the API returns a non-success status.
    pub async fn send_embed_message(
        &self,
        channel_id: &str,
        title: Option<&str>,
        description: &str,
        color: Option<u32>,
    ) -> Result<String> {
        let url = format!("{API_BASE}/channels/{channel_id}/messages");
        let mut embed = json!({ "description": description });
        if let Some(embed_title) = title {
            embed["title"] = json!(embed_title);
        }
        if let Some(embed_color) = color {
            embed["color"] = json!(embed_color);
        }
        let response = self
            .request(
                Method::POST,
                &url,
                Some(json!({ "embeds": [embed], "allowed_mentions": { "parse": [] } })),
            )
            .await
            .context("send Discord embed")?;
        let payload: serde_json::Value = response
            .json()
            .await
            .context("parse Discord send embed response JSON")?;
        payload
            .get("id")
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| anyhow::anyhow!("Discord embed response missing message id"))
    }

    /// Upload a file attachment to a channel via multipart form.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails, response
    /// payload parsing fails, or the API returns a non-success status.
    pub async fn send_media(
        &self,
        channel_id: &str,
        bytes: Vec<u8>,
        filename: &str,
        mime_type: &str,
    ) -> Result<()> {
        let url = format!("{API_BASE}/channels/{channel_id}/messages");
        self.send_media_to_url(&url, bytes, filename, mime_type)
            .await
    }

    async fn send_media_to_url(
        &self,
        url: &str,
        bytes: Vec<u8>,
        filename: &str,
        mime_type: &str,
    ) -> Result<()> {
        let filename_owned = filename.to_owned();
        let mime_type_owned = mime_type.to_owned();
        let bytes: Arc<[u8]> = bytes.into();

        let route_key = Self::bucket_key(url);
        self.wait_for_limits(&route_key).await;

        for attempt in 0..=MAX_RATE_LIMIT_RETRIES {
            let part = Self::media_part(bytes.clone(), filename_owned.clone())
                .mime_str(&mime_type_owned)
                .context("set Discord media MIME type")?;
            let form = reqwest::multipart::Form::new().part("files[0]", part);

            let response = self
                .client
                .post(url)
                .header("Authorization", format!("Bot {}", self.bot_token))
                .multipart(form)
                .send()
                .await
                .context("send Discord media request")?;

            self.update_bucket_from_headers(&route_key, response.headers())
                .await;

            if response.status().as_u16() == 429 {
                if attempt == MAX_RATE_LIMIT_RETRIES {
                    anyhow::bail!(
                        "Discord media request exceeded rate limit after {MAX_RATE_LIMIT_RETRIES} retries"
                    );
                }
                let is_global = Self::is_global_limit(response.headers());
                let retry_after = Self::parse_retry_after(response.headers())
                    .unwrap_or_else(|| Duration::from_secs(1));
                self.handle_429_wait(is_global, retry_after, &route_key)
                    .await;
                continue;
            }

            if !response.status().is_success() {
                let status = response.status();
                let body = response
                    .text()
                    .await
                    .unwrap_or_else(|error| format!("<failed to read response body: {error}>"));
                let sanitized_body = sanitize_discord_error_body(&body);
                anyhow::bail!("Discord media request failed ({status}): {sanitized_body}");
            }

            return Ok(());
        }

        anyhow::bail!("Discord media request failed due to rate limiting")
    }

    fn media_part(bytes: Arc<[u8]>, filename: String) -> reqwest::multipart::Part {
        let length = u64::try_from(bytes.len()).expect("media byte length must fit u64");
        let body = reqwest::Body::from(Bytes::from_owner(bytes));

        reqwest::multipart::Part::stream_with_length(body, length).file_name(filename)
    }

    /// Edit the content of an existing message.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails, response
    /// payload parsing fails, or the API returns a non-success status.
    pub async fn edit_message(
        &self,
        channel_id: &str,
        message_id: &str,
        content: &str,
    ) -> Result<()> {
        let url = format!("{API_BASE}/channels/{channel_id}/messages/{message_id}");
        let _response = self
            .request(
                Method::PATCH,
                &url,
                Some(json!({ "content": content, "allowed_mentions": { "parse": [] } })),
            )
            .await
            .context("edit Discord message")?;
        Ok(())
    }

    /// Edit an existing embed message.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails, response
    /// payload parsing fails, or the API returns a non-success status.
    pub async fn edit_embed(
        &self,
        channel_id: &str,
        message_id: &str,
        title: Option<&str>,
        description: &str,
        color: Option<u32>,
    ) -> Result<()> {
        let url = format!("{API_BASE}/channels/{channel_id}/messages/{message_id}");
        let mut embed = json!({ "description": description });
        if let Some(embed_title) = title {
            embed["title"] = json!(embed_title);
        }
        if let Some(embed_color) = color {
            embed["color"] = json!(embed_color);
        }
        let _response = self
            .request(
                Method::PATCH,
                &url,
                Some(json!({ "embeds": [embed], "allowed_mentions": { "parse": [] } })),
            )
            .await
            .context("edit Discord embed")?;
        Ok(())
    }

    /// Delete a message from a channel.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails or the API
    /// returns a non-success status.
    pub async fn delete_message(&self, channel_id: &str, message_id: &str) -> Result<()> {
        let url = format!("{API_BASE}/channels/{channel_id}/messages/{message_id}");
        let _response = self
            .request(Method::DELETE, &url, None)
            .await
            .context("delete Discord message")?;
        Ok(())
    }

    /// Fetch a single message by ID from a channel.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails, response payload parsing fails,
    /// or the API returns a non-success status.
    pub async fn get_message(
        &self,
        channel_id: &str,
        message_id: &str,
    ) -> Result<serde_json::Value> {
        let url = format!("{API_BASE}/channels/{channel_id}/messages/{message_id}");
        let response = self
            .request(Method::GET, &url, None)
            .await
            .context("fetch Discord message")?;
        response.json().await.context("parse Discord message JSON")
    }

    /// Fetch message history from a channel.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails, response payload parsing fails,
    /// or the API returns a non-success status (including unrecoverable rate limits).
    pub async fn get_messages(
        &self,
        channel_id: &str,
        limit: Option<u8>,
        before: Option<&str>,
        after: Option<&str>,
    ) -> Result<Vec<serde_json::Value>> {
        let mut url = format!("{API_BASE}/channels/{channel_id}/messages");
        let mut params = Vec::new();
        if let Some(l) = limit {
            params.push(format!("limit={l}"));
        }
        if let Some(b) = before {
            anyhow::ensure!(
                b.chars().all(|c| c.is_ascii_digit()),
                "Discord snowflake 'before' must be numeric"
            );
            params.push(format!("before={b}"));
        }
        if let Some(a) = after {
            anyhow::ensure!(
                a.chars().all(|c| c.is_ascii_digit()),
                "Discord snowflake 'after' must be numeric"
            );
            params.push(format!("after={a}"));
        }
        if !params.is_empty() {
            url.push('?');
            url.push_str(&params.join("&"));
        }
        let response = self
            .request(Method::GET, &url, None)
            .await
            .context("fetch Discord channel messages")?;
        response
            .json()
            .await
            .context("parse Discord channel messages JSON")
    }

    /// Create a thread from an existing message.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails, response payload parsing fails,
    /// or the API returns a non-success status (including unrecoverable rate limits).
    pub async fn create_thread_from_message(
        &self,
        channel_id: &str,
        message_id: &str,
        name: &str,
        auto_archive_duration: Option<u16>,
    ) -> Result<serde_json::Value> {
        let url = format!("{API_BASE}/channels/{channel_id}/messages/{message_id}/threads");
        let mut body = json!({ "name": name });
        if let Some(duration) = auto_archive_duration {
            body["auto_archive_duration"] = json!(duration);
        }
        let response = self
            .request(Method::POST, &url, Some(body))
            .await
            .context("create Discord thread from message")?;
        response
            .json()
            .await
            .context("parse Discord create thread response JSON")
    }

    /// Create a new thread in a channel (without a starter message).
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails, response payload parsing fails,
    /// or the API returns a non-success status (including unrecoverable rate limits).
    pub async fn create_thread(
        &self,
        channel_id: &str,
        name: &str,
        thread_type: Option<u8>,
        auto_archive_duration: Option<u16>,
    ) -> Result<serde_json::Value> {
        let url = format!("{API_BASE}/channels/{channel_id}/threads");
        let mut body = json!({ "name": name });
        if let Some(t) = thread_type {
            body["type"] = json!(t);
        }
        if let Some(duration) = auto_archive_duration {
            body["auto_archive_duration"] = json!(duration);
        }
        let response = self
            .request(Method::POST, &url, Some(body))
            .await
            .context("create Discord thread")?;
        response
            .json()
            .await
            .context("parse Discord create thread response JSON")
    }

    /// # Errors
    /// Returns an error if the Discord API request fails or the API returns a non-success status.
    pub async fn add_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        let encoded_emoji = encode_uri_component(emoji);
        let url = format!(
            "{API_BASE}/channels/{channel_id}/messages/{message_id}/reactions/{encoded_emoji}/@me"
        );
        let _response = self
            .request(Method::PUT, &url, None)
            .await
            .context("add Discord reaction")?;
        Ok(())
    }

    /// Add the bot to a thread.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails or the API returns a non-success status.
    pub async fn join_thread(&self, thread_id: &str) -> Result<()> {
        let url = format!("{API_BASE}/channels/{thread_id}/thread-members/@me");
        let _response = self
            .request(Method::PUT, &url, None)
            .await
            .context("join Discord thread")?;
        Ok(())
    }

    /// Remove the bot from a thread.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails or the API returns a non-success status.
    pub async fn leave_thread(&self, thread_id: &str) -> Result<()> {
        let url = format!("{API_BASE}/channels/{thread_id}/thread-members/@me");
        let _response = self
            .request(Method::DELETE, &url, None)
            .await
            .context("leave Discord thread")?;
        Ok(())
    }

    /// Add a user to a thread.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails or the API returns a non-success status.
    pub async fn add_thread_member(&self, thread_id: &str, user_id: &str) -> Result<()> {
        let url = format!("{API_BASE}/channels/{thread_id}/thread-members/{user_id}");
        let _response = self
            .request(Method::PUT, &url, None)
            .await
            .context("add Discord thread member")?;
        Ok(())
    }

    /// Remove a user from a thread.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails or the API returns a non-success status.
    pub async fn remove_thread_member(&self, thread_id: &str, user_id: &str) -> Result<()> {
        let url = format!("{API_BASE}/channels/{thread_id}/thread-members/{user_id}");
        let _response = self
            .request(Method::DELETE, &url, None)
            .await
            .context("remove Discord thread member")?;
        Ok(())
    }

    /// Trigger the typing indicator in a channel.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails or the API
    /// returns a non-success status.
    pub async fn send_typing(&self, channel_id: &str) -> Result<()> {
        let url = format!("{API_BASE}/channels/{channel_id}/typing");
        let _response = self
            .request(Method::POST, &url, None)
            .await
            .context("send Discord typing indicator")?;
        Ok(())
    }

    /// Fetch the currently authenticated bot user.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails or the API
    /// returns a non-success status.
    pub async fn get_current_user(&self) -> Result<serde_json::Value> {
        let url = format!("{API_BASE}/users/@me");
        let response = self
            .request(Method::GET, &url, None)
            .await
            .context("fetch current Discord user")?;
        response
            .json()
            .await
            .context("parse current Discord user JSON")
    }

    /// Fetch the gateway bot endpoint (URL and shard info).
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails or the API
    /// returns a non-success status.
    pub async fn get_gateway_bot(&self) -> Result<serde_json::Value> {
        let url = format!("{API_BASE}/gateway/bot");
        let response = self
            .request(Method::GET, &url, None)
            .await
            .context("fetch Discord gateway bot data")?;
        response
            .json()
            .await
            .context("parse Discord gateway bot JSON")
    }

    /// Respond to an interaction with the given callback type and data.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails or the API
    /// returns a non-success status.
    pub async fn create_interaction_response(
        &self,
        interaction_id: &str,
        interaction_token: &str,
        response_type: u8,
        data: Option<serde_json::Value>,
    ) -> Result<()> {
        let url = format!("{API_BASE}/interactions/{interaction_id}/{interaction_token}/callback");
        let mut body = json!({ "type": response_type });
        if let Some(payload) = data {
            body["data"] = payload;
        }
        let _response = self
            .request(Method::POST, &url, Some(body))
            .await
            .context("create Discord interaction response")?;
        Ok(())
    }

    /// Respond to an interaction by opening a modal dialog.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails or the API
    /// returns a non-success status.
    pub async fn send_modal_response(
        &self,
        interaction_id: &str,
        interaction_token: &str,
        custom_id: &str,
        title: &str,
        components: Vec<serde_json::Value>,
    ) -> Result<()> {
        self.create_interaction_response(
            interaction_id,
            interaction_token,
            InteractionCallbackType::Modal as u8,
            Some(json!({
                "custom_id": custom_id,
                "title": title,
                "components": components,
            })),
        )
        .await
        .context("send Discord modal response")
    }

    /// Edit the original deferred interaction response.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails or the API
    /// returns a non-success status.
    pub async fn edit_original_interaction_response(
        &self,
        application_id: &str,
        interaction_token: &str,
        content: &str,
    ) -> Result<()> {
        let url =
            format!("{API_BASE}/webhooks/{application_id}/{interaction_token}/messages/@original");
        let _response = self
            .request(
                Method::PATCH,
                &url,
                Some(json!({ "content": content, "allowed_mentions": { "parse": [] } })),
            )
            .await
            .context("edit original Discord interaction response")?;
        Ok(())
    }

    /// Post a new follow-up message to an interaction via the webhook endpoint.
    ///
    /// Unlike [`edit_original_interaction_response`], this creates a
    /// **new** message rather than editing the deferred response.
    /// Pass `flags` to control visibility (e.g. [`message_flags::EPHEMERAL`]).
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails, response payload
    /// parsing fails, or the API returns a non-success status.
    pub async fn create_interaction_followup(
        &self,
        application_id: &str,
        interaction_token: &str,
        content: &str,
        flags: Option<u64>,
    ) -> Result<serde_json::Value> {
        let url = format!("{API_BASE}/webhooks/{application_id}/{interaction_token}");
        let mut body = json!({
            "content": content,
            "allowed_mentions": { "parse": [] },
        });
        if let Some(f) = flags {
            body["flags"] = json!(f);
        }
        let response = self
            .request(Method::POST, &url, Some(body))
            .await
            .context("create Discord interaction followup")?;
        response
            .json()
            .await
            .context("parse Discord interaction followup response JSON")
    }

    /// Bulk-overwrite application commands globally or for a guild.
    ///
    /// # Errors
    /// Returns an error if the Discord API request fails or the API
    /// returns a non-success status.
    pub async fn register_commands(
        &self,
        application_id: &str,
        guild_id: Option<&str>,
        commands: &[serde_json::Value],
    ) -> Result<()> {
        let url = if let Some(guild) = guild_id {
            format!("{API_BASE}/applications/{application_id}/guilds/{guild}/commands")
        } else {
            format!("{API_BASE}/applications/{application_id}/commands")
        };

        let _response = self
            .request(Method::PUT, &url, Some(json!(commands)))
            .await
            .context("register Discord application commands")?;
        Ok(())
    }

    async fn request(
        &self,
        method: Method,
        url: &str,
        body: Option<serde_json::Value>,
    ) -> Result<Response> {
        let route_key = Self::bucket_key(url);
        self.wait_for_limits(&route_key).await;

        for attempt in 0..=MAX_RATE_LIMIT_RETRIES {
            let mut request_builder = self
                .client
                .request(method.clone(), url)
                .header("Authorization", format!("Bot {}", self.bot_token));
            if let Some(payload) = body.clone() {
                request_builder = request_builder.json(&payload);
            }

            let response = request_builder.send().await.with_context(|| {
                format!(
                    "send Discord request {} {}",
                    method.as_str(),
                    Self::safe_request_target(url)
                )
            })?;

            self.update_bucket_from_headers(&route_key, response.headers())
                .await;

            if response.status().as_u16() == 429 {
                if attempt == MAX_RATE_LIMIT_RETRIES {
                    anyhow::bail!(
                        "Discord request {} {} exceeded rate limit after {} retries",
                        method.as_str(),
                        Self::safe_request_target(url),
                        MAX_RATE_LIMIT_RETRIES
                    );
                }
                let is_global = Self::is_global_limit(response.headers());
                let retry_after = Self::parse_retry_after(response.headers())
                    .unwrap_or_else(|| Duration::from_secs(1));
                self.handle_429_wait(is_global, retry_after, &route_key)
                    .await;
                continue;
            }

            if !response.status().is_success() {
                let status = response.status();
                let body_text = response
                    .text()
                    .await
                    .unwrap_or_else(|error| format!("<failed to read response body: {error}>"));
                let sanitized_body = sanitize_discord_error_body(&body_text);
                anyhow::bail!(
                    "Discord request {} {} failed ({status}): {body_text}",
                    method.as_str(),
                    Self::safe_request_target(url),
                    body_text = sanitized_body
                );
            }

            return Ok(response);
        }

        anyhow::bail!(
            "Discord request {} {} failed due to rate limiting",
            method.as_str(),
            Self::safe_request_target(url)
        )
    }

    fn parse_header_u32(headers: &HeaderMap, name: &str) -> Option<u32> {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u32>().ok())
    }

    fn parse_header_f64(headers: &HeaderMap, name: &str) -> Option<f64> {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<f64>().ok())
    }

    fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
        let seconds = Self::parse_header_f64(headers, "Retry-After")?;
        if seconds <= 0.0 {
            return Some(Duration::from_secs(0));
        }
        Some(Duration::from_secs_f64(seconds))
    }

    fn is_global_limit(headers: &HeaderMap) -> bool {
        headers
            .get("X-RateLimit-Global")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    }

    fn now_unix_timestamp() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    fn bucket_key(url: &str) -> String {
        let path = reqwest::Url::parse(url)
            .map_or_else(|_| url.to_string(), |parsed| parsed.path().to_string());
        let path_without_api_prefix = path
            .strip_prefix("/api/v10")
            .map_or(path.as_str(), |stripped| stripped);

        let mut normalized = String::new();
        let segments: Vec<_> = path_without_api_prefix
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect();
        for (index, segment) in segments.iter().enumerate() {
            if !normalized.is_empty() {
                normalized.push('/');
            }
            if segment.bytes().all(|b| b.is_ascii_digit()) {
                normalized.push_str("{id}");
            } else if is_discord_interaction_token_segment(&segments, index) {
                normalized.push_str("{token}");
            } else {
                normalized.push_str(segment);
            }
        }

        format!("/{normalized}")
    }

    fn safe_request_target(url: &str) -> String {
        let Ok(parsed) = reqwest::Url::parse(url) else {
            return "<invalid-discord-url>".to_string();
        };
        let path = parsed.path().to_string();
        let path_without_api_prefix = path
            .strip_prefix("/api/v10")
            .map_or(path.as_str(), |stripped| stripped);

        let segments: Vec<_> = path_without_api_prefix
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect();
        let mut normalized = String::new();
        for (index, segment) in segments.iter().enumerate() {
            if !normalized.is_empty() {
                normalized.push('/');
            }

            let redacted = if segment.bytes().all(|b| b.is_ascii_digit()) {
                "{id}"
            } else if is_discord_interaction_token_segment(&segments, index) {
                "{token}"
            } else {
                segment
            };
            normalized.push_str(redacted);
        }

        if normalized.is_empty() {
            "/".to_string()
        } else {
            format!("/{normalized}")
        }
    }

    async fn wait_for_limits(&self, route_key: &str) {
        let now = Self::now_unix_timestamp();
        let global_wait = {
            let global_guard = self.global_reset_at.lock().await;
            global_guard.and_then(|reset_at| (reset_at > now).then_some(reset_at - now))
        };
        if let Some(wait_secs) = global_wait {
            sleep(Duration::from_secs_f64(wait_secs)).await;
        }

        let route_wait = {
            let buckets = self.buckets.lock().await;
            buckets.get(route_key).and_then(|bucket| {
                if bucket.remaining == 0 && bucket.reset_at > now {
                    Some(bucket.reset_at - now)
                } else {
                    None
                }
            })
        };
        if let Some(wait_secs) = route_wait {
            sleep(Duration::from_secs_f64(wait_secs)).await;
        }
    }

    async fn handle_429_wait(&self, is_global: bool, retry_after: Duration, route_key: &str) {
        let now = Self::now_unix_timestamp();
        let reset_at = now + retry_after.as_secs_f64();
        if is_global {
            let mut global = self.global_reset_at.lock().await;
            *global = Some(reset_at);
        } else {
            let mut buckets = self.buckets.lock().await;
            buckets.insert(
                route_key.to_string(),
                RateLimitBucket {
                    remaining: 0,
                    reset_at,
                },
            );
        }
        sleep(retry_after).await;
    }

    async fn update_bucket_from_headers(&self, route_key: &str, headers: &HeaderMap) {
        let _limit = Self::parse_header_u32(headers, "X-RateLimit-Limit");
        let remaining = Self::parse_header_u32(headers, "X-RateLimit-Remaining");
        let reset_at = Self::parse_header_f64(headers, "X-RateLimit-Reset");
        let _bucket = headers.get("X-RateLimit-Bucket");

        if let (Some(remaining), Some(reset_at)) = (remaining, reset_at) {
            let mut buckets = self.buckets.lock().await;
            buckets.insert(
                route_key.to_string(),
                RateLimitBucket {
                    remaining,
                    reset_at,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::DiscordHttpClient;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    fn capture_request_body(
        captured: Arc<Mutex<Vec<Vec<u8>>>>,
    ) -> impl Fn(&Request) -> bool + Send + Sync + 'static {
        move |request: &Request| {
            captured
                .lock()
                .expect("captured request bodies lock should not be poisoned")
                .push(request.body.clone());
            true
        }
    }

    fn body_contains(body: &[u8], needle: &[u8]) -> bool {
        body.windows(needle.len()).any(|window| window == needle)
    }

    #[test]
    fn extracts_rate_limit_bucket_key_from_url_path() {
        let url = "https://discord.com/api/v10/channels/123456789/messages";
        assert_eq!(
            DiscordHttpClient::bucket_key(url),
            "/channels/{id}/messages"
        );
    }

    #[test]
    fn bucket_key_for_thread_url() {
        let url = "https://discord.com/api/v10/channels/111/messages/222/threads";
        assert_eq!(
            DiscordHttpClient::bucket_key(url),
            "/channels/{id}/messages/{id}/threads"
        );
    }

    #[test]
    fn bucket_key_for_thread_members_url() {
        let url = "https://discord.com/api/v10/channels/111/thread-members/@me";
        assert_eq!(
            DiscordHttpClient::bucket_key(url),
            "/channels/{id}/thread-members/@me"
        );
    }

    #[test]
    fn bucket_key_for_messages_url() {
        let url = "https://discord.com/api/v10/channels/111/messages";
        assert_eq!(
            DiscordHttpClient::bucket_key(url),
            "/channels/{id}/messages"
        );
    }

    #[test]
    fn bucket_key_redacts_interaction_callback_token() {
        let url =
            "https://discord.com/api/v10/interactions/1234567890/discord-secret-token/callback";

        let key = DiscordHttpClient::bucket_key(url);

        assert_eq!(key, "/interactions/{id}/{token}/callback");
        assert!(!key.contains("discord-secret-token"));
    }

    #[test]
    fn bucket_key_redacts_interaction_webhook_token() {
        let url = "https://discord.com/api/v10/webhooks/1234567890/discord-secret-token/messages/@original";

        let key = DiscordHttpClient::bucket_key(url);

        assert_eq!(key, "/webhooks/{id}/{token}/messages/@original");
        assert!(!key.contains("discord-secret-token"));
    }

    #[test]
    fn safe_request_target_redacts_interaction_callback_token() {
        let url =
            "https://discord.com/api/v10/interactions/1234567890/discord-secret-token/callback";

        let target = DiscordHttpClient::safe_request_target(url);

        assert_eq!(target, "/interactions/{id}/{token}/callback");
        assert!(!target.contains("discord-secret-token"));
        assert!(!target.contains("1234567890"));
    }

    #[test]
    fn safe_request_target_redacts_interaction_webhook_token() {
        let url = "https://discord.com/api/v10/webhooks/1234567890/discord-secret-token/messages/@original";

        let target = DiscordHttpClient::safe_request_target(url);

        assert_eq!(target, "/webhooks/{id}/{token}/messages/@original");
        assert!(!target.contains("discord-secret-token"));
        assert!(!target.contains("1234567890"));
    }

    #[test]
    fn discord_error_body_sanitizer_redacts_provider_echoed_secret() {
        let body = "Discord upstream echoed sk-leaked-secret-token in error body";

        let sanitized = super::sanitize_discord_error_body(body);

        assert!(!sanitized.contains("sk-leaked-secret-token"));
        assert!(sanitized.contains("[REDACTED]"));
    }

    #[test]
    fn safe_request_target_does_not_preserve_malformed_urls() {
        let url = "not a url with discord-secret-token";

        let target = DiscordHttpClient::safe_request_target(url);

        assert_eq!(target, "<invalid-discord-url>");
        assert!(!target.contains("discord-secret-token"));
    }

    #[test]
    fn parses_retry_after_float_seconds() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Retry-After",
            reqwest::header::HeaderValue::from_static("1.75"),
        );

        let retry_after = DiscordHttpClient::parse_retry_after(&headers);
        assert!(retry_after.is_some());
        let duration = retry_after.unwrap_or_default();
        assert_eq!(duration.as_secs(), 1);
        assert_eq!(duration.subsec_millis(), 750);
    }

    #[tokio::test]
    async fn constructor_initializes_http_client_state() {
        let client = DiscordHttpClient::new("token");

        assert_eq!(client.bot_token, "token");
        assert!(client.buckets.lock().await.is_empty());
        assert!(client.global_reset_at.lock().await.is_none());
    }

    #[tokio::test]
    async fn send_media_retries_with_identical_bytes_and_filename() {
        let server = MockServer::start().await;
        let captured = Arc::new(Mutex::new(Vec::new()));
        let route = "/api/v10/channels/123/messages";

        Mock::given(method("POST"))
            .and(path(route))
            .and(capture_request_body(captured.clone()))
            .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(route))
            .and(capture_request_body(captured.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .with_priority(2)
            .mount(&server)
            .await;

        let payload = b"discord retry payload \0 \xff".to_vec();
        let client = DiscordHttpClient::new("test-token");
        client
            .send_media_to_url(
                &format!("{}{route}", server.uri()),
                payload.clone(),
                "retry.bin",
                "application/octet-stream",
            )
            .await
            .expect("second Discord media attempt should succeed");

        let bodies = captured
            .lock()
            .expect("captured request bodies lock should not be poisoned");
        assert_eq!(bodies.len(), 2);
        assert_ne!(
            bodies[0], bodies[1],
            "multipart boundaries are rebuilt per attempt"
        );
        for body in bodies.iter() {
            assert!(body_contains(body, &payload));
            assert!(body_contains(body, b"retry.bin"));
        }
    }
}
