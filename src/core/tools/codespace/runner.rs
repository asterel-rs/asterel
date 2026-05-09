//! Sandboxed command runner for codespace projects.
//!
//! # What it does
//!
//! This module provides the low-level execution layer used by the codespace
//! tool's `exec`, `run_tests`, and `run` actions. Each command runs as a child
//! process with:
//!
//! * **Whitespace-aware tokenization** via `shlex` — quoted arguments survive
//!   intact without invoking a shell interpreter.
//! * **Shell-metacharacter validation** — `SHELL_METACHAR` is checked before
//!   `shlex` so that injection via cleverly quoted strings is prevented.
//! * **Env isolation** — only the `SAFE_ENV_VARS` list is inherited; all other
//!   host environment variables are stripped with `env_clear()`.
//! * **TMPDIR confinement** — `TMPDIR` is redirected to a project-local
//!   `.asterel-tmp` directory created with mode `0700` on Unix.
//! * **Timeout** — runs are killed after `timeout_secs` via
//!   `tokio::time::timeout`; timed-out processes are killed on drop.
//! * **Output caps** — stdout and stderr are each capped at 1 MB; excess is
//!   truncated with `"... [output truncated at 1MB]"` so the agent is not
//!   surprised by missing content.

use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail};
use chrono::Utc;

use super::types::TestResult;

/// Maximum output size in bytes (1 MB).
const MAX_OUTPUT_BYTES: usize = 1_048_576;

/// Environment variables safe to pass to child processes.
const SAFE_ENV_VARS: &[&str] = &[
    "PATH", "HOME", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "USER", "SHELL",
];

/// Shell metacharacters that must never appear in commands passed to `sh -c`.
pub(crate) const SHELL_METACHAR: &[char] = &[';', '|', '&', '`', '$', '\n', '\r', '\0'];

fn starts_with_env_assignment(word: &str) -> bool {
    let Some((name, _value)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && name
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
}

pub(crate) fn parse_command_words(command: &str) -> Result<Vec<String>> {
    if command.contains(SHELL_METACHAR) {
        bail!("Command contains disallowed shell metacharacters");
    }

    let words = shlex::split(command)
        .ok_or_else(|| anyhow::anyhow!("command contains invalid shell quoting in '{command}': check for unmatched quotes or backslash sequences"))?;
    let Some(first) = words.first() else {
        bail!("Command must not be empty");
    };
    if starts_with_env_assignment(first) {
        bail!("Command cannot start with environment variable assignments");
    }
    if first.contains('/') || first.contains('\\') {
        bail!("Command executable must be allowlist-style basename, not a path");
    }
    Ok(words)
}

/// Validate a command string for shell injection.
///
/// Returns `Err` if the command contains dangerous shell metacharacters.
pub(crate) fn validate_command(command: &str) -> Result<()> {
    let _ = parse_command_words(command)?;
    Ok(())
}

/// Run an arbitrary command inside a project directory with sandboxing.
///
/// # Errors
///
/// Returns an error when the command cannot be spawned or contains
/// disallowed shell metacharacters.
pub(crate) async fn run_command(
    project_dir: &Path,
    command: &str,
    timeout_secs: u64,
) -> Result<TestResult> {
    let words = parse_command_words(command)?;
    run_command_words(project_dir, &words, timeout_secs).await
}

async fn run_command_words(
    project_dir: &Path,
    words: &[String],
    timeout_secs: u64,
) -> Result<TestResult> {
    let start = std::time::Instant::now();

    let mut cmd = tokio::process::Command::new(&words[0]);
    cmd.args(&words[1..])
        .current_dir(project_dir)
        .kill_on_drop(true)
        .env_clear();

    for var in SAFE_ENV_VARS {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }

    // Isolate TMPDIR to project-local .asterel-tmp.
    let controlled_tmp = project_dir.join(".asterel-tmp");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&controlled_tmp)
            .or_else(|error| {
                if error.kind() == std::io::ErrorKind::AlreadyExists && controlled_tmp.is_dir() {
                    Ok(())
                } else {
                    Err(error)
                }
            })?;
        let metadata = std::fs::symlink_metadata(&controlled_tmp)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            bail!("controlled TMPDIR is not a real directory");
        }
        let mut permissions = metadata.permissions();
        if permissions.mode() & 0o777 != 0o700 {
            permissions.set_mode(0o700);
            std::fs::set_permissions(&controlled_tmp, permissions)?;
        }
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(&controlled_tmp)?;
    }
    cmd.env("TMPDIR", &controlled_tmp);

    let result = tokio::time::timeout(Duration::from_secs(timeout_secs), cmd.output()).await;

    let duration_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);

    match result {
        Ok(Ok(output)) => {
            let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();

            if stdout.len() > MAX_OUTPUT_BYTES {
                stdout.truncate(crate::utils::text::floor_char_boundary(
                    &stdout,
                    MAX_OUTPUT_BYTES,
                ));
                stdout.push_str("\n... [output truncated at 1MB]");
            }
            if stderr.len() > MAX_OUTPUT_BYTES {
                stderr.truncate(crate::utils::text::floor_char_boundary(
                    &stderr,
                    MAX_OUTPUT_BYTES,
                ));
                stderr.push_str("\n... [stderr truncated at 1MB]");
            }

            Ok(TestResult {
                success: output.status.success(),
                stdout,
                stderr,
                exit_code: output.status.code(),
                duration_ms,
                ran_at: Utc::now(),
            })
        }
        Ok(Err(e)) => bail!("Failed to execute command: {e}"),
        Err(_) => Ok(TestResult {
            success: false,
            stdout: String::new(),
            stderr: format!("Command timed out after {timeout_secs}s and was killed"),
            exit_code: None,
            duration_ms,
            ran_at: Utc::now(),
        }),
    }
}

