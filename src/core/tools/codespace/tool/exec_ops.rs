//! Codespace execution operations — `run_tests`, exec, run, promote, `git_init`, git.
//!
//! Each handler validates command strings against the security policy via
//! `CodespaceTool::enforce_command_policy` before delegating to `runner`.
//! The `handle_promote` handler additionally requires a passing test result
//! recorded in the project's `PROJECT.toml` before writing skill files.

use std::path::Path;

use super::super::{project, runner};
use super::CodespaceTool;
use crate::core::tools::traits::ToolResult;

impl CodespaceTool {
    pub(super) async fn handle_run_tests(
        &self,
        ctx: &crate::core::tools::middleware::ExecutionContext,
        workspace: &Path,
        project_name: &str,
    ) -> anyhow::Result<ToolResult> {
        let proj_dir = match self.resolve_project_dir(workspace, project_name) {
            Ok(d) => d,
            Err(error) => return Ok(*error),
        };

        let mut proj = match project::load_project(&proj_dir).await {
            Ok(p) => p,
            Err(e) => return Ok(Self::err_result(format!("Failed to load project: {e}"))),
        };
        let Some(test_command) = proj.test_command.as_deref() else {
            return Ok(Self::err_result(
                "Failed to load project: No test_command configured for this project",
            ));
        };
        if let Err(error) = Self::enforce_command_policy(ctx, test_command) {
            return Ok(*error);
        }

        let test_result =
            match runner::run_tests(&proj_dir, Some(test_command), self.config.test_timeout_secs)
                .await
            {
                Ok(r) => r,
                Err(e) => return Ok(Self::err_result(format!("Test execution failed: {e}"))),
            };

        proj.last_test_result = Some(test_result.clone());
        project::save_project(&proj).await.ok();

        Ok(Self::semantic_process_result(
            test_command,
            test_result,
            "Tests did not pass",
        ))
    }

    pub(super) async fn handle_exec(
        &self,
        ctx: &crate::core::tools::middleware::ExecutionContext,
        workspace: &Path,
        project_name: &str,
        command: &str,
    ) -> anyhow::Result<ToolResult> {
        let proj_dir = match self.resolve_project_dir(workspace, project_name) {
            Ok(d) => d,
            Err(error) => return Ok(*error),
        };
        if !proj_dir.join("PROJECT.toml").exists() {
            return Ok(Self::err_result(format!(
                "Project '{project_name}' does not exist"
            )));
        }

        // Validate command before passing to shell — prevent injection.
        if let Err(e) = runner::validate_command(command) {
            return Ok(Self::err_result(format!("Disallowed command: {e}")));
        }
        if let Err(error) = Self::enforce_command_policy(ctx, command) {
            return Ok(*error);
        }
        let result = runner::run_command(&proj_dir, command, self.config.test_timeout_secs).await;
        match result {
            Ok(r) => Ok(Self::semantic_process_result(
                command,
                r,
                "Command exited with non-zero status",
            )),
            Err(e) => Ok(Self::err_result(format!("Execution failed: {e}"))),
        }
    }

    pub(super) async fn handle_run(
        &self,
        ctx: &crate::core::tools::middleware::ExecutionContext,
        workspace: &Path,
        project_name: &str,
        args: Option<String>,
    ) -> anyhow::Result<ToolResult> {
        let proj_dir = match self.resolve_project_dir(workspace, project_name) {
            Ok(d) => d,
            Err(error) => return Ok(*error),
        };

        let proj = match project::load_project(&proj_dir).await {
            Ok(p) => p,
            Err(e) => return Ok(Self::err_result(format!("Failed to load project: {e}"))),
        };
        let Some(entry_point) = proj.entry_point.as_deref() else {
            return Ok(Self::err_result(
                "Failed to load project: No entry_point configured for this project",
            ));
        };

        let result = runner::run_entry_point(
            &proj_dir,
            Some(entry_point),
            args.as_deref(),
            self.config.test_timeout_secs,
        );
        let command_preview = match args.as_deref() {
            Some(args) => format!("{entry_point} {args}"),
            None => entry_point.to_string(),
        };
        if let Err(error) = Self::enforce_command_policy(ctx, &command_preview) {
            return Ok(*error);
        }
        let result = result.await;
        match result {
            Ok(r) => Ok(Self::semantic_process_result(
                &command_preview,
                r,
                "Entry point exited with non-zero status",
            )),
            Err(e) => Ok(Self::err_result(format!("Run failed: {e}"))),
        }
    }

