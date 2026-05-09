//! Sandboxed file-delete tool (`file_delete`).
//!
//! Deletes a workspace file that was created or modified by the agent this
//! session (tracked by the process-global [`FileOwnershipTracker`]).
//!
//! # Ownership gate
//!
//! Only files recorded by `file_write` in the current session may be deleted.
//! Attempting to delete a foreign file returns a `ToolResult` error; use the
//! `shell` tool with operator approval for foreign-file deletion.
//!
//! # Security model
//!
//! Path validation (workspace confinement, symlink escape rejection) runs in
//! `SecurityMiddleware` via `check_file_delete_policy` before this tool
//! executes.  The tool then applies the ownership gate as a second layer.

use std::future::Future;
use std::pin::Pin;

use serde_json::json;

use super::file_tracker::global_tracker;
use super::schema_helpers::{failed_tool_result, workspace_path_property};
use super::traits::{Tool, ToolResult};
use crate::core::tools::middleware::ExecutionContext;

/// Tool that deletes an agent-owned workspace file.
pub struct FileDeleteTool;

impl FileDeleteTool {
    /// Create a new file-delete tool instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for FileDeleteTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for FileDeleteTool {
    fn name(&self) -> &'static str {
        "file_delete"
    }

    fn description(&self) -> &'static str {
        "Delete a workspace file that was created or modified by the agent this session"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": workspace_path_property()
            },
            "required": ["path"]
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

impl FileDeleteTool {
    async fn execute_impl(
        &self,
        args: serde_json::Value,
        ctx: &ExecutionContext,
    ) -> anyhow::Result<ToolResult> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

        let full_path = ctx.workspace_dir.join(path);

        // Ownership gate: only files created or modified by the agent this session.
        if !global_tracker().is_owned(&full_path) {
            return Ok(failed_tool_result(format!(
                "Cannot delete '{path}': file was not created or modified by the agent this \
                 session. Use the shell tool (with operator approval) to delete foreign files."
            )));
        }

        match tokio::fs::remove_file(&full_path).await {
            Ok(()) => {
                global_tracker().remove(&full_path);
                let remaining = global_tracker().owned_count();
                Ok(ToolResult {
                    success: true,
                    output: format!(
                        "Deleted {path} ({remaining} agent-owned file{} remaining in session)",
                        if remaining == 1 { "" } else { "s" }
                    ),
                    error: None,
                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                })
            }
            Err(e) => Ok(failed_tool_result(format!(
                "Failed to delete '{path}': {e}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tools::file_tracker::global_tracker;
    use crate::core::tools::middleware::ExecutionContext;
    use crate::core::tools::schema_helpers::test_security_policy;

    #[tokio::test]
    async fn file_delete_name() {
        let tool = FileDeleteTool::new();
        assert_eq!(tool.name(), "file_delete");
    }

    #[tokio::test]
    async fn file_delete_schema_has_path() {
        let tool = FileDeleteTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["path"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("path")));
    }

    #[tokio::test]
    async fn file_delete_rejects_unowned_file() {
        let dir = std::env::temp_dir().join("asterel_test_file_delete_foreign");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("foreign.txt"), "data")
            .await
            .unwrap();

        let tool = FileDeleteTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(dir.clone()));
        let result = tool
            .execute(json!({"path": "foreign.txt"}), &ctx)
            .await
            .unwrap();

        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|e| e.contains("not created or modified"))
        );
        // File must still exist.
        assert!(dir.join("foreign.txt").exists());

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_delete_removes_owned_file() {
        let dir = std::env::temp_dir().join("asterel_test_file_delete_owned");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let file_path = dir.join("owned.txt");
        tokio::fs::write(&file_path, "data").await.unwrap();

        // Register ownership manually (as file_write would).
        global_tracker().record_create(&file_path, 1);

        let tool = FileDeleteTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(dir.clone()));
        let result = tool
            .execute(json!({"path": "owned.txt"}), &ctx)
            .await
            .unwrap();

        assert!(result.success, "delete of owned file must succeed");
        assert!(result.output.contains("Deleted owned.txt"));
        assert!(!file_path.exists(), "file must be removed");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_delete_missing_path_param() {
        let tool = FileDeleteTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(std::env::temp_dir()));
        let result = tool.execute(json!({}), &ctx).await;
        assert!(result.is_err());
    }
}
