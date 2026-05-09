//! Open-skills repository sync: clones/pulls the community skill
//! repository with periodic refresh (weekly by default).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use directories::UserDirs;

use crate::security::{ProcessSpawnClass, SecurityPolicy, enforce_spawn_policy};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct OpenSkillsCacheState {
    pub(super) enabled: bool,
    pub(super) repo_dir: Option<PathBuf>,
    pub(super) sync_required: bool,
}

/// Git URL for the community open-skills repository.
pub(super) const OPEN_SKILLS_REPO_URL: &str = "https://github.com/besoeasy/open-skills";
/// Marker file name used to track the last sync timestamp.
pub(super) const OPEN_SKILLS_SYNC_MARKER: &str = ".asterel-open-skills-sync";
/// Minimum interval between open-skills syncs (7 days).
pub(super) const OPEN_SKILLS_SYNC_INTERVAL_SECS: u64 = 60 * 60 * 24 * 7;

/// Returns `true` if open-skills loading is enabled via the
/// `ASTEREL_OPEN_SKILLS_ENABLED` environment variable.
pub(super) fn open_skills_enabled() -> bool {
    if let Ok(raw) = std::env::var("ASTEREL_OPEN_SKILLS_ENABLED") {
        let value = raw.trim().to_ascii_lowercase();
        return matches!(value.as_str(), "1" | "true" | "on" | "yes");
    }

    false
}

/// Resolve the open-skills directory from the environment or the
/// default `~/open-skills` path.
pub(super) fn resolve_open_skills_dir() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ASTEREL_OPEN_SKILLS_DIR") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }

    UserDirs::new().map(|dirs| dirs.home_dir().join("open-skills"))
}

/// Clone or pull the open-skills repository if enabled, returning
/// the local path.
pub(super) fn ensure_open_skills_repo(security: &SecurityPolicy) -> Option<PathBuf> {
    if !open_skills_enabled() {
        return None;
    }

    tracing::info!("open-skills loading enabled via ASTEREL_OPEN_SKILLS_ENABLED");

    let repo_dir = resolve_open_skills_dir()?;

    if !repo_dir.exists() {
        if !clone_open_skills_repo(&repo_dir, security) {
            return None;
        }
        if let Err(error) = mark_open_skills_synced(&repo_dir) {
            tracing::warn!(error = %error, repo_dir = %repo_dir.display(), "failed to write open-skills sync marker after clone");
        }
        return Some(repo_dir);
    }

    if should_sync_open_skills(&repo_dir) {
        if pull_open_skills_repo(&repo_dir, security) {
            if let Err(error) = mark_open_skills_synced(&repo_dir) {
                tracing::warn!(error = %error, repo_dir = %repo_dir.display(), "failed to update open-skills sync marker after pull");
            }
        } else {
            tracing::warn!(
                "open-skills update failed; using local copy from {}",
                repo_dir.display()
            );
        }
    }

    Some(repo_dir)
}

pub(super) fn open_skills_cache_state() -> OpenSkillsCacheState {
    let enabled = open_skills_enabled();
    let repo_dir = enabled.then(resolve_open_skills_dir).flatten();
    let sync_required = repo_dir
        .as_deref()
        .is_some_and(|repo_dir| !repo_dir.exists() || should_sync_open_skills(repo_dir));

    OpenSkillsCacheState {
        enabled,
        repo_dir,
        sync_required,
    }
}

fn clone_open_skills_repo(repo_dir: &Path, security: &SecurityPolicy) -> bool {
    if let Some(parent) = repo_dir.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(
            "failed to create open-skills parent directory {}: {err}",
            parent.display()
        );
        return false;
    }

    if let Err(err) = enforce_spawn_policy(
        security,
        "git",
        "plugins_skills_open_clone",
        ProcessSpawnClass::ExternalConnector,
    ) {
        tracing::warn!("failed to clone open-skills: {err}");
        return false;
    }

    let output = Command::new("git")
        .args(["clone", "--depth", "1", OPEN_SKILLS_REPO_URL])
        .arg(repo_dir)
        .output();

    match output {
        Ok(result) if result.status.success() => {
            tracing::info!("initialized open-skills at {}", repo_dir.display());
            true
        }
        Ok(result) => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            tracing::warn!("failed to clone open-skills: {stderr}");
            false
        }
        Err(err) => {
            tracing::warn!("failed to run git clone for open-skills: {err}");
            false
        }
    }
}

fn pull_open_skills_repo(repo_dir: &Path, security: &SecurityPolicy) -> bool {
    // If user points to a non-git directory via env var, keep using it without pulling.
    if !repo_dir.join(".git").exists() {
        return true;
    }

    if let Err(err) = enforce_spawn_policy(
        security,
        "git",
        "plugins_skills_open_pull",
        ProcessSpawnClass::ExternalConnector,
    ) {
        tracing::warn!("failed to pull open-skills updates: {err}");
        return false;
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["pull", "--ff-only"])
        .output();

    match output {
        Ok(result) if result.status.success() => true,
        Ok(result) => {
            let stderr = String::from_utf8_lossy(&result.stderr);
            tracing::warn!("failed to pull open-skills updates: {stderr}");
            false
        }
        Err(err) => {
            tracing::warn!("failed to run git pull for open-skills: {err}");
            false
        }
    }
}

fn should_sync_open_skills(repo_dir: &Path) -> bool {
    let marker = repo_dir.join(OPEN_SKILLS_SYNC_MARKER);
    let Ok(metadata) = std::fs::metadata(marker) else {
        return true;
    };
    let Ok(modified_at) = metadata.modified() else {
        return true;
    };
    let Ok(age) = SystemTime::now().duration_since(modified_at) else {
        return true;
    };

    age >= Duration::from_secs(OPEN_SKILLS_SYNC_INTERVAL_SECS)
}

fn mark_open_skills_synced(repo_dir: &Path) -> Result<()> {
    std::fs::write(repo_dir.join(OPEN_SKILLS_SYNC_MARKER), b"synced")?;
    Ok(())
}
