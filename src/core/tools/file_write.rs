//! Sandboxed file-write tool (`file_write`).
//!
//! # Security model
//!
//! Writes are confined to the workspace directory through two complementary
//! layers:
//!
//! 1. **Pre-execution middleware** (`SecurityMiddleware`) rejects lexically
//!    and canonically out-of-workspace paths, including bootstrap files
//!    (`SOUL.md`, `CHARACTER.md`, `USER.md`, `AGENTS.md`).
//! 2. **Atomic write with in-tool checks** — The tool writes to a randomly
//!    named `.asterel_tmp_<uuid>` file and only renames it into place.
//!    Before the rename it performs:
//!    - `..` component rejection at path resolution time (prevents path
//!      traversal even when `SecurityMiddleware` is bypassed in tests).
//!    - Hard-link rejection on the target path (guards against `link(2)`
//!      attacks where a workspace-visible entry points outside).
//!    - Symlink rejection using `O_NOFOLLOW` on Unix (or `symlink_metadata`
//!      on other platforms) so the rename cannot overwrite a symlink target
//!      outside the workspace.
//!
//! # File-tracker integration
//!
//! Every successful write is recorded in the process-global
//! [`FileOwnershipTracker`] so the agent can later identify files it
//! created or modified during a session.  This enables safe cleanup and
//! attribution in audit trails.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use serde_json::json;

use super::file_tracker::global_tracker;
use super::schema_helpers::{failed_tool_result, has_multiple_hard_links, workspace_path_property};
use super::traits::{Tool, ToolResult};
use crate::core::tools::middleware::ExecutionContext;

/// Tool that writes content to a workspace file, creating parent directories
/// as needed and atomically replacing any existing file.
pub struct FileWriteTool;

struct WriteRequest<'a> {
    path: &'a str,
    content: &'a str,
}

struct WriteTarget {
    resolved_parent: PathBuf,
    resolved_target: PathBuf,
}

impl FileWriteTool {
    /// Create a new file-write tool instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for FileWriteTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for FileWriteTool {
    fn name(&self) -> &'static str {
        "file_write"
    }

    fn description(&self) -> &'static str {
        "Write contents to a file in the workspace"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": workspace_path_property(),
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move { self.execute_impl(args, ctx).await })
    }
}

impl FileWriteTool {
    async fn execute_impl(
        &self,
        args: serde_json::Value,
        ctx: &ExecutionContext,
    ) -> anyhow::Result<ToolResult> {
        let request = Self::parse_write_request(&args)?;
        let target = match Self::resolve_write_target(ctx, request.path).await? {
            Ok(target) => target,
            Err(result) => return Ok(result),
        };

        let file_existed = tokio::fs::metadata(&target.resolved_target).await.is_ok();
        let result = Self::write_atomically(&target, &request).await;
        if result.success {
            let turn = ctx.turn_number;
            if file_existed {
                global_tracker().record_modify(&target.resolved_target, turn);
            } else {
                global_tracker().record_create(&target.resolved_target, turn);
            }
        }
        Ok(result)
    }

    fn parse_write_request(args: &serde_json::Value) -> anyhow::Result<WriteRequest<'_>> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' parameter"))?;

