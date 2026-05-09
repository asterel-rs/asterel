//! Process spawn policy enforcement.
//!
//! Classifies child process spawns (tool-equivalent, external
//! connector, operator plane) and blocks disallowed executions
//! based on the active security policy and autonomy level.

use anyhow::{Result, bail};

use crate::contracts::strings::verdicts::SECURITY_POLICY_BLOCK_PREFIX;
use crate::security::SecurityPolicy;

/// Classification of a child process spawn for policy enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessSpawnClass {
    /// Process that acts as a tool replacement (e.g., `git`, `cargo`).
    ToolEquivalent,
    /// Process connecting to an external service (e.g., `ngrok`).
    ExternalConnector,
    /// Process used for operator-plane management (e.g., `codex login`).
    OperatorPlane,
}

impl ProcessSpawnClass {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ToolEquivalent => "tool_equivalent",
            Self::ExternalConnector => "external_connector",
            Self::OperatorPlane => "operator_plane",
        }
    }
}

fn normalize_executable_name(command: &str) -> Option<&str> {
    let command = command.trim();
    if command.is_empty() {
        return None;
    }

    if command.contains('/') || command.contains('\\') {
        return None;
    }

    Some(command)
}

/// Enforce direct process spawn policy for routes outside tool middleware.
///
/// # Errors
/// Returns an error if command execution is blocked by autonomy level or command policy.
pub fn enforce_spawn_policy(
    security: &SecurityPolicy,
    command: &str,
    route_marker: &str,
    class: ProcessSpawnClass,
) -> Result<()> {
    enforce_process_spawn_policy_with_args(security, command, &[], route_marker, class)
}

/// Enforce direct process spawn policy, including argument-level checks.
///
/// # Errors
/// Returns an error if command or argument policy checks fail.
pub fn enforce_process_spawn_policy_with_args(
    security: &SecurityPolicy,
    command: &str,
    args: &[String],
    route_marker: &str,
    class: ProcessSpawnClass,
) -> Result<()> {
    if !security.can_act() {
        bail!(
            "{SECURITY_POLICY_BLOCK_PREFIX}read-only autonomy forbids process execution \
             (route='{route_marker}', class='{}')",
            class.as_str()
        );
    }

    let executable = normalize_executable_name(command)
        .ok_or_else(|| anyhow::anyhow!("invalid command executable (route='{route_marker}')"))?;

    if !security
        .allowed_commands
        .iter()
        .any(|allowed| allowed == executable)
    {
        bail!(
            "{SECURITY_POLICY_BLOCK_PREFIX}command '{executable}' is not allowlisted \
             (route='{route_marker}', class='{}')",
            class.as_str()
        );
    }

    for arg in args {
        if arg.contains('\0') {
            bail!(
                "{SECURITY_POLICY_BLOCK_PREFIX}command argument contains null byte \
                 (route='{route_marker}', class='{}')",
                class.as_str()
            );
        }
        // Reject arguments with embedded whitespace — the policy checker uses
        // naive whitespace splitting, so spaces inside a single argument would
        // cause it to be parsed as multiple tokens and potentially bypass
        // forbidden-argument checks.
        if arg.chars().any(char::is_whitespace) {
            bail!(
                "{SECURITY_POLICY_BLOCK_PREFIX}command argument contains embedded whitespace \
                 (route='{route_marker}', class='{}')",
                class.as_str()
            );
        }
    }

    let mut command_line = executable.to_string();
    for arg in args {
        command_line.push(' ');
        command_line.push_str(arg);
    }

    if !security.is_command_allowed(&command_line) {
        bail!(
            "{SECURITY_POLICY_BLOCK_PREFIX}command arguments denied for '{command_line}' \
             (route='{route_marker}', class='{}')",
            class.as_str()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ProcessSpawnClass, enforce_process_spawn_policy_with_args, enforce_spawn_policy};
    use crate::security::{AutonomyLevel, SecurityPolicy};

    #[test]
    fn allows_allowlisted_binary() {
        let policy = SecurityPolicy {
            allowed_commands: vec!["git".to_string()],
            ..SecurityPolicy::default()
        };
        enforce_spawn_policy(
            &policy,
            "git",
            "test_route",
            ProcessSpawnClass::OperatorPlane,
        )
        .expect("allowlisted command should pass");
    }

    #[test]
    fn rejects_path_prefixed_binary() {
        let policy = SecurityPolicy {
            allowed_commands: vec!["git".to_string()],
            ..SecurityPolicy::default()
        };
        let err = enforce_spawn_policy(
            &policy,
            "/usr/bin/git",
            "test_route",
            ProcessSpawnClass::ToolEquivalent,
        )
        .expect_err("path-prefixed command should be rejected");
        assert!(err.to_string().contains("invalid command executable"));
    }

    #[test]
    fn blocks_non_allowlisted_binary() {
        let policy = SecurityPolicy {
            allowed_commands: vec!["git".to_string()],
            ..SecurityPolicy::default()
        };
        let err = enforce_spawn_policy(
            &policy,
            "ngrok",
            "test_route",
            ProcessSpawnClass::ExternalConnector,
        )
        .expect_err("non-allowlisted command should fail");
        assert!(err.to_string().contains("not allowlisted"));
    }

    #[test]
    fn blocks_when_read_only() {
        let policy = SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        };
        let err = enforce_spawn_policy(
            &policy,
            "git",
            "test_route",
            ProcessSpawnClass::ToolEquivalent,
        )
        .expect_err("read-only autonomy should fail");
        assert!(err.to_string().contains("read-only autonomy"));
    }

    #[test]
    fn blocks_allowlisted_binary_when_arguments_are_dangerous() {
        let policy = SecurityPolicy {
            allowed_commands: vec!["git".to_string()],
            ..SecurityPolicy::default()
        };
        let args = vec![
            "-c".to_string(),
            "core.sshCommand=sh".to_string(),
            "status".to_string(),
        ];
        let err = enforce_process_spawn_policy_with_args(
            &policy,
            "git",
            &args,
            "test_route",
            ProcessSpawnClass::ToolEquivalent,
        )
        .expect_err("dangerous git args should be blocked");
        assert!(err.to_string().contains("arguments denied"));
    }

    #[test]
    fn allows_safe_allowlisted_arguments() {
        let policy = SecurityPolicy {
            allowed_commands: vec!["git".to_string()],
            ..SecurityPolicy::default()
        };
        let args = vec!["status".to_string()];
        enforce_process_spawn_policy_with_args(
            &policy,
            "git",
            &args,
            "test_route",
            ProcessSpawnClass::ToolEquivalent,
        )
        .expect("safe args should pass");
    }
}
