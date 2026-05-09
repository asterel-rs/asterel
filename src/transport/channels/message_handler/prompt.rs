//! Channel prompt helpers: reply-target resolution, thinking-state
//! persistence, and conversation-command parsing (think, forget, etc.).
use anyhow::Result;

use super::super::startup::{ChannelRuntime, ChannelThinkingState};
use super::super::traits::ChannelMessage;
use super::reply::reply_to_origin;
use crate::config::{CommandsConfig, DmScope, ResetPolicy};
use crate::core::conversation_commands::{Command, parse_command};
use crate::core::providers::ThinkingLevel;

/// Returns the reply target for a message (conversation ID for Discord,
/// reply destination for channel-based transports, sender fallback otherwise).
pub(super) fn reply_target_for_message(
    config: &crate::config::Config,
    msg: &ChannelMessage,
) -> String {
    let primary_target = if matches!(
        msg.channel.as_str(),
        "discord" | "slack" | "telegram" | "irc"
    ) {
        msg.conversation_id.clone().or_else(|| {
            if msg.sender.trim().is_empty() {
                None
            } else {
                Some(msg.sender.clone())
            }
        })
    } else if msg.sender.trim().is_empty() {
        None
    } else {
        Some(msg.sender.clone())
    };

    if let Some(target) = primary_target {
        return target;
    }

    default_reply_target_for_channel(config, &msg.channel).unwrap_or_else(|| msg.sender.clone())
}

fn default_reply_target_for_channel(
    config: &crate::config::Config,
    channel_name: &str,
) -> Option<String> {
    match channel_name {
        "telegram" => config
            .channels_config
            .telegram
            .as_ref()
            .and_then(|cfg| cfg.default_to.clone()),
        "discord" => config
            .channels_config
            .discord
            .as_ref()
            .and_then(|cfg| cfg.default_to.clone()),
        "slack" => config
            .channels_config
            .slack
            .as_ref()
            .and_then(|cfg| cfg.default_to.clone()),
        _ => None,
    }
}

/// Builds a thinking-state lookup key scoped by channel, conversation,
/// and sender.
pub(super) fn channel_thinking_state_key(
    config: &crate::config::Config,
    msg: &ChannelMessage,
) -> String {
    let reset_policy = config.session.reset_policy_for_channel(&msg.channel);
    match reset_policy {
        ResetPolicy::Conversation => msg.conversation_id.as_deref().map_or_else(
            || dm_scope_key(config, msg),
            |conversation_id| {
                sender_scoped_key("conversation", &msg.channel, conversation_id, &msg.sender)
            },
        ),
        ResetPolicy::Thread => {
            if let Some(thread_id) = msg.thread_id.as_deref() {
                sender_scoped_key("thread", &msg.channel, thread_id, &msg.sender)
            } else if let Some(conversation_id) = msg.conversation_id.as_deref() {
                sender_scoped_key("conversation", &msg.channel, conversation_id, &msg.sender)
            } else {
                dm_scope_key(config, msg)
            }
        }
        ResetPolicy::Manual => format!("manual::{}::{}", msg.channel, msg.sender),
    }
}

fn sender_scoped_key(prefix: &str, channel: &str, scope_id: &str, sender: &str) -> String {
    format!("{prefix}::{channel}::{scope_id}::sender::{sender}")
}

fn dm_scope_key(config: &crate::config::Config, msg: &ChannelMessage) -> String {
    match config.session.dm_scope {
        DmScope::Global => "dm::global".to_string(),
        DmScope::Account => format!("dm::account::{}", msg.sender),
        DmScope::ChannelSender => format!("dm::channel_sender::{}::{}", msg.channel, msg.sender),
    }
}

fn command_allowlist_for_channel<'a>(commands: &'a CommandsConfig, channel: &str) -> &'a [String] {
    commands
        .by_channel
        .iter()
        .find_map(|(name, allowlist)| {
            if name.eq_ignore_ascii_case(channel) {
                Some(allowlist.as_slice())
            } else {
                None
            }
        })
        .unwrap_or(commands.allow_from.as_slice())
}

fn is_command_sender_allowed(commands: &CommandsConfig, channel: &str, sender: &str) -> bool {
    let allowlist = command_allowlist_for_channel(commands, channel);
    if allowlist.iter().any(|entry| entry.trim() == "*") {
        return true;
    }
    allowlist
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|allowed| allowed.eq_ignore_ascii_case(sender))
}

