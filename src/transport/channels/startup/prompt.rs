//! System prompt assembly for the channel runtime: merges tool
//! descriptions, skills, and the most-capable channel's capabilities.
use std::sync::Arc;

use crate::config::Config;
use crate::security::SecurityPolicy;
use crate::transport::channels::traits::{Channel, ChannelCapabilities};

pub(super) fn build_channel_system_prompt(
    config: &Config,
    workspace: &std::path::Path,
    model: &str,
    channels: &[Arc<dyn Channel>],
    skill_entries: &[crate::plugins::skills::PromptSkillIndexEntry],
    security: &SecurityPolicy,
) -> String {
    let most_capable_channel_capabilities = most_capable_channel_capabilities(channels);
    let mcp_tool_provider = crate::runtime::services::runtime_mcp_tool_provider();
    let tool_descs = crate::core::tools::tool_desc_with_security_and_mcp_provider(
        config.browser.enabled,
        config.composio.enabled,
        Some(&config.mcp),
        security,
        most_capable_channel_capabilities.as_ref(),
        mcp_tool_provider.as_ref(),
    );
    let prompt_tool_descs: Vec<(&str, &str)> = tool_descs
        .iter()
        .map(|(name, description)| (name.as_str(), description.as_str()))
        .collect();
    let channel_capabilities = build_channel_capabilities_section(channels);
    let prompt_options = crate::transport::channels::SystemPromptOptions {
        companion_behavior: Some(config.persona.companion.clone()),
    };
    crate::transport::channels::build_system_prompt_from_index_opts(
        workspace,
        model,
        &prompt_tool_descs,
        skill_entries,
        channel_capabilities.as_deref(),
        &prompt_options,
    )
}

fn capability_score(caps: &ChannelCapabilities) -> usize {
    [
        caps.can_edit_message,
        caps.can_delete_message,
        caps.can_send_media,
        caps.can_send_embed,
        caps.can_send_typing,
        caps.can_create_thread,
        caps.can_manage_thread_members,
        caps.can_add_reaction,
        caps.can_read_reactions,
        caps.can_send_buttons,
        caps.can_send_select_menu,
        caps.can_send_modal,
        caps.can_fetch_history,
        caps.can_receive_reactions,
        caps.can_receive_edits,
        caps.can_receive_deletes,
        caps.can_receive_typing,
    ]
    .into_iter()
    .filter(|flag| *flag)
    .count()
}

fn most_capable_channel_capabilities(channels: &[Arc<dyn Channel>]) -> Option<ChannelCapabilities> {
    channels
        .iter()
        .map(|channel| channel.capabilities())
        .filter(|caps| capability_score(caps) > 0)
        .max_by_key(capability_score)
}

fn has_non_default_capabilities(caps: &ChannelCapabilities) -> bool {
    caps.can_edit_message
        || caps.can_delete_message
        || caps.can_send_media
        || caps.can_send_embed
        || caps.can_send_typing
        || caps.can_create_thread
        || caps.can_manage_thread_members
        || caps.can_add_reaction
        || caps.can_read_reactions
        || caps.can_send_buttons
        || caps.can_send_select_menu
        || caps.can_send_modal
        || caps.can_fetch_history
        || caps.can_receive_reactions
        || caps.can_receive_edits
        || caps.can_receive_deletes
        || caps.can_receive_typing
}

fn format_channel_display_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    for segment in name.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        if segment.is_empty() {
            continue;
        }
        if !result.is_empty() {
            result.push(' ');
        }
        let mut chars = segment.chars();
        if let Some(first) = chars.next() {
            result.push(first.to_ascii_uppercase());
            for ch in chars {
                result.push(ch.to_ascii_lowercase());
            }
        }
    }
    result
}

