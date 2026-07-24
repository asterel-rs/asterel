//! Skill loader: discovers, parses, and deduplicates skills from
//! workspace, extra directories, and the open-skills repository.

mod commands;
mod directory;
mod open_skills;
mod parse;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::Result;
pub use commands::handle_command;
use directory::{
    dedupe_skills_by_name, load_extra_dir_skill_metadata, load_extra_dir_skills,
    load_open_skill_metadata, load_open_skills, load_workspace_skill_metadata,
    load_workspace_skills, normalized_skill_sources,
};
use open_skills::{OpenSkillsCacheState, ensure_open_skills_repo, open_skills_cache_state};

use super::{Skill, SkillCatalogEntry, SkillMetadata, SkillSearchIndex};
use crate::config::{SkillSource, SkillsRuntimeConfig};
use crate::security::SecurityPolicy;
use crate::utils::text::{sanitize_prompt_line, strip_internal_prompt_blocks, truncate_ellipsis};

const MAX_SKILL_METADATA_SNAPSHOT_CACHE_ENTRIES: usize = 32;
const SKILL_PROMPT_NAME_MAX_CHARS: usize = 80;
const SKILL_PROMPT_VERSION_MAX_CHARS: usize = 40;
const SKILL_PROMPT_DESCRIPTION_MAX_CHARS: usize = 240;
const SKILL_PROMPT_TOOL_KIND_MAX_CHARS: usize = 40;

#[derive(Debug, Clone)]
pub struct SkillMetadataSnapshot {
    fingerprint: u64,
    metadata: Vec<SkillMetadata>,
    search_index: SkillSearchIndex,
}

impl SkillMetadataSnapshot {
    fn new(metadata: Vec<SkillMetadata>, workspace_dir: &Path, fingerprint: u64) -> Self {
        let search_index = SkillSearchIndex::new(&metadata, workspace_dir);
        Self {
            fingerprint,
            metadata,
            search_index,
        }
    }

    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    #[must_use]
    pub fn metadata(&self) -> &[SkillMetadata] {
        &self.metadata
    }

