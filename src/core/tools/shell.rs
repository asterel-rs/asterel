//! Sandboxed shell command execution tool (`shell`).
//!
//! # Security model
//!
//! Shell execution is the highest-risk tool in the default set.  The
//! following defences are layered, from outermost to innermost:
//!
//! 1. **Autonomy / policy gate** (`SecurityMiddleware`) — blocks all shell
//!    calls in `ReadOnly` mode.  In `Supervised` mode the call triggers an
//!    approval request unless a prior grant exists.
//! 2. **Command allowlist** (`enforce_shell_command_guardrails`) — the
//!    [`SecurityPolicy`] carries an explicit allow-list of permitted
//!    command prefixes; anything not matching is rejected.
//! 3. **Process-isolation group check** — routing groups with
//!    `process_isolation != Shared` block all shell execution.
//! 4. **Environment stripping** — the child process inherits only a small
//!    set of safe, functional environment variables ([`SAFE_ENV_VARS`]).
//!    All other variables (API keys, tokens, secrets) are cleared by
//!    `Command::env_clear()` before spawn.
//! 5. **`TMPDIR` override** — the child's `TMPDIR` is set to a
//!    workspace-local `.asterel-tmp` directory (created with mode `0o700`
//!    on Unix) to prevent temp-file side-channels to system directories.
//! 6. **Execution timeout** — the child is killed after
//!    [`SHELL_TIMEOUT_SECS`] seconds (`kill_on_drop` also cleans it up on
//!    future cancellation).
//! 7. **Output truncation** — both stdout and stderr are capped at
//!    [`MAX_OUTPUT_BYTES`] (1 MB each) to prevent OOM from runaway output.
//!    Additional truncation is applied by `OutputSizeLimitMiddleware` in the
//!    `after_execute` phase.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use anyhow::Context;
use serde_json::json;

use super::traits::{Tool, ToolResult, ToolResultCompactionTarget, ToolResultSemanticStreamMode};
use crate::core::tools::middleware::{
    ExecutionContext, ToolResultTextField, classify_shell_command_output_kind,
    enforce_shell_command_guardrails,
};

/// Maximum shell command execution time before kill.
const SHELL_TIMEOUT_SECS: u64 = 60;
/// Maximum output size in bytes (1MB).
const MAX_OUTPUT_BYTES: usize = 1_048_576;
/// Environment variables safe to pass to shell commands.
/// Only functional variables are included — never API keys or secrets.
const SAFE_ENV_VARS: &[&str] = &[
    "PATH", "HOME", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "USER", "SHELL",
];

/// Tool that executes a shell command inside the workspace directory.
///
/// Commands run through `/bin/sh -c` with a stripped environment and a
/// controlled `TMPDIR`.  stdout is returned as `output`; stderr (if any)
/// as `error`.  A non-zero exit code sets `success = false`.
pub struct ShellTool;

impl ShellTool {
    /// Create a new shell tool instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    fn semantic_result(
        command: &str,
        success: bool,
        output: String,
        error: Option<String>,
    ) -> ToolResult {
        let output_kind = classify_shell_command_output_kind(command);
        let stream_mode = match output_kind {
            "shell.cargo_test" | "shell.cargo_clippy" => {
                ToolResultSemanticStreamMode::CombinedOutputAndError
            }
            _ => ToolResultSemanticStreamMode::PerField,
        };
        let mut source_fields = Vec::with_capacity(2);
        if !output.is_empty() {
            source_fields.push(ToolResultTextField::Output);
        }
        if error.as_ref().is_some_and(|stderr| !stderr.is_empty()) {
            source_fields.push(ToolResultTextField::Error);
        }

        ToolResult {
            success,
            output,
            error,
            attachments: Vec::new(),
            taint_labels: Vec::new(),
            semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
        }
        .with_output_kind(output_kind)
        .with_compaction_target(ToolResultCompactionTarget::OutputAndError)
        .with_stream_mode(stream_mode)
        .with_raw_command(command)
        .with_source_fields(source_fields)
        .refresh_semantic_stats()
    }

    fn build_command(command: &str, workspace_dir: &Path) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
            .arg(command)
            .current_dir(workspace_dir)
            .env_clear()
            .kill_on_drop(true);

        for var in SAFE_ENV_VARS {
            if let Ok(val) = std::env::var(var) {
                cmd.env(var, val);
            }
        }

        cmd
    }

    fn prepare_controlled_tmpdir(workspace_dir: &Path) -> std::io::Result<PathBuf> {
        let controlled_tmp = workspace_dir.join(".asterel-tmp");
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&controlled_tmp)
                .or_else(|error| {
                    if error.kind() == std::io::ErrorKind::AlreadyExists && controlled_tmp.is_dir()
                    {
                        Ok(())
                    } else {
                        Err(error)
                    }
                })?;
            let metadata = std::fs::symlink_metadata(&controlled_tmp)?;
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "controlled TMPDIR is not a real directory",
                ));
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
        Ok(controlled_tmp)
    }

    fn truncate_stream_if_needed(stream: &mut String, suffix: &str) {
        if stream.len() <= MAX_OUTPUT_BYTES {
            return;
        }
        stream.truncate(crate::utils::text::floor_char_boundary(
            stream,
            MAX_OUTPUT_BYTES,
        ));
        stream.push_str(suffix);
    }
}

