//! Twitter API v2 helpers: token refresh, tweet posting, mention + DM polling.
//!
//! Each API call retries once on 401 by refreshing the OAuth 2.0 access token.
//! Retry is implemented as a loop (not recursion) to keep futures sized.

use std::fmt::Write as FmtWrite;

use reqwest::{Method, Response, StatusCode};
use serde_json::Value;

use super::TwitterChannel;
use crate::contracts::ids::EventId;
use crate::transport::channels::api_request::{
    CHANNEL_API_MAX_RATE_LIMIT_RETRIES, channel_api_error_message, wait_for_rate_limit,
};

/// A tweet from the mentions timeline.
#[derive(Debug)]
pub(super) struct TweetItem {
    pub id: String,
    pub text: String,
    pub author_username: String,
    pub author_id: String,
}

/// A DM event from the direct messages API.
#[derive(Debug)]
pub(super) struct DmItem {
    pub event_id: EventId,
    pub sender_id: String,
    pub text: String,
}

impl TwitterChannel {
    async fn send_authenticated_request(
        &self,
        method: Method,
        url: &str,
        body: Option<&Value>,
        operation: &str,
    ) -> anyhow::Result<Response> {
        let mut refreshed_token = false;
        let mut rate_limit_retries = 0_u8;

        loop {
            let mut request = self
                .client
                .request(method.clone(), url)
                .bearer_auth(self.read_access_token());
            if let Some(body) = body {
                request = request.json(body);
            }

            let resp = request.send().await?;

            if resp.status() == StatusCode::UNAUTHORIZED && !refreshed_token {
                self.refresh_access_token().await?;
                refreshed_token = true;
                continue;
            }

            if resp.status() == StatusCode::TOO_MANY_REQUESTS
                && rate_limit_retries < CHANNEL_API_MAX_RATE_LIMIT_RETRIES
            {
                wait_for_rate_limit(resp.headers()).await;
                rate_limit_retries += 1;
                continue;
            }

            if !resp.status().is_success() {
                let err = channel_api_error_message("Twitter", operation, resp).await;
                anyhow::bail!(err);
            }

            return Ok(resp);
        }
    }

    /// Refresh the access token using the stored refresh token.
    ///
    /// On success, updates both `access_token` and `refresh_token` in place.
    pub(super) async fn refresh_access_token(&self) -> anyhow::Result<()> {
        let refresh_token = self
            .refresh_token
            .read()
            .map_err(|e| anyhow::anyhow!("refresh_token lock poisoned: {e}"))?
            .clone();

        let resp: reqwest::Response = self
            .client
            .post(self.api_url("oauth2/token"))
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body: String = resp
                .text()
                .await
                .unwrap_or_else(|e| format!("<read error: {e}>"));
            let sanitized_body = crate::security::scrub::sanitize_api_error(&body);
            anyhow::bail!("Twitter token refresh failed ({status}): {sanitized_body}");
        }

        let data: Value = resp.json().await?;

        let new_access_token = data["access_token"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing access_token in refresh response"))?;
        let new_refresh_token = data["refresh_token"].as_str().unwrap_or(&refresh_token);

        *self
            .access_token
            .write()
            .map_err(|e| anyhow::anyhow!("access_token write lock poisoned: {e}"))? =
            new_access_token.to_string();
        *self
            .refresh_token
            .write()
            .map_err(|e| anyhow::anyhow!("refresh_token write lock poisoned: {e}"))? =
            new_refresh_token.to_string();

        tracing::info!("Twitter: access token refreshed successfully");
        Ok(())
    }

    /// Post a standalone tweet, returning the new tweet ID.
    pub(super) async fn post_tweet(&self, text: &str) -> anyhow::Result<String> {
        self.post_tweet_with_reply(text, None).await
    }

