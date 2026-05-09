//! Directory-based skill discovery: scans workspace, extra dirs,
//! and open-skills paths for extension manifests with
//! deduplication and watch fingerprinting.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;

use super::super::{Skill, SkillCatalogEntry, SkillMetadata};
use super::open_skills::{open_skills_enabled, resolve_open_skills_dir};
use super::parse::{
    load_extension_skill, load_extension_skill_metadata, load_open_skill_md,
    load_open_skill_md_metadata,
};
use crate::config::{SkillSource, SkillsRuntimeConfig};
use crate::plugins::extensions::load_extension_runtime;
use crate::security::{RootBoundPathKind, canonicalize_path_within_root};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SkillWatchEntry {
    path: PathBuf,
    modified_nanos: u128,
    byte_len: u64,
}

/// Return skill source priority with deduplication and fallback
/// to the default ordering.
pub(super) fn normalized_skill_sources(skills_config: &SkillsRuntimeConfig) -> Vec<SkillSource> {
    let mut sources = Vec::new();
    for source in &skills_config.source_priority {
        if !sources.contains(source) {
            sources.push(*source);
        }
    }

    for source in [
        SkillSource::Workspace,
        SkillSource::ExtraDirs,
        SkillSource::OpenSkills,
    ] {
        if !sources.contains(&source) {
            sources.push(source);
        }
    }

    sources
}

/// Compute an FNV-1a fingerprint over all skill manifest files
/// for change detection.
pub(super) fn skills_watch_fingerprint_inner(
    workspace_dir: &Path,
    skills_config: &SkillsRuntimeConfig,
) -> u64 {
    let mut entries = Vec::new();
    for source in normalized_skill_sources(skills_config) {
        match source {
            SkillSource::Workspace => {
                append_skill_dir_watch_entries(&workspace_dir.join("skills"), &mut entries);
            }
            SkillSource::ExtraDirs => {
                for dir in resolve_extra_skill_dirs(workspace_dir, skills_config) {
                    append_skill_dir_watch_entries(&dir, &mut entries);
                }
            }
            SkillSource::OpenSkills => {
                if open_skills_enabled()
                    && let Some(open_skills_dir) = resolve_open_skills_dir()
                {
                    append_open_skills_watch_entries(&open_skills_dir, &mut entries);
                }
            }
        }
    }

    entries.sort();
    let mut hash = 14_695_981_039_346_656_037_u64;
    for entry in entries {
        hash = fnv1a64_update(hash, entry.path.to_string_lossy().as_bytes());
        hash = fnv1a64_update(hash, &entry.modified_nanos.to_le_bytes());
        hash = fnv1a64_update(hash, &entry.byte_len.to_le_bytes());
    }
    hash
}

fn fnv1a64_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(1_099_511_628_211_u64);
    }
    hash
}

pub(super) fn canonicalize_under_root(path: &Path, root: &Path) -> Result<PathBuf> {
    canonicalize_path_within_root(path, root, RootBoundPathKind::Any)
}

/// Returns `true` if `path` is a symbolic link.
pub(super) fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
}

fn append_skill_dir_watch_entries(skills_dir: &Path, entries: &mut Vec<SkillWatchEntry>) {
    if !skills_dir.exists() {
        return;
    }

    let Ok(read_dir) = std::fs::read_dir(skills_dir) else {
        return;
    };

    let mut skill_dirs: Vec<PathBuf> = read_dir
        .flatten()
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if file_type.is_symlink() || !file_type.is_dir() {
                return None;
            }
            let path = entry.path();
            canonicalize_under_root(&path, skills_dir).ok()
        })
        .collect();
    skill_dirs.sort();

    for dir in skill_dirs {
        let extension_path = dir.join("extension.toml");
        if !extension_path.exists() || is_symlink(&extension_path) {
            continue;
        }

        let Ok(safe_extension_path) = canonicalize_under_root(&extension_path, skills_dir) else {
            continue;
        };
        if let Some(watch_entry) = to_watch_entry(&safe_extension_path) {
            entries.push(watch_entry);
        }
        if let Ok(runtime) = load_extension_runtime(&safe_extension_path) {
            for body in runtime.bodies {
                if let Some(watch_entry) = to_watch_entry(&body.absolute_path) {
                    entries.push(watch_entry);
                }
            }
        }
    }
}

fn append_open_skills_watch_entries(repo_dir: &Path, entries: &mut Vec<SkillWatchEntry>) {
    if !repo_dir.exists() {
        return;
    }

    let Ok(read_dir) = std::fs::read_dir(repo_dir) else {
        return;
    };

    let mut markdown_files: Vec<PathBuf> = read_dir
        .flatten()
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if file_type.is_symlink() || !file_type.is_file() {
                return None;
            }
            let path = entry.path();
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                .then_some(path)
        })
        .filter(|path| {
            !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("README.md"))
        })
        .filter_map(|path| canonicalize_under_root(&path, repo_dir).ok())
        .collect();
    markdown_files.sort();

    for path in markdown_files {
        if let Some(watch_entry) = to_watch_entry(&path) {
            entries.push(watch_entry);
        }
    }
}

