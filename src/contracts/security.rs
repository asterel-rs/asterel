//! Security policy contracts shared between `security` and `config`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Controls whether the agent is allowed to execute actions that have
/// side-effects in the external world (e.g., sending messages, writing files,
/// calling external APIs).
///
/// This gate is independent of `AutonomyLevel`: even a `Full`-autonomy agent
/// will not perform external writes when set to `Disabled`. Use `Enabled`
/// only in deployments that have been explicitly reviewed for risk.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExternalActionExecution {
    /// External side-effecting actions are blocked. The agent may still read
    /// from external sources but cannot write, post, or mutate external state.
    #[default]
    Disabled,
    /// External side-effecting actions are permitted, subject to the
    /// `AutonomyLevel` approval rules.
    Enabled,
}

/// Determines how much latitude the agent has to act without human approval.
///
/// This value is checked by the tool loop before every tool call. The three
/// levels form a strict escalation: each level is a superset of the one above
/// it in terms of what is auto-approved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutonomyLevel {
    /// The agent is observation-only. `can_act()` returns `false` for every
    /// tool, so no tool execution occurs at all. Useful for dry-run analysis
    /// or audit modes where the agent must produce a report without touching
    /// any state.
    ReadOnly,
    /// The agent may auto-approve read-only tools (file reads, searches,
    /// lookups) but must request human approval before executing any mutating
    /// or side-effecting tool (writes, deletes, external API calls). This is
    /// the default and the recommended level for production deployments.
    #[default]
    Supervised,
    /// All tools are auto-approved without human intervention. Suitable for
    /// fully trusted automation pipelines where an operator has pre-reviewed
    /// the agent's capabilities and accepts responsibility for its actions.
    Full,
}

/// Source-of-truth store for encrypted secrets.
///
/// `SecretStore` is the single place the agent retrieves sensitive values
/// such as API keys, tokens, and credentials. Callers must not hard-code
/// secrets elsewhere; they should request them through this store so that
/// rotation and access-control policies are enforced uniformly.
#[derive(Debug, Clone)]
pub struct SecretStore {
    /// Filesystem path to the encrypted secrets file or directory.
    ///
    /// The store implementation reads from this path at runtime. The path
    /// must be resolvable by the process and protected by OS-level
    /// permissions (e.g., mode `0600`). Relative paths are resolved against
    /// the process working directory.
    pub(crate) key_path: PathBuf,
    /// Whether the secret store is active.
    ///
    /// When `false`, calls to the store return errors or empty results rather
    /// than attempting to read from `key_path`. This allows the agent to
    /// start in environments where the secret store is intentionally absent
    /// (e.g., local dev) without hard-failing on startup.
    pub(crate) enabled: bool,
}

#[must_use]
pub fn default_allowed_commands() -> Vec<String> {
    vec![
        "git".into(),
        "npm".into(),
        "cargo".into(),
        "ls".into(),
        "cat".into(),
        "grep".into(),
        "find".into(),
        "echo".into(),
        "pwd".into(),
        "wc".into(),
        "head".into(),
        "tail".into(),
    ]
}

#[must_use]
pub fn default_forbidden_paths() -> Vec<String> {
    vec![
        "/etc".into(),
        "/root".into(),
        "/home".into(),
        "/usr".into(),
        "/bin".into(),
        "/sbin".into(),
        "/lib".into(),
        "/opt".into(),
        "/boot".into(),
        "/dev".into(),
        "/proc".into(),
        "/sys".into(),
        "/var".into(),
        "/tmp".into(),
        "~/.ssh".into(),
        "~/.gnupg".into(),
        "~/.aws".into(),
        "~/.config".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_action_execution_roundtrip_and_default() {
        let cases = [
            (ExternalActionExecution::Disabled, "disabled"),
            (ExternalActionExecution::Enabled, "enabled"),
        ];

        for (variant, expected) in cases {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, format!("\"{expected}\""));
            let parsed: ExternalActionExecution = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }

        assert_eq!(
            ExternalActionExecution::default(),
            ExternalActionExecution::Disabled
        );
    }

    #[test]
    fn autonomy_level_roundtrip_and_default() {
        let cases = [
            (AutonomyLevel::ReadOnly, "readonly"),
            (AutonomyLevel::Supervised, "supervised"),
            (AutonomyLevel::Full, "full"),
        ];

        for (variant, expected) in cases {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, format!("\"{expected}\""));
            let parsed: AutonomyLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, variant);
        }

        assert_eq!(AutonomyLevel::default(), AutonomyLevel::Supervised);
    }
}