pub(super) fn build_channel_capabilities_section(channels: &[Arc<dyn Channel>]) -> Option<String> {
    use std::fmt::Write;

    let mut section = String::new();
    let mut has_any = false;

    for channel in channels {
        let caps = channel.capabilities();
        if !has_non_default_capabilities(&caps) {
            continue;
        }

        has_any = true;
        if section.is_empty() {
            section.push_str("# Your Communication Channels\n\n");
        }

        let display_name = format_channel_display_name(channel.name());
        let _ = writeln!(section, "## {display_name} (active)");

        let mut actions = Vec::new();
        if caps.can_edit_message && caps.can_delete_message {
            actions.push("Edit and delete messages".to_string());
        } else {
            if caps.can_edit_message {
                actions.push("Edit messages".to_string());
            }
            if caps.can_delete_message {
                actions.push("Delete messages".to_string());
            }
        }

        if caps.can_create_thread && caps.can_manage_thread_members {
            actions.push("Create and manage threads".to_string());
        } else {
            if caps.can_create_thread {
                actions.push("Create threads".to_string());
            }
            if caps.can_manage_thread_members {
                actions.push("Manage thread members".to_string());
            }
        }

        if caps.can_add_reaction {
            actions.push("Add reactions to messages".to_string());
        }

        if caps.can_send_buttons && caps.can_send_select_menu && caps.can_send_modal {
            actions.push("Send buttons, select menus, and modals".to_string());
        } else {
            if caps.can_send_buttons {
                actions.push("Send buttons".to_string());
            }
            if caps.can_send_select_menu {
                actions.push("Send select menus".to_string());
            }
            if caps.can_send_modal {
                actions.push("Send modals".to_string());
            }
        }

        if caps.can_fetch_history {
            actions.push("Fetch message history".to_string());
        }
        if caps.can_send_embed {
            actions.push("Send rich embeds".to_string());
        }
        if caps.can_send_media {
            actions.push("Send files and images".to_string());
        }
        if caps.can_send_typing {
            actions.push("Send typing indicators".to_string());
        }

        if !actions.is_empty() {
            section.push_str("You can perform these actions:\n");
            for action in actions {
                let _ = writeln!(section, "- {action}");
            }
            section.push('\n');
        }

        let mut events = Vec::new();
        if caps.can_receive_reactions {
            events.push("User reactions (add/remove)".to_string());
        }
        if caps.can_receive_edits && caps.can_receive_deletes {
            events.push("Message edits and deletions".to_string());
        } else {
            if caps.can_receive_edits {
                events.push("Message edits".to_string());
            }
            if caps.can_receive_deletes {
                events.push("Message deletions".to_string());
            }
        }
        if caps.can_receive_typing {
            events.push("Typing indicators".to_string());
        }

        if !events.is_empty() {
            section.push_str("You receive these events:\n");
            for event in events {
                let _ = writeln!(section, "- {event}");
            }
            section.push('\n');
        }

        append_channel_style(&mut section, channel.name());
    }

    if has_any { Some(section) } else { None }
}

fn append_channel_style(section: &mut String, channel_name: &str) {
    let style = crate::transport::channels::style_profile::profile_for_channel(channel_name);
    let block = crate::transport::channels::style_profile::render_channel_style_block(&style);
    section.push_str(&block);
    section.push('\n');
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use super::*;
    use crate::transport::channels::traits::ChannelEvent;

    struct TestChannel {
        name: &'static str,
        capabilities: ChannelCapabilities,
    }

    impl Channel for TestChannel {
        fn name(&self) -> &str {
            self.name
        }

        fn capabilities(&self) -> ChannelCapabilities {
            self.capabilities.clone()
        }

        fn send<'a>(
            &'a self,
            _message: &'a str,
            _recipient: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            Box::pin(async move { Ok(()) })
        }

        fn listen<'a>(
            &'a self,
            _tx: tokio::sync::mpsc::Sender<ChannelEvent>,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            Box::pin(async move { Ok(()) })
        }
    }

    fn make_channel(name: &'static str, capabilities: ChannelCapabilities) -> Arc<dyn Channel> {
        Arc::new(TestChannel { name, capabilities })
    }

    #[test]
    fn discord_capabilities_generate_expected_section() {
        let channels = vec![make_channel(
            "discord",
            ChannelCapabilities {
                can_edit_message: true,
                can_delete_message: true,
                can_send_media: true,
                can_send_embed: true,
                can_send_typing: true,
                can_create_thread: true,
                can_manage_thread_members: true,
                can_add_reaction: true,
                can_read_reactions: true,
                can_send_buttons: true,
                can_send_select_menu: true,
                can_send_modal: true,
                can_fetch_history: true,
                can_receive_reactions: true,
                can_receive_edits: true,
                can_receive_deletes: true,
                can_receive_typing: true,
                ..ChannelCapabilities::default()
            },
        )];

        let section = build_channel_capabilities_section(&channels).expect("section should exist");
        assert!(section.contains("# Your Communication Channels"));
        assert!(section.contains("## Discord (active)"));
        assert!(section.contains("- Edit and delete messages"));
        assert!(section.contains("- Create and manage threads"));
        assert!(section.contains("- Add reactions to messages"));
        assert!(section.contains("- Send buttons, select menus, and modals"));
        assert!(section.contains("- Fetch message history"));
        assert!(section.contains("- Send rich embeds"));
        assert!(section.contains("- Send files and images"));
        assert!(section.contains("You receive these events:"));
        assert!(section.contains("- User reactions (add/remove)"));
        assert!(section.contains("- Message edits and deletions"));
        assert!(section.contains("- Typing indicators"));
    }

    #[test]
    fn text_only_channel_produces_no_section() {
        let channels = vec![make_channel("cli", ChannelCapabilities::default())];
        assert!(build_channel_capabilities_section(&channels).is_none());
    }

    #[test]
    fn mixed_channels_only_describe_non_default_capabilities() {
        let channels = vec![
            make_channel(
                "discord",
                ChannelCapabilities {
                    can_send_embed: true,
                    can_add_reaction: true,
                    ..ChannelCapabilities::default()
                },
            ),
            make_channel("cli", ChannelCapabilities::default()),
        ];

        let section = build_channel_capabilities_section(&channels).expect("section should exist");
        assert!(section.contains("## Discord (active)"));
        assert!(section.contains("- Add reactions to messages"));
        assert!(section.contains("- Send rich embeds"));
        assert!(!section.contains("## Cli (active)"));
    }
}
