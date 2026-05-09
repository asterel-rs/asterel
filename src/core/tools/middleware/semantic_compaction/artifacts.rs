#[cfg(unix)]
use std::io::Write as _;
#[cfg(unix)]
use std::os::fd::OwnedFd;
#[cfg(unix)]
use std::path::Component;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use uuid::Uuid;

use crate::core::tools::traits::ToolResultTextField;

const SEMANTIC_ARTIFACT_DIR: &str = ".asterel/artifacts/tool-output/semantic";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedSemanticArtifact {
    pub key: String,
    pub path: PathBuf,
}

pub async fn persist_semantic_artifact(
    workspace_dir: &Path,
    tool_name: &str,
    output_kind: &str,
    field: ToolResultTextField,
    raw_text: &str,
) -> anyhow::Result<PersistedSemanticArtifact> {
    let relative_key = build_semantic_artifact_key(tool_name, output_kind, field, raw_text);
    let path = persist_semantic_artifact_secure(workspace_dir, &relative_key, raw_text).await?;

    Ok(PersistedSemanticArtifact {
        key: relative_key,
        path,
    })
}

#[cfg(not(unix))]
async fn persist_semantic_artifact_secure(
    _workspace_dir: &Path,
    _relative_key: &str,
    _raw_text: &str,
) -> anyhow::Result<PathBuf> {
    tokio::task::spawn_blocking(|| -> anyhow::Result<PathBuf> {
        bail!("semantic artifact persistence requires fd-relative filesystem support")
    })
    .await
    .context("join semantic artifact writer")?
}

#[cfg(unix)]
async fn persist_semantic_artifact_secure(
    workspace_dir: &Path,
    relative_key: &str,
    raw_text: &str,
) -> anyhow::Result<PathBuf> {
    let workspace_dir = workspace_dir.to_path_buf();
    let relative_key = relative_key.to_string();
    let raw_text = raw_text.to_string();
    tokio::task::spawn_blocking(move || {
        persist_semantic_artifact_secure_unix(&workspace_dir, &relative_key, &raw_text)
    })
    .await
    .context("join semantic artifact writer")?
}

#[cfg(unix)]
fn persist_semantic_artifact_secure_unix(
    workspace_dir: &Path,
    relative_key: &str,
    raw_text: &str,
) -> anyhow::Result<PathBuf> {
    let workspace_fd = rustix::fs::open(
        workspace_dir,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .with_context(|| {
        format!(
            "open semantic artifact workspace {}",
            workspace_dir.display()
        )
    })?;
    let root_fd = ensure_secure_relative_dir_fd(
        &workspace_fd,
        Path::new(SEMANTIC_ARTIFACT_DIR),
        "artifact root",
    )?;
    let relative_path = Path::new(relative_key);
    let parent_relative = relative_path.parent().unwrap_or_else(|| Path::new(""));
    let parent_fd = ensure_secure_relative_dir_fd(&root_fd, parent_relative, "artifact directory")?;
    let file_name = normal_file_name(relative_path)?;

    reject_symlink_target_at(&parent_fd, file_name)?;

    let temp_name = format!(".semantic-artifact-tmp-{}", Uuid::new_v4());
    let temp_fd = rustix::fs::openat(
        &parent_fd,
        temp_name.as_str(),
        rustix::fs::OFlags::WRONLY
            | rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::EXCL
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::from_raw_mode(0o600),
    )
    .with_context(|| format!("create semantic artifact temp file {temp_name}"))?;
    let mut temp_file = std::fs::File::from(temp_fd);
    if let Err(error) = temp_file.write_all(raw_text.as_bytes()) {
        drop(temp_file);
        let _ = rustix::fs::unlinkat(&parent_fd, temp_name.as_str(), rustix::fs::AtFlags::empty());
        return Err(error)
            .with_context(|| format!("write semantic artifact temp file {temp_name}"));
    }
    if let Err(error) = temp_file.sync_all() {
        drop(temp_file);
        let _ = rustix::fs::unlinkat(&parent_fd, temp_name.as_str(), rustix::fs::AtFlags::empty());
        return Err(error).with_context(|| format!("sync semantic artifact temp file {temp_name}"));
    }
    drop(temp_file);

    if let Err(error) = rustix::fs::renameat(&parent_fd, temp_name.as_str(), &parent_fd, file_name)
    {
        let _ = rustix::fs::unlinkat(&parent_fd, temp_name.as_str(), rustix::fs::AtFlags::empty());
        return Err(error)
            .with_context(|| format!("commit semantic artifact {}", relative_path.display()));
    }
    sync_directory_fd(&parent_fd, "semantic artifact directory")?;

    Ok(workspace_dir
        .join(SEMANTIC_ARTIFACT_DIR)
        .join(relative_path))
}

#[cfg(unix)]
fn sync_directory_fd(fd: &OwnedFd, label: &str) -> anyhow::Result<()> {
    let sync_fd = rustix::io::fcntl_dupfd_cloexec(fd, 3)
        .with_context(|| format!("duplicate {label} descriptor for sync"))?;
    let dir = std::fs::File::from(sync_fd);
    dir.sync_all().with_context(|| format!("sync {label}"))
}

fn build_semantic_artifact_key(
    tool_name: &str,
    output_kind: &str,
    field: ToolResultTextField,
    raw_text: &str,
) -> String {
    let digest = semantic_artifact_digest(raw_text);
    let tool_component = sanitize_key_component(tool_name);
    let output_kind_component = sanitize_key_component(output_kind);
    format!(
        "{tool_component}/{output_kind_component}/{}-{digest}.txt",
        field.as_str()
    )
}

fn semantic_artifact_digest(raw_text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_text.as_bytes());
    hex::encode(hasher.finalize())
}