/// Loads the thinking state for a channel session key, falling back to
/// persistent storage and then config defaults.
pub(super) async fn load_channel_thinking_state(
    rt: &ChannelRuntime,
    channel_name: &str,
    key: &str,
) -> ChannelThinkingState {
    let guard = rt.thinking_states.read().await;
    if let Some(state) = guard.get(key).copied() {
        return state;
    }
    drop(guard);

    if let Some(ref session_manager) = rt.session_manager {
        let owner_scope = super::stream::tenant_scoped_owner_scope(key, &rt.tenant_policy_context);
        if let Ok(Some(session)) = session_manager
            .get_active_session_for_scope(channel_name, &owner_scope)
            .await
            && let Ok(Some(level)) = session_manager.load_thinking_level(&session.id).await
        {
            let state = ChannelThinkingState {
                thinking_level: level,
                show_reasoning: false,
            };
            let mut guard = rt.thinking_states.write().await;
            guard.insert(key.to_string(), state);
            return state;
        }
    }

    ChannelThinkingState::from_config(&rt.config)
}

/// Persists a thinking state to the in-memory cache and session storage.
pub(super) async fn save_channel_thinking_state(
    rt: &ChannelRuntime,
    channel_name: &str,
    key: &str,
    state: ChannelThinkingState,
) {
    let mut guard = rt.thinking_states.write().await;
    guard.insert(key.to_string(), state);
    drop(guard);

    if let Some(ref session_manager) = rt.session_manager {
        let owner_scope = super::stream::tenant_scoped_owner_scope(key, &rt.tenant_policy_context);
        match session_manager
            .resolve_session(channel_name, &owner_scope)
            .await
        {
            Ok(session) => {
                if let Err(error) = session_manager
                    .save_thinking_level(&session.id, state.thinking_level)
                    .await
                {
                    tracing::warn!(error = %error, session_key = %key, "failed to persist channel thinking level");
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, session_key = %key, "failed to resolve channel session for thinking level");
            }
        }
    }
}

/// Removes a thinking state entry from cache and persistent storage.
pub(super) async fn reset_channel_thinking_state(
    rt: &ChannelRuntime,
    channel_name: &str,
    key: &str,
) {
    let mut guard = rt.thinking_states.write().await;
    guard.remove(key);
    drop(guard);

    if let Some(ref session_manager) = rt.session_manager {
        let owner_scope = super::stream::tenant_scoped_owner_scope(key, &rt.tenant_policy_context);
        if let Ok(Some(session)) = session_manager
            .get_active_session_for_scope(channel_name, &owner_scope)
            .await
            && let Err(error) = session_manager
                .clear_session_thinking_level(&session.id)
                .await
        {
            tracing::warn!(error = %error, session_key = %key, "failed to clear channel thinking level");
        }
    }
}

/// Renders a human-readable summary of the current thinking state.
pub(super) fn render_think_status(state: ChannelThinkingState) -> String {
    format!(
        "Thinking level: {}, visibility: {}",
        state.thinking_level.as_str(),
        if state.show_reasoning { "show" } else { "hide" }
    )
}

/// Applies a `/think` command argument to the thinking state, returning
/// a human-readable status message.
///
/// # Errors
///
/// Returns an error if the argument is not a recognized level or keyword.
pub(super) fn apply_think_command(
    state: &mut ChannelThinkingState,
    level: Option<&str>,
) -> Result<String> {
    if let Some(raw_level) = level {
        if let Some(parsed) = ThinkingLevel::parse(raw_level) {
            state.thinking_level = parsed;
            return Ok(format!(
                "Thinking level set to: {}",
                state.thinking_level.as_str()
            ));
        }

        if raw_level.eq_ignore_ascii_case("show") {
            state.show_reasoning = true;
            return Ok("Thinking visibility set to: show".to_string());
        }
        if raw_level.eq_ignore_ascii_case("hide") {
            state.show_reasoning = false;
            return Ok("Thinking visibility set to: hide".to_string());
        }
        if raw_level.eq_ignore_ascii_case("status") {
            return Ok(render_think_status(*state));
        }

        anyhow::bail!(
            "Unsupported /think argument: {raw_level} (use off|low|medium|high|show|hide|status)"
        );
    }

    state.thinking_level = state.thinking_level.toggled();
    Ok(format!(
        "Thinking level set to: {}",
        state.thinking_level.as_str()
    ))
}

