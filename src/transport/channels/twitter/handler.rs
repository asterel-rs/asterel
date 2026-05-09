//! `Channel` trait implementation for X (Twitter): mention + DM polling,
//! tweet posting, and surface realization policy.
use std::future::Future;
use std::pin::Pin;

use uuid::Uuid;

use super::TwitterChannel;
use crate::contracts::channels::SurfaceRealizationPolicy;
use crate::transport::channels::traits::{
    Channel, ChannelCapabilities, ChannelEvent, ChannelMessage,
};

impl TwitterChannel {
    /// Build a `ChannelMessage` from a mention tweet.
    #[expect(
        clippy::unused_self,
        reason = "kept as an adapter method alongside other channel conversion helpers"
    )]
    pub(super) fn mention_to_channel_event(
        &self,
        tweet_id: &str,
        author_id: &str,
        text: &str,
    ) -> ChannelEvent {
        ChannelEvent::Message(ChannelMessage {
            id: Uuid::new_v4().to_string(),
            sender: author_id.to_string(),
            content: text.to_string(),
            channel: "twitter".to_string(),
            context_hint: Some("mention".to_string()),
            conversation_id: Some(tweet_id.to_string()),
            thread_id: None,
            reply_to: Some(tweet_id.to_string()),
            message_id: Some(tweet_id.to_string()),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            attachments: vec![],
        })
    }

    /// Build a `ChannelMessage` from a DM event.
    #[expect(
        clippy::unused_self,
        reason = "kept as an adapter method alongside other channel conversion helpers"
    )]
    pub(super) fn dm_to_channel_event(
        &self,
        event_id: &str,
        sender_id: &str,
        text: &str,
    ) -> ChannelEvent {
        ChannelEvent::Message(ChannelMessage {
            id: Uuid::new_v4().to_string(),
            sender: sender_id.to_string(),
            content: text.to_string(),
            channel: "twitter".to_string(),
            context_hint: Some("dm".to_string()),
            conversation_id: Some(format!("dm:{sender_id}")),
            thread_id: None,
            reply_to: None,
            message_id: Some(event_id.to_string()),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            attachments: vec![],
        })
    }
}

