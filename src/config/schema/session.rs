//! Channel-session scoping and reset policy configuration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Scope mode used for direct-message sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DmScope {
    /// Share one DM session key globally across all users/channels.
    Global,
    /// Scope DM session keys by sender account.
    #[default]
    Account,
    /// Scope DM session keys by channel and sender.
    ChannelSender,
}

/// Reset policy selection for each channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResetPolicy {
    /// Scope session key by conversation/channel ID.
    #[default]
    Conversation,
    /// Scope session key by thread ID when available, else conversation.
    Thread,
    /// Keep session continuity until manually reset.
    Manual,
}

fn default_parent_fork_max_tokens() -> u32 {
    100_000
}

/// Root session behavior configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRoutingConfig {
    /// DM scoping policy.
    #[serde(default)]
    pub dm_scope: DmScope,
    /// Optional per-channel session reset policy.
    #[serde(default)]
    pub reset_by_channel: BTreeMap<String, ResetPolicy>,
    /// Maximum tokens inherited when forking from a parent session.
    #[serde(default = "default_parent_fork_max_tokens")]
    pub parent_fork_max_tokens: u32,
}

impl Default for SessionRoutingConfig {
    fn default() -> Self {
        Self {
            dm_scope: DmScope::default(),
            reset_by_channel: BTreeMap::new(),
            parent_fork_max_tokens: default_parent_fork_max_tokens(),
        }
    }
}

impl SessionRoutingConfig {
    /// Resolve reset policy for a channel name.
    #[must_use]
    pub fn reset_policy_for_channel(&self, channel: &str) -> ResetPolicy {
        self.reset_by_channel
            .get(channel)
            .copied()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{DmScope, ResetPolicy, SessionRoutingConfig};

    #[test]
    fn defaults_match_expected_values() {
        let cfg = SessionRoutingConfig::default();
        assert_eq!(cfg.dm_scope, DmScope::Account);
        assert!(cfg.reset_by_channel.is_empty());
        assert_eq!(cfg.parent_fork_max_tokens, 100_000);
    }

    #[test]
    fn reset_policy_lookup_requires_exact_channel_key() {
        let mut cfg = SessionRoutingConfig::default();
        cfg.reset_by_channel
            .insert("Discord".to_string(), ResetPolicy::Thread);

        assert_eq!(cfg.reset_policy_for_channel("Discord"), ResetPolicy::Thread);
        assert_eq!(
            cfg.reset_policy_for_channel("telegram"),
            ResetPolicy::Conversation
        );
    }
}
