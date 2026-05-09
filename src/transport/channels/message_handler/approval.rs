//! Channel approval-broker context builders.
use std::time::Duration;

use super::super::traits::ChannelMessage;
use crate::config::Config;
use crate::security::ChannelApprovalCtx;

fn operator_fallback_enabled() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

/// Builds a channel-specific approval context from an inbound message.
pub(super) fn approval_context_for_message(
    config: &Config,
    msg: &ChannelMessage,
) -> ChannelApprovalCtx {
    let mut context = ChannelApprovalCtx {
        timeout: Duration::from_secs(60),
        operator_fallback: operator_fallback_enabled(),
        ..ChannelApprovalCtx::default()
    };

    match msg.channel.as_str() {
        "discord" => {
            context.bot_token = config
                .channels_config
                .discord
                .as_ref()
                .map(|discord| discord.bot_token.clone());
            context.channel_id = msg
                .conversation_id
                .clone()
                .or_else(|| Some(msg.sender.clone()));
        }
        "telegram" => {
            context.bot_token = config
                .channels_config
                .telegram
                .as_ref()
                .map(|telegram| telegram.bot_token.clone());
            context.channel_id = Some(msg.sender.clone());
        }
        "slack" => {
            context.bot_token = config
                .channels_config
                .slack
                .as_ref()
                .map(|slack| slack.bot_token.clone());
            context.channel_id = Some(msg.sender.clone());
        }
        "matrix" => {
            if let Some(matrix) = config.channels_config.matrix.as_ref() {
                context.bot_token = Some(matrix.access_token.clone());
                context.channel_id = Some(matrix.room_id.clone());
                context.homeserver = Some(matrix.homeserver.clone());
            }
        }
        _ => {}
    }

    context
}

/// Builds a channel-specific approval context from a non-message event.
pub(super) fn approval_context_for_event(
    config: &Config,
    channel_name: &str,
    conversation_id: Option<&str>,
    sender: &str,
) -> ChannelApprovalCtx {
    let mut context = ChannelApprovalCtx {
        timeout: Duration::from_secs(60),
        operator_fallback: operator_fallback_enabled(),
        ..ChannelApprovalCtx::default()
    };

    match channel_name {
        "discord" => {
            context.bot_token = config
                .channels_config
                .discord
                .as_ref()
                .map(|discord| discord.bot_token.clone());
            context.channel_id = conversation_id
                .map(ToString::to_string)
                .or_else(|| Some(sender.to_string()));
        }
        "telegram" => {
            context.bot_token = config
                .channels_config
                .telegram
                .as_ref()
                .map(|telegram| telegram.bot_token.clone());
            context.channel_id = Some(sender.to_string());
        }
        "slack" => {
            context.bot_token = config
                .channels_config
                .slack
                .as_ref()
                .map(|slack| slack.bot_token.clone());
            context.channel_id = Some(sender.to_string());
        }
        "matrix" => {
            if let Some(matrix) = config.channels_config.matrix.as_ref() {
                context.bot_token = Some(matrix.access_token.clone());
                context.channel_id = Some(matrix.room_id.clone());
                context.homeserver = Some(matrix.homeserver.clone());
            }
        }
        _ => {}
    }

    context
}

#[cfg(test)]
pub(super) fn operator_fallback_for_tests() -> bool {
    operator_fallback_enabled()
}
