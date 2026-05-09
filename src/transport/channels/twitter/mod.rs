//! X (Twitter) channel adapter: OAuth 2.0 PKCE, mention + DM polling,
//! tweet posting, and user allowlist enforcement.
pub mod api;
pub mod handler;

#[cfg(test)]
mod tests;

use std::sync::{Arc, RwLock};

use crate::contracts::ids::UserId;
use crate::transport::channels::policy::{AllowlistMatch, is_allowed_user};

/// X (Twitter) channel — polls mentions and DMs via Twitter API v2.
pub struct TwitterChannel {
    pub(super) client_id: String,
    pub(super) client_secret: String,
    /// Current OAuth 2.0 access token (may be refreshed).
    pub(super) access_token: Arc<RwLock<String>>,
    /// OAuth 2.0 refresh token for token renewal.
    pub(super) refresh_token: Arc<RwLock<String>>,
    /// Numeric Twitter user ID of the bot account.
    pub(super) user_id: UserId,
    pub(super) allowed_users: Vec<String>,
    /// Seconds between mention polls. Default: 180.
    pub(super) mention_poll_interval_secs: u64,
    /// Seconds between DM polls. Default: 300.
    pub(super) dm_poll_interval_secs: u64,
    pub(super) client: reqwest::Client,
    api_base_url: String,
}

impl TwitterChannel {
    #[must_use]
    pub fn new(
        client_id: String,
        client_secret: String,
        access_token: String,
        refresh_token: String,
        user_id: UserId,
        allowed_users: Vec<String>,
        mention_poll_interval_secs: u64,
        dm_poll_interval_secs: u64,
    ) -> Self {
        Self {
            client_id,
            client_secret,
            access_token: Arc::new(RwLock::new(access_token)),
            refresh_token: Arc::new(RwLock::new(refresh_token)),
            user_id,
            allowed_users,
            mention_poll_interval_secs,
            dm_poll_interval_secs,
            client: crate::utils::http::build_http_client(),
            api_base_url: "https://api.twitter.com/2".to_string(),
        }
    }

    #[cfg(test)]
    pub(super) fn new_with_api_base_url(
        client_id: String,
        client_secret: String,
        access_token: String,
        refresh_token: String,
        user_id: UserId,
        allowed_users: Vec<String>,
        mention_poll_interval_secs: u64,
        dm_poll_interval_secs: u64,
        api_base_url: String,
    ) -> Self {
        Self {
            client_id,
            client_secret,
            access_token: Arc::new(RwLock::new(access_token)),
            refresh_token: Arc::new(RwLock::new(refresh_token)),
            user_id,
            allowed_users,
            mention_poll_interval_secs,
            dm_poll_interval_secs,
            client: crate::utils::http::build_http_client(),
            api_base_url,
        }
    }

    pub(super) fn api_url(&self, path_and_query: &str) -> String {
        format!(
            "{}/{}",
            self.api_base_url.trim_end_matches('/'),
            path_and_query.trim_start_matches('/')
        )
    }

    fn is_user_allowed(&self, username: &str) -> bool {
        is_allowed_user(
            &self.allowed_users,
            username,
            AllowlistMatch::AsciiCaseInsensitive,
        )
    }

    fn allows_all_users(&self) -> bool {
        self.allowed_users.iter().any(|user| user == "*")
    }

    fn dm_allowlist_requires_username_resolution(&self) -> bool {
        !self.allowed_users.is_empty() && !self.allows_all_users()
    }

    fn is_dm_sender_allowed(&self, sender_username: Option<&str>) -> bool {
        if self.allowed_users.is_empty() || self.allows_all_users() {
            return true;
        }

        sender_username.is_some_and(|username| self.is_user_allowed(username))
    }

    pub(super) fn read_access_token(&self) -> String {
        self.access_token
            .read()
            .map(|t| t.clone())
            .unwrap_or_default()
    }
}