    pub(super) async fn handle_promote(
        &self,
        _ctx: &crate::core::tools::middleware::ExecutionContext,
        workspace: &Path,
        project_name: &str,
        tool_name: &str,
        tool_description: &str,
    ) -> anyhow::Result<ToolResult> {
        let proj_dir = match self.resolve_project_dir(workspace, project_name) {
            Ok(d) => d,
            Err(error) => return Ok(*error),
        };

        let mut proj = match project::load_project(&proj_dir).await {
            Ok(p) => p,
            Err(e) => return Ok(Self::err_result(format!("Failed to load project: {e}"))),
        };

        // Verify tests passed.
        match &proj.last_test_result {
            Some(r) if r.success => {}
            Some(_) => {
                return Ok(Self::err_result(
                    "Cannot promote: last test run did not pass. Run tests first.",
                ));
            }
            None => {
                return Ok(Self::err_result(
                    "Cannot promote: no test results. Run tests first.",
                ));
            }
        }

        let skills_dir = workspace.join("skills");

        match crate::core::tools::codespace::promotion::promote_project(
            &proj,
            tool_name,
            tool_description,
            &skills_dir,
        )
        .await
        {
            Ok(result) => {
                proj.promoted = true;
                project::save_project(&proj).await.ok();
                Ok(Self::ok_result(format!(
                    "Promoted '{}' as skill '{}' at {}\nSummary: {}",
                    project_name,
                    tool_name,
                    result.output_dir.display(),
                    result.summary,
                )))
            }
            Err(e) => Ok(Self::err_result(format!("Promotion failed: {e}"))),
        }
    }

    pub(super) async fn handle_git_init(
        &self,
        ctx: &crate::core::tools::middleware::ExecutionContext,
        workspace: &Path,
        project_name: &str,
    ) -> anyhow::Result<ToolResult> {
        let proj_dir = match self.resolve_project_dir(workspace, project_name) {
            Ok(d) => d,
            Err(error) => return Ok(*error),
        };
        if !proj_dir.join("PROJECT.toml").exists() {
            return Ok(Self::err_result(format!(
                "Project '{project_name}' does not exist"
            )));
        }

        if let Err(error) = Self::enforce_command_policy(ctx, "git init") {
            return Ok(*error);
        }
        let result =
            runner::run_command(&proj_dir, "git init", self.config.test_timeout_secs).await;
        match result {
            Ok(r) if r.success => Ok(Self::ok_result(format!(
                "Initialized git repository in project '{project_name}'"
            ))),
            Ok(r) => Ok(Self::err_result(format!("git init failed: {}", r.stderr))),
            Err(e) => Ok(Self::err_result(format!("git init failed: {e}"))),
        }
    }

    pub(super) async fn handle_git(
        &self,
        ctx: &crate::core::tools::middleware::ExecutionContext,
        workspace: &Path,
        project_name: &str,
        git_command: &str,
    ) -> anyhow::Result<ToolResult> {
        let proj_dir = match self.resolve_project_dir(workspace, project_name) {
            Ok(d) => d,
            Err(error) => return Ok(*error),
        };
        if !proj_dir.join("PROJECT.toml").exists() {
            return Ok(Self::err_result(format!(
                "Project '{project_name}' does not exist"
            )));
        }

        // Only allow git subcommands, block shell injection.
        if git_command.contains(runner::SHELL_METACHAR) {
            return Ok(Self::err_result(
                "Git command contains disallowed shell metacharacters",
            ));
        }

        let full_cmd = format!("git {git_command}");
        if let Err(error) = Self::enforce_command_policy(ctx, &full_cmd) {
            return Ok(*error);
        }
        let result = runner::run_command(&proj_dir, &full_cmd, self.config.test_timeout_secs).await;
        match result {
            Ok(r) => Ok(Self::semantic_process_result(
                &full_cmd,
                r,
                "git command exited with non-zero status",
            )),
            Err(e) => Ok(Self::err_result(format!("git command failed: {e}"))),
        }
    }
}
