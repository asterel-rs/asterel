//! Agent system prompt assembly.
//!
//! Combines workspace files, tool descriptions, skills, and persona
//! options into the final system prompt passed to the agent loop.

use asterel::config::Config;
use asterel::security::SecurityPolicy;

/// Build the agent system prompt from workspace files, tool descriptions,
/// skills, and persona options.
///
/// This is app-level orchestration that combines plugins, tools, and
/// transport-layer prompt building.
pub fn build_agent_system_prompt(
    config: &Config,
    model_name: &str,
    security: &SecurityPolicy,
) -> String {
    let skill_snapshot =
        asterel::plugins::skills::load_skill_metadata_snapshot_with_policy_and_config(
            &config.workspace_dir,
            security,
            &config.skills,
        );
    let skill_entries = skill_snapshot.search_index().prompt_index_entries();
    let mcp_tool_provider = asterel::runtime::services::runtime_mcp_tool_provider();
    let tool_descs = asterel::core::tools::tool_desc_with_security_and_mcp_provider(
        config.browser.enabled,
        config.composio.enabled,
        Some(&config.mcp),
        security,
        None,
        mcp_tool_provider.as_ref(),
    );
    let prompt_tool_descs: Vec<(&str, &str)> = tool_descs
        .iter()
        .map(|(name, description)| (name.as_str(), description.as_str()))
        .collect();
    let prompt_options = asterel::transport::channels::prompt_builder::SystemPromptOptions {
        companion_behavior: Some(config.persona.companion.clone()),
    };
    asterel::transport::channels::prompt_builder::build_system_prompt_from_index_opts(
        &config.workspace_dir,
        model_name,
        &prompt_tool_descs,
        &skill_entries,
        None,
        &prompt_options,
    )
}
