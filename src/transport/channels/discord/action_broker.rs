//! Discord-backed implementation of the channel action broker.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use serde_json::Value;

use crate::core::tools::channel::ChannelActionBroker;

/// Discord implementation of the channel action broker.
pub struct DiscordActionBroker {
    http: super::http_client::DiscordHttpClient,
}

impl DiscordActionBroker {
    /// Create a Discord action broker from a bot token.
    #[must_use]
    pub fn new(bot_token: &str) -> Self {
        Self {
            http: super::http_client::DiscordHttpClient::new(bot_token),
        }
    }
}

impl ChannelActionBroker for DiscordActionBroker {
    fn channel_name(&self) -> &'static str {
        "discord"
    }

    fn create_thread<'a>(
        &'a self,
        channel_id: &'a str,
        name: &'a str,
        message_id: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            let payload = if let Some(starter_message_id) = message_id {
                self.http
                    .create_thread_from_message(channel_id, starter_message_id, name, None)
                    .await?
            } else {
                self.http
                    .create_thread(channel_id, name, None, None)
                    .await?
            };
            payload
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string)
                .ok_or_else(|| anyhow::anyhow!("Discord thread response missing thread id"))
        })
    }

    fn add_reaction<'a>(
        &'a self,
        channel_id: &'a str,
        message_id: &'a str,
        emoji: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move { self.http.add_reaction(channel_id, message_id, emoji).await })
    }

    fn send_with_components<'a>(
        &'a self,
        channel_id: &'a str,
        content: &'a str,
        components: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move {
            self.http
                .send_rich_message(channel_id, content, components)
                .await
        })
    }

    fn get_messages<'a>(
        &'a self,
        channel_id: &'a str,
        limit: Option<u32>,
        before: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>> {
        Box::pin(async move {
            let limit_u8 = match limit {
                Some(value) => Some(u8::try_from(value).map_err(|_| {
                    anyhow::anyhow!("Discord message history limit must be <= 255")
                })?),
                None => None,
            };
            let messages = self
                .http
                .get_messages(channel_id, limit_u8, before, None)
                .await?;
            Ok(Value::Array(messages))
        })
    }

    fn send_embed<'a>(
        &'a self,
        channel_id: &'a str,
        title: &'a str,
        description: &'a str,
        color: Option<u32>,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            self.http
                .send_embed_message(channel_id, Some(title), description, color)
                .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::DiscordActionBroker;
    use crate::core::tools::channel::ChannelActionBroker;

    #[test]
    fn discord_action_broker_new_constructs() {
        let broker = DiscordActionBroker::new("bot-token");
        assert_eq!(broker.channel_name(), "discord");
    }
}