    /// Post a tweet, optionally as a reply to `reply_to_tweet_id`.
    ///
    /// Retries once on 401 after refreshing the access token.
    pub(super) async fn post_tweet_with_reply(
        &self,
        text: &str,
        reply_to_tweet_id: Option<&str>,
    ) -> anyhow::Result<String> {
        let mut body = serde_json::json!({ "text": text });
        if let Some(id) = reply_to_tweet_id {
            body["reply"] = serde_json::json!({ "in_reply_to_tweet_id": id });
        }

        let resp = self
            .send_authenticated_request(
                Method::POST,
                &self.api_url("tweets"),
                Some(&body),
                "post tweet",
            )
            .await?;
        let data: Value = resp.json().await?;
        let tweet_id = data["data"]["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing tweet id in response"))?;
        Ok(tweet_id.to_string())
    }

    /// Send a DM to `recipient_id`.
    ///
    /// Retries once on 401 after refreshing the access token.
    pub(super) async fn send_dm(&self, recipient_id: &str, text: &str) -> anyhow::Result<()> {
        let body = serde_json::json!({
            "participant_id": recipient_id,
            "text": text,
        });

        let url = self.api_url(&format!("dm_conversations/with/{recipient_id}/messages"));
        self.send_authenticated_request(Method::POST, &url, Some(&body), "send DM")
            .await?;
        Ok(())
    }

    /// Fetch recent mentions since `since_id`, returning up to 5 results.
    ///
    /// Returns `(items, newest_id)` where `newest_id` is the latest tweet ID
    /// to use as `since_id` on the next poll.
    ///
    /// Retries once on 401 after refreshing the access token.
    pub(super) async fn get_mentions(
        &self,
        since_id: Option<&str>,
    ) -> anyhow::Result<(Vec<TweetItem>, Option<String>)> {
        let mut url = format!(
            "users/{}/mentions?max_results=5&expansions=author_id&user.fields=username",
            self.user_id
        );
        if let Some(id) = since_id {
            let _ = write!(url, "&since_id={id}");
        }

        let resp = self
            .send_authenticated_request(Method::GET, &self.api_url(&url), None, "get mentions")
            .await?;

        let data: Value = resp.json().await?;

        let Some(tweets) = data["data"].as_array() else {
            return Ok((vec![], None));
        };

        // Build username lookup from `includes.users`
        let mut user_map: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        if let Some(users) = data["includes"]["users"].as_array() {
            for user in users {
                if let (Some(id), Some(name)) = (user["id"].as_str(), user["username"].as_str()) {
                    user_map.insert(id, name);
                }
            }
        }

        let mut items = Vec::with_capacity(tweets.len());
        let mut newest_id: Option<String> = None;

        for tweet in tweets {
            let Some(id) = tweet["id"].as_str() else {
                continue;
            };
            let text = tweet["text"].as_str().unwrap_or("").to_string();
            let author_id = tweet["author_id"].as_str().unwrap_or("").to_string();
            let author_username = user_map
                .get(author_id.as_str())
                .copied()
                .unwrap_or("unknown")
                .to_string();

            if newest_id.as_deref().is_none_or(|prev| id > prev) {
                newest_id = Some(id.to_string());
            }

            items.push(TweetItem {
                id: id.to_string(),
                text,
                author_username,
                author_id,
            });
        }

        Ok((items, newest_id))
    }

    /// Fetch recent DM events since `since_id`, returning up to 5 results.
    ///
    /// Retries once on 401 after refreshing the access token.
    pub(super) async fn get_dm_events(
        &self,
        since_id: Option<&str>,
    ) -> anyhow::Result<(Vec<DmItem>, Option<String>)> {
        let mut url = "dm_events?dm_event.fields=id,text,sender_id&max_results=5".to_string();
        if let Some(id) = since_id {
            let _ = write!(url, "&since_id={id}");
        }

        let resp = self
            .send_authenticated_request(Method::GET, &self.api_url(&url), None, "get DM events")
            .await?;

        let data: Value = resp.json().await?;

        let Some(events) = data["data"].as_array() else {
            return Ok((vec![], None));
        };

        let mut items = Vec::with_capacity(events.len());
        let mut newest_id: Option<String> = None;

        for event in events {
            let Some(event_id) = event["id"].as_str() else {
                continue;
            };
            let sender_id = event["sender_id"].as_str().unwrap_or("").to_string();
            let text = event["text"].as_str().unwrap_or("").to_string();

            if newest_id.as_deref().is_none_or(|prev| event_id > prev) {
                newest_id = Some(event_id.to_string());
            }

            items.push(DmItem {
                event_id: EventId::new(event_id),
                sender_id,
                text,
            });
        }

        Ok((items, newest_id))
    }

    /// Resolve a Twitter username for `user_id`.
    ///
    /// Retries once on 401 after refreshing the access token.
    pub(super) async fn get_username_for_user_id(
        &self,
        user_id: &str,
    ) -> anyhow::Result<Option<String>> {
        let url = self.api_url(&format!("users/{user_id}?user.fields=username"));
        let resp = self
            .send_authenticated_request(Method::GET, &url, None, "get user")
            .await?;
        let data: Value = resp.json().await?;
        Ok(data["data"]["username"].as_str().map(str::to_string))
    }

    /// Verify credentials by calling GET /2/users/me.
    ///
    /// Retries once on 401 after refreshing the access token.
    pub(super) async fn verify_credentials(&self) -> anyhow::Result<()> {
        self.send_authenticated_request(
            Method::GET,
            &self.api_url("users/me"),
            None,
            "verify credentials",
        )
        .await?;
        Ok(())
    }
}
