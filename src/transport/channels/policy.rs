//! Per-channel security policy: autonomy-level clamping, tool allowlist
//! matching, and user authorization checks (exact and case-insensitive).
use std::collections::HashSet;
use std::sync::Arc;

use crate::security::AutonomyLevel;
use crate::transport::channels::traits::Channel;

/// Per-channel security policy overrides (autonomy level and tool allowlist).
#[derive(Debug, Clone)]
pub struct ChannelPolicy {
    pub autonomy_level: Option<AutonomyLevel>,
    pub tool_allowlist: Option<HashSet<String>>,
}

/// A channel adapter paired with its display name and security policy.
#[derive(Clone)]
pub struct ChannelEntry {
    pub name: &'static str,
    pub channel: Arc<dyn Channel>,
    pub policy: ChannelPolicy,
}

/// Strategy for matching user IDs against an allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllowlistMatch {
    Exact,
    AsciiCaseInsensitive,
}

/// Effective autonomy = min(global, channel). Channel cannot escalate beyond global.
#[must_use]
pub fn min_autonomy(global: AutonomyLevel, channel: AutonomyLevel) -> AutonomyLevel {
    match (global, channel) {
        (AutonomyLevel::ReadOnly, _) | (_, AutonomyLevel::ReadOnly) => AutonomyLevel::ReadOnly,
        (AutonomyLevel::Supervised, _) | (_, AutonomyLevel::Supervised) => {
            AutonomyLevel::Supervised
        }
        (AutonomyLevel::Full, AutonomyLevel::Full) => AutonomyLevel::Full,
    }
}

/// Checks whether a user ID appears in the allowlist. Supports `"*"`
/// wildcard, exact match, and ASCII case-insensitive modes.
#[must_use]
pub fn is_allowed_user(allowed_users: &[String], user_id: &str, mode: AllowlistMatch) -> bool {
    if allowed_users.iter().any(|user| user == "*") {
        return true;
    }

    match mode {
        AllowlistMatch::Exact => allowed_users.iter().any(|user| user == user_id),
        AllowlistMatch::AsciiCaseInsensitive => allowed_users
            .iter()
            .any(|user| user.eq_ignore_ascii_case(user_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_autonomy_read_only_wins_when_global_read_only() {
        assert_eq!(
            min_autonomy(AutonomyLevel::ReadOnly, AutonomyLevel::Full),
            AutonomyLevel::ReadOnly
        );
    }

    #[test]
    fn min_autonomy_supervised_wins_over_full() {
        assert_eq!(
            min_autonomy(AutonomyLevel::Supervised, AutonomyLevel::Full),
            AutonomyLevel::Supervised
        );
    }

    #[test]
    fn min_autonomy_full_only_when_both_full() {
        assert_eq!(
            min_autonomy(AutonomyLevel::Full, AutonomyLevel::Full),
            AutonomyLevel::Full
        );
    }

    #[test]
    fn min_autonomy_read_only_wins_when_channel_read_only() {
        assert_eq!(
            min_autonomy(AutonomyLevel::Full, AutonomyLevel::ReadOnly),
            AutonomyLevel::ReadOnly
        );
    }

    #[test]
    fn is_allowed_user_supports_wildcard() {
        let allowed = vec!["alice".to_string(), "*".to_string()];
        assert!(is_allowed_user(&allowed, "anyone", AllowlistMatch::Exact));
    }

    #[test]
    fn is_allowed_user_exact_match_is_case_sensitive() {
        let allowed = vec!["Alice".to_string()];
        assert!(!is_allowed_user(&allowed, "alice", AllowlistMatch::Exact));
        assert!(is_allowed_user(&allowed, "Alice", AllowlistMatch::Exact));
    }

    #[test]
    fn is_allowed_user_ascii_case_insensitive_mode() {
        let allowed = vec!["Alice".to_string()];
        assert!(is_allowed_user(
            &allowed,
            "alice",
            AllowlistMatch::AsciiCaseInsensitive
        ));
    }
}
