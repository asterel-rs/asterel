use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::SkillsRuntimeConfig;
use crate::security::SecurityPolicy;

#[derive(Debug, Clone, PartialEq)]
pub struct AgentExtensionProfile {
    pub id: String,
    pub role: Option<String>,
    pub system_prompt: String,
    pub model: Option<String>,
    pub temperature: Option<f64>,
    pub manifest_path: PathBuf,
}

pub trait ExtensionLoader: Send + Sync {
    fn load_agent_extensions_from_workspace(
        &self,
        workspace_dir: &Path,
    ) -> Vec<AgentExtensionProfile>;
}

#[derive(Debug, Default)]
pub struct NoopExtensionLoader;

impl NoopExtensionLoader {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl ExtensionLoader for NoopExtensionLoader {
    fn load_agent_extensions_from_workspace(
        &self,
        _workspace_dir: &Path,
    ) -> Vec<AgentExtensionProfile> {
        Vec::new()
    }
}

pub trait SkillMetadataSnapshotView: Send + Sync {
    fn render_relevant_block(
        &self,
        user_message: &str,
        description_char_limit: usize,
        limit: usize,
    ) -> String;
}

#[derive(Debug, Default)]
pub struct EmptySkillMetadataSnapshot;

impl EmptySkillMetadataSnapshot {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl SkillMetadataSnapshotView for EmptySkillMetadataSnapshot {
    fn render_relevant_block(
        &self,
        _user_message: &str,
        _description_char_limit: usize,
        _limit: usize,
    ) -> String {
        String::new()
    }
}

pub trait SkillMetadataProvider: Send + Sync {
    fn load_skill_metadata_snapshot_with_policy_and_config(
        &self,
        workspace_dir: &Path,
        security: &SecurityPolicy,
        skills_config: &SkillsRuntimeConfig,
    ) -> Arc<dyn SkillMetadataSnapshotView>;
}

#[derive(Debug, Default)]
pub struct NoopSkillMetadataProvider;

impl NoopSkillMetadataProvider {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl SkillMetadataProvider for NoopSkillMetadataProvider {
    fn load_skill_metadata_snapshot_with_policy_and_config(
        &self,
        _workspace_dir: &Path,
        _security: &SecurityPolicy,
        _skills_config: &SkillsRuntimeConfig,
    ) -> Arc<dyn SkillMetadataSnapshotView> {
        Arc::new(EmptySkillMetadataSnapshot::new())
    }
}
