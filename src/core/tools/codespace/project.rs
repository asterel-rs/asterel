//! Codespace project lifecycle — creation, persistence, listing, and deletion.
//!
//! # Directory layout
//!
//! Each project lives at `<codespace_root>/<project_name>/` and contains:
//!
//! ```text
//! <project_name>/
//!   PROJECT.toml          — TOML-serialized CodespaceProject metadata
//!   src/                  — source files
//!   tests/                — test files
//!   .asterel-tmp/     — isolated TMPDIR for sandboxed execution (0700)
//! ```
//!
//! A directory is recognized as a project if and only if it contains a
//! `PROJECT.toml` file. The file is the single source of truth for metadata
//! including the last test result, promotion status, and configured commands.
//!
//! # Validation
//!
//! `validate_project_name` enforces: non-empty, not `.` or `..`, no `/`,
//! `\`, or NUL bytes, no leading `.`, and at most 64 characters. This
//! prevents path traversal in all file operations derived from the name.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use tokio::fs;
use tracing::debug;

use super::types::CodespaceProject;
use crate::config::schema::CodespaceConfig;

/// Resolve the codespace root directory under workspace.
pub(super) fn codespace_root(workspace_dir: &Path, config: &CodespaceConfig) -> PathBuf {
    workspace_dir.join(&config.root_dir)
}

/// Resolve and validate a project directory path.
///
/// # Errors
///
/// Returns an error when the project name fails validation.
pub(super) fn project_dir(
    workspace_dir: &Path,
    config: &CodespaceConfig,
    name: &str,
) -> Result<PathBuf> {
    validate_project_name(name)?;
    Ok(codespace_root(workspace_dir, config).join(name))
}

/// Create a new codespace project with initial directory structure.
///
/// # Errors
///
/// Returns an error when the project already exists, the project limit
/// is reached, or directory creation fails.
pub(super) async fn create_project(
    workspace_dir: &Path,
    config: &CodespaceConfig,
    name: &str,
    language: &str,
    test_command: Option<String>,
    entry_point: Option<String>,
) -> Result<CodespaceProject> {
    validate_project_name(name)?;

    if !config
        .allowed_languages
        .iter()
        .any(|l| l.eq_ignore_ascii_case(language))
    {
        bail!(
            "Language '{language}' is not allowed. Allowed: {}",
            config.allowed_languages.join(", ")
        );
    }

    let root = codespace_root(workspace_dir, config);
    let proj_dir = root.join(name);

    if proj_dir.exists() {
        bail!("Project '{name}' already exists");
    }

    let existing = count_projects(&root).await;
    if existing >= config.max_projects {
        bail!(
            "Project limit reached ({}/{})",
            existing,
            config.max_projects
        );
    }

    fs::create_dir_all(proj_dir.join("src"))
        .await
        .with_context(|| format!("Failed to create project src dir: {name}"))?;
    fs::create_dir_all(proj_dir.join("tests"))
        .await
        .with_context(|| format!("Failed to create project tests dir: {name}"))?;

    // Isolated TMPDIR for sandboxed execution.
    let tmp_dir = proj_dir.join(".asterel-tmp");
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&tmp_dir)
            .ok();
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(&tmp_dir).ok();
    }

    let project = CodespaceProject {
        name: name.to_string(),
        language: language.to_string(),
        root: proj_dir.clone(),
        created_at: Utc::now(),
        test_command,
        entry_point,
        promoted: false,
        last_test_result: None,
    };

    save_project(&project).await?;
    debug!(project = name, "Created codespace project");

    Ok(project)
}

/// List all projects in the codespace directory.
///
/// # Errors
///
/// Returns an error when the codespace root cannot be read.
pub(super) async fn list_projects(
    workspace_dir: &Path,
    config: &CodespaceConfig,
) -> Result<Vec<String>> {
    let root = codespace_root(workspace_dir, config);
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut names = Vec::new();
    let mut entries = fs::read_dir(&root)
        .await
        .with_context(|| format!("Failed to read codespace root: {}", root.display()))?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_dir()
            && path.join("PROJECT.toml").exists()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            names.push(name.to_string());
        }
    }

    names.sort();
    Ok(names)
}

/// Load a project from its PROJECT.toml.
///
/// # Errors
///
/// Returns an error when the file cannot be read or parsed.
pub(super) async fn load_project(project_dir: &Path) -> Result<CodespaceProject> {
    let toml_path = project_dir.join("PROJECT.toml");
    let content = fs::read_to_string(&toml_path)
        .await
        .with_context(|| format!("Failed to read {}", toml_path.display()))?;
    let project: CodespaceProject =
        toml::from_str(&content).with_context(|| "Failed to parse PROJECT.toml")?;
    Ok(project)
}

/// Save project metadata to PROJECT.toml.
///
/// # Errors
///
/// Returns an error when the file cannot be written.
pub(super) async fn save_project(project: &CodespaceProject) -> Result<()> {
    let toml_path = project.root.join("PROJECT.toml");
    let content = toml::to_string_pretty(project).context("Failed to serialize PROJECT.toml")?;
    fs::write(&toml_path, content)
        .await
        .with_context(|| format!("Failed to write {}", toml_path.display()))?;
    Ok(())
}