    #[must_use]
    pub fn search_index(&self) -> &SkillSearchIndex {
        &self.search_index
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SkillSourceCacheKey {
    Workspace,
    ExtraDirs,
    OpenSkills,
}

impl From<SkillSource> for SkillSourceCacheKey {
    fn from(value: SkillSource) -> Self {
        match value {
            SkillSource::Workspace => Self::Workspace,
            SkillSource::ExtraDirs => Self::ExtraDirs,
            SkillSource::OpenSkills => Self::OpenSkills,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SkillMetadataSnapshotCacheKey {
    workspace_dir: PathBuf,
    source_priority: Vec<SkillSourceCacheKey>,
    extra_dirs: Vec<String>,
    disabled_skills: Vec<String>,
    enforce_requirements: bool,
    open_skills: OpenSkillsCacheState,
}

impl SkillMetadataSnapshotCacheKey {
    fn new(workspace_dir: &Path, skills_config: &SkillsRuntimeConfig) -> Self {
        Self {
            workspace_dir: workspace_dir.to_path_buf(),
            source_priority: normalized_skill_sources(skills_config)
                .into_iter()
                .map(SkillSourceCacheKey::from)
                .collect(),
            extra_dirs: skills_config.extra_dirs.clone(),
            disabled_skills: normalized_disabled_skills(skills_config),
            enforce_requirements: skills_config.enforce_requirements,
            open_skills: open_skills_cache_state(),
        }
    }
}

#[derive(Debug, Clone)]
struct PreparedSkillSources {
    ordered_sources: Vec<SkillSource>,
    open_skills_dir: Option<PathBuf>,
}

static SKILL_METADATA_SNAPSHOT_CACHE: OnceLock<
    Mutex<HashMap<SkillMetadataSnapshotCacheKey, Arc<SkillMetadataSnapshot>>>,
> = OnceLock::new();

fn skill_metadata_snapshot_cache()
-> &'static Mutex<HashMap<SkillMetadataSnapshotCacheKey, Arc<SkillMetadataSnapshot>>> {
    SKILL_METADATA_SNAPSHOT_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn prepare_skill_sources(
    security: &SecurityPolicy,
    skills_config: &SkillsRuntimeConfig,
) -> PreparedSkillSources {
    let ordered_sources = normalized_skill_sources(skills_config);
    let open_skills_dir = ordered_sources
        .contains(&SkillSource::OpenSkills)
        .then(|| ensure_open_skills_repo(security))
        .flatten();

    PreparedSkillSources {
        ordered_sources,
        open_skills_dir,
    }
}

fn load_skills_from_prepared_sources(
    workspace_dir: &Path,
    skills_config: &SkillsRuntimeConfig,
    prepared: &PreparedSkillSources,
) -> Vec<Skill> {
    let mut loaded = Vec::new();
    for source in &prepared.ordered_sources {
        match source {
            SkillSource::Workspace => {
                loaded.extend(load_workspace_skills(
                    workspace_dir,
                    skills_config.enforce_requirements,
                ));
            }
            SkillSource::ExtraDirs => {
                loaded.extend(load_extra_dir_skills(
                    workspace_dir,
                    skills_config,
                    skills_config.enforce_requirements,
                ));
            }
            SkillSource::OpenSkills => {
                if let Some(open_skills_dir) = prepared.open_skills_dir.as_ref() {
                    loaded.extend(load_open_skills(open_skills_dir));
                }
            }
        }
    }

    filter_disabled_skill_records(dedupe_skills_by_name(loaded), skills_config)
}

fn load_skill_metadata_from_prepared_sources(
    workspace_dir: &Path,
    skills_config: &SkillsRuntimeConfig,
    prepared: &PreparedSkillSources,
) -> Vec<SkillMetadata> {
    let mut loaded = Vec::new();
    for source in &prepared.ordered_sources {
        match source {
            SkillSource::Workspace => {
                loaded.extend(load_workspace_skill_metadata(
                    workspace_dir,
                    skills_config.enforce_requirements,
                ));
            }
            SkillSource::ExtraDirs => {
                loaded.extend(load_extra_dir_skill_metadata(
                    workspace_dir,
                    skills_config,
                    skills_config.enforce_requirements,
                ));
            }
            SkillSource::OpenSkills => {
                if let Some(open_skills_dir) = prepared.open_skills_dir.as_ref() {
                    loaded.extend(load_open_skill_metadata(open_skills_dir));
                }
            }
        }
    }

    filter_disabled_skill_records(dedupe_skills_by_name(loaded), skills_config)
}

fn normalized_skill_name(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

fn normalized_disabled_skills(skills_config: &SkillsRuntimeConfig) -> Vec<String> {
    let mut disabled = skills_config
        .disabled_skills
        .iter()
        .map(|name| normalized_skill_name(name))
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    disabled.sort();
    disabled.dedup();
    disabled
}

fn disabled_skill_name_set(skills_config: &SkillsRuntimeConfig) -> HashSet<String> {
    normalized_disabled_skills(skills_config)
        .into_iter()
        .collect()
}

fn filter_disabled_skill_records<T>(records: Vec<T>, skills_config: &SkillsRuntimeConfig) -> Vec<T>
where
    T: SkillCatalogEntry,
{
    let disabled = disabled_skill_name_set(skills_config);
    if disabled.is_empty() {
        return records;
    }

    records
        .into_iter()
        .filter(|record| !disabled.contains(&normalized_skill_name(record.name())))
        .collect()
}

/// Load all skills from the workspace skills directory
#[must_use]
pub fn load_skills(workspace_dir: &Path) -> Vec<Skill> {
    let security = SecurityPolicy::default();
    let skills_config = SkillsRuntimeConfig::default();
    load_skills_with_policy_and_config(workspace_dir, &security, &skills_config)
}

/// Load all skills with explicit spawn policy for external sync routes.
#[must_use]
pub fn load_skills_with_policy(workspace_dir: &Path, security: &SecurityPolicy) -> Vec<Skill> {
    let skills_config = SkillsRuntimeConfig::default();
    load_skills_with_policy_and_config(workspace_dir, security, &skills_config)
}

/// Load skills from all configured sources with explicit policy
/// and runtime config.
#[must_use]
pub fn load_skills_with_policy_and_config(
    workspace_dir: &Path,
    security: &SecurityPolicy,
    skills_config: &SkillsRuntimeConfig,
) -> Vec<Skill> {
    let prepared = prepare_skill_sources(security, skills_config);
    load_skills_from_prepared_sources(workspace_dir, skills_config, &prepared)
}

/// Load skill metadata from all configured sources with explicit policy and
/// runtime config without reading prompt bodies.
#[must_use]
pub fn load_skill_metadata_with_policy_and_config(
    workspace_dir: &Path,
    security: &SecurityPolicy,
    skills_config: &SkillsRuntimeConfig,
) -> Vec<SkillMetadata> {
    load_skill_metadata_snapshot_with_policy_and_config(workspace_dir, security, skills_config)
        .metadata()
        .to_vec()
}

/// Load a cached metadata snapshot and prompt search index for all configured
/// skill sources. The snapshot is invalidated whenever the watched skill
/// fingerprint changes.
#[must_use]
pub fn load_skill_metadata_snapshot_with_policy_and_config(
    workspace_dir: &Path,
    security: &SecurityPolicy,
    skills_config: &SkillsRuntimeConfig,
) -> Arc<SkillMetadataSnapshot> {
    let fingerprint = skills_watch_fingerprint_with_config(workspace_dir, skills_config);
    let cache_key = SkillMetadataSnapshotCacheKey::new(workspace_dir, skills_config);

    if !cache_key.open_skills.sync_required
        && let Some(snapshot) = skill_metadata_snapshot_cache()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&cache_key)
            .filter(|snapshot| snapshot.fingerprint() == fingerprint)
            .cloned()
    {
        return snapshot;
    }

    let prepared = prepare_skill_sources(security, skills_config);
    let metadata =
        load_skill_metadata_from_prepared_sources(workspace_dir, skills_config, &prepared);
    let fingerprint = skills_watch_fingerprint_with_config(workspace_dir, skills_config);
    let snapshot = Arc::new(SkillMetadataSnapshot::new(
        metadata,
        workspace_dir,
        fingerprint,
    ));

    let mut cache = skill_metadata_snapshot_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.len() >= MAX_SKILL_METADATA_SNAPSHOT_CACHE_ENTRIES && !cache.contains_key(&cache_key) {
        evict_unpinned_skill_metadata_snapshot(&mut cache);
    }
    cache.insert(cache_key, Arc::clone(&snapshot));
    snapshot
}

fn evict_unpinned_skill_metadata_snapshot(
    cache: &mut HashMap<SkillMetadataSnapshotCacheKey, Arc<SkillMetadataSnapshot>>,
) {
    if let Some(eviction_key) = cache
        .iter()
        .find(|(_, snapshot)| Arc::strong_count(snapshot) == 1)
        .map(|(key, _)| key.clone())
    {
        cache.remove(&eviction_key);
        return;
    }

    cache.retain(|_, snapshot| Arc::strong_count(snapshot) > 1);
}

/// Compute a fingerprint hash over all skill manifest files for
/// change detection.
#[must_use]
pub fn skills_watch_fingerprint(workspace_dir: &Path) -> u64 {
    let skills_config = SkillsRuntimeConfig::default();
    skills_watch_fingerprint_with_config(workspace_dir, &skills_config)
}

/// Compute a fingerprint hash with explicit runtime config for
/// change detection.
#[must_use]
pub fn skills_watch_fingerprint_with_config(
    workspace_dir: &Path,
    skills_config: &SkillsRuntimeConfig,
) -> u64 {
    directory::skills_watch_fingerprint_inner(workspace_dir, skills_config)
}

/// Build a system prompt addition from all loaded skills
#[must_use]
pub fn skills_to_prompt(skills: &[Skill]) -> String {
    use std::fmt::Write;

    if skills.is_empty() {
        return String::new();
    }

    let mut prompt = String::from("\n## Active Skills\n\n");

    for skill in skills {
        let name = sanitize_skill_prompt_field(&skill.name, SKILL_PROMPT_NAME_MAX_CHARS);
        let version = sanitize_skill_prompt_field(&skill.version, SKILL_PROMPT_VERSION_MAX_CHARS);
        let description =
            sanitize_skill_prompt_field(&skill.description, SKILL_PROMPT_DESCRIPTION_MAX_CHARS);
        let name = if name.is_empty() {
            "unknown-skill"
        } else {
            name.as_str()
        };
        let version = if version.is_empty() {
            "unknown"
        } else {
            version.as_str()
        };

        let _ = writeln!(prompt, "### {name} (v{version})");
        if !description.is_empty() {
            let _ = writeln!(prompt, "{description}");
        }

        if !skill.tools.is_empty() {
            prompt.push_str("Tools:\n");
            for tool in &skill.tools {
                let name = sanitize_skill_prompt_field(&tool.name, SKILL_PROMPT_NAME_MAX_CHARS);
                let description = sanitize_skill_prompt_field(
                    &tool.description,
                    SKILL_PROMPT_DESCRIPTION_MAX_CHARS,
                );
                let kind =
                    sanitize_skill_prompt_field(&tool.kind, SKILL_PROMPT_TOOL_KIND_MAX_CHARS);
                let name = if name.is_empty() {
                    "unknown-tool"
                } else {
                    name.as_str()
                };
                let kind = if kind.is_empty() {
                    "unknown"
                } else {
                    kind.as_str()
                };
                let _ = writeln!(prompt, "- **{name}**: {description} ({kind})");
            }
        }

        for p in &skill.prompts {
            let body = strip_internal_prompt_blocks(p);
            if body.trim().is_empty() {
                continue;
            }
            prompt.push_str(body.trim());
            prompt.push('\n');
        }

        prompt.push('\n');
    }

    prompt
}

fn sanitize_skill_prompt_field(value: &str, max_chars: usize) -> String {
    truncate_ellipsis(sanitize_prompt_line(value).as_str(), max_chars)
}

/// Get the skills directory path
#[must_use]
pub fn skills_dir(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("skills")
}

/// Initialize the skills directory with a README
///
/// # Errors
///
/// Returns an error when creating the skills directory or writing the initial
/// README fails.
pub fn init_skills_dir(workspace_dir: &Path) -> Result<()> {
    let dir = skills_dir(workspace_dir);
    std::fs::create_dir_all(&dir)?;

    let readme = dir.join("README.md");
    if !readme.exists() {
        std::fs::write(
            &readme,
            "# Asterel Skills\n\n\
             Each subdirectory is a skill. Every skill must provide `extension.toml`\n\
             plus one or more prompt bodies such as `SKILL.md`.\n\n\
             ## extension.toml format\n\n\
             ```toml\n\
             [extension]\n\
             id = \"my-skill\"\n\
             kind = \"skill\"\n\
             description = \"What this skill does\"\n\
             version = \"0.1.0\"\n\
             tags = [\"productivity\", \"automation\"]\n\
             capabilities = [\"workspace.read\"]\n\
             permissions = [\"shell.exec\"]\n\n\
             [skill]\n\
             prompt_bodies = [\"SKILL.md\"]\n\n\
             [[skill.tools]]\n\
             name = \"my_tool\"\n\
             description = \"What this tool does\"\n\
             kind = \"shell\"\n\
             command = \"echo hello\"\n\
             ```\n\n\
             ## Prompt body\n\n\
             Put the skill instructions in `SKILL.md` and reference that file from\n\
             `prompt_bodies`.\n\n\
             ## Installing local skills\n\n\
             ```bash\n\
             asterel skills install <local-skill-dir>\n\
             asterel skills list\n\
             ```\n",
        )?;
    }

    Ok(())
}
