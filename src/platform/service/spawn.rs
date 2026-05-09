//! Utility functions for service subprocess execution.
//!
//! Wraps `std::process::Command` with security policy checks
//! and provides XML escaping for plist generation.

use std::process::Command;

use anyhow::{Context, Result};

use crate::security::{ProcessSpawnClass, SecurityPolicy, enforce_process_spawn_policy_with_args};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObservedCommandOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl ObservedCommandOutput {
    #[must_use]
    pub fn primary_text(&self) -> &str {
        if self.stdout.trim().is_empty() {
            self.stderr.as_str()
        } else {
            self.stdout.as_str()
        }
    }

    #[must_use]
    pub fn combined(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }
}

pub(super) fn run_observed(
    security: &SecurityPolicy,
    route_marker: &str,
    class: ProcessSpawnClass,
    command: &mut Command,
) -> Result<ObservedCommandOutput> {
    let program = command.get_program().to_string_lossy().to_string();
    let args = command
        .get_args()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    enforce_process_spawn_policy_with_args(security, &program, &args, route_marker, class)?;

    let output = command.output().context("Failed to spawn command")?;
    Ok(ObservedCommandOutput {
        success: output.status.success(),
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

/// Runs a command after enforcing the security spawn policy,
/// returning an error if the command fails.
///
/// # Errors
///
/// Returns an error if the policy blocks the spawn or the
/// command exits with a non-zero status.
pub(super) fn run_checked(
    security: &SecurityPolicy,
    route_marker: &str,
    class: ProcessSpawnClass,
    command: &mut Command,
) -> Result<()> {
    let output = run_observed(security, route_marker, class, command)?;
    if !output.success {
        anyhow::bail!("Command failed: {}", output.primary_text().trim());
    }
    Ok(())
}

/// Runs a command and captures its stdout (or stderr if stdout
/// is empty).
///
/// # Errors
///
/// Returns an error if the policy blocks the spawn or the
/// command cannot be executed.
pub(super) fn run_capture(
    security: &SecurityPolicy,
    route_marker: &str,
    class: ProcessSpawnClass,
    command: &mut Command,
) -> Result<String> {
    Ok(run_observed(security, route_marker, class, command)?
        .primary_text()
        .to_string())
}

/// Builds a command from a program name and configuration callback,
/// then runs it through [`run_checked`].
pub(super) fn run_program_checked<F>(
    security: &SecurityPolicy,
    route_marker: &str,
    class: ProcessSpawnClass,
    program: &str,
    configure: F,
) -> Result<()>
where
    F: FnOnce(&mut Command),
{
    let mut command = Command::new(program);
    configure(&mut command);
    run_checked(security, route_marker, class, &mut command)
}

/// Builds a command from a program name and configuration callback,
/// then runs it through [`run_capture`].
pub(super) fn run_program_capture<F>(
    security: &SecurityPolicy,
    route_marker: &str,
    class: ProcessSpawnClass,
    program: &str,
    configure: F,
) -> Result<String>
where
    F: FnOnce(&mut Command),
{
    let mut command = Command::new(program);
    configure(&mut command);
    run_capture(security, route_marker, class, &mut command)
}

/// Escapes XML special characters for safe embedding in plist
/// values.
pub(super) fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
