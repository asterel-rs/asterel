//! Channel routing and isolation helpers.
use super::super::ingress_policy::{
    apply_external_ingress_policy, tenant_scoped_channel_entity_id,
};
use super::super::startup::ChannelRuntime;
use super::super::traits::{ChannelEvent, ChannelMessage};
use super::prompt::{channel_thinking_state_key, reply_target_for_message};
use super::{ChannelMessageProcessingState, GroupIsolationProfile};
use crate::config::{Config, GroupIsolationLevel, GroupIsolationMode};
use crate::runtime::RuntimeSandboxClass;

/// Converts a channel event into a human-readable context string for the LLM.
pub(super) fn build_event_context_message(event: &ChannelEvent) -> Option<String> {
    match event {
        ChannelEvent::ReactionAdd {
            emoji, message_id, ..
        } => Some(format!(
            "A user reacted with {emoji} on message {message_id} in this channel."
        )),
        ChannelEvent::ReactionRemove {
            emoji, message_id, ..
        } => Some(format!(
            "A user removed their {emoji} reaction from message {message_id}."
        )),
        ChannelEvent::MessageEdit {
            message_id,
            new_content,
            ..
        } => Some(format!(
            "A user edited message {message_id}. New content: {new_content}"
        )),
        _ => None,
    }
}

/// Determines the routing group for a message using configured rules.
pub(super) fn resolve_routing_group(config: &Config, msg: &ChannelMessage) -> String {
    for rule in &config.channels_config.routing_rules {
        if rule.channel != msg.channel {
            continue;
        }
        if let Some(sender) = &rule.sender
            && sender != &msg.sender
        {
            continue;
        }
        if let Some(conversation_id) = &rule.conversation_id
            && msg.conversation_id.as_deref() != Some::<&str>(conversation_id.as_str())
        {
            continue;
        }

        return rule.group.clone();
    }

    format!("{}::{}", msg.channel, msg.sender)
}

/// Sanitizes a routing group name for filesystem-safe directory names.
pub(super) fn normalize_group_component(group: &str) -> String {
    group
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

/// Returns the default group isolation profile for the given mode.
pub(super) fn default_group_isolation(mode: GroupIsolationMode) -> GroupIsolationProfile {
    if mode == GroupIsolationMode::Global {
        return GroupIsolationProfile {
            filesystem: GroupIsolationLevel::Container,
            process: GroupIsolationLevel::Container,
            network: GroupIsolationLevel::Container,
        };
    }

    GroupIsolationProfile {
        filesystem: GroupIsolationLevel::Shared,
        process: GroupIsolationLevel::Shared,
        network: GroupIsolationLevel::Shared,
    }
}

/// Caps a container-level isolation to workspace when no container runtime
/// is available.
pub(super) fn runtime_level_cap(
    level: GroupIsolationLevel,
    runtime_class: RuntimeSandboxClass,
) -> GroupIsolationLevel {
    if level == GroupIsolationLevel::Container && runtime_class != RuntimeSandboxClass::Container {
        return GroupIsolationLevel::Workspace;
    }
    level
}

/// Resolves the group isolation profile, applying runtime sandbox caps.
pub(super) fn resolve_group_isolation(
    config: &Config,
    routing_group: &str,
    runtime_sandbox_class: RuntimeSandboxClass,
) -> GroupIsolationProfile {
    if let Some(rule) = config
        .channels_config
        .group_isolation_rules
        .iter()
        .find(|rule| rule.group == routing_group)
    {
        return GroupIsolationProfile {
            filesystem: runtime_level_cap(rule.filesystem, runtime_sandbox_class),
            process: runtime_level_cap(rule.process, runtime_sandbox_class),
            network: runtime_level_cap(rule.network, runtime_sandbox_class),
        };
    }

    let default_profile = default_group_isolation(config.channels_config.group_isolation_mode);
    GroupIsolationProfile {
        filesystem: runtime_level_cap(default_profile.filesystem, runtime_sandbox_class),
        process: runtime_level_cap(default_profile.process, runtime_sandbox_class),
        network: runtime_level_cap(default_profile.network, runtime_sandbox_class),
    }
}

/// Assembles the full processing state for a channel message (policy,
/// routing, isolation, ingress, and reply metadata).
pub(super) fn build_message_processing_state(
    rt: &ChannelRuntime,
    msg: &ChannelMessage,
    effective_autonomy: crate::security::policy::AutonomyLevel,
    tool_allowlist: Option<std::collections::HashSet<String>>,
) -> ChannelMessageProcessingState {
    let routing_group = resolve_routing_group(&rt.config, msg);
    let group_isolation =
        resolve_group_isolation(&rt.config, &routing_group, rt.runtime_sandbox_class);
    let source = format!("channel:{}", msg.channel);
    let ingress = apply_external_ingress_policy(
        &source,
        &msg.content,
        &rt.config.security.external_knowledge_trust,
    );
    let autosave_entity_id =
        tenant_scoped_channel_entity_id(&msg.channel, &msg.sender, &rt.tenant_policy_context);
    let reply_target = reply_target_for_message(&rt.config, msg);
    let thinking_key = channel_thinking_state_key(&rt.config, msg);

    ChannelMessageProcessingState {
        effective_autonomy,
        tool_allowlist,
        routing_group,
        group_isolation,
        ingress,
        source,
        autosave_entity_id,
        reply_target,
        thinking_key,
    }
}