fn sanitize_key_component(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "_".to_string()
    } else {
        sanitized
    }
}

#[cfg(unix)]
fn ensure_secure_relative_dir_fd(
    base_fd: &OwnedFd,
    relative: &Path,
    label: &str,
) -> anyhow::Result<OwnedFd> {
    let mut current = rustix::io::fcntl_dupfd_cloexec(base_fd, 3)
        .with_context(|| format!("duplicate semantic {label} base descriptor"))?;
    for component in relative.components() {
        let Component::Normal(part) = component else {
            bail!(
                "semantic {label} contains unsupported path component: {}",
                relative.display()
            );
        };
        current = open_or_create_child_dir(&current, part, label)?;
    }
    Ok(current)
}

#[cfg(unix)]
fn open_or_create_child_dir(
    parent_fd: &OwnedFd,
    name: &std::ffi::OsStr,
    label: &str,
) -> anyhow::Result<OwnedFd> {
    let name_display = Path::new(name).display().to_string();
    match open_child_dir(parent_fd, name) {
        Ok(fd) => Ok(fd),
        Err(error) if error == rustix::io::Errno::NOENT => {
            match rustix::fs::mkdirat(parent_fd, name, rustix::fs::Mode::from_raw_mode(0o700)) {
                Ok(()) => {}
                Err(error) if error == rustix::io::Errno::EXIST => {}
                Err(error) if error == rustix::io::Errno::LOOP => {
                    bail!("refusing to use semantic {label} symlink: {name_display}");
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("create semantic {label} component {name_display}")
                    });
                }
            }
            open_child_dir(parent_fd, name)
                .with_context(|| format!("open semantic {label} component {name_display}"))
        }
        Err(error) if error == rustix::io::Errno::LOOP => {
            bail!("refusing to use semantic {label} symlink: {name_display}")
        }
        Err(error) => {
            Err(error).with_context(|| format!("open semantic {label} component {name_display}"))
        }
    }
}

