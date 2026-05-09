//! Sandboxed file-read tool (`file_read`).
//!
//! # Security model
//!
//! Reading is strictly confined to the workspace directory.  The tool
//! enforces three independent layers of protection so that a single bypass
//! does not suffice for an escape:
//!
//! 1. **Pre-execution middleware** (`SecurityMiddleware`) checks the
//!    lexical path and the canonicalised path against the workspace root
//!    before the tool runs.
//! 2. **Symlink rejection** — On Unix the file is opened with `O_NOFOLLOW`
//!    at every path component via `openat(2)`, so even a symlink buried
//!    deep in a relative path cannot redirect the read.  On Windows, the
//!    `FILE_FLAG_OPEN_REPARSE_POINT` flag achieves the same for reparse
//!    points.
//! 3. **Post-open metadata checks** — After acquiring a file descriptor the
//!    tool verifies the opened handle is a regular file (not a symlink,
//!    directory, or device), is within the 10 MB size limit, and has only
//!    one hard link.  The hard-link check prevents an attacker from creating
//!    a workspace-visible link to a secret file outside the workspace.
//!
//! These checks are performed on the *open file handle* (TOCTOU-safe),
//! not on a separately stat-ed path.

use std::future::Future;
use std::path::{Component, Path};
use std::pin::Pin;

use serde_json::json;
use tokio::io::AsyncReadExt;

use super::schema_helpers::{failed_tool_result, has_multiple_hard_links, workspace_path_property};
use super::traits::{Tool, ToolResult};
use crate::core::tools::middleware::ExecutionContext;

const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Tool that reads a workspace file and returns its UTF-8 content.
///
/// Paths may be workspace-relative (preferred) or absolute.  Absolute paths
/// are still validated against the workspace root by `SecurityMiddleware`
/// before this tool runs.
pub struct FileReadTool;

impl FileReadTool {
    /// Create a new file-read tool instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for FileReadTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for FileReadTool {
    fn name(&self) -> &'static str {
        "file_read"
    }

    fn description(&self) -> &'static str {
        "Read the contents of a file in the workspace"
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
        Box::pin(async move {
            let path = args
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

            let requested_path = Path::new(path);
            let full_path = if requested_path.is_absolute() {
                requested_path.to_path_buf()
            } else {
                ctx.workspace_dir.join(requested_path)
            };

            let std_file = match open_file_secure(&ctx.workspace_dir, requested_path, &full_path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(failed_tool_result(format!(
                        "Failed to resolve file path: {error}"
                    )));
                }
                Err(error) => {
                    return Ok(failed_tool_result(format!("Failed to read file: {error}")));
                }
            };
            let mut file = tokio::fs::File::from_std(std_file);

            // Validate file properties on the opened handle to avoid path-race
            // checks against a potentially swapped pathname.
            let metadata = match file.metadata().await {
                Ok(metadata) => metadata,
                Err(e) => {
                    return Ok(failed_tool_result(format!(
                        "Failed to read file metadata: {e}"
                    )));
                }
            };
            if metadata.file_type().is_symlink() {
                return Ok(failed_tool_result("Refusing to read through symlink"));
            }
            if !metadata.is_file() {
                return Ok(failed_tool_result(
                    "Failed to read file: not a regular file",
                ));
            }
            if metadata.len() > MAX_FILE_SIZE {
                return Ok(failed_tool_result(format!(
                    "File too large: {} bytes (limit: {MAX_FILE_SIZE} bytes)",
                    metadata.len()
                )));
            }
            if has_multiple_hard_links(&metadata) {
                return Ok(failed_tool_result(
                    "Refusing to read file with multiple hard links",
                ));
            }

            let Ok(capacity) = usize::try_from(metadata.len()) else {
                return Ok(failed_tool_result(
                    "File too large for platform pointer width",
                ));
            };
            let mut bytes = Vec::with_capacity(capacity);
            if let Err(e) = file.read_to_end(&mut bytes).await {
                return Ok(failed_tool_result(format!("Failed to read file: {e}")));
            }

            match String::from_utf8(bytes) {
                Ok(contents) => Ok(ToolResult {
                    success: true,
                    output: contents,
                    error: None,

                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                }),
                Err(_) => Ok(failed_tool_result(
                    "Failed to read file: file is not valid UTF-8",
                )),
            }
        })
    }
}

fn open_file_secure(
    workspace_dir: &Path,
    requested_path: &Path,
    full_path: &Path,
) -> std::io::Result<std::fs::File> {
    #[cfg(unix)]
    {
        open_file_secure_unix(workspace_dir, requested_path, full_path)
    }
    #[cfg(windows)]
    {
        let _ = workspace_dir;
        let _ = requested_path;
        return open_file_secure_windows(full_path);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = workspace_dir;
        let _ = requested_path;
        std::fs::File::open(full_path)
    }
}

#[cfg(unix)]
fn open_file_secure_unix(
    workspace_dir: &Path,
    requested_path: &Path,
    full_path: &Path,
) -> std::io::Result<std::fs::File> {
    if requested_path.is_absolute() {
        return open_absolute_nofollow_unix(full_path);
    }
    open_workspace_relative_nofollow_unix(workspace_dir, requested_path)
}

#[cfg(unix)]
fn open_absolute_nofollow_unix(full_path: &Path) -> std::io::Result<std::fs::File> {
    use rustix::fs::{CWD, Mode, OFlags, openat};
    match openat(
        CWD,
        full_path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    ) {
        Ok(fd) => Ok(std::fs::File::from(fd)),
        Err(error) if error == rustix::io::Errno::LOOP => Err(std::io::Error::other(format!(
            "Refusing to read through symlink: {}",
            full_path.display()
        ))),
        Err(error) => Err(std::io::Error::from_raw_os_error(error.raw_os_error())),
    }
}