/// Run the project's configured test command.
///
/// # Errors
///
/// Returns an error when no test command is configured or execution fails.
pub(crate) async fn run_tests(
    project_dir: &Path,
    test_command: Option<&str>,
    timeout_secs: u64,
) -> Result<TestResult> {
    let cmd = test_command
        .ok_or_else(|| anyhow::anyhow!("No test_command configured for this project"))?;
    run_command(project_dir, cmd, timeout_secs).await
}

/// Run the project's configured entry point.
///
/// # Errors
///
/// Returns an error when no entry point is configured or execution fails.
pub(crate) async fn run_entry_point(
    project_dir: &Path,
    entry_point: Option<&str>,
    args: Option<&str>,
    timeout_secs: u64,
) -> Result<TestResult> {
    let ep =
        entry_point.ok_or_else(|| anyhow::anyhow!("No entry_point configured for this project"))?;
    let shell_arg = match args {
        Some(a) => format!("{ep} {a}"),
        None => ep.to_string(),
    };
    run_command(project_dir, &shell_arg, timeout_secs).await
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn run_command_echo() {
        let tmp = TempDir::new().unwrap();
        let result = run_command(tmp.path(), "echo hello", 10).await.unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn run_command_failure() {
        let tmp = TempDir::new().unwrap();
        let result = run_command(tmp.path(), "false", 10).await.unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn run_command_timeout() {
        let tmp = TempDir::new().unwrap();
        let result = run_command(tmp.path(), "sleep 30", 1).await.unwrap();
        assert!(!result.success);
        assert!(result.stderr.contains("timed out"));
    }

    #[tokio::test]
    async fn run_tests_no_command() {
        let tmp = TempDir::new().unwrap();
        let err = run_tests(tmp.path(), None, 10).await.unwrap_err();
        assert!(err.to_string().contains("No test_command"));
    }

    #[tokio::test]
    async fn run_entry_point_no_entry() {
        let tmp = TempDir::new().unwrap();
        let err = run_entry_point(tmp.path(), None, None, 10)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("No entry_point"));
    }

    #[tokio::test]
    async fn run_command_env_isolation() {
        let tmp = TempDir::new().unwrap();
        let result = run_command(tmp.path(), "printenv TMPDIR", 10)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.stdout.contains(".asterel-tmp"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_command_tightens_existing_tmpdir_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let controlled_tmp = tmp.path().join(".asterel-tmp");
        std::fs::create_dir(&controlled_tmp).unwrap();
        std::fs::set_permissions(&controlled_tmp, std::fs::Permissions::from_mode(0o777)).unwrap();

        let result = run_command(tmp.path(), "printenv TMPDIR", 10)
            .await
            .unwrap();

        assert!(result.success);
        assert_eq!(
            std::fs::metadata(&controlled_tmp)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn validate_command_rejects_env_assignment_prefix() {
        let err = validate_command("FOO=bar ls").unwrap_err();
        assert!(err.to_string().contains("environment variable assignments"));
    }

    #[test]
    fn validate_command_rejects_path_qualified_executable() {
        let err = validate_command("/tmp/git status").unwrap_err();
        assert!(err.to_string().contains("not a path"));
    }
}
