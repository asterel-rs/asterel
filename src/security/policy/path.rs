//! Filesystem path validation against the security policy.
//!
//! Blocks path traversal, null bytes, forbidden directories, and
//! enforces workspace-only access when configured.

use std::path::{Path, PathBuf};

use super::SecurityPolicy;

fn expand_tilde(path: &str, home: Option<&Path>) -> String {
    if let Some(stripped) = path.strip_prefix("~/")
        && let Some(home) = home
    {
        return home.join(stripped).to_string_lossy().to_string();
    }
    path.to_string()
}

impl SecurityPolicy {
    /// Check if a file path is allowed (no path traversal, within workspace)
    pub fn is_path_allowed(&self, path: &str) -> bool {
        // Block null bytes (can truncate paths in C-backed syscalls)
        if path.contains('\0') {
            return false;
        }

        // Block path traversal: check for ".." as a path component
        if Path::new(path)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return false;
        }

        // Block URL-encoded traversal attempts (e.g. %2e%2e, ..%2f)
        let lower = path.to_lowercase();
        if lower.contains("%2e%2e") || lower.contains("..%2f") || lower.contains("%2f..") {
            return false;
        }

        // Cache HOME once for all tilde expansions in this call
        let home = std::env::var("HOME").ok().map(PathBuf::from);
        let home_ref = home.as_deref();

        // Expand tilde for comparison
        let expanded = expand_tilde(path, home_ref);

        // Block absolute paths when workspace_only is set
        if self.workspace_only && Path::new(&expanded).is_absolute() {
            return false;
        }

        // Block forbidden paths using path-component-aware matching
        let expanded_path = Path::new(&expanded);
        for forbidden in &self.forbidden_paths {
            let forbidden_expanded = expand_tilde(forbidden, home_ref);
            let forbidden_path = Path::new(&forbidden_expanded);
            if expanded_path.starts_with(forbidden_path) {
                return false;
            }
        }

