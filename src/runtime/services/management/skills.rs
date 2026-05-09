use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

use super::config_store::{
    load_persisted_runtime_config, runtime_apply_mode, save_persisted_runtime_config,
};
use super::{ManagedSkillRecord, SkillMutationResult};
use crate::config::{Config, SkillsRuntimeConfig};
use crate::plugins::skills::{self, SkillMetadata};
use crate::security::SecurityPolicy;

fn normalize_skill_name(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

fn retain_disabled_skill(
    config: &mut SkillsRuntimeConfig,
    skill_name: &str,
    enabled: bool,
) -> bool {
    let normalized = normalize_skill_name(skill_name);
    let before_len = config.disabled_skills.len();
    config
        .disabled_skills
        .retain(|entry| normalize_skill_name(entry) != normalized);
    let mut changed = config.disabled_skills.len() != before_len;
    if !enabled {
        config.disabled_skills.push(normalized);
        changed = true;
    }
    config.disabled_skills.sort();
    config.disabled_skills.dedup();
    changed
}

fn all_skill_metadata(config: &Config, security: &SecurityPolicy) -> Vec<SkillMetadata> {
    let mut inventory_config = config.skills.clone();
    inventory_config.disabled_skills.clear();
    let managed_roots = managed_skill_roots(config);
    skills::load_skill_metadata_with_policy_and_config(
        &config.workspace_dir,
        security,
        &inventory_config,
    )
    .into_iter()
    .filter(|skill| is_managed_skill_location(skill.location.as_deref(), &managed_roots))
    .collect()
}

fn managed_skill_roots(config: &Config) -> Vec<PathBuf> {
    let mut roots = vec![config.workspace_dir.join("skills")];
    for dir in &config.skills.extra_dirs {
        let trimmed = dir.trim();
        if trimmed.is_empty() {
            continue;
        }
        if Path::new(trimmed).is_absolute() {
            roots.push(PathBuf::from(trimmed));
        } else {
            roots.push(config.workspace_dir.join(trimmed));
        }
    }
    roots
}

fn is_managed_skill_location(location: Option<&Path>, roots: &[PathBuf]) -> bool {
    let Some(location) = location else {
        return false;
    };
    roots.iter().any(|root| location.starts_with(root))
}

fn find_skill_metadata<'a>(
    skills: &'a [SkillMetadata],
    skill_id: &str,
) -> Option<&'a SkillMetadata> {
    let normalized = normalize_skill_name(skill_id);
    skills
        .iter()
        .find(|skill| normalize_skill_name(&skill.name) == normalized)
}

pub(super) fn list_admin_skills(
    current: &Config,
    security: &SecurityPolicy,
) -> Result<Vec<ManagedSkillRecord>> {
    let config = load_persisted_runtime_config(current)?;
    let mut items = all_skill_metadata(&config, security)
        .into_iter()
        .map(|skill| {
            let enabled = !config
                .skills
                .disabled_skills
                .iter()
                .any(|entry| normalize_skill_name(entry) == normalize_skill_name(&skill.name));
            ManagedSkillRecord {
                name: skill.name,
                description: skill.description,
                version: skill.version,
                author: skill.author,
                tags: skill.tags,
                tools: skill.tools,
                enabled,
                location: skill.location.map(|path| path.display().to_string()),
            }
        })
        .collect::<Vec<_>>();
    items.sort_by(|lhs, rhs| lhs.name.cmp(&rhs.name));
    Ok(items)
}

pub(super) fn install_admin_skill(
    current: &Config,
    security: &SecurityPolicy,
    source: &str,
) -> Result<()> {
    let config = load_persisted_runtime_config(current)?;
    skills::handle_command(
        crate::SkillCommands::Install {
            source: source.to_string(),
        },
        &config.workspace_dir,
        security,
        &config.skills,
    )
}

pub(super) fn remove_admin_skill(
    current: &Config,
    security: &SecurityPolicy,
    skill_id: &str,
) -> Result<()> {
    let mut config = load_persisted_runtime_config(current)?;
    skills::handle_command(
        crate::SkillCommands::Remove {
            name: skill_id.to_string(),
        },
        &config.workspace_dir,
        security,
        &config.skills,
    )?;
    if retain_disabled_skill(&mut config.skills, skill_id, true) {
        save_persisted_runtime_config(&config)?;
    }
    Ok(())
}

pub(super) fn update_admin_skill(
    current: &Config,
    security: &SecurityPolicy,
    skill_id: &str,
    enabled: bool,
) -> Result<SkillMutationResult> {
    let mut config = load_persisted_runtime_config(current)?;
    let metadata = all_skill_metadata(&config, security);
    let Some(skill) = find_skill_metadata(&metadata, skill_id) else {
        bail!("skill '{skill_id}' not found");
    };

    let changed = retain_disabled_skill(&mut config.skills, &skill.name, enabled);
    if changed {
        save_persisted_runtime_config(&config)?;
    }

    Ok(SkillMutationResult {
        skill_id: skill.name.clone(),
        enabled,
        changes: if changed {
            vec!["enabled".to_string()]
        } else {
            Vec::new()
        },
        apply_mode: runtime_apply_mode(&config),
    })
}
