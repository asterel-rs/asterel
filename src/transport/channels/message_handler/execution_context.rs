//! Channel execution-context builders.
use std::collections::HashSet;
use std::sync::Arc;

use super::super::ingress_policy::tenant_scoped_channel_entity_id;
use super::super::startup::ChannelRuntime;
use super::super::traits::ChannelMessage;
use super::GroupIsolationProfile;
use super::approval::{approval_context_for_event, approval_context_for_message};
use super::routing::normalize_group_component;
use crate::config::GroupIsolationLevel;
use crate::contracts::channels::ChannelCapabilities;
use crate::contracts::ids::EntityId;
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::middleware::tool_names;
use crate::security::broker_for_channel;
use crate::security::policy::AutonomyLevel;

fn supported_channel_tools(capabilities: Option<&ChannelCapabilities>) -> HashSet<String> {
    let mut tools = HashSet::new();
    let Some(capabilities) = capabilities else {
        return tools;
    };

    if capabilities.can_create_thread {
        tools.insert(tool_names::CHANNEL_CREATE_THREAD.to_string());
    }
    if capabilities.can_add_reaction {
        tools.insert(tool_names::CHANNEL_ADD_REACTION.to_string());
    }
    if capabilities.can_send_buttons || capabilities.can_send_embed {
        tools.insert(tool_names::CHANNEL_SEND_RICH.to_string());
    }
    if capabilities.can_fetch_history {
        tools.insert(tool_names::CHANNEL_GET_HISTORY.to_string());
    }
    if capabilities.can_send_embed {
        tools.insert(tool_names::CHANNEL_SEND_EMBED.to_string());
    }

    tools
}

fn channel_capability_scoped_allowlist(
    available_tool_names: &[&str],
    policy_allowlist: Option<HashSet<String>>,
    capabilities: Option<&ChannelCapabilities>,
) -> HashSet<String> {
    let supported_channel_tools = supported_channel_tools(capabilities);
    let mut allowed = available_tool_names
        .iter()
        .map(|name| (*name).to_string())
        .collect::<HashSet<_>>();

    for channel_tool in [
        tool_names::CHANNEL_CREATE_THREAD,
        tool_names::CHANNEL_ADD_REACTION,
        tool_names::CHANNEL_SEND_RICH,
        tool_names::CHANNEL_GET_HISTORY,
        tool_names::CHANNEL_SEND_EMBED,
    ] {
        if !supported_channel_tools.contains(channel_tool) {
            allowed.remove(channel_tool);
        }
    }

    if let Some(policy_allowlist) = policy_allowlist {
        allowed.retain(|tool_name| policy_allowlist.contains(tool_name));
    }

    allowed
}

async fn channel_workspace_dir(
    rt: &ChannelRuntime,
    routing_group: Option<&str>,
    filesystem_isolation: GroupIsolationLevel,
) -> std::path::PathBuf {
    let tenant_context = rt.tenant_policy_context.clone();
    let mut workspace_dir = if tenant_context.tenant_mode_enabled {
        let tenant_id = tenant_context.tenant_id.as_deref().unwrap_or("default");
        let scoped = rt.config.workspace_dir.join("tenants").join(tenant_id);
        if let Err(error) = tokio::fs::create_dir_all(&scoped).await {
            tracing::warn!(
                error = %error,
                tenant_id,
                "failed to create tenant scoped workspace"
            );
        }
        scoped
    } else {
        rt.config.workspace_dir.clone()
    };

    if let Some(routing_group) = routing_group
        && filesystem_isolation != GroupIsolationLevel::Shared
    {
        workspace_dir = workspace_dir
            .join("groups")
            .join(normalize_group_component(routing_group));
        if let Err(error) = tokio::fs::create_dir_all(&workspace_dir).await {
            tracing::warn!(
                error = %error,
                routing_group,
                "failed to create group-isolated workspace"
            );
        }
    }

    workspace_dir
}

