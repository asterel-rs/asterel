//! Skill loading, parsing, catalog rendering, and CLI management.
//!
//! Skills are Markdown prompt files (`.md`) stored in the skills directory. Each file may carry
//! optional YAML front-matter with a `name`, `description`, and `tools` list. The loader
//! assembles them into a [`SkillMetadataSnapshot`] that is injected into the agent's system
//! prompt on every turn.
//!
//! ## Key responsibilities
//!
//! - **[`loader`]** — Discovers skill files, parses metadata, watches for live-reload changes via
//!   a content fingerprint, and handles the `skills` CLI sub-command.
//! - **[`catalog`]** — Builds a searchable [`SkillSearchIndex`] and renders relevance-filtered
//!   skill blocks for context injection.
//! - **[`layers`]** — Merges per-channel and global skill layers, applying policy filtering.
//! - **[`types`]** — Core types: [`Skill`], [`SkillMetadata`], [`SkillCatalogEntry`], [`SkillTool`].

pub mod catalog;
pub mod layers;
pub mod loader;
pub mod types;

pub use catalog::{
    PromptSkillEntry, PromptSkillIndexEntry, SkillSearchIndex, prompt_skill_catalog,
    prompt_skill_catalog_metadata, prompt_skill_index, prompt_skill_index_metadata,
    render_relevant_skill_metadata_block, render_relevant_skills_block,
    select_relevant_skill_metadata, select_relevant_skills,
};
pub use loader::{
    SkillMetadataSnapshot, handle_command, init_skills_dir,
    load_skill_metadata_snapshot_with_policy_and_config,
    load_skill_metadata_with_policy_and_config, load_skills, load_skills_with_policy,
    load_skills_with_policy_and_config, skills_dir, skills_to_prompt, skills_watch_fingerprint,
    skills_watch_fingerprint_with_config,
};
pub use types::{Skill, SkillCatalogEntry, SkillMetadata, SkillTool};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod symlink_tests;