#[cfg(unix)]
fn open_child_dir(parent_fd: &OwnedFd, name: &std::ffi::OsStr) -> rustix::io::Result<OwnedFd> {
    rustix::fs::openat(
        parent_fd,
        name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
}

#[cfg(unix)]
fn normal_file_name(path: &Path) -> anyhow::Result<&std::ffi::OsStr> {
    let mut components = path.components();
    let Some(last) = components.next_back() else {
        bail!("semantic artifact path is empty");
    };
    if components.any(|component| !matches!(component, Component::Normal(_))) {
        bail!(
            "semantic artifact path contains unsupported component: {}",
            path.display()
        );
    }
    let Component::Normal(name) = last else {
        bail!(
            "semantic artifact filename contains unsupported component: {}",
            path.display()
        );
    };
    Ok(name)
}

#[cfg(unix)]
fn reject_symlink_target_at(parent_fd: &OwnedFd, name: &std::ffi::OsStr) -> anyhow::Result<()> {
    let name_display = Path::new(name).display().to_string();
    match rustix::fs::statat(parent_fd, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => {
            if rustix::fs::FileType::from_raw_mode(stat.st_mode).is_symlink() {
                bail!("refusing to write semantic artifact through symlink");
            }
            Ok(())
        }
        Err(error) if error == rustix::io::Errno::NOENT => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("inspect semantic artifact target {name_display}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tools::traits::ToolResultTextField;
    #[cfg(unix)]
    use tokio::fs;

    #[cfg(unix)]
    #[tokio::test]
    async fn persist_semantic_artifact_writes_in_workspace() {
        let root = tempfile::tempdir().unwrap();

        let artifact = persist_semantic_artifact(
            root.path(),
            "shell",
            "cargo-test",
            ToolResultTextField::Output,
            "raw output",
        )
        .await
        .unwrap();

        assert!(artifact.path.starts_with(root.path()));
        assert_eq!(
            fs::read_to_string(&artifact.path).await.unwrap(),
            "raw output"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn concurrent_semantic_artifact_writes_share_directory_creation() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().to_path_buf();
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(24));
        let mut tasks = tokio::task::JoinSet::new();

        for index in 0..24 {
            let root_path = root_path.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            tasks.spawn(async move {
                barrier.wait().await;
                persist_semantic_artifact(
                    &root_path,
                    "browser",
                    "snapshot",
                    ToolResultTextField::Output,
                    &format!("raw output {index}"),
                )
                .await
            });
        }

        while let Some(result) = tasks.join_next().await {
            let artifact = result.unwrap().unwrap();
            assert!(artifact.path.starts_with(root.path()));
            assert!(artifact.path.exists());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn empty_key_components_stay_relative() {
        let root = tempfile::tempdir().unwrap();

        let artifact = persist_semantic_artifact(
            root.path(),
            "",
            "",
            ToolResultTextField::Output,
            "raw output",
        )
        .await
        .unwrap();

        assert_eq!(
            artifact.key,
            format!("_/_/output-{}.txt", semantic_artifact_digest("raw output"))
        );
        assert!(artifact.path.starts_with(root.path()));
    }

    #[cfg(not(unix))]
    #[tokio::test]
    async fn non_unix_artifact_persistence_fails_closed() {
        let root = tempfile::tempdir().unwrap();

        let result = persist_semantic_artifact(
            root.path(),
            "shell",
            "cargo-test",
            ToolResultTextField::Output,
            "raw output",
        )
        .await;

        assert!(result.is_err());
        assert!(!root.path().join(SEMANTIC_ARTIFACT_DIR).exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlinked_artifact_root_without_outside_mutation() {
        use std::os::unix::fs::symlink;

        let base = tempfile::tempdir().unwrap();
        let workspace = base.path().join("workspace");
        let outside = base.path().join("outside");
        fs::create_dir_all(&workspace).await.unwrap();
        fs::create_dir_all(&outside).await.unwrap();
        symlink(&outside, workspace.join(".asterel")).unwrap();

        let result = persist_semantic_artifact(
            &workspace,
            "shell",
            "cargo test",
            ToolResultTextField::Output,
            "raw output",
        )
        .await;

        assert!(result.is_err(), "symlinked root should be rejected");
        assert!(!outside.join("artifacts").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlinked_nested_directory_without_outside_mutation() {
        use std::os::unix::fs::symlink;

        let base = tempfile::tempdir().unwrap();
        let workspace = base.path().join("workspace");
        let outside = base.path().join("outside");
        let semantic_root = workspace.join(SEMANTIC_ARTIFACT_DIR);
        fs::create_dir_all(semantic_root.parent().unwrap())
            .await
            .unwrap();
        fs::create_dir_all(&outside).await.unwrap();
        symlink(&outside, &semantic_root).unwrap();

        let result = persist_semantic_artifact(
            &workspace,
            "shell",
            "cargo test",
            ToolResultTextField::Output,
            "raw output",
        )
        .await;

        assert!(
            result.is_err(),
            "symlinked semantic root should be rejected"
        );
        assert!(!outside.join("shell").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlinked_tool_component_without_outside_mutation() {
        use std::os::unix::fs::symlink;

        let base = tempfile::tempdir().unwrap();
        let workspace = base.path().join("workspace");
        let outside = base.path().join("outside");
        let semantic_root = workspace.join(SEMANTIC_ARTIFACT_DIR);
        fs::create_dir_all(&semantic_root).await.unwrap();
        fs::create_dir_all(&outside).await.unwrap();
        symlink(&outside, semantic_root.join("shell")).unwrap();

        let result = persist_semantic_artifact(
            &workspace,
            "shell",
            "cargo test",
            ToolResultTextField::Output,
            "raw output",
        )
        .await;

        assert!(
            result.is_err(),
            "symlinked output directory should be rejected"
        );
        assert!(!outside.join("cargo_test").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlinked_output_kind_without_outside_mutation() {
        use std::os::unix::fs::symlink;

        let base = tempfile::tempdir().unwrap();
        let workspace = base.path().join("workspace");
        let outside = base.path().join("outside");
        let semantic_root = workspace.join(SEMANTIC_ARTIFACT_DIR);
        fs::create_dir_all(semantic_root.join("shell"))
            .await
            .unwrap();
        fs::create_dir_all(&outside).await.unwrap();
        symlink(&outside, semantic_root.join("shell/cargo_test")).unwrap();

        let result = persist_semantic_artifact(
            &workspace,
            "shell",
            "cargo test",
            ToolResultTextField::Output,
            "raw output",
        )
        .await;

        assert!(
            result.is_err(),
            "symlinked output directory should be rejected"
        );
        assert!(!outside.join("output").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlinked_final_target_without_outside_write() {
        use std::os::unix::fs::symlink;

        let base = tempfile::tempdir().unwrap();
        let workspace = base.path().join("workspace");
        let outside = base.path().join("outside");
        let outside_target = outside.join("leak.txt");
        let relative_key = build_semantic_artifact_key(
            "shell",
            "cargo test",
            ToolResultTextField::Output,
            "raw output",
        );
        let artifact_path = workspace.join(SEMANTIC_ARTIFACT_DIR).join(&relative_key);
        fs::create_dir_all(artifact_path.parent().unwrap())
            .await
            .unwrap();
        fs::create_dir_all(&outside).await.unwrap();
        symlink(&outside_target, &artifact_path).unwrap();

        let result = persist_semantic_artifact(
            &workspace,
            "shell",
            "cargo test",
            ToolResultTextField::Output,
            "raw output",
        )
        .await;

        assert!(
            result.is_err(),
            "symlinked artifact target should be rejected"
        );
        assert!(!outside_target.exists());
    }
}
