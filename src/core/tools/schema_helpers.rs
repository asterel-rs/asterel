//! Shared utility functions for tool implementations.
//!
//! Provides workspace path schema helpers, hard-link detection,
//! and convenience constructors for failed tool results.

#[cfg(test)]
use std::sync::Arc;

use serde_json::json;

use super::traits::ToolResult;
#[cfg(test)]
use crate::security::{AutonomyLevel, SecurityPolicy};

pub(super) fn workspace_path_property() -> serde_json::Value {
    json!({
        "type": "string",
        "description": "Relative path to the file within the workspace"
    })
}

#[cfg(unix)]
pub(super) fn has_multiple_hard_links(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() > 1
}

#[cfg(windows)]
pub(super) fn has_multiple_hard_links(_metadata: &std::fs::Metadata) -> bool {
    // Windows does not yet expose a stable hard-link count API in std.
    false
}

#[cfg(not(any(unix, windows)))]
pub(super) fn has_multiple_hard_links(_metadata: &std::fs::Metadata) -> bool {
    false
}

pub(super) fn failed_tool_result(message: impl Into<String>) -> ToolResult {
    ToolResult::failure(message)
}

#[cfg(test)]
pub(super) fn test_security_policy(workspace: std::path::PathBuf) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        workspace_dir: workspace,
        ..SecurityPolicy::default()
    })
}
