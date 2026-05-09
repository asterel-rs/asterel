use uuid::Uuid;

use super::channel::DiscordChannel;
use super::gateway::GatewayEvent;
use crate::contracts::ids::{ChannelId, MessageId, UserId};
use crate::transport::channels::traits::{ChannelEvent, ChannelMessage};

pub(super) struct MessageCreateParams<'a> {
    tx: &'a tokio::sync::mpsc::Sender<ChannelEvent>,
    channel_id: &'a str,
    author_id: &'a str,
    author_is_bot: bool,
    reply_to_author_id: Option<UserId>,
    content: String,
    guild_id: Option<&'a str>,
    thread_id: Option<String>,
    message_id: &'a str,
    attachments: &'a [super::gateway::RawAttachment],
}

impl DiscordChannel {
    #[allow(clippy::too_many_lines)]
    pub(super) async fn handle_gateway_event(
        &self,
        event: GatewayEvent,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
    ) {
        match event {
            GatewayEvent::Ready { user_id, .. } => self.handle_ready(&user_id).await,
            GatewayEvent::MessageCreate {
                channel_id,
                author_id,
                author_is_bot,
                reply_to_author_id,
                content,
                guild_id,
                thread_id,
                message_id,
                attachments,
            } => {
                self.handle_message_create(MessageCreateParams {
                    tx,
                    channel_id: channel_id.as_str(),
                    author_id: author_id.as_str(),
                    author_is_bot,
                    reply_to_author_id,
                    content,
                    guild_id: guild_id.as_deref(),
                    thread_id,
                    message_id: message_id.as_str(),
                    attachments: &attachments,
                })
                .await;
            }
            GatewayEvent::InteractionCreate {
                interaction_key,
                interaction_token,
                interaction_type,
                channel_id,
                user_id,
                guild_id,
                data,
            } => {
                self.handle_interaction_create(
                    super::interaction_handler::InteractionCreateParams {
                        tx,
                        interaction_id: &interaction_key,
                        interaction_token: &interaction_token,
                        interaction_type,
                        channel_id: channel_id.as_str(),
                        user_id: user_id.as_str(),
                        guild_id: guild_id.as_deref(),
                        data: &data,
                    },
                )
                .await;
            }
            GatewayEvent::ReactionAdd {
                channel_id,
                message_id,
                user_id,
                emoji,
                guild_id,
            } => {
                self.dispatch_reaction_add_event(
                    tx, channel_id, message_id, user_id, emoji, guild_id,
                )
                .await;
            }
            GatewayEvent::ReactionRemove {
                channel_id,
                message_id,
                user_id,
                emoji,
                guild_id,
            } => {
                self.dispatch_reaction_remove_event(
                    tx, channel_id, message_id, user_id, emoji, guild_id,
                )
                .await;
            }
            GatewayEvent::MessageUpdate {
                channel_id,
                message_id,
                content,
                author_id,
                guild_id,
            } => {
                self.dispatch_message_update_event(
                    tx, channel_id, message_id, content, author_id, guild_id,
                )
                .await;
            }
            GatewayEvent::MessageDelete {
                channel_id,
                message_id,
                guild_id,
            } => {
                self.dispatch_message_delete_event(tx, channel_id, message_id, guild_id)
                    .await;
            }
            GatewayEvent::TypingStart {
                channel_id,
                user_id,
                guild_id,
                ..
            } => {
                self.dispatch_typing_start_event(tx, channel_id, user_id, guild_id)
                    .await;
            }
        }
    }

    async fn dispatch_reaction_add_event(
        &self,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
        channel_id: ChannelId,
        message_id: MessageId,
        user_id: UserId,
        emoji: String,
        guild_id: Option<String>,
    ) {
        self.handle_reaction_add(tx, channel_id, message_id, user_id, emoji, guild_id)
            .await;
    }

    async fn dispatch_reaction_remove_event(
        &self,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
        channel_id: ChannelId,
        message_id: MessageId,
        user_id: UserId,
        emoji: String,
        guild_id: Option<String>,
    ) {
        self.handle_reaction_remove(tx, channel_id, message_id, user_id, emoji, guild_id)
            .await;
    }

    async fn dispatch_message_update_event(
        &self,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
        channel_id: ChannelId,
        message_id: MessageId,
        content: Option<String>,
        author_id: Option<UserId>,
        guild_id: Option<String>,
    ) {
        self.handle_message_update(tx, channel_id, message_id, content, author_id, guild_id)
            .await;
    }

    async fn dispatch_message_delete_event(
        &self,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
        channel_id: ChannelId,
        message_id: MessageId,
        guild_id: Option<String>,
    ) {
        self.handle_message_delete(tx, channel_id, message_id, guild_id)
            .await;
    }

    async fn dispatch_typing_start_event(
        &self,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
        channel_id: ChannelId,
        user_id: UserId,
        guild_id: Option<String>,
    ) {
        self.handle_typing_start(tx, channel_id, user_id, guild_id)
            .await;
    }

    async fn handle_reaction_add(
        &self,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
        channel_id: ChannelId,
        message_id: MessageId,
        user_id: UserId,
        emoji: String,
        guild_id: Option<String>,
    ) {
        if !self.matches_guild_filter(guild_id.as_deref()) {
            return;
        }
        if self.is_bot_user(user_id.as_str()) {
            return;
        }
        if !self.is_user_allowed(user_id.as_str()) {
            return;
        }
        let event = ChannelEvent::ReactionAdd {
            channel_name: "discord".to_string(),
            channel_id,
            message_id,
            user_id,
            emoji,
        };
        self.send_channel_event(tx, event).await;
    }