/// Delete a project and its entire directory tree.
///
/// # Errors
///
/// Returns an error when the directory cannot be removed.
pub(super) async fn delete_project(
    workspace_dir: &Path,
    config: &CodespaceConfig,
    name: &str,
) -> Result<()> {
    let dir = project_dir(workspace_dir, config, name)?;
    if !dir.exists() {
        bail!("Project '{name}' does not exist");
    }
    fs::remove_dir_all(&dir)
        .await
        .with_context(|| format!("Failed to delete project '{name}'"))?;
    debug!(project = name, "Deleted codespace project");
    Ok(())
}

/// Compute total disk usage of a project directory in bytes.
///
/// # Errors
///
/// Returns an error when directory traversal fails.
pub(super) async fn codespace_size_bytes(dir: &Path) -> Result<u64> {
    let mut total: u64 = 0;
    let mut stack = vec![dir.to_path_buf()];

    while let Some(current) = stack.pop() {
        let mut entries = fs::read_dir(&current).await?;
        while let Some(entry) = entries.next_entry().await? {
            let meta = entry.metadata().await?;
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }

    Ok(total)
}

/// Validate a project name: reject empty, traversal, separators, NUL.
fn validate_project_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        bail!("Project name cannot be empty");
    }
    if trimmed == "." || trimmed == ".." {
        bail!("Project name cannot be '.' or '..'");
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains('\0') {
        bail!(
            "Project name '{trimmed}' contains invalid characters: '/', '\\', and NUL are not allowed"
        );
    }
    if trimmed.starts_with('.') {
        bail!("Project name cannot start with '.'");
    }
    if trimmed.len() > 64 {
        bail!("Project name too long (max 64 characters)");
    }
    Ok(())
}

async fn count_projects(root: &Path) -> usize {
    let Ok(mut entries) = fs::read_dir(root).await else {
        return 0;
    };
    let mut count = 0;
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.path().join("PROJECT.toml").exists() {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn test_config() -> CodespaceConfig {
        CodespaceConfig {
            enabled: true,
            max_projects: 3,
            ..CodespaceConfig::default()
        }
    }

    #[test]
    fn validate_rejects_empty() {
        assert!(validate_project_name("").is_err());
        assert!(validate_project_name("  ").is_err());
    }

    #[test]
    fn validate_rejects_traversal() {
        assert!(validate_project_name("..").is_err());
        assert!(validate_project_name(".").is_err());
        assert!(validate_project_name("foo/bar").is_err());
        assert!(validate_project_name("foo\\bar").is_err());
    }

    #[test]
    fn validate_rejects_hidden() {
        assert!(validate_project_name(".hidden").is_err());
    }

    #[test]
    fn validate_rejects_long_names() {
        let long = "a".repeat(65);
        assert!(validate_project_name(&long).is_err());
    }

    #[test]
    fn validate_accepts_valid_names() {
        assert!(validate_project_name("my-project").is_ok());
        assert!(validate_project_name("data_cleaner_v2").is_ok());
    }

    #[tokio::test]
    async fn create_and_load_project() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config();

        let project = create_project(tmp.path(), &cfg, "demo", "python", None, None)
            .await
            .unwrap();
        assert_eq!(project.name, "demo");
        assert_eq!(project.language, "python");
        assert!(!project.promoted);

        let loaded = load_project(&project.root).await.unwrap();
        assert_eq!(loaded.name, "demo");
    }

    #[tokio::test]
    async fn create_rejects_duplicate() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config();

        create_project(tmp.path(), &cfg, "dup", "python", None, None)
            .await
            .unwrap();
        let err = create_project(tmp.path(), &cfg, "dup", "python", None, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[tokio::test]
    async fn create_rejects_disallowed_language() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config();

        let err = create_project(tmp.path(), &cfg, "proj", "cobol", None, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not allowed"));
    }

    #[tokio::test]
    async fn create_enforces_project_limit() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(); // max_projects = 3

        for i in 0..3 {
            create_project(tmp.path(), &cfg, &format!("p{i}"), "python", None, None)
                .await
                .unwrap();
        }
        let err = create_project(tmp.path(), &cfg, "p3", "python", None, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("limit reached"));
    }

    #[tokio::test]
    async fn list_projects_returns_sorted_names() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config();

        create_project(tmp.path(), &cfg, "beta", "python", None, None)
            .await
            .unwrap();
        create_project(tmp.path(), &cfg, "alpha", "bash", None, None)
            .await
            .unwrap();

        let names = list_projects(tmp.path(), &cfg).await.unwrap();
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[tokio::test]
    async fn delete_project_removes_dir() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config();

        let project = create_project(tmp.path(), &cfg, "gone", "python", None, None)
            .await
            .unwrap();
        assert!(project.root.exists());

        delete_project(tmp.path(), &cfg, "gone").await.unwrap();
        assert!(!project.root.exists());
    }

    #[tokio::test]
    async fn codespace_size_bytes_counts_files() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config();

        let project = create_project(tmp.path(), &cfg, "sized", "python", None, None)
            .await
            .unwrap();
        fs::write(project.root.join("src/main.py"), "x".repeat(100))
            .await
            .unwrap();

        let size = codespace_size_bytes(&project.root).await.unwrap();
        assert!(size >= 100);
    }
}