/// Intercepts Discord runtime commands (`/think`, `/new`) and handles
/// them locally. Returns `true` if the message was consumed.
pub(super) async fn try_handle_runtime_command(
    rt: &ChannelRuntime,
    msg: &ChannelMessage,
    reply_target: &str,
    thinking_key: &str,
) -> bool {
    if msg.channel != "discord" {
        return false;
    }

    let Some(command) = parse_command(&msg.content) else {
        return false;
    };

    if !is_command_sender_allowed(&rt.config.commands, &msg.channel, &msg.sender) {
        if let Err(error) = reply_to_origin(
            &rt.channels,
            &msg.channel,
            "Command execution is not allowed for this sender.",
            reply_target,
        )
        .await
        {
            tracing::warn!(%error, "failed to send command-allowlist denial reply");
        }
        return true;
    }

    match command {
        Command::Think { level } => {
            let mut state = load_channel_thinking_state(rt, &msg.channel, thinking_key).await;
            match apply_think_command(&mut state, level.as_deref()) {
                Ok(text) => {
                    save_channel_thinking_state(rt, &msg.channel, thinking_key, state).await;
                    let status_line = render_think_status(state);
                    let response = if text == status_line {
                        text
                    } else {
                        format!("{text}\n{status_line}")
                    };
                    if let Err(error) =
                        reply_to_origin(&rt.channels, &msg.channel, &response, reply_target).await
                    {
                        tracing::warn!(%error, "failed to send /think command reply");
                    }
                }
                Err(error) => {
                    if let Err(reply_error) = reply_to_origin(
                        &rt.channels,
                        &msg.channel,
                        &error.to_string(),
                        reply_target,
                    )
                    .await
                    {
                        tracing::warn!(%reply_error, "failed to send /think error reply");
                    }
                }
            }
            true
        }
        Command::New => {
            reset_channel_thinking_state(rt, &msg.channel, thinking_key).await;
            let tenant_context = rt.tenant_policy_context.clone();
            let owner_scope =
                super::stream::tenant_scoped_owner_scope(thinking_key, &tenant_context);
            if let Some(session_manager) = rt.session_manager.as_deref()
                && let Err(error) = session_manager
                    .reset_session(&msg.channel, &owner_scope)
                    .await
            {
                tracing::warn!(
                    channel = %msg.channel,
                    sender = %msg.sender,
                    session_key = owner_scope,
                    error = %error,
                    "failed to reset channel transcript session"
                );
            }
            let response =
                "Session reset. Thinking level set to off and visibility set to hide.".to_string();
            if let Err(error) =
                reply_to_origin(&rt.channels, &msg.channel, &response, reply_target).await
            {
                tracing::warn!(%error, "failed to send /new command reply");
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ChannelSecurityPolicy;
    use crate::transport::channels::traits::{MediaAttachment, MediaContent};

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
    fn think_command_accepts_level_and_visibility_tokens() {
        let mut state = ChannelThinkingState::default();

        let level_response = apply_think_command(&mut state, Some("high")).unwrap();
        assert_eq!(level_response, "Thinking level set to: high");
        assert_eq!(state.thinking_level.as_str(), "high");

        let visibility_response = apply_think_command(&mut state, Some("show")).unwrap();
        assert_eq!(visibility_response, "Thinking visibility set to: show");
        assert!(state.show_reasoning);
    }

    #[test]
    fn think_command_status_and_toggle_behave() {
        let mut state = ChannelThinkingState {
            thinking_level: crate::core::providers::ThinkingLevel::Low,
            show_reasoning: true,
        };

        let status_response = apply_think_command(&mut state, Some("status")).unwrap();
        assert_eq!(status_response, "Thinking level: low, visibility: show");
        assert_eq!(render_think_status(state), status_response);

        let toggle_response = apply_think_command(&mut state, None).unwrap();
        assert_eq!(toggle_response, "Thinking level set to: off");
        assert_eq!(state.thinking_level.as_str(), "off");
    }

    #[test]
    fn think_command_rejects_unknown_argument() {
        let mut state = ChannelThinkingState::default();
        let error = apply_think_command(&mut state, Some("mystery")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Unsupported /think argument: mystery")
        );
    }

    #[test]
    fn thinking_state_key_uses_channel_scope_and_sender() {
        let msg = discord_message_with_attachments();
        assert_eq!(
            channel_thinking_state_key(&crate::config::Config::default(), &msg),
            "conversation::discord::channel-77::sender::user-42"
        );
    }

    #[test]
    fn thread_reset_policy_scopes_channel_key_by_sender() {
        let mut config = crate::config::Config::default();
        config
            .session
            .reset_by_channel
            .insert("discord".to_string(), crate::config::ResetPolicy::Thread);
        let msg = discord_message_with_attachments();
        let mut other_sender = msg.clone();
        other_sender.sender = "user-99".to_string();

        assert_eq!(
            channel_thinking_state_key(&config, &msg),
            "thread::discord::thread-9::sender::user-42"
        );
        assert_eq!(
            channel_thinking_state_key(&config, &other_sender),
            "thread::discord::thread-9::sender::user-99"
        );
        assert_ne!(
            channel_thinking_state_key(&config, &msg),
            channel_thinking_state_key(&config, &other_sender)
        );
    }

    #[test]
    fn dm_scope_global_uses_single_shared_key() {
        let mut config = crate::config::Config::default();
        config.session.dm_scope = crate::config::DmScope::Global;

        let mut msg = discord_message_with_attachments();
        msg.conversation_id = None;
        msg.thread_id = None;

        assert_eq!(channel_thinking_state_key(&config, &msg), "dm::global");
    }

    #[test]
    fn reply_target_falls_back_to_configured_default_to() {
        let mut config = crate::config::Config::default();
        config.channels_config.discord = Some(crate::config::DiscordConfig {
            bot_token: "token".to_string(),
            application_id: None,
            guild_id: None,
            allowed_users: Vec::new(),
            intents: None,
            status: None,
            default_account: None,
            default_to: Some("channel:123".to_string()),
            activity_type: None,
            activity_name: None,
            thinking_embed: false,
            thinking_embed_include_preview: false,
            pickup_policy: crate::config::DiscordPickupPolicyConfig::default(),
            security: ChannelSecurityPolicy::default(),
        });

        let mut msg = discord_message_with_attachments();
        msg.sender.clear();
        msg.conversation_id = None;

        assert_eq!(reply_target_for_message(&config, &msg), "channel:123");
    }

    #[test]
    fn command_sender_allowlist_prefers_channel_override() {
        let commands = crate::config::CommandsConfig {
            allow_from: vec!["global-user".to_string()],
            by_channel: std::collections::BTreeMap::from([(
                "discord".to_string(),
                vec!["discord-user".to_string()],
            )]),
        };

        assert!(is_command_sender_allowed(
            &commands,
            "discord",
            "discord-user"
        ));
        assert!(!is_command_sender_allowed(
            &commands,
            "discord",
            "global-user"
        ));
        assert!(is_command_sender_allowed(
            &commands,
            "telegram",
            "global-user"
        ));
    }

    #[test]
    fn reply_target_for_slack_prefers_conversation_id() {
        let msg = ChannelMessage {
            id: "msg-1".to_string(),
            sender: "U123".to_string(),
            content: "hello".to_string(),
            channel: "slack".to_string(),
            context_hint: None,
            conversation_id: Some("C456".to_string()),
            thread_id: None,
            reply_to: None,
            message_id: None,
            timestamp: 0,
            attachments: Vec::new(),
        };

        assert_eq!(
            reply_target_for_message(&crate::config::Config::default(), &msg),
            "C456"
        );
    }

    #[test]
    fn reply_target_for_telegram_prefers_conversation_id() {
        let msg = ChannelMessage {
            id: "msg-1".to_string(),
            sender: "user-123".to_string(),
            content: "hello".to_string(),
            channel: "telegram".to_string(),
            context_hint: None,
            conversation_id: Some("chat-456".to_string()),
            thread_id: None,
            reply_to: None,
            message_id: Some("77".to_string()),
            timestamp: 0,
            attachments: Vec::new(),
        };

        assert_eq!(
            reply_target_for_message(&crate::config::Config::default(), &msg),
            "chat-456"
        );
    }
}