impl Channel for TwitterChannel {
    #[expect(
        clippy::unnecessary_literal_bound,
        reason = "trait signature returns &str across channel adapters"
    )]
    fn name(&self) -> &str {
        "twitter"
    }

    fn max_message_length(&self) -> usize {
        // Standard (verified) accounts have 25 000 char limit; unverified: 280.
        // Use 280 as the conservative default so chunking works correctly for
        // all account types. Operators can override via config if needed.
        280
    }

    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            max_message_length: 280,
            ..ChannelCapabilities::default()
        }
    }

    fn surface_realization_policy(&self) -> SurfaceRealizationPolicy {
        // Mentions are public; DMs are handled in-thread. Use the public
        // policy as the default surface constraint for the channel.
        SurfaceRealizationPolicy::twitter_public()
    }

    /// Send a tweet or DM.
    ///
    /// `recipient` routing:
    /// - `"dm:<user_id>"` → send as a DM to the given user ID
    /// - `"<tweet_id>"` (non-empty, no `dm:` prefix) → reply to that tweet
    /// - `""` (empty) → standalone tweet
    fn send<'a>(
        &'a self,
        message: &'a str,
        recipient: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(user_id) = recipient.strip_prefix("dm:") {
                self.send_dm(user_id, message).await
            } else if recipient.is_empty() {
                self.post_tweet(message).await.map(|_| ())
            } else {
                self.post_tweet_with_reply(message, Some(recipient))
                    .await
                    .map(|_| ())
            }
        })
    }

    #[allow(clippy::too_many_lines)]
    fn listen<'a>(
        &'a self,
        tx: tokio::sync::mpsc::Sender<ChannelEvent>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            tracing::info!(
                "Twitter channel listening (mentions every {}s, DMs every {}s)",
                self.mention_poll_interval_secs,
                self.dm_poll_interval_secs,
            );

            let mention_interval = std::time::Duration::from_secs(self.mention_poll_interval_secs);
            let dm_interval = std::time::Duration::from_secs(self.dm_poll_interval_secs);

            let mut mention_ticker = tokio::time::interval(mention_interval);
            let mut dm_ticker = tokio::time::interval(dm_interval);
            // Skip first tick (fires immediately) so we don't double-poll on startup.
            mention_ticker.tick().await;
            dm_ticker.tick().await;

            let mut last_mention_id: Option<String> = None;
            let mut last_dm_id: Option<String> = None;

            loop {
                tokio::select! {
                    _ = mention_ticker.tick() => {
                        match self.get_mentions(last_mention_id.as_deref()).await {
                            Ok((items, newest_id)) => {
                                if let Some(id) = newest_id {
                                    last_mention_id = Some(id);
                                }
                                for item in items {
                                    // Skip our own tweets
                                    if item.author_id == self.user_id.as_str() {
                                        continue;
                                    }
                                    // Allowlist check
                                    if !self.allowed_users.is_empty()
                                        && !self.is_user_allowed(&item.author_username)
                                    {
                                        tracing::debug!(
                                            "Twitter: ignoring mention from @{} (not in allowlist)",
                                            item.author_username
                                        );
                                        continue;
                                    }
                                    let event = self.mention_to_channel_event(
                                        &item.id,
                                        &item.author_id,
                                        &item.text,
                                    );
                                    if tx.send(event).await.is_err() {
                                        return Ok(());
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Twitter mention poll error: {e}");
                            }
                        }
                    }

                    _ = dm_ticker.tick() => {
                        match self.get_dm_events(last_dm_id.as_deref()).await {
                            Ok((mut items, _newest_id)) => {
                                items.sort_by(|left, right| {
                                    compare_event_ids(
                                        left.event_id.as_str(),
                                        right.event_id.as_str(),
                                    )
                                });
                                let mut latest_processed_dm_id = last_dm_id.clone();
                                for item in items {
                                    // Skip DMs sent by the bot itself
                                    if item.sender_id == self.user_id.as_str() {
                                        update_latest_event_id(&mut latest_processed_dm_id, item.event_id.as_str());
                                        continue;
                                    }
                                    let sender_username = if self.dm_allowlist_requires_username_resolution() {
                                        match self.get_username_for_user_id(&item.sender_id).await {
                                            Ok(username) => username,
                                            Err(error) => {
                                                tracing::warn!(
                                                    sender_id = %item.sender_id,
                                                    %error,
                                                    "Twitter: failed to resolve DM sender username; dropping DM because allowlist is configured"
                                                );
                                                break;
                                            }
                                        }
                                    } else {
                                        None
                                    };
                                    if !self.is_dm_sender_allowed(sender_username.as_deref()) {
                                        if let Some(username) = sender_username.as_deref() {
                                            tracing::debug!(
                                                "Twitter: ignoring DM from @{} (not in allowlist)",
                                                username
                                            );
                                        } else {
                                            tracing::debug!(
                                                sender_id = %item.sender_id,
                                                "Twitter: ignoring DM because sender username could not be resolved under allowlist enforcement"
                                            );
                                        }
                                        update_latest_event_id(&mut latest_processed_dm_id, item.event_id.as_str());
                                        continue;
                                    }
                                    let event = self.dm_to_channel_event(
                                        item.event_id.as_str(),
                                        &item.sender_id,
                                        &item.text,
                                    );
                                    if tx.send(event).await.is_err() {
                                        return Ok(());
                                    }
                                    update_latest_event_id(&mut latest_processed_dm_id, item.event_id.as_str());
                                }

                                if latest_processed_dm_id != last_dm_id {
                                    last_dm_id = latest_processed_dm_id;
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Twitter DM poll error: {e}");
                            }
                        }
                    }
                }
            }
        })
    }

    fn health_check<'a>(&'a self) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
        Box::pin(async move { self.verify_credentials().await.is_ok() })
    }
}

pub(super) fn compare_event_ids(left: &str, right: &str) -> std::cmp::Ordering {
    match (left.parse::<u128>(), right.parse::<u128>()) {
        (Ok(left_id), Ok(right_id)) => left_id.cmp(&right_id),
        _ => left.cmp(right),
    }
}

fn update_latest_event_id(current: &mut Option<String>, candidate: &str) {
    if current
        .as_deref()
        .is_none_or(|existing| compare_event_ids(candidate, existing).is_gt())
    {
        *current = Some(candidate.to_string());
    }
}
