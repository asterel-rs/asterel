//! `CodespaceTool` — `Tool` impl and top-level action dispatch.
//!
//! This module contains the `CodespaceTool` struct, its helper methods, and
//! the `Tool` trait implementation. Execution is split across two submodules
//! to keep each file focused:
//!
//! * `exec_ops` — `run_tests`, exec, run, promote, `git_init`, git.
//! * `project_ops` — create, list, `write_file`, `read_file`, delete, status.

mod exec_ops;
mod project_ops;

use std::path::Path;

use serde_json::json;

use super::project;
use super::types::CodespaceAction;
use super::types::TestResult;
use crate::config::schema::CodespaceConfig;
use crate::core::tools::middleware::{
    ExecutionContext, classify_shell_command_output_kind, enforce_process_spawn_guardrails,
};
use crate::core::tools::traits::{
    Tool, ToolResult, ToolResultCompactionTarget, ToolResultSemanticStreamMode, ToolResultTextField,
};
use crate::security::ProcessSpawnClass;

/// Tool that exposes a sandboxed multi-project development environment.
///
/// Each action is dispatched from a single `CodespaceAction` enum value
/// deserialized from the tool arguments. Security checks (path confinement,
/// shell-injection prevention, command policy) are applied per-action in the
/// `exec_ops` and `project_ops` handlers.
pub struct CodespaceTool {
    config: CodespaceConfig,
}

impl CodespaceTool {
    /// Create a new codespace tool with the given configuration.
    #[must_use]
    pub fn new(config: CodespaceConfig) -> Self {
        Self { config }
    }

    fn ok_result(output: impl Into<String>) -> ToolResult {
        ToolResult::success(output)
    }

    fn err_result(error: impl Into<String>) -> ToolResult {
        ToolResult::failure(error)
    }

    fn semantic_process_result(
        raw_command: &str,
        process_result: TestResult,
        fallback_error: &'static str,
    ) -> ToolResult {
        let output_kind = classify_shell_command_output_kind(raw_command);
        let stream_mode = match output_kind {
            "shell.cargo_test" | "shell.cargo_clippy" => {
                ToolResultSemanticStreamMode::CombinedOutputAndError
            }
            _ => ToolResultSemanticStreamMode::PerField,
        };
        let mut source_fields = Vec::with_capacity(2);
        if !process_result.stdout.is_empty() {
            source_fields.push(ToolResultTextField::Output);
        }
        if !process_result.stderr.is_empty() {
            source_fields.push(ToolResultTextField::Error);
        }

        let error = if process_result.stderr.is_empty() {
            (!process_result.success).then(|| fallback_error.to_string())
        } else {
            Some(process_result.stderr)
        };

        ToolResult {
            success: process_result.success,
            output: process_result.stdout,
            error,
            attachments: Vec::new(),
            taint_labels: Vec::new(),
            semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
        }
        .with_output_kind(output_kind)
        .with_compaction_target(ToolResultCompactionTarget::OutputAndError)
        .with_stream_mode(stream_mode)
        .with_raw_command(raw_command)
        .with_source_fields(source_fields)
        .refresh_semantic_stats()
    }

    /// Resolve and validate a project directory, returning an error `ToolResult`
    /// on failure.
    fn resolve_project_dir(
        &self,
        workspace: &Path,
        project_name: &str,
    ) -> Result<std::path::PathBuf, Box<ToolResult>> {
        project::project_dir(workspace, &self.config, project_name)
            .map_err(|e| Box::new(Self::err_result(format!("Invalid project: {e}"))))
    }

    fn enforce_command_policy(
        ctx: &ExecutionContext,
        command: &str,
    ) -> Result<(), Box<ToolResult>> {
        let words = super::runner::parse_command_words(command)
            .map_err(|error| Box::new(Self::err_result(format!("Invalid command: {error}"))))?;
        let executable = words.first().cloned().unwrap_or_default();
        let args = words.into_iter().skip(1).collect::<Vec<_>>();

        if let Err(error) = enforce_process_spawn_guardrails(
            ctx,
            &executable,
            &args,
            "tool:codespace",
            ProcessSpawnClass::ToolEquivalent,
        ) {
            return Err(Box::new(Self::err_result(error.to_string())));
        }

        Ok(())
    }
}