        true
    }

    /// Validate that a resolved path is still inside the workspace and not
    /// under a forbidden prefix.
    ///
    /// Call this AFTER joining `workspace_dir` + relative path and canonicalizing.
    #[must_use]
    pub fn is_path_allowed_resolved(&self, resolved: &Path) -> bool {
        // Always block resolved paths that fall under a forbidden path prefix,
        // regardless of workspace_only setting.
        if self.is_resolved_path_forbidden(resolved) {
            return false;
        }

        // When workspace_only is disabled, skip the workspace-root containment
        // check to allow intentional access to paths outside the workspace.
        if !self.workspace_only {
            return true;
        }

        // Must be under workspace_dir (prevents symlink escapes). If the
        // workspace root itself cannot be canonicalized, fail closed rather
        // than comparing a resolved target against an untrusted raw root.
        let Ok(workspace_root) = self.workspace_dir.canonicalize() else {
            return false;
        };
        resolved.starts_with(workspace_root)
    }

    /// Check if a resolved (absolute) path falls under any forbidden prefix.
    ///
    /// Paths that are inside the workspace directory are exempt from the
    /// forbidden-path check because they have already passed workspace
    /// containment validation.
    fn is_resolved_path_forbidden(&self, resolved: &Path) -> bool {
        if let Ok(workspace_root) = self.workspace_dir.canonicalize()
            && resolved.starts_with(&workspace_root)
        {
            return false;
        }

        let home = std::env::var("HOME").ok().map(PathBuf::from);
        let home_ref = home.as_deref();

        for forbidden in &self.forbidden_paths {
            let forbidden_expanded = expand_tilde(forbidden, home_ref);
            let forbidden_path = Path::new(&forbidden_expanded);
            if resolved.starts_with(forbidden_path) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::config::AutonomyConfig;

    fn policy_with(
        workspace: &Path,
        workspace_only: bool,
        forbidden_paths: Vec<String>,
    ) -> SecurityPolicy {
        SecurityPolicy {
            workspace_dir: workspace.to_path_buf(),
            workspace_only,
            forbidden_paths,
            ..SecurityPolicy::default()
        }
    }

    #[test]
    fn allows_normal_workspace_relative_paths() {
        let workspace = TempDir::new().expect("tempdir");
        let policy = policy_with(workspace.path(), true, vec![]);

        assert!(policy.is_path_allowed("src/main.rs"));
        assert!(policy.is_path_allowed("nested/dir/file.txt"));
    }

    #[test]
    fn blocks_parent_directory_traversal() {
        let workspace = TempDir::new().expect("tempdir");
        let policy = policy_with(workspace.path(), true, vec![]);

        assert!(!policy.is_path_allowed("../../etc/passwd"));
    }

    #[test]
    fn blocks_null_bytes() {
        let workspace = TempDir::new().expect("tempdir");
        let policy = policy_with(workspace.path(), true, vec![]);

        assert!(!policy.is_path_allowed("file\0.txt"));
    }

    #[test]
    fn blocks_url_encoded_traversal_variants() {
        let workspace = TempDir::new().expect("tempdir");
        let policy = policy_with(workspace.path(), true, vec![]);

        assert!(!policy.is_path_allowed("..%2f..%2fetc/passwd"));
        assert!(!policy.is_path_allowed("..%2F..%2Fetc/passwd"));
        assert!(!policy.is_path_allowed("%2e%2e"));
        assert!(!policy.is_path_allowed("%2E%2E"));
    }

    #[test]
    fn handles_tilde_paths_with_forbidden_prefix() {
        let workspace = TempDir::new().expect("tempdir");
        let policy = policy_with(workspace.path(), false, vec!["~/.ssh".to_string()]);

        assert!(!policy.is_path_allowed("~/.ssh/id_rsa"));
    }

    #[test]
    fn blocks_absolute_paths_when_workspace_only_enabled() {
        let workspace = TempDir::new().expect("tempdir");
        let policy = policy_with(workspace.path(), true, vec![]);

        assert!(!policy.is_path_allowed("/tmp/file with spaces.txt"));
    }

    #[test]
    fn allows_absolute_paths_when_workspace_only_disabled_and_not_forbidden() {
        let workspace = TempDir::new().expect("tempdir");
        let policy = policy_with(workspace.path(), false, vec!["/etc".to_string()]);

        assert!(policy.is_path_allowed("/my/project/data.txt"));
    }

    #[test]
    fn empty_path_and_space_paths_are_handled() {
        let workspace = TempDir::new().expect("tempdir");
        let policy = policy_with(workspace.path(), true, vec![]);

        assert!(policy.is_path_allowed(""));
        assert!(policy.is_path_allowed("folder with spaces/file name.txt"));
    }

    #[test]
    fn forbidden_matching_is_component_aware() {
        let workspace = TempDir::new().expect("tempdir");
        let policy = policy_with(workspace.path(), false, vec!["/etc".to_string()]);

        assert!(!policy.is_path_allowed("/etc/shadow"));
        assert!(policy.is_path_allowed("/my/etc/shadow"));
    }

    #[test]
    fn symbolic_link_escape_requires_resolved_path_check() {
        let workspace = TempDir::new().expect("tempdir");
        let policy = policy_with(workspace.path(), true, vec![]);

        assert!(policy.is_path_allowed("link_to_outside/secret.txt"));
    }

    #[test]
    fn default_autonomy_forbidden_paths_are_all_blocked() {
        let workspace = TempDir::new().expect("tempdir");
        let autonomy = AutonomyConfig {
            workspace_only: false,
            ..AutonomyConfig::default()
        };
        let policy = SecurityPolicy::from_config(&autonomy, workspace.path());

        for forbidden in &autonomy.forbidden_paths {
            let candidate = format!("{forbidden}/sensitive.txt");
            assert!(
                !policy.is_path_allowed(&candidate),
                "forbidden path should be blocked: {forbidden}"
            );
        }
    }

    #[test]
    fn forbidden_paths_resist_simple_case_and_encoding_variants() {
        let workspace = TempDir::new().expect("tempdir");
        let policy = policy_with(workspace.path(), false, vec!["/etc".to_string()]);

        assert!(!policy.is_path_allowed("/etc/shadow"));
        assert!(!policy.is_path_allowed("/etc/%2e%2e/shadow"));
    }

    #[test]
    fn resolved_path_inside_workspace_is_allowed() {
        let workspace = TempDir::new().expect("tempdir");
        let nested = workspace.path().join("src");
        fs::create_dir_all(&nested).expect("create nested dir");
        let file_path = nested.join("main.rs");
        fs::write(&file_path, "fn main() {}\n").expect("write file");

        let policy = policy_with(workspace.path(), true, vec![]);
        let resolved = file_path.canonicalize().expect("canonicalize inside path");
        assert!(policy.is_path_allowed_resolved(&resolved));
    }

    #[test]
    fn resolved_path_outside_workspace_is_blocked() {
        let workspace = TempDir::new().expect("tempdir");
        let outside_root = TempDir::new().expect("tempdir");
        let outside_file = outside_root.path().join("escape.txt");
        fs::write(&outside_file, "escape\n").expect("write outside file");

        let policy = policy_with(workspace.path(), true, vec![]);
        let resolved = outside_file
            .canonicalize()
            .expect("canonicalize outside path");
        assert!(!policy.is_path_allowed_resolved(&resolved));
    }

    #[test]
    fn resolved_path_check_fails_closed_when_workspace_root_is_missing() {
        let workspace = TempDir::new().expect("tempdir");
        let missing_workspace = workspace.path().join("missing");
        let outside_root = TempDir::new().expect("tempdir");
        let outside_file = outside_root.path().join("escape.txt");
        fs::write(&outside_file, "escape\n").expect("write outside file");

        let policy = policy_with(&missing_workspace, true, vec![]);
        let resolved = outside_file
            .canonicalize()
            .expect("canonicalize outside path");
        assert!(!policy.is_path_allowed_resolved(&resolved));
    }
}
