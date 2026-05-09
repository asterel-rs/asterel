//! Message-dispatch orchestration: composes lower-level approval,
//! policy, routing, and execution-context helpers.
pub(super) use super::execution_context::{build_event_execution_context, build_execution_context};
pub(super) use super::policy::resolve_channel_policy_for_name;
pub(super) use super::routing::build_event_context_message;

use super::super::startup::ChannelRuntime;
use super::super::traits::ChannelMessage;
#[cfg(test)]
use super::approval::approval_context_for_message;
use super::policy::resolve_channel_policy;
#[cfg(test)]
use super::routing::{resolve_group_isolation, resolve_routing_group};
use super::{ChannelMessageProcessingState, routing};

pub(super) fn build_message_processing_state(
    rt: &ChannelRuntime,
    msg: &ChannelMessage,
) -> ChannelMessageProcessingState {
    let (effective_autonomy, tool_allowlist) = resolve_channel_policy(rt, msg);
    routing::build_message_processing_state(rt, msg, effective_autonomy, tool_allowlist)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::config::{
        ChannelSecurityPolicy, DiscordConfig, MatrixConfig, SlackConfig, TelegramConfig,
    };
    use crate::contracts::ids::{ChannelId, MessageId, UserId};
    use crate::runtime::RuntimeSandboxClass;
    use crate::transport::channels::traits::{ChannelEvent, MediaAttachment, MediaContent};

    fn discord_message_with_attachments() -> ChannelMessage {
        ChannelMessage {
            id: "msg-1".to_string(),
            sender: "user-42".to_string(),
            content: "hello from discord".to_string(),
            channel: "discord".to_string(),
            context_hint: None,
            conversation_id: Some("channel-77".to_string()),
            thread_id: Some("thread-9".to_string()),
            reply_to: None,
            message_id: Some("discord-msg-abc".to_string()),
            timestamp: 1_716_171_717,
            attachments: vec![
                MediaAttachment {
                    mime_type: "image/png".to_string(),
                    data: MediaContent::Url("https://cdn.discord.test/img.png".to_string()),
                    filename: Some("img.png".to_string()),
                },
                MediaAttachment {
                    mime_type: "application/pdf".to_string(),
                    data: MediaContent::Url("https://cdn.discord.test/doc.pdf".to_string()),
                    filename: Some("doc.pdf".to_string()),
                },
            ],
        }
    }

    #[test]
    fn routing_group_resolution_prefers_matching_rule() {
        let mut config = crate::config::Config::default();
        config.channels_config.routing_rules = vec![crate::config::schema::RoutingRuleConfig {
            channel: "discord".to_string(),
            sender: Some("user-42".to_string()),
            conversation_id: Some("channel-77".to_string()),
            group: "ops".to_string(),
        }];

        let msg = discord_message_with_attachments();
        let group = resolve_routing_group(&config, &msg);
        assert_eq!(group, "ops");
    }

    #[test]
    fn group_isolation_rule_is_runtime_capped_when_container_unavailable() {
        let mut config = crate::config::Config::default();
        config.channels_config.group_isolation_rules =
            vec![crate::config::GroupIsolationRuleConfig {
                group: "ops".to_string(),
                filesystem: crate::config::GroupIsolationLevel::Container,
                process: crate::config::GroupIsolationLevel::Container,
                network: crate::config::GroupIsolationLevel::Container,
            }];

        let profile = resolve_group_isolation(&config, "ops", RuntimeSandboxClass::Workspace);
        assert_eq!(
            profile.filesystem,
            crate::config::GroupIsolationLevel::Workspace
        );
        assert_eq!(
            profile.process,
            crate::config::GroupIsolationLevel::Workspace
        );
        assert_eq!(
            profile.network,
            crate::config::GroupIsolationLevel::Workspace
        );
    }

    #[test]
    fn group_isolation_global_mode_defaults_to_container_for_container_runtime() {
        let mut config = crate::config::Config::default();
        config.channels_config.group_isolation_mode = crate::config::GroupIsolationMode::Global;

        let profile =
            resolve_group_isolation(&config, "discord::user-42", RuntimeSandboxClass::Container);
        assert_eq!(
            profile.filesystem,
            crate::config::GroupIsolationLevel::Container
        );
        assert_eq!(
            profile.process,
            crate::config::GroupIsolationLevel::Container
        );
        assert_eq!(
            profile.network,
            crate::config::GroupIsolationLevel::Container
        );
    }

    #[test]
    fn approval_context_discord_prefers_conversation_id() {
        let mut config = crate::config::Config::default();
        config.channels_config.discord = Some(DiscordConfig {
            bot_token: "discord-bot-token".to_string(),
            application_id: None,
            guild_id: None,
            allowed_users: Vec::new(),
            intents: None,
            status: None,
            default_account: None,
            default_to: None,
            activity_type: None,
            activity_name: None,
            thinking_embed: false,
            thinking_embed_include_preview: false,
            pickup_policy: crate::config::DiscordPickupPolicyConfig::default(),
            security: ChannelSecurityPolicy::default(),
        });
        let msg = discord_message_with_attachments();

        let context = approval_context_for_message(&config, &msg);
        assert_eq!(context.bot_token.as_deref(), Some("discord-bot-token"));
        assert_eq!(context.channel_id.as_deref(), Some("channel-77"));
        assert_eq!(context.timeout, Duration::from_secs(60));
        assert_eq!(
            context.operator_fallback,
            super::super::approval::operator_fallback_for_tests()
        );
    }

    #[test]
    fn approval_context_discord_falls_back_to_sender_without_conversation() {
        let mut config = crate::config::Config::default();
        config.channels_config.discord = Some(DiscordConfig {
            bot_token: "discord-bot-token".to_string(),
            application_id: None,
            guild_id: None,
            allowed_users: Vec::new(),
            intents: None,
            status: None,
            default_account: None,
            default_to: None,
            activity_type: None,
            activity_name: None,
            thinking_embed: false,
            thinking_embed_include_preview: false,
            pickup_policy: crate::config::DiscordPickupPolicyConfig::default(),
            security: ChannelSecurityPolicy::default(),
        });
        let mut msg = discord_message_with_attachments();
        msg.conversation_id = None;

        let context = approval_context_for_message(&config, &msg);
        assert_eq!(context.channel_id.as_deref(), Some("user-42"));
        assert_eq!(
            context.operator_fallback,
            super::super::approval::operator_fallback_for_tests()
        );
    }

    #[test]
    fn approval_context_telegram_uses_sender_chat_id() {
        let mut config = crate::config::Config::default();
        config.channels_config.telegram = Some(TelegramConfig {
            bot_token: "telegram-bot-token".to_string(),
            allowed_users: Vec::new(),
            default_account: None,
            default_to: None,
            security: ChannelSecurityPolicy::default(),
        });
        let mut msg = discord_message_with_attachments();
        msg.channel = "telegram".to_string();
        msg.sender = "998877".to_string();

        let context = approval_context_for_message(&config, &msg);
        assert_eq!(context.bot_token.as_deref(), Some("telegram-bot-token"));
        assert_eq!(context.channel_id.as_deref(), Some("998877"));
        assert_eq!(
            context.operator_fallback,
            super::super::approval::operator_fallback_for_tests()
        );
    }

    #[test]
    fn approval_context_slack_uses_sender_channel_id() {
        let mut config = crate::config::Config::default();
        config.channels_config.slack = Some(SlackConfig {
            bot_token: "xoxb-slack-token".to_string(),
            app_token: None,
            channel_id: None,
            default_account: None,
            default_to: None,
            allowed_users: Vec::new(),
            security: ChannelSecurityPolicy::default(),
        });
        let mut msg = discord_message_with_attachments();
        msg.channel = "slack".to_string();
        msg.sender = "C123456".to_string();

        let context = approval_context_for_message(&config, &msg);
        assert_eq!(context.bot_token.as_deref(), Some("xoxb-slack-token"));
        assert_eq!(context.channel_id.as_deref(), Some("C123456"));
        assert_eq!(
            context.operator_fallback,
            super::super::approval::operator_fallback_for_tests()
        );
    }

    #[test]
    fn approval_context_matrix_uses_configured_room_and_homeserver() {
        let mut config = crate::config::Config::default();
        config.channels_config.matrix = Some(MatrixConfig {
            homeserver: "https://matrix.example.org/".to_string(),
            access_token: "matrix-access-token".to_string(),
            room_id: "!ops:example.org".to_string(),
            allowed_users: Vec::new(),
            security: ChannelSecurityPolicy::default(),
        });
        let mut msg = discord_message_with_attachments();
        msg.channel = "matrix".to_string();
        msg.sender = "@user:example.org".to_string();

        let context = approval_context_for_message(&config, &msg);
        assert_eq!(context.bot_token.as_deref(), Some("matrix-access-token"));
        assert_eq!(context.channel_id.as_deref(), Some("!ops:example.org"));
        assert_eq!(
            context.homeserver.as_deref(),
            Some("https://matrix.example.org/")
        );
        assert_eq!(
            context.operator_fallback,
            super::super::approval::operator_fallback_for_tests()
        );
    }

    #[test]
    fn event_context_message_reaction_add() {
        let event = ChannelEvent::ReactionAdd {
            channel_name: "discord".to_string(),
            channel_id: ChannelId::new("chan-1"),
            message_id: MessageId::new("m-1"),
            user_id: UserId::new("u-1"),
            emoji: ":thumbs_up:".to_string(),
        };

        assert_eq!(
            build_event_context_message(&event).as_deref(),
            Some("A user reacted with :thumbs_up: on message m-1 in this channel.")
        );
    }

    #[test]
    fn event_context_message_reaction_remove() {
        let event = ChannelEvent::ReactionRemove {
            channel_name: "discord".to_string(),
            channel_id: ChannelId::new("chan-1"),
            message_id: MessageId::new("m-1"),
            user_id: UserId::new("u-1"),
            emoji: ":fire:".to_string(),
        };

        assert_eq!(
            build_event_context_message(&event).as_deref(),
            Some("A user removed their :fire: reaction from message m-1.")
        );
    }

    #[test]
    fn event_context_message_message_edit() {
        let event = ChannelEvent::MessageEdit {
            channel_name: "discord".to_string(),
            channel_id: ChannelId::new("chan-1"),
            message_id: MessageId::new("m-1"),
            new_content: "updated text".to_string(),
            user_id: UserId::new("u-1"),
        };

        assert_eq!(
            build_event_context_message(&event).as_deref(),
            Some("A user edited message m-1. New content: updated text")
        );
    }
}