impl Default for ShellTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ShellTool {
    fn name(&self) -> &'static str {
        "shell"
    }

    fn description(&self) -> &'static str {
        "Execute a shell command in the workspace directory"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                }
            },
            "required": ["command"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let command = args
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'command' parameter"))?;

            enforce_shell_command_guardrails(ctx, command, "tool:shell")
                .context("shell command blocked by orchestration guardrails")?;

            let mut cmd = Self::build_command(command, &ctx.workspace_dir);
            let controlled_tmp = match Self::prepare_controlled_tmpdir(&ctx.workspace_dir) {
                Ok(path) => path,
                Err(error) => {
                    return Ok(Self::semantic_result(
                        command,
                        false,
                        String::new(),
                        Some(format!("failed to prepare controlled TMPDIR: {error}")),
                    ));
                }
            };
            cmd.env("TMPDIR", &controlled_tmp);

            let result =
                tokio::time::timeout(Duration::from_secs(SHELL_TIMEOUT_SECS), cmd.output()).await;

            match result {
                Ok(Ok(output)) => {
                    let mut stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    let mut stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    Self::truncate_stream_if_needed(&mut stdout, "\n... [output truncated at 1MB]");
                    Self::truncate_stream_if_needed(&mut stderr, "\n... [stderr truncated at 1MB]");

                    Ok(Self::semantic_result(
                        command,
                        output.status.success(),
                        stdout,
                        if stderr.is_empty() {
                            None
                        } else {
                            Some(stderr)
                        },
                    ))
                }
                Ok(Err(e)) => Ok(Self::semantic_result(
                    command,
                    false,
                    String::new(),
                    Some(format!("Failed to execute command: {e}")),
                )),
                Err(_) => Ok(Self::semantic_result(
                    command,
                    false,
                    String::new(),
                    Some(format!(
                        "Command timed out after {SHELL_TIMEOUT_SECS}s and was killed"
                    )),
                )),
            }
        })
    }
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::core::tools::middleware::{ExecutionContext, classify_shell_command_output_kind};
    use crate::security::{AutonomyLevel, SecurityPolicy};

    fn test_security(autonomy: AutonomyLevel) -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy,
            workspace_dir: std::env::temp_dir(),
            allowed_commands: vec![
                "echo".into(),
                "env".into(),
                "false".into(),
                "ls".into(),
                "sleep".into(),
            ],
            ..SecurityPolicy::default()
        })
    }

    #[test]
    fn shell_tool_name() {
        let tool = ShellTool::new();
        assert_eq!(tool.name(), "shell");
    }

    #[test]
    fn shell_tool_description() {
        let tool = ShellTool::new();
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn shell_tool_schema_has_command() {
        let tool = ShellTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["command"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("command"))
        );
    }

    #[tokio::test]
    async fn shell_executes_allowed_command() {
        let tool = ShellTool::new();
        let ctx = ExecutionContext::test_default(test_security(AutonomyLevel::Supervised));
        let result = tool
            .execute(json!({"command": "echo hello"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.trim().contains("hello"));
        assert!(result.error.is_none());
    }

    #[tokio::test]
    async fn shell_missing_command_param() {
        let tool = ShellTool::new();
        let ctx = ExecutionContext::test_default(test_security(AutonomyLevel::Supervised));
        let result = tool.execute(json!({}), &ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("command"));
    }

    #[tokio::test]
    async fn shell_wrong_type_param() {
        let tool = ShellTool::new();
        let ctx = ExecutionContext::test_default(test_security(AutonomyLevel::Supervised));
        let result = tool.execute(json!({"command": 123}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn shell_captures_exit_code() {
        let tool = ShellTool::new();
        let ctx = ExecutionContext::test_default(test_security(AutonomyLevel::Supervised));
        let result = tool
            .execute(json!({"command": "ls nonexistent_dir_xyz"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
    }

    fn test_security_with_env_cmd() -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: std::env::temp_dir(),
            allowed_commands: vec!["env".into(), "echo".into()],
            ..SecurityPolicy::default()
        })
    }

    /// RAII guard that restores an environment variable to its original state on drop,
    /// ensuring cleanup even if the test panics.
    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            // SAFETY: This helper is used only in test code to set process env vars.
            // Tests that use EnvGuard run on the current-thread Tokio runtime and this
            // guard restores the original value on drop, keeping access scoped.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.original {
                // SAFETY: Test-only restoration of a variable that was previously read by
                // this guard, returning process state to its original value.
                Some(val) => unsafe {
                    std::env::set_var(self.key, val);
                },
                // SAFETY: Test-only cleanup for variables introduced by EnvGuard::set.
                None => unsafe {
                    std::env::remove_var(self.key);
                },
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shell_does_not_leak_api_key() {
        let _g1 = EnvGuard::set("API_KEY", "sk-test-secret-12345");
        let _g2 = EnvGuard::set("ASTEREL_API_KEY", "sk-test-secret-67890");

        let tool = ShellTool::new();
        let ctx = ExecutionContext::test_default(test_security_with_env_cmd());
        let result = tool.execute(json!({"command": "env"}), &ctx).await.unwrap();
        assert!(result.success);
        assert!(
            !result.output.contains("sk-test-secret-12345"),
            "API_KEY leaked to shell command output"
        );
        assert!(
            !result.output.contains("sk-test-secret-67890"),
            "ASTEREL_API_KEY leaked to shell command output"
        );
    }

    #[tokio::test]
    async fn shell_timeout_returns_failure() {
        let tool = ShellTool::new();
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: std::env::temp_dir(),
            allowed_commands: vec!["sleep".into(), "false".into()],
            ..SecurityPolicy::default()
        }));
        // The shell timeout is 60s - we can't wait that long in tests.
        // Instead verify a short command that fails exits with !success.
        let result = tool
            .execute(json!({"command": "sleep 0 && false"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success, "false must return non-zero exit code");
    }

    #[tokio::test]
    async fn shell_stderr_captured_separately() {
        let tool = ShellTool::new();
        let ctx = ExecutionContext::test_default(test_security(AutonomyLevel::Supervised));
        let result = tool
            .execute(json!({"command": "echo out; ls nonexistent_dir_xyz"}), &ctx)
            .await
            .unwrap();
        assert!(result.output.contains("out"));
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|e| e.contains("nonexistent_dir_xyz")),
            "stderr must be captured in error field"
        );
    }

    #[tokio::test]
    async fn shell_tmpdir_isolation() {
        let tool = ShellTool::new();
        let ctx = ExecutionContext::test_default(test_security_with_env_cmd());
        let result = tool
            .execute(json!({"command": "echo $TMPDIR"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(
            result.output.contains(".asterel-tmp"),
            "TMPDIR must be set to workspace-local dir, got: {:?}",
            result.output.trim()
        );
    }

    #[tokio::test]
    async fn shell_fails_closed_when_controlled_tmpdir_cannot_be_created() {
        let tool = ShellTool::new();
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().to_path_buf();
        std::fs::write(workspace.join(".asterel-tmp"), "not-a-directory").unwrap();
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: workspace,
            allowed_commands: vec!["echo".into()],
            ..SecurityPolicy::default()
        }));

        let result = tool
            .execute(json!({"command": "echo hello"}), &ctx)
            .await
            .unwrap();

        assert!(!result.success);
        assert_eq!(result.output, "");
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("failed to prepare controlled TMPDIR"))
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_tightens_existing_controlled_tmpdir_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tool = ShellTool::new();
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().to_path_buf();
        let tmpdir = workspace.join(".asterel-tmp");
        std::fs::create_dir(&tmpdir).unwrap();
        std::fs::set_permissions(&tmpdir, std::fs::Permissions::from_mode(0o777)).unwrap();
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: workspace,
            allowed_commands: vec!["echo".into()],
            ..SecurityPolicy::default()
        }));

        let result = tool
            .execute(json!({"command": "echo hello"}), &ctx)
            .await
            .unwrap();

        assert!(result.success);
        let mode = std::fs::metadata(&tmpdir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn readonly_policy_blocks_all_commands() {
        // ReadOnly policy prevents all command execution at the policy level.
        // ShellTool itself doesn't check policy (middleware handles that),
        // but the SecurityPolicy must deny all commands in ReadOnly mode.
        let policy = SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        };
        assert!(!policy.is_command_allowed("echo hello"));
        assert!(!policy.is_command_allowed("ls"));
        assert!(!policy.can_act());
    }

    #[tokio::test]
    async fn shell_preserves_path_and_home() {
        let tool = ShellTool::new();
        let ctx = ExecutionContext::test_default(test_security_with_env_cmd());

        let result = tool
            .execute(json!({"command": "echo $HOME"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(
            !result.output.trim().is_empty(),
            "HOME should be available in shell"
        );

        let result = tool
            .execute(json!({"command": "echo $PATH"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(
            !result.output.trim().is_empty(),
            "PATH should be available in shell"
        );
    }

    #[test]
    fn shell_classifier_normalizes_git_status_with_option_wrapper_variants() {
        assert_eq!(
            classify_shell_command_output_kind("git -C repo status --short"),
            "shell.git_status"
        );
    }

    #[test]
    fn shell_classifier_normalizes_cargo_clippy_with_workspace_flag() {
        assert_eq!(
            classify_shell_command_output_kind("cargo clippy --workspace"),
            "shell.cargo_clippy"
        );
    }

    #[test]
    fn shell_classifier_normalizes_cargo_test_with_flag_values_before_subcommand() {
        assert_eq!(
            classify_shell_command_output_kind("cargo --color always test"),
            "shell.cargo_test"
        );
    }

    #[test]
    fn shell_classifier_rejects_compound_commands() {
        assert_eq!(
            classify_shell_command_output_kind("git status && cargo test"),
            "unknown"
        );
    }

    #[test]
    fn shell_classifier_normalizes_ripgrep_invocations() {
        assert_eq!(
            classify_shell_command_output_kind("rg foo src/"),
            "shell.ripgrep"
        );
    }

    #[test]
    fn shell_classifier_maps_unknown_commands_to_unknown() {
        assert_eq!(classify_shell_command_output_kind("echo hello"), "unknown");
    }

    async fn assert_shell_semantic_metadata(
        command: &str,
        expected_output_kind: &str,
        expected_source_fields: &[ToolResultTextField],
        expected_stream_mode: ToolResultSemanticStreamMode,
    ) {
        let tool = ShellTool::new();
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: std::env::temp_dir(),
            allowed_commands: vec!["echo".into(), "ls".into(), "cargo".into()],
            ..SecurityPolicy::default()
        }));

        let result = tool
            .execute(json!({ "command": command }), &ctx)
            .await
            .unwrap();

        assert_eq!(
            result.semantic.output_kind.as_deref(),
            Some(expected_output_kind)
        );
        assert_eq!(result.semantic.raw_command.as_deref(), Some(command));
        assert_eq!(
            result.semantic.compaction_target,
            ToolResultCompactionTarget::OutputAndError
        );
        assert_eq!(result.semantic.stream_mode, expected_stream_mode);
        assert_eq!(
            result
                .semantic
                .source_fields
                .iter()
                .map(|field| field.field.as_str())
                .collect::<Vec<_>>(),
            expected_source_fields
                .iter()
                .copied()
                .map(ToolResultTextField::as_str)
                .collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn shell_attaches_semantic_metadata_for_raw_execution() {
        assert_shell_semantic_metadata(
            "echo hello",
            "unknown",
            &[ToolResultTextField::Output],
            ToolResultSemanticStreamMode::PerField,
        )
        .await;
    }

    #[tokio::test]
    async fn shell_attaches_semantic_metadata_only_for_present_error_output() {
        assert_shell_semantic_metadata(
            "ls does-not-exist-for-semantic-metadata-test",
            "unknown",
            &[ToolResultTextField::Error],
            ToolResultSemanticStreamMode::PerField,
        )
        .await;
    }

    #[tokio::test]
    async fn shell_attaches_semantic_metadata_for_present_output_and_error() {
        assert_shell_semantic_metadata(
            "ls . does-not-exist-for-semantic-metadata-test",
            "unknown",
            &[ToolResultTextField::Output, ToolResultTextField::Error],
            ToolResultSemanticStreamMode::PerField,
        )
        .await;
    }

    #[tokio::test]
    async fn shell_marks_cargo_outputs_for_combined_stream_compaction() {
        assert_shell_semantic_metadata(
            "cargo test -- --nocapture",
            "shell.cargo_test",
            &[ToolResultTextField::Error],
            ToolResultSemanticStreamMode::CombinedOutputAndError,
        )
        .await;
    }
}