pub(super) async fn build_execution_context(
    rt: &ChannelRuntime,
    msg: &ChannelMessage,
    turn_entity_id: EntityId,
    routing_group: String,
    group_isolation: GroupIsolationProfile,
    effective_autonomy: AutonomyLevel,
    tool_allowlist: Option<HashSet<String>>,
) -> ExecutionContext {
    let tenant_context = rt.tenant_policy_context.clone();
    let workspace_dir =
        channel_workspace_dir(rt, Some(&routing_group), group_isolation.filesystem).await;

    let mut ctx = ExecutionContext::runtime_root(
        Arc::clone(&rt.security),
        workspace_dir,
        Arc::clone(&rt.rate_limiter),
        Some(Arc::clone(&rt.permission_store)),
        tenant_context,
    );
    ctx.autonomy_level = effective_autonomy;
    ctx.entity_id = turn_entity_id;
    ctx.memory = Some(Arc::clone(&rt.mem));
    ctx.observer = Arc::clone(&rt.observer);
    ctx.session_id.clone_from(&msg.conversation_id);
    ctx.subagent_manager = Some(Arc::clone(&rt.subagent_manager));
    ctx.allowed_tools = Some(channel_capability_scoped_allowlist(
        &rt.registry.tool_names(),
        tool_allowlist,
        rt.channel_capabilities_by_name.get(&msg.channel),
    ));
    ctx.approval_broker = Some(broker_for_channel(
        &msg.channel,
        &approval_context_for_message(&rt.config, msg),
    ));
    ctx.channel_action_broker = rt.channel_action_brokers.get(&msg.channel).map(Arc::clone);
    ctx.source_channel = Some(msg.channel.clone());
    ctx.source_channel_id.clone_from(&msg.conversation_id);
    ctx.routing_group = Some(routing_group);
    ctx.process_isolation = group_isolation.process;
    ctx.network_isolation = group_isolation.network;
    ctx
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn build_event_execution_context(
    rt: &ChannelRuntime,
    channel_name: &str,
    sender: &str,
    turn_key: &str,
    conversation_id: Option<&str>,
    routing_group: String,
    group_isolation: GroupIsolationProfile,
    effective_autonomy: AutonomyLevel,
    tool_allowlist: Option<HashSet<String>>,
) -> ExecutionContext {
    let tenant_context = rt.tenant_policy_context.clone();
    let workspace_dir =
        channel_workspace_dir(rt, Some(&routing_group), group_isolation.filesystem).await;
    let scoped_entity_id = tenant_scoped_channel_entity_id(channel_name, turn_key, &tenant_context);

    let mut ctx = ExecutionContext::runtime_root(
        Arc::clone(&rt.security),
        workspace_dir,
        Arc::clone(&rt.rate_limiter),
        Some(Arc::clone(&rt.permission_store)),
        tenant_context,
    );
    ctx.memory = Some(Arc::clone(&rt.mem));
    ctx.observer = Arc::clone(&rt.observer);
    ctx.session_id = conversation_id.map(std::string::ToString::to_string);
    ctx.autonomy_level = effective_autonomy;
    ctx.entity_id = scoped_entity_id;
    ctx.subagent_manager = Some(Arc::clone(&rt.subagent_manager));
    ctx.allowed_tools = Some(channel_capability_scoped_allowlist(
        &rt.registry.tool_names(),
        tool_allowlist,
        rt.channel_capabilities_by_name.get(channel_name),
    ));
    ctx.approval_broker = Some(broker_for_channel(
        channel_name,
        &approval_context_for_event(&rt.config, channel_name, conversation_id, sender),
    ));
    ctx.channel_action_broker = rt.channel_action_brokers.get(channel_name).map(Arc::clone);
    ctx.source_channel = Some(channel_name.to_string());
    ctx.source_channel_id = conversation_id.map(ToString::to_string);
    ctx.routing_group = Some(routing_group);
    ctx.process_isolation = group_isolation.process;
    ctx.network_isolation = group_isolation.network;
    ctx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_capability_scoped_allowlist_removes_unsupported_channel_tools() {
        let allowed = channel_capability_scoped_allowlist(
            &[
                tool_names::FILE_READ,
                tool_names::CHANNEL_CREATE_THREAD,
                tool_names::CHANNEL_SEND_EMBED,
            ],
            None,
            Some(&ChannelCapabilities::default()),
        );

        assert!(allowed.contains(tool_names::FILE_READ));
        assert!(!allowed.contains(tool_names::CHANNEL_CREATE_THREAD));
        assert!(!allowed.contains(tool_names::CHANNEL_SEND_EMBED));
    }

    #[test]
    fn channel_capability_scoped_allowlist_intersects_policy_allowlist() {
        let allowed = channel_capability_scoped_allowlist(
            &[
                tool_names::FILE_READ,
                tool_names::CHANNEL_CREATE_THREAD,
                tool_names::CHANNEL_SEND_EMBED,
            ],
            Some(HashSet::from([
                tool_names::FILE_READ.to_string(),
                tool_names::CHANNEL_CREATE_THREAD.to_string(),
            ])),
            Some(&ChannelCapabilities {
                can_create_thread: true,
                ..ChannelCapabilities::default()
            }),
        );

        assert!(allowed.contains(tool_names::FILE_READ));
        assert!(allowed.contains(tool_names::CHANNEL_CREATE_THREAD));
        assert!(!allowed.contains(tool_names::CHANNEL_SEND_EMBED));
    }
}