        Ok(WriteRequest { path, content })
    }

    async fn resolve_write_target(
        ctx: &ExecutionContext,
        path: &str,
    ) -> anyhow::Result<Result<WriteTarget, ToolResult>> {
        let full_path = ctx.workspace_dir.join(path);
        let Some(parent) = full_path.parent() else {
            return Ok(Err(failed_tool_result(
                "Invalid path: missing parent directory",
            )));
        };

        // Validate that the parent does not escape the workspace before
        // creating any directories, preventing side-effects outside the
        // sandbox when the path contains `..` components.
        if std::path::absolute(parent).is_ok_and(|logical| !logical.starts_with(&ctx.workspace_dir))
        {
            return Ok(Err(failed_tool_result(
                "Path traversal rejected: resolved parent is outside workspace",
            )));
        }

        if let Some(result) = Self::ensure_parent_directory(parent).await? {
            return Ok(Err(result));
        }

        let resolved_parent = match tokio::fs::canonicalize(parent).await {
            Ok(path) => path,
            Err(e) => {
                return Ok(Err(failed_tool_result(format!(
                    "Failed to resolve file path: {e}"
                ))));
            }
        };

        let Some(file_name) = full_path.file_name() else {
            return Ok(Err(failed_tool_result("Invalid path: missing file name")));
        };

        let resolved_target = resolved_parent.join(file_name);
        if let Some(result) = Self::reject_hard_link_target(&resolved_target).await {
            return Ok(Err(result));
        }

        Ok(Ok(WriteTarget {
            resolved_parent,
            resolved_target,
        }))
    }

    async fn ensure_parent_directory(parent: &Path) -> anyhow::Result<Option<ToolResult>> {
        match tokio::fs::metadata(parent).await {
            Ok(meta) if meta.is_dir() => Ok(None),
            Ok(_) => Ok(Some(failed_tool_result(format!(
                "Invalid path: parent is not a directory: {}",
                parent.display()
            )))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tokio::fs::create_dir_all(parent).await?;
                Ok(None)
            }
            Err(e) => Ok(Some(failed_tool_result(format!(
                "Failed to inspect parent directory: {e}"
            )))),
        }
    }

    async fn reject_hard_link_target(resolved_target: &Path) -> Option<ToolResult> {
        if let Ok(meta) = tokio::fs::metadata(resolved_target).await
            && has_multiple_hard_links(&meta)
        {
            return Some(failed_tool_result(format!(
                "Refusing to write file with multiple hard links: {}",
                resolved_target.display()
            )));
        }

        None
    }

    async fn write_atomically(target: &WriteTarget, request: &WriteRequest<'_>) -> ToolResult {
        let temp_path = target
            .resolved_parent
            .join(format!(".asterel_tmp_{}", uuid::Uuid::new_v4()));

        if let Err(e) = tokio::fs::write(&temp_path, request.content).await {
            return failed_tool_result(format!("Failed to write file: {e}"));
        }

        if let Some(result) = Self::reject_symlink_target(&target.resolved_target, &temp_path).await
        {
            return result;
        }

        match tokio::fs::rename(&temp_path, &target.resolved_target).await {
            Ok(()) => ToolResult {
                success: true,
                output: format!(
                    "Written {} bytes to {}",
                    request.content.len(),
                    request.path
                ),
                error: None,
                attachments: Vec::new(),
                taint_labels: Vec::new(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            },
            Err(e) => {
                Self::cleanup_temp_file(&temp_path).await;
                failed_tool_result(format!("Failed to write file: {e}"))
            }
        }
    }

    async fn cleanup_temp_file(temp_path: &Path) {
        if let Err(error) = tokio::fs::remove_file(temp_path).await {
            tracing::warn!(error = %error, path = %temp_path.display(), "failed to remove temporary file after write failure");
        }
    }

    async fn reject_symlink_target(resolved_target: &Path, temp_path: &Path) -> Option<ToolResult> {
        #[cfg(unix)]
        {
            let nofollow_check = rustix::fs::open(
                resolved_target,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            );
            if let Err(e) = nofollow_check
                && e == rustix::io::Errno::LOOP
            {
                Self::cleanup_temp_file(temp_path).await;
                return Some(failed_tool_result(format!(
                    "Refusing to write through symlink: {}",
                    resolved_target.display()
                )));
            }
        }

        #[cfg(not(unix))]
        {
            if let Ok(meta) = tokio::fs::symlink_metadata(resolved_target).await
                && meta.file_type().is_symlink()
            {
                Self::cleanup_temp_file(temp_path).await;
                return Some(failed_tool_result(format!(
                    "Refusing to write through symlink: {}",
                    resolved_target.display()
                )));
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tools::middleware::ExecutionContext;
    use crate::core::tools::schema_helpers::test_security_policy;

    #[test]
    fn file_write_name() {
        let tool = FileWriteTool::new();
        assert_eq!(tool.name(), "file_write");
    }

    #[test]
    fn file_write_schema_has_path_and_content() {
        let tool = FileWriteTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["path"].is_object());
        assert!(schema["properties"]["content"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("path")));
        assert!(required.contains(&json!("content")));
    }

    #[tokio::test]
    async fn file_write_creates_file() {
        let dir = std::env::temp_dir().join("asterel_test_file_write");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = FileWriteTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(dir.clone()));
        let result = tool
            .execute(json!({"path": "out.txt", "content": "written!"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("8 bytes"));

        let content = tokio::fs::read_to_string(dir.join("out.txt"))
            .await
            .unwrap();
        assert_eq!(content, "written!");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_write_creates_parent_dirs() {
        let dir = std::env::temp_dir().join("asterel_test_file_write_nested");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = FileWriteTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(dir.clone()));
        let result = tool
            .execute(json!({"path": "a/b/c/deep.txt", "content": "deep"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);

        let content = tokio::fs::read_to_string(dir.join("a/b/c/deep.txt"))
            .await
            .unwrap();
        assert_eq!(content, "deep");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_write_overwrites_existing() {
        let dir = std::env::temp_dir().join("asterel_test_file_write_overwrite");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("exist.txt"), "old")
            .await
            .unwrap();

        let tool = FileWriteTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(dir.clone()));
        let result = tool
            .execute(json!({"path": "exist.txt", "content": "new"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);

        let content = tokio::fs::read_to_string(dir.join("exist.txt"))
            .await
            .unwrap();
        assert_eq!(content, "new");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_write_missing_path_param() {
        let tool = FileWriteTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(std::env::temp_dir()));
        let result = tool.execute(json!({"content": "data"}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn file_write_missing_content_param() {
        let tool = FileWriteTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(std::env::temp_dir()));
        let result = tool.execute(json!({"path": "file.txt"}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn file_write_empty_content() {
        let dir = std::env::temp_dir().join("asterel_test_file_write_empty");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = FileWriteTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(dir.clone()));
        let result = tool
            .execute(json!({"path": "empty.txt", "content": ""}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("0 bytes"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn file_write_rejects_hard_linked_target() {
        let root = std::env::temp_dir().join("asterel_test_file_write_hardlink");
        let workspace = root.join("workspace");
        let outside = root.join("outside");
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::create_dir_all(&outside).await.unwrap();

        let outside_file = outside.join("shared.txt");
        tokio::fs::write(&outside_file, "outside").await.unwrap();
        tokio::fs::hard_link(&outside_file, workspace.join("shared.txt"))
            .await
            .unwrap();

        let tool = FileWriteTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(workspace));
        let result = tool
            .execute(json!({"path": "shared.txt", "content": "mutated"}), &ctx)
            .await
            .unwrap();

        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|err| err.contains("multiple hard links"))
        );
        let outside_content = tokio::fs::read_to_string(&outside_file).await.unwrap();
        assert_eq!(outside_content, "outside");

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn file_write_rejects_symlink_target() {
        let root = std::env::temp_dir().join("asterel_test_file_write_symlink");
        let workspace = root.join("workspace");
        let outside = root.join("outside");
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::create_dir_all(&outside).await.unwrap();

        let target = outside.join("secret.txt");
        tokio::fs::write(&target, "secret data").await.unwrap();
        tokio::fs::symlink(&target, workspace.join("link.txt"))
            .await
            .unwrap();

        let tool = FileWriteTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(workspace));
        let result = tool
            .execute(
                json!({"path": "link.txt", "content": "overwrite attempt"}),
                &ctx,
            )
            .await
            .unwrap();

        assert!(!result.success, "symlink target write must be rejected");
        // Original file must not be modified.
        let content = tokio::fs::read_to_string(&target).await.unwrap();
        assert_eq!(content, "secret data");

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn file_write_path_outside_workspace_rejected() {
        let workspace = std::env::temp_dir().join("asterel_test_file_write_outside");
        let _ = tokio::fs::remove_dir_all(&workspace).await;
        tokio::fs::create_dir_all(&workspace).await.unwrap();

        let tool = FileWriteTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(workspace.clone()));
        let result = tool
            .execute(
                json!({"path": "../../../etc/evil.txt", "content": "hack"}),
                &ctx,
            )
            .await;

        // Must either return an error or a non-success ToolResult.
        if let Ok(r) = result {
            assert!(!r.success, "path traversal write must be rejected");
        }

        let _ = tokio::fs::remove_dir_all(&workspace).await;
    }

    #[tokio::test]
    async fn file_write_sequential_multi_file_no_corruption() {
        let dir = std::env::temp_dir().join("asterel_test_file_write_sequential");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = FileWriteTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(dir.clone()));

        // Write multiple files sequentially to verify no corruption.
        for i in 0..5 {
            let fname = format!("file_{i}.txt");
            let content = format!("content_{i}");
            let result = tool
                .execute(json!({"path": fname, "content": content}), &ctx)
                .await
                .unwrap();
            assert!(result.success, "write {i} must succeed");
        }

        // Verify all files have correct content.
        for i in 0..5 {
            let content = tokio::fs::read_to_string(dir.join(format!("file_{i}.txt")))
                .await
                .unwrap();
            assert_eq!(content, format!("content_{i}"));
        }

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
