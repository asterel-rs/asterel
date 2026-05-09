//! Codespace project management operations — create, list, `write_file`, `read_file`, delete, status.
//!
//! `handle_write_file` enforces three file-safety rules in order:
//! 1. Rejects paths containing `..` or absolute path prefixes.
//! 2. Verifies the canonical parent directory remains inside the project root
//!    after `create_dir_all` (catches symlink escapes).
//! 3. Checks that the new content will not push the project over the
//!    `max_project_size_mb` disk quota.
//!
//! Writes are atomic via a UUID-named temp file followed by `rename`.
//!
//! `handle_read_file` performs the same traversal and symlink checks before
//! reading.

use std::fmt::Write as _;
use std::path::Path;

use super::super::project;
use super::CodespaceTool;
use crate::core::tools::traits::ToolResult;

impl CodespaceTool {
    pub(super) async fn handle_create(
        &self,
        workspace: &Path,
        name: String,
        language: String,
        test_command: Option<String>,
        entry_point: Option<String>,
    ) -> anyhow::Result<ToolResult> {
        match project::create_project(
            workspace,
            &self.config,
            &name,
            &language,
            test_command,
            entry_point,
        )
        .await
        {
            Ok(proj) => Ok(Self::ok_result(format!(
                "Created project '{}' (language: {}, root: {})",
                proj.name,
                proj.language,
                proj.root.display()
            ))),
            Err(e) => Ok(Self::err_result(format!("Failed to create project: {e}"))),
        }
    }

    pub(super) async fn handle_list(&self, workspace: &Path) -> anyhow::Result<ToolResult> {
        match project::list_projects(workspace, &self.config).await {
            Ok(names) => {
                if names.is_empty() {
                    Ok(Self::ok_result("No projects in codespace."))
                } else {
                    let mut list = String::new();
                    for n in &names {
                        if !list.is_empty() {
                            list.push('\n');
                        }
                        let _ = write!(list, "  - {n}");
                    }
                    Ok(Self::ok_result(format!(
                        "Projects ({}):\n{list}",
                        names.len()
                    )))
                }
            }
            Err(e) => Ok(Self::err_result(format!("Failed to list projects: {e}"))),
        }
    }

    pub(super) async fn handle_write_file(
        &self,
        workspace: &Path,
        project_name: &str,
        path: &str,
        content: &str,
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

        // Reject path traversal.
        if path.contains("..") || path.starts_with('/') || path.starts_with('\\') {
            return Ok(Self::err_result(
                "Path must be relative and cannot contain '..'",
            ));
        }

        let file_path = proj_dir.join(path);
        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }

        // Verify the resolved path does not escape the project directory via symlinks.
        let canonical_parent = tokio::fs::canonicalize(file_path.parent().unwrap_or(&proj_dir))
            .await
            .unwrap_or_else(|_| file_path.clone());
        let canonical_proj = tokio::fs::canonicalize(&proj_dir)
            .await
            .unwrap_or_else(|_| proj_dir.clone());
        if !canonical_parent.starts_with(&canonical_proj) {
            return Ok(Self::err_result("Path escapes the project directory"));
        }

        // Check size limit.
        let new_len = u64::try_from(content.len()).unwrap_or(u64::MAX);
        let current_size = project::codespace_size_bytes(&proj_dir).await.unwrap_or(0);
        let limit = self.config.max_project_size_mb * 1_024 * 1_024;
        if current_size.saturating_add(new_len) > limit {
            return Ok(Self::err_result(format!(
                "Write would exceed project size limit ({} MB)",
                self.config.max_project_size_mb
            )));
        }

