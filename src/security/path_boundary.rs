//! Shared root-bounded filesystem helpers for operator and plugin surfaces.
//!
//! These helpers canonicalize candidate paths and ensure they remain within a
//! declared root, preventing traversal and symlink escapes.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootBoundPathKind {
    Any,
    File,
    Directory,
}

/// Canonicalize `path` and ensure the resolved path remains inside `root`.
///
/// # Errors
///
/// Returns an error when canonicalization fails, the resolved path escapes the
/// root, or the resolved target is not of the expected kind.
pub fn canonicalize_path_within_root(
    path: &Path,
    root: &Path,
    expected: RootBoundPathKind,
) -> Result<PathBuf> {
    let canonical_root = std::fs::canonicalize(root)
        .with_context(|| format!("canonicalize root '{}'", root.display()))?;
    let canonical_path = std::fs::canonicalize(path)
        .with_context(|| format!("canonicalize path '{}'", path.display()))?;

    if !canonical_path.starts_with(&canonical_root) {
        bail!(
            "path '{}' resolves outside allowed root '{}'",
            canonical_path.display(),
            canonical_root.display()
        );
    }

    ensure_expected_kind(&canonical_path, expected)?;
    Ok(canonical_path)
}

/// Join `declared_path` under `root`, reject traversal tokens, and canonicalize
/// the result within the root.
///
/// # Errors
///
/// Returns an error when `declared_path` is absolute, traverses upward, escapes
/// the root after canonicalization, or is not of the expected kind.
pub fn resolve_relative_path_within_root(
    root: &Path,
    declared_path: &Path,
    expected: RootBoundPathKind,
) -> Result<PathBuf> {
    if declared_path.is_absolute() {
        bail!(
            "declared path '{}' must be relative to '{}'",
            declared_path.display(),
            root.display()
        );
    }

    if declared_path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        bail!(
            "declared path '{}' escapes allowed root '{}'",
            declared_path.display(),
            root.display()
        );
    }

    canonicalize_path_within_root(&root.join(declared_path), root, expected)
}

fn ensure_expected_kind(path: &Path, expected: RootBoundPathKind) -> Result<()> {
    match expected {
        RootBoundPathKind::Any => Ok(()),
        RootBoundPathKind::File if path.is_file() => Ok(()),
        RootBoundPathKind::Directory if path.is_dir() => Ok(()),
        RootBoundPathKind::File => bail!("path '{}' is not a file", path.display()),
        RootBoundPathKind::Directory => bail!("path '{}' is not a directory", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn canonicalize_path_within_root_blocks_symlink_escape() {
        let root = TempDir::new().expect("temp root");
        let outside = TempDir::new().expect("temp outside");
        let outside_file = outside.path().join("secret.txt");
        fs::write(&outside_file, "secret").expect("write outside file");
        let link = root.path().join("escape.txt");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside_file, &link).expect("create symlink");
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&outside_file, &link).expect("create symlink");

        let error = canonicalize_path_within_root(&link, root.path(), RootBoundPathKind::File)
            .expect_err("escape should be rejected");
        assert!(error.to_string().contains("outside allowed root"));
    }

    #[test]
    fn resolve_relative_path_within_root_rejects_parent_segments() {
        let root = TempDir::new().expect("temp root");

        let error = resolve_relative_path_within_root(
            root.path(),
            Path::new("../escape.txt"),
            RootBoundPathKind::File,
        )
        .expect_err("parent traversal should be rejected");
        assert!(error.to_string().contains("escapes allowed root"));
    }
}