impl Tool for CodespaceTool {
    fn name(&self) -> &'static str {
        "codespace"
    }

    fn description(&self) -> &'static str {
        "Sandboxed development environment for writing, testing, and promoting code as skills"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action to perform: create_project, list_projects, write_file, read_file, run_tests, exec, run, promote, git_init, git, delete_project, status"
                },
                "name": { "type": "string", "description": "Project name (for create_project)" },
                "language": { "type": "string", "description": "Programming language (for create_project)" },
                "project": { "type": "string", "description": "Target project name" },
                "path": { "type": "string", "description": "Relative file path within the project" },
                "content": { "type": "string", "description": "File content to write" },
                "command": { "type": "string", "description": "Command to execute (for exec/git)" },
                "args": { "type": "string", "description": "Arguments for run action" },
                "test_command": { "type": "string", "description": "Test command (for create_project)" },
                "entry_point": { "type": "string", "description": "Entry point command (for create_project)" },
                "tool_name": { "type": "string", "description": "Skill name (for promote)" },
                "tool_description": { "type": "string", "description": "Skill description (for promote)" }
            },
            "required": ["action"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a ExecutionContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<ToolResult>> + Send + 'a>>
    {
        Box::pin(async move {
            let action: CodespaceAction = serde_json::from_value(args)
                .map_err(|e| anyhow::anyhow!("Invalid codespace action: {e}"))?;

            let workspace = ctx.workspace_dir.clone();

            match action {
                CodespaceAction::CreateProject {
                    name,
                    language,
                    test_command,
                    entry_point,
                } => {
                    self.handle_create(&workspace, name, language, test_command, entry_point)
                        .await
                }
                CodespaceAction::ListProjects => self.handle_list(&workspace).await,
                CodespaceAction::WriteFile {
                    project,
                    path,
                    content,
                } => {
                    self.handle_write_file(&workspace, &project, &path, &content)
                        .await
                }
                CodespaceAction::ReadFile { project, path } => {
                    self.handle_read_file(&workspace, &project, &path).await
                }
                CodespaceAction::RunTests { project } => {
                    self.handle_run_tests(ctx, &workspace, &project).await
                }
                CodespaceAction::Exec { project, command } => {
                    self.handle_exec(ctx, &workspace, &project, &command).await
                }
                CodespaceAction::Run { project, args } => {
                    self.handle_run(ctx, &workspace, &project, args).await
                }
                CodespaceAction::Promote {
                    project,
                    tool_name,
                    tool_description,
                } => {
                    self.handle_promote(ctx, &workspace, &project, &tool_name, &tool_description)
                        .await
                }
                CodespaceAction::GitInit { project } => {
                    self.handle_git_init(ctx, &workspace, &project).await
                }
                CodespaceAction::Git { project, command } => {
                    self.handle_git(ctx, &workspace, &project, &command).await
                }
                CodespaceAction::DeleteProject { project } => {
                    self.handle_delete(&workspace, &project).await
                }
                CodespaceAction::Status { project } => {
                    self.handle_status(&workspace, &project).await
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::config::schema::CodespaceConfig;
    use crate::core::tools::middleware::ExecutionContext;
    use crate::security::{AutonomyLevel, SecurityPolicy};

    fn test_tool() -> CodespaceTool {
        CodespaceTool::new(CodespaceConfig {
            enabled: true,
            max_projects: 5,
            ..CodespaceConfig::default()
        })
    }

    fn test_ctx(workspace: std::path::PathBuf) -> ExecutionContext {
        ExecutionContext::test_default(Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: workspace,
            ..SecurityPolicy::default()
        }))
    }

    #[test]
    fn tool_name_and_description() {
        let tool = test_tool();
        assert_eq!(tool.name(), "codespace");
        assert!(!tool.description().is_empty());
    }

    #[tokio::test]
    async fn create_and_list_projects() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tool = test_tool();
        let ctx = test_ctx(tmp.path().to_path_buf());

        let result = tool
            .execute(
                json!({"action": "create_project", "name": "test1", "language": "python"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success, "create failed: {:?}", result.error);

        let result = tool
            .execute(json!({"action": "list_projects"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("test1"));
    }

    #[tokio::test]
    async fn write_and_read_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tool = test_tool();
        let ctx = test_ctx(tmp.path().to_path_buf());

        tool.execute(
            json!({"action": "create_project", "name": "rw", "language": "python"}),
            &ctx,
        )
        .await
        .unwrap();

        let result = tool
            .execute(
                json!({"action": "write_file", "project": "rw", "path": "src/main.py", "content": "print('hi')"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);

        let result = tool
            .execute(
                json!({"action": "read_file", "project": "rw", "path": "src/main.py"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("print('hi')"));
    }

    #[tokio::test]
    async fn write_file_rejects_traversal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tool = test_tool();
        let ctx = test_ctx(tmp.path().to_path_buf());

        tool.execute(
            json!({"action": "create_project", "name": "sec", "language": "bash"}),
            &ctx,
        )
        .await
        .unwrap();

        let result = tool
            .execute(
                json!({"action": "write_file", "project": "sec", "path": "../../etc/evil", "content": "x"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn exec_runs_command() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tool = test_tool();
        let ctx = test_ctx(tmp.path().to_path_buf());

        tool.execute(
            json!({"action": "create_project", "name": "ex", "language": "bash"}),
            &ctx,
        )
        .await
        .unwrap();

        let result = tool
            .execute(
                json!({"action": "exec", "project": "ex", "command": "echo hello"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("hello"));
    }

    #[tokio::test]
    async fn exec_rejects_command_outside_security_allowlist() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tool = test_tool();
        let ctx = test_ctx(tmp.path().to_path_buf());

        tool.execute(
            json!({"action": "create_project", "name": "blocked", "language": "bash"}),
            &ctx,
        )
        .await
        .unwrap();

        let result = tool
            .execute(
                json!({"action": "exec", "project": "blocked", "command": "python -c pass"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|msg| msg.contains("security policy"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_file_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let tmp = tempfile::TempDir::new().unwrap();
            let tool = test_tool();
            let ctx = test_ctx(tmp.path().to_path_buf());

            tool.execute(
                json!({"action": "create_project", "name": "symlink", "language": "python"}),
                &ctx,
            )
            .await
            .unwrap();

            let outside = tmp.path().join("outside.txt");
            std::fs::write(&outside, "secret").unwrap();
            let project_secret = tmp
                .path()
                .join(&tool.config.root_dir)
                .join("symlink")
                .join("src")
                .join("secret.txt");
            std::fs::create_dir_all(project_secret.parent().unwrap()).unwrap();
            symlink(&outside, &project_secret).unwrap();

            let result = tool
                .execute(
                    json!({"action": "read_file", "project": "symlink", "path": "src/secret.txt"}),
                    &ctx,
                )
                .await
                .unwrap();
            assert!(!result.success);
            assert!(
                result
                    .error
                    .as_deref()
                    .is_some_and(|msg| msg.contains("escapes the project directory"))
            );
        });
    }

    #[tokio::test]
    async fn delete_project_works() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tool = test_tool();
        let ctx = test_ctx(tmp.path().to_path_buf());

        tool.execute(
            json!({"action": "create_project", "name": "del", "language": "python"}),
            &ctx,
        )
        .await
        .unwrap();

        let result = tool
            .execute(json!({"action": "delete_project", "project": "del"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);

        let result = tool
            .execute(json!({"action": "list_projects"}), &ctx)
            .await
            .unwrap();
        assert!(!result.output.contains("del"));
    }

    #[tokio::test]
    async fn git_rejects_shell_metacharacters() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tool = test_tool();
        let ctx = test_ctx(tmp.path().to_path_buf());

        tool.execute(
            json!({"action": "create_project", "name": "gitproj", "language": "bash"}),
            &ctx,
        )
        .await
        .unwrap();

        let result = tool
            .execute(
                json!({"action": "git", "project": "gitproj", "command": "log; rm -rf /"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|e| e.contains("metacharacters"))
        );
    }

    #[tokio::test]
    async fn git_status_emits_shell_semantic_metadata() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tool = test_tool();
        let ctx = test_ctx(tmp.path().to_path_buf());

        tool.execute(
            json!({"action": "create_project", "name": "gitmeta", "language": "bash"}),
            &ctx,
        )
        .await
        .unwrap();

        tool.execute(json!({"action": "git_init", "project": "gitmeta"}), &ctx)
            .await
            .unwrap();

        let result = tool
            .execute(
                json!({"action": "git", "project": "gitmeta", "command": "status --short"}),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(
            result.semantic.output_kind.as_deref(),
            Some("shell.git_status")
        );
        assert_eq!(
            result.semantic.stream_mode,
            crate::core::tools::traits::ToolResultSemanticStreamMode::PerField
        );
        assert_eq!(
            result.semantic.raw_command.as_deref(),
            Some("git status --short")
        );
    }

    #[tokio::test]
    async fn run_tests_marks_cargo_for_combined_stream_compaction() {
        let tmp = tempfile::TempDir::new().unwrap();
        let tool = test_tool();
        let ctx = test_ctx(tmp.path().to_path_buf());

        tool.execute(
            json!({
                "action": "create_project",
                "name": "cargo-meta",
                "language": "bash",
                "test_command": "cargo test --help"
            }),
            &ctx,
        )
        .await
        .unwrap();

        let result = tool
            .execute(
                json!({"action": "run_tests", "project": "cargo-meta"}),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(
            result.semantic.output_kind.as_deref(),
            Some("shell.cargo_test")
        );
        assert_eq!(
            result.semantic.stream_mode,
            crate::core::tools::traits::ToolResultSemanticStreamMode::CombinedOutputAndError
        );
        assert_eq!(
            result.semantic.raw_command.as_deref(),
            Some("cargo test --help")
        );
    }
}
