use std::sync::Arc;

use crate::core::subagents::{
    AgentExtensionProfile, ExtensionLoader, SkillMetadataProvider, SkillMetadataSnapshotView,
};
use crate::core::tools::{McpToolProvider, Tool};
use crate::security::SecurityPolicy;

struct RuntimeExtensionLoader;

struct RuntimeSkillMetadataProvider;

struct RuntimeSkillMetadataSnapshot {
    snapshot: Arc<crate::plugins::skills::SkillMetadataSnapshot>,
}

struct RuntimeMcpToolProvider;

impl ExtensionLoader for RuntimeExtensionLoader {
    fn load_agent_extensions_from_workspace(
        &self,
        workspace_dir: &std::path::Path,
    ) -> Vec<AgentExtensionProfile> {
        crate::plugins::extensions::load_agent_extensions_from_workspace(workspace_dir)
            .into_iter()
            .map(|extension| AgentExtensionProfile {
                id: extension.id,
                role: extension.role,
                system_prompt: extension.system_prompt,
                model: extension.model,
                temperature: extension.temperature,
                manifest_path: extension.manifest_path,
            })
            .collect()
    }
}

impl SkillMetadataSnapshotView for RuntimeSkillMetadataSnapshot {
    fn render_relevant_block(
        &self,
        user_message: &str,
        description_char_limit: usize,
        limit: usize,
    ) -> String {
        self.snapshot.search_index().render_relevant_block(
            user_message,
            description_char_limit,
            limit,
        )
    }
}

impl SkillMetadataProvider for RuntimeSkillMetadataProvider {
    fn load_skill_metadata_snapshot_with_policy_and_config(
        &self,
        workspace_dir: &std::path::Path,
        security: &SecurityPolicy,
        skills_config: &crate::config::SkillsRuntimeConfig,
    ) -> Arc<dyn SkillMetadataSnapshotView> {
        Arc::new(RuntimeSkillMetadataSnapshot {
            snapshot: crate::plugins::skills::load_skill_metadata_snapshot_with_policy_and_config(
                workspace_dir,
                security,
                skills_config,
            ),
        })
    }
}

impl McpToolProvider for RuntimeMcpToolProvider {
    #[cfg(feature = "mcp")]
    fn create_mcp_tools(
        &self,
        config: &crate::config::schema::McpConfig,
        security: &SecurityPolicy,
    ) -> Vec<Box<dyn Tool>> {
        crate::plugins::mcp::create_mcp_tools_with_policy(config, security)
    }

    #[cfg(not(feature = "mcp"))]
    fn create_mcp_tools(
        &self,
        _config: &crate::config::schema::McpConfig,
        _security: &SecurityPolicy,
    ) -> Vec<Box<dyn Tool>> {
        Vec::new()
    }
}

pub(super) fn runtime_extension_loader() -> Arc<dyn ExtensionLoader> {
    Arc::new(RuntimeExtensionLoader)
}

#[must_use]
pub fn runtime_skill_metadata_provider() -> Arc<dyn SkillMetadataProvider> {
    Arc::new(RuntimeSkillMetadataProvider)
}

#[must_use]
pub fn runtime_mcp_tool_provider() -> Arc<dyn McpToolProvider> {
    Arc::new(RuntimeMcpToolProvider)
}
