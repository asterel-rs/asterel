//! Channel-level autonomy and tool policy resolution.
use std::collections::HashSet;

use super::super::policy::min_autonomy;
use super::super::startup::ChannelRuntime;
use super::super::traits::ChannelMessage;
use crate::security::policy::AutonomyLevel;

/// Resolves the effective autonomy level and tool allowlist for a message.
pub(super) fn resolve_channel_policy(
    rt: &ChannelRuntime,
    msg: &ChannelMessage,
) -> (AutonomyLevel, Option<HashSet<String>>) {
    resolve_channel_policy_for_name(rt, &msg.channel)
}

/// Resolves the effective autonomy level and tool allowlist by channel name.
pub(super) fn resolve_channel_policy_for_name(
    rt: &ChannelRuntime,
    channel_name: &str,
) -> (AutonomyLevel, Option<HashSet<String>>) {
    let global_autonomy = rt.config.autonomy.effective_autonomy_lvl();
    if rt.config.channels_config.high_freedom_all_channels {
        tracing::debug!(
            channel = %channel_name,
            effective_autonomy = ?global_autonomy,
            "high_freedom_all_channels enabled; using global autonomy with unrestricted channel tools"
        );
        return (global_autonomy, None);
    }

    let channel_policy = rt.channel_policies.get(channel_name);
    let channel_level = channel_policy
        .and_then(|policy| policy.autonomy_level)
        .unwrap_or(global_autonomy);
    let effective_autonomy = min_autonomy(global_autonomy, channel_level);
    let tool_allowlist = channel_policy.and_then(|policy| policy.tool_allowlist.clone());

    tracing::debug!(
        channel = %channel_name,
        effective_autonomy = ?effective_autonomy,
        has_tool_allowlist = tool_allowlist.is_some(),
        "resolved channel runtime policy"
    );

    (effective_autonomy, tool_allowlist)
}
