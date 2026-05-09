//! Telegram Bot API channel adapter: long-polling, message dispatch,
//! media upload, and user allowlist enforcement.
pub mod api;
pub mod handler;
use crate::transport::channels::policy::{AllowlistMatch, is_allowed_user};

#[cfg(test)]
mod tests;

/// Telegram channel — long-polls the Bot API for updates
pub struct TelegramChannel {
    bot_token: String,
    allowed_users: Vec<String>,
    client: reqwest::Client,
    api_base_url: String,
}

impl TelegramChannel {
    #[must_use]
    pub fn new(bot_token: String, allowed_users: Vec<String>) -> Self {
        Self {
            bot_token,
            allowed_users,
            client: crate::utils::http::build_http_client(),
            api_base_url: "https://api.telegram.org".to_string(),
        }
    }

    #[cfg(test)]
    pub(super) fn new_with_api_base_url(
        bot_token: String,
        allowed_users: Vec<String>,
        api_base_url: String,
    ) -> Self {
        Self {
            bot_token,
            allowed_users,
            client: crate::utils::http::build_http_client(),
            api_base_url,
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!(
            "{}/bot{}/{method}",
            self.api_base_url.trim_end_matches('/'),
            self.bot_token
        )
    }

    fn is_user_allowed(&self, username: &str) -> bool {
        is_allowed_user(&self.allowed_users, username, AllowlistMatch::Exact)
    }

    fn is_any_user_allowed<'a, I>(&self, identities: I) -> bool
    where
        I: IntoIterator<Item = &'a str>,
    {
        identities.into_iter().any(|id| self.is_user_allowed(id))
    }
}