        // Atomic write via temp file.
        let temp_name = format!(".asterel_tmp_{}", uuid::Uuid::new_v4());
        let temp_path = file_path.parent().unwrap_or(&proj_dir).join(&temp_name);
        if let Err(e) = tokio::fs::write(&temp_path, content).await {
            return Ok(Self::err_result(format!("Failed to write file: {e}")));
        }
        match tokio::fs::rename(&temp_path, &file_path).await {
            Ok(()) => Ok(Self::ok_result(format!(
                "Written {} bytes to {path}",
                content.len()
            ))),
            Err(e) => {
                let _ = tokio::fs::remove_file(&temp_path).await;
                Ok(Self::err_result(format!("Failed to write file: {e}")))
            }
        }
    }

    pub(super) async fn handle_read_file(
        &self,
        workspace: &Path,
        project_name: &str,
        path: &str,
    ) -> anyhow::Result<ToolResult> {
        let proj_dir = match self.resolve_project_dir(workspace, project_name) {
            Ok(d) => d,
            Err(error) => return Ok(*error),
        };

        if path.contains("..") || path.starts_with('/') || path.starts_with('\\') {
            return Ok(Self::err_result(
                "Path must be relative and cannot contain '..'",
            ));
        }

        let file_path = proj_dir.join(path);

        // Verify the resolved path does not escape the project directory via symlinks.
        let canonical_parent = tokio::fs::canonicalize(file_path.parent().unwrap_or(&proj_dir))
            .await
            .unwrap_or_else(|_| file_path.clone());
        let canonical_proj = tokio::fs::canonicalize(&proj_dir)
            .await
            .unwrap_or_else(|_| proj_dir.clone());
        if !canonical_parent.starts_with(&canonical_proj) {
            return Ok(Self::err_result("Path escapes the project directory"));
        }

        if let Ok(metadata) = tokio::fs::symlink_metadata(&file_path).await
            && metadata.file_type().is_symlink()
        {
            let resolved_file = match tokio::fs::canonicalize(&file_path).await {
                Ok(path) => path,
                Err(e) => {
                    return Ok(Self::err_result(format!(
                        "Failed to resolve file path: {e}"
                    )));
                }
            };
            if !resolved_file.starts_with(&canonical_proj) {
                return Ok(Self::err_result("Path escapes the project directory"));
            }
        }

        match tokio::fs::read_to_string(&file_path).await {
            Ok(content) => Ok(Self::ok_result(content)),
            Err(e) => Ok(Self::err_result(format!("Failed to read file: {e}"))),
        }
    }

    pub(super) async fn handle_delete(
        &self,
        workspace: &Path,
        project_name: &str,
    ) -> anyhow::Result<ToolResult> {
        match project::delete_project(workspace, &self.config, project_name).await {
            Ok(()) => Ok(Self::ok_result(format!("Deleted project '{project_name}'"))),
            Err(e) => Ok(Self::err_result(format!("Failed to delete project: {e}"))),
        }
    }

    pub(super) async fn handle_status(
        &self,
        workspace: &Path,
        project_name: &str,
    ) -> anyhow::Result<ToolResult> {
        let proj_dir = match self.resolve_project_dir(workspace, project_name) {
            Ok(d) => d,
            Err(error) => return Ok(*error),
        };

        let proj = match project::load_project(&proj_dir).await {
            Ok(p) => p,
            Err(e) => return Ok(Self::err_result(format!("Failed to load project: {e}"))),
        };

        let size = project::codespace_size_bytes(&proj_dir).await.unwrap_or(0);
        // Cast safety: project byte size is operationally bounded and MB display tolerates f64 precision.
        #[allow(clippy::cast_precision_loss)]
        let size_mb = size as f64 / (1024.0 * 1024.0);

        let test_status = match &proj.last_test_result {
            Some(r) if r.success => format!("PASSED ({}ms, {})", r.duration_ms, r.ran_at),
            Some(r) => format!("FAILED ({}ms, {})", r.duration_ms, r.ran_at),
            None => "No tests run".into(),
        };

        Ok(Self::ok_result(format!(
            "Project: {}\nLanguage: {}\nCreated: {}\nPromoted: {}\nSize: {size_mb:.2} MB\n\
             Test command: {}\nEntry point: {}\nLast test: {test_status}",
            proj.name,
            proj.language,
            proj.created_at,
            proj.promoted,
            proj.test_command.as_deref().unwrap_or("(none)"),
            proj.entry_point.as_deref().unwrap_or("(none)"),
        )))
    }
}