#[cfg(unix)]
fn open_workspace_relative_nofollow_unix(
    workspace_dir: &Path,
    requested_path: &Path,
) -> std::io::Result<std::fs::File> {
    use std::ffi::OsString;

    use rustix::fs::{CWD, Mode, OFlags, openat};

    let mut components = Vec::<OsString>::new();
    for component in requested_path.components() {
        match component {
            Component::Normal(segment) => components.push(segment.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "parent path traversal is not allowed",
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "absolute path is not allowed in workspace-relative open",
                ));
            }
        }
    }

    if components.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty path is not allowed",
        ));
    }

    let mut dir_fd = openat(
        CWD,
        workspace_dir,
        OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))?;

    for (index, component) in components.iter().enumerate() {
        let is_last = index + 1 == components.len();
        let flags = if is_last {
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW
        } else {
            OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW
        };

        match openat(&dir_fd, component.as_os_str(), flags, Mode::empty()) {
            Ok(fd) if is_last => return Ok(std::fs::File::from(fd)),
            Ok(fd) => {
                dir_fd = fd;
            }
            Err(error) if error == rustix::io::Errno::LOOP => {
                return Err(std::io::Error::other(format!(
                    "Refusing to traverse symlink in path: {}",
                    requested_path.display()
                )));
            }
            Err(error) => {
                return Err(std::io::Error::from_raw_os_error(error.raw_os_error()));
            }
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "missing file path component",
    ))
}

#[cfg(windows)]
fn open_file_secure_windows(full_path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;

    // Open reparse points directly so symlink/junction targets are not
    // implicitly followed at open time.
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(full_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tools::middleware::ExecutionContext;
    use crate::core::tools::schema_helpers::test_security_policy;

    #[test]
    fn file_read_name() {
        let tool = FileReadTool::new();
        assert_eq!(tool.name(), "file_read");
    }

    #[test]
    fn file_read_schema_has_path() {
        let tool = FileReadTool::new();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["path"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&json!("path"))
        );
    }

    #[tokio::test]
    async fn file_read_existing_file() {
        let dir = std::env::temp_dir().join("asterel_test_file_read");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("test.txt"), "hello world")
            .await
            .unwrap();

        let tool = FileReadTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(dir.clone()));
        let result = tool
            .execute(json!({"path": "test.txt"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output, "hello world");
        assert!(result.error.is_none());

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_read_nonexistent_file() {
        let dir = std::env::temp_dir().join("asterel_test_file_read_missing");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let tool = FileReadTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(dir.clone()));
        let result = tool
            .execute(json!({"path": "nope.txt"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("Failed to resolve"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_read_missing_path_param() {
        let tool = FileReadTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(std::env::temp_dir()));
        let result = tool.execute(json!({}), &ctx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn file_read_empty_file() {
        let dir = std::env::temp_dir().join("asterel_test_file_read_empty");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("empty.txt"), "").await.unwrap();

        let tool = FileReadTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(dir.clone()));
        let result = tool
            .execute(json!({"path": "empty.txt"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output, "");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_read_nested_path() {
        let dir = std::env::temp_dir().join("asterel_test_file_read_nested");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(dir.join("sub/dir"))
            .await
            .unwrap();
        tokio::fs::write(dir.join("sub/dir/deep.txt"), "deep content")
            .await
            .unwrap();

        let tool = FileReadTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(dir.clone()));
        let result = tool
            .execute(json!({"path": "sub/dir/deep.txt"}), &ctx)
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output, "deep content");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn file_read_rejects_oversized_file() {
        let dir = std::env::temp_dir().join("asterel_test_file_read_large");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        // Create a file just over 10 MB
        let big = vec![b'x'; 10 * 1024 * 1024 + 1];
        tokio::fs::write(dir.join("huge.bin"), &big).await.unwrap();

        let tool = FileReadTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(dir.clone()));
        let result = tool
            .execute(json!({"path": "huge.bin"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("File too large"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn file_read_rejects_hard_linked_file() {
        let root = std::env::temp_dir().join("asterel_test_file_read_hardlink");
        let workspace = root.join("workspace");
        let outside = root.join("outside");
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::create_dir_all(&outside).await.unwrap();

        let outside_file = outside.join("secret.txt");
        tokio::fs::write(&outside_file, "do-not-read")
            .await
            .unwrap();
        tokio::fs::hard_link(&outside_file, workspace.join("leak.txt"))
            .await
            .unwrap();

        let tool = FileReadTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(workspace));
        let result = tool
            .execute(json!({"path": "leak.txt"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|err| err.contains("multiple hard links"))
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn file_read_rejects_symlinked_target() {
        let root = std::env::temp_dir().join("asterel_test_file_read_symlink");
        let workspace = root.join("workspace");
        let outside = root.join("outside");
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        tokio::fs::create_dir_all(&outside).await.unwrap();

        let outside_file = outside.join("secret.txt");
        tokio::fs::write(&outside_file, "do-not-read")
            .await
            .unwrap();
        tokio::fs::symlink(&outside_file, workspace.join("link.txt"))
            .await
            .unwrap();

        let tool = FileReadTool::new();
        let ctx = ExecutionContext::test_default(test_security_policy(workspace));
        let result = tool
            .execute(json!({"path": "link.txt"}), &ctx)
            .await
            .unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|err| err.contains("symlink"))
        );

        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