fn to_watch_entry(path: &Path) -> Option<SkillWatchEntry> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_nanos();

    Some(SkillWatchEntry {
        path: path.to_path_buf(),
        modified_nanos: modified,
        byte_len: metadata.len(),
    })
}

/// Deduplicate skills by name, keeping the first occurrence.
pub(super) fn dedupe_skills_by_name<T: SkillCatalogEntry>(skills: Vec<T>) -> Vec<T> {
    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for skill in skills {
        if seen.insert(skill.name().to_string()) {
            deduped.push(skill);
        }
    }
    deduped
}

/// Load skills from the workspace `skills/` directory.
pub(super) fn load_workspace_skills(
    workspace_dir: &Path,
    enforce_requirements: bool,
) -> Vec<Skill> {
    let skills_dir = workspace_dir.join("skills");
    load_records_from_directory(&skills_dir, enforce_requirements, load_extension_skill)
}

/// Load skill metadata from the workspace `skills/` directory.
pub(super) fn load_workspace_skill_metadata(
    workspace_dir: &Path,
    enforce_requirements: bool,
) -> Vec<SkillMetadata> {
    let skills_dir = workspace_dir.join("skills");
    load_records_from_directory(
        &skills_dir,
        enforce_requirements,
        load_extension_skill_metadata,
    )
}

/// Load skills from configured extra directories.
pub(super) fn load_extra_dir_skills(
    workspace_dir: &Path,
    skills_config: &SkillsRuntimeConfig,
    enforce_requirements: bool,
) -> Vec<Skill> {
    let mut skills = Vec::new();
    for path in resolve_extra_skill_dirs(workspace_dir, skills_config) {
        skills.extend(load_records_from_directory(
            &path,
            enforce_requirements,
            load_extension_skill,
        ));
    }
    skills
}

/// Load skill metadata from configured extra directories.
pub(super) fn load_extra_dir_skill_metadata(
    workspace_dir: &Path,
    skills_config: &SkillsRuntimeConfig,
    enforce_requirements: bool,
) -> Vec<SkillMetadata> {
    let mut skills = Vec::new();
    for path in resolve_extra_skill_dirs(workspace_dir, skills_config) {
        skills.extend(load_records_from_directory(
            &path,
            enforce_requirements,
            load_extension_skill_metadata,
        ));
    }
    skills
}

/// Resolve configured extra skill directories to absolute paths.
pub(super) fn resolve_extra_skill_dirs(
    workspace_dir: &Path,
    skills_config: &SkillsRuntimeConfig,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for dir in &skills_config.extra_dirs {
        let trimmed = dir.trim();
        if trimmed.is_empty() {
            continue;
        }
        if Path::new(trimmed).is_absolute() {
            dirs.push(PathBuf::from(trimmed));
        } else {
            dirs.push(workspace_dir.join(trimmed));
        }
    }
    dirs
}

fn load_records_from_directory<T>(
    skills_dir: &Path,
    enforce_requirements: bool,
    load_entry: impl Fn(&Path, bool) -> Result<T>,
) -> Vec<T> {
    if !skills_dir.exists() {
        return Vec::new();
    }

    let mut skills = Vec::new();

    let Ok(entries) = std::fs::read_dir(skills_dir) else {
        return skills;
    };

    let mut skill_dirs: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if file_type.is_symlink() || !file_type.is_dir() {
                return None;
            }
            let path = entry.path();
            canonicalize_under_root(&path, skills_dir).ok()
        })
        .collect();
    skill_dirs.sort();

    for path in skill_dirs {
        if !path.is_dir() {
            continue;
        }

        let extension_path = path.join("extension.toml");
        if extension_path.exists()
            && !is_symlink(&extension_path)
            && let Ok(safe_extension_path) = canonicalize_under_root(&extension_path, skills_dir)
            && let Ok(skill) = load_entry(&safe_extension_path, enforce_requirements)
        {
            skills.push(skill);
        }
    }

    skills
}

/// Load skills from a community open-skills repository directory.
pub(super) fn load_open_skills(repo_dir: &Path) -> Vec<Skill> {
    load_open_skill_records(repo_dir, load_open_skill_md)
}

/// Load skill metadata from a community open-skills repository directory.
pub(super) fn load_open_skill_metadata(repo_dir: &Path) -> Vec<SkillMetadata> {
    load_open_skill_records(repo_dir, load_open_skill_md_metadata)
}

fn load_open_skill_records<T>(repo_dir: &Path, load_entry: impl Fn(&Path) -> Result<T>) -> Vec<T> {
    let mut skills = Vec::new();

    let Ok(entries) = std::fs::read_dir(repo_dir) else {
        return skills;
    };

    let mut markdown_files: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if file_type.is_symlink() || !file_type.is_file() {
                return None;
            }
            let path = entry.path();
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                .then_some(path)
        })
        .filter(|path| {
            !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("README.md"))
        })
        .filter_map(|path| canonicalize_under_root(&path, repo_dir).ok())
        .collect();
    markdown_files.sort();

    for path in markdown_files {
        if let Ok(skill) = load_entry(&path) {
            skills.push(skill);
        }
    }

    skills
}