    async fn handle_reaction_remove(
        &self,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
        channel_id: ChannelId,
        message_id: MessageId,
        user_id: UserId,
        emoji: String,
        guild_id: Option<String>,
    ) {
        if !self.matches_guild_filter(guild_id.as_deref()) {
            return;
        }
        if self.is_bot_user(user_id.as_str()) {
            return;
        }
        if !self.is_user_allowed(user_id.as_str()) {
            return;
        }
        let event = ChannelEvent::ReactionRemove {
            channel_name: "discord".to_string(),
            channel_id,
            message_id,
            user_id,
            emoji,
        };
        self.send_channel_event(tx, event).await;
    }

    async fn handle_message_update(
        &self,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
        channel_id: ChannelId,
        message_id: MessageId,
        content: Option<String>,
        author_id: Option<UserId>,
        guild_id: Option<String>,
    ) {
        if !self.matches_guild_filter(guild_id.as_deref()) {
            return;
        }
        if let Some(ref user_id) = author_id
            && self.is_bot_user(user_id.as_str())
        {
            return;
        }

        if let Some(new_content) = content {
            let event = ChannelEvent::MessageEdit {
                channel_name: "discord".to_string(),
                channel_id,
                message_id,
                new_content,
                user_id: author_id.unwrap_or_else(|| UserId::new("")),
            };
            self.send_channel_event(tx, event).await;
        }
    }

    async fn handle_message_delete(
        &self,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
        channel_id: ChannelId,
        message_id: MessageId,
        guild_id: Option<String>,
    ) {
        if !self.matches_guild_filter(guild_id.as_deref()) {
            return;
        }

        let event = ChannelEvent::MessageDelete {
            channel_name: "discord".to_string(),
            channel_id,
            message_id,
        };
        self.send_channel_event(tx, event).await;
    }

    async fn handle_typing_start(
        &self,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
        channel_id: ChannelId,
        user_id: UserId,
        guild_id: Option<String>,
    ) {
        if !self.matches_guild_filter(guild_id.as_deref()) {
            return;
        }
        if self.is_bot_user(user_id.as_str()) {
            return;
        }

        let event = ChannelEvent::TypingStart {
            channel_name: "discord".to_string(),
            channel_id,
            user_id,
        };
        self.send_channel_event(tx, event).await;
    }

    async fn send_channel_event(
        &self,
        tx: &tokio::sync::mpsc::Sender<ChannelEvent>,
        event: ChannelEvent,
    ) {
        if tx.send(event).await.is_err() {
            tracing::warn!("Discord: channel event receiver dropped");
        }
    }

    async fn handle_ready(&self, user_id: &UserId) {
        self.set_bot_user_id(user_id);
        tracing::info!("Discord: connected as user {user_id}");

        if let Some(app_id) = &self.config.application_id {
            let cmds = super::commands::build_default_commands();
            if let Err(e) = super::commands::register_commands(
                &self.http,
                app_id,
                self.config.guild_id.as_deref(),
                &cmds,
            )
            .await
            {
                tracing::warn!("Discord: failed to register slash commands: {e}");
            }
        }
    }

    async fn handle_message_create(&self, params: MessageCreateParams<'_>) {
        let MessageCreateParams {
            tx,
            channel_id,
            author_id,
            author_is_bot,
            reply_to_author_id,
            content,
            guild_id,
            thread_id,
            message_id,
            attachments,
        } = params;

        if self.is_bot_user(author_id) || author_is_bot {
            return;
        }
        if !self.is_user_allowed(author_id) {
            tracing::warn!("Discord: ignoring message from unauthorized user: {author_id}");
            return;
        }
        if !self.matches_guild_filter(guild_id) {
            return;
        }
        if content.is_empty() && attachments.is_empty() {
            return;
        }

        let bot_id = self.current_bot_user_id();
        let is_dm = guild_id.is_none();
        let mentions_bot = super::addressability::detect_bot_mention(
            &content,
            bot_id.as_ref().map(UserId::as_str),
        );
        let is_reply_to_bot = bot_id
            .as_ref()
            .is_some_and(|bot_id| reply_to_author_id.as_ref() == Some(bot_id));
        let is_thread = thread_id.is_some();

        let addr = super::addressability::AddressabilityContext {
            is_dm,
            mentions_bot,
            is_thread_with_bot: is_thread,
            is_reply_to_bot,
        };
        let mode = addr.classify_with_pickup_policy(self.config.pickup_policy.mode);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if mode == super::addressability::AddressabilityMode::Passive {
            tracing::trace!(
                %channel_id,
                %author_id,
                "Discord: passive message, not responding"
            );
            return;
        }
        if mode == super::addressability::AddressabilityMode::AmbientCandidate
            && !self.should_forward_ambient_message(&content, timestamp)
        {
            tracing::trace!(
                %channel_id,
                %author_id,
                "Discord: ambient message did not satisfy sparse pickup policy"
            );
            return;
        }

        let stripped_content = if mentions_bot {
            super::addressability::strip_bot_mention(&content, bot_id.as_ref().map(UserId::as_str))
        } else {
            content
        };

        if stripped_content.is_empty() && attachments.is_empty() {
            return;
        }

        let msg = ChannelMessage {
            id: Uuid::new_v4().to_string(),
            sender: author_id.to_string(),
            content: stripped_content,
            channel: "discord".to_string(),
            context_hint: super::addressability::channel_context_hint(mode, is_dm)
                .map(ToString::to_string),
            conversation_id: Some(channel_id.to_string()),
            thread_id,
            reply_to: None,
            message_id: Some(message_id.to_string()),
            timestamp,
            attachments: attachments.iter().map(Self::attachment_to_media).collect(),
        };

        if tx.send(ChannelEvent::Message(msg)).await.is_err() {
            tracing::warn!("Discord: channel message receiver dropped");
        }
    }
}
