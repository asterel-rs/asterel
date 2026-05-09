//! Tool and operator construction from configuration.
//!
//! Builds the default tool set, action operator, and optional
//! channel/memory/taste tools based on runtime config.

#[cfg(feature = "taste")]
use std::future::Future;
#[cfg(feature = "taste")]
use std::pin::Pin;
use std::sync::Arc;

use super::middleware::tool_names;
use super::{
    ActionOperator, BrowserOpenTool, BrowserTool, ChannelAddReactionTool, ChannelCreateThreadTool,
    ChannelGetHistoryTool, ChannelSendEmbedTool, ChannelSendRichTool, ComposioTool, DelegateTool,
    FileDeleteTool, FileReadTool, FileWriteTool, McpToolProvider, MemoryCorrectTool,
    MemoryForgetTool, MemoryGovernanceTool, MemoryLookupTool, MemoryRecallTool, MemoryStoreTool,
    NoopMcpToolProvider, NoopOperator, ShellTool, SubagentCancelTool, SubagentOutputTool,
    SubagentSpawnTool, Tool, ToolRegistry, default_middleware_chain,
};
use crate::config::schema::{McpConfig, ToolsConfig};
use crate::contracts::channels::ChannelCapabilities;
use crate::core::memory::Memory;
use crate::security::SecurityPolicy;

#[cfg(feature = "taste")]
struct UnavailableTasteTool {
    name: &'static str,
    description: &'static str,
    reason: String,
}

#[cfg(feature = "taste")]
impl UnavailableTasteTool {
    fn new(name: &'static str, description: &'static str, reason: impl Into<String>) -> Self {
        Self {
            name,
            description,
            reason: reason.into(),
        }
    }
}

#[cfg(feature = "taste")]
impl Tool for UnavailableTasteTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "additionalProperties": true,
            "description": "Taste is configured but unavailable; any invocation returns a visible failure."
        })
    }

    fn execute<'a>(
        &'a self,
        _args: serde_json::Value,
        _ctx: &'a super::middleware::ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<super::ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            Ok(super::ToolResult::failure(format!(
                "Taste tool unavailable: {}",
                self.reason
            )))
        })
    }
}

#[cfg(feature = "taste")]
fn append_unavailable_taste_tools(tools: &mut Vec<Box<dyn Tool>>, reason: impl Into<String>) {
    let reason = reason.into();
    tools.push(Box::new(UnavailableTasteTool::new(
        tool_names::TASTE_EVALUATE,
        "Taste evaluation is configured but unavailable; invoking this tool reports the backend error.",
        reason.clone(),
    )));
    tools.push(Box::new(UnavailableTasteTool::new(
        tool_names::TASTE_COMPARE,
        "Taste comparison is configured but unavailable; invoking this tool reports the backend error.",
        reason,
    )));
}

/// Create the default tool registry
#[must_use]
pub fn default_tools(_security: &Arc<SecurityPolicy>) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ShellTool::new()),
        Box::new(FileReadTool::new()),
        Box::new(FileWriteTool::new()),
        Box::new(FileDeleteTool::new()),
    ]
}

/// Create the default no-op action operator for the given policy.
#[must_use]
pub fn default_action_operator(security: Arc<SecurityPolicy>) -> Arc<dyn ActionOperator> {
    Arc::new(NoopOperator::new(security))
}

/// Generate tool descriptions for system prompts
///
/// Returns a vector of (`tool_name`, description) tuples.
/// Includes `browser_open` if `browser_enabled` is true.
/// Includes `composio` if `composio_enabled` is true.
#[must_use]
pub fn tool_descriptions(
    browser_enabled: bool,
    composio_enabled: bool,
    mcp_config: Option<&McpConfig>,
) -> Vec<(String, String)> {
    let mcp_tool_provider = NoopMcpToolProvider::new();
    tool_descriptions_with_mcp_provider(
        browser_enabled,
        composio_enabled,
        mcp_config,
        &mcp_tool_provider,
    )
}

#[must_use]
pub fn tool_descriptions_with_mcp_provider(
    browser_enabled: bool,
    composio_enabled: bool,
    mcp_config: Option<&McpConfig>,
    mcp_tool_provider: &dyn McpToolProvider,
) -> Vec<(String, String)> {
    let security = SecurityPolicy::default();
    tool_desc_with_security_and_mcp_provider(
        browser_enabled,
        composio_enabled,
        mcp_config,
        &security,
        None,
        mcp_tool_provider,
    )
}

#[must_use]
pub fn tool_desc_with_security(
    browser_enabled: bool,
    composio_enabled: bool,
    mcp_config: Option<&McpConfig>,
    security: &SecurityPolicy,
    channel_capabilities: Option<&ChannelCapabilities>,
) -> Vec<(String, String)> {
    let mcp_tool_provider = NoopMcpToolProvider::new();
    tool_desc_with_security_and_mcp_provider(
        browser_enabled,
        composio_enabled,
        mcp_config,
        security,
        channel_capabilities,
        &mcp_tool_provider,
    )
}

#[must_use]
pub fn tool_desc_with_security_and_mcp_provider(
    browser_enabled: bool,
    composio_enabled: bool,
    mcp_config: Option<&McpConfig>,
    security: &SecurityPolicy,
    channel_capabilities: Option<&ChannelCapabilities>,
    mcp_tool_provider: &dyn McpToolProvider,
) -> Vec<(String, String)> {
    let mut descs = base_tool_descriptions();
    append_optional_tool_descriptions(&mut descs, browser_enabled, composio_enabled);
    append_channel_tool_descriptions(&mut descs, channel_capabilities);
    append_taste_tool_descriptions(&mut descs);
    append_codespace_tool_description(&mut descs);
    append_introspection_tool_descriptions(&mut descs);
    append_web_tool_descriptions(&mut descs);

    append_mcp_tool_descriptions_with_security(&mut descs, mcp_config, security, mcp_tool_provider);

    descs
}

fn base_tool_descriptions() -> Vec<(String, String)> {
    vec![
        (
            tool_names::SHELL.to_string(),
            "Execute terminal commands. Use when: running local checks, build/test commands, diagnostics. Don't use when: a safer dedicated tool exists, or command is destructive without approval.".to_string(),
        ),
        (
            tool_names::FILE_READ.to_string(),
            "Read file contents. Use when: inspecting project files, configs, logs. Don't use when: a targeted search is enough.".to_string(),
        ),
        (
            tool_names::FILE_WRITE.to_string(),
            "Write file contents. Use when: applying focused edits, scaffolding files, updating docs/code. Don't use when: side effects are unclear or file ownership is uncertain.".to_string(),
        ),
        (
            tool_names::FILE_DELETE.to_string(),
            "Delete a workspace file created or modified by the agent this session. Foreign files require shell + operator approval.".to_string(),
        ),
        (
            tool_names::MEMORY_STORE.to_string(),
            "Save to memory. Use when: preserving durable preferences, decisions, key context. Don't use when: information is transient/noisy/sensitive without need.".to_string(),
        ),
        (
            tool_names::MEMORY_RECALL.to_string(),
            "Search memory. Use when: retrieving prior decisions, user preferences, historical context. Don't use when: answer is already in current context.".to_string(),
        ),
        (
            tool_names::MEMORY_LOOKUP.to_string(),
            "Resolve one memory slot. Use when: confirming the current value of a specific belief or fact.".to_string(),
        ),
        (
            tool_names::MEMORY_CORRECT.to_string(),
            "Correct a stored memory slot after confirming the previous value matches what needs fixing.".to_string(),
        ),
        (
            tool_names::MEMORY_FORGET.to_string(),
            "Delete a memory entry. Use when: memory is incorrect/stale or explicitly requested for removal. Don't use when: impact is uncertain.".to_string(),
        ),
        (
            tool_names::DELEGATE.to_string(),
            "Simple delegation interface with actions: run, status, list, cancel.".to_string(),
        ),
        (
            tool_names::SUBAGENT_SPAWN.to_string(),
            "Spawn an isolated sub-agent run for delegated work. Supports background and inline execution.".to_string(),
        ),
        (
            tool_names::SUBAGENT_OUTPUT.to_string(),
            "Retrieve status and output for a delegated sub-agent run by run_id.".to_string(),
        ),
        (
            tool_names::SUBAGENT_CANCEL.to_string(),
            "Cancel a running delegated sub-agent by run_id.".to_string(),
        ),
    ]
}

fn append_optional_tool_descriptions(
    descs: &mut Vec<(String, String)>,
    browser_enabled: bool,
    composio_enabled: bool,
) {
    if browser_enabled {
        descs.push((
            tool_names::BROWSER_OPEN.to_string(),
            "Open approved HTTPS URLs in Brave Browser (allowlist-only, no scraping)".to_string(),
        ));
    }

    if composio_enabled {
        descs.push((
            tool_names::COMPOSIO.to_string(),
            "Execute actions on 1000+ apps via Composio (Gmail, Notion, GitHub, Slack, etc.). Use action='list' to discover, 'execute' to run, 'connect' to OAuth.".to_string(),
        ));
    }
}

fn append_channel_tool_descriptions(
    descs: &mut Vec<(String, String)>,
    channel_capabilities: Option<&ChannelCapabilities>,
) {
    let Some(capabilities) = channel_capabilities else {
        return;
    };

    if capabilities.can_create_thread {
        descs.push((
            tool_names::CHANNEL_CREATE_THREAD.to_string(),
            "Create a thread in the current channel or from a specific message when channel threading is supported.".to_string(),
        ));
    }
    if capabilities.can_add_reaction {
        descs.push((
            tool_names::CHANNEL_ADD_REACTION.to_string(),
            "Add a reaction emoji to a message in the active channel.".to_string(),
        ));
    }
    if capabilities.can_send_buttons || capabilities.can_send_embed {
        descs.push((
            tool_names::CHANNEL_SEND_RICH.to_string(),
            "Send richer channel messages with optional button components and embed payloads."
                .to_string(),
        ));
    }
    if capabilities.can_fetch_history {
        descs.push((
            tool_names::CHANNEL_GET_HISTORY.to_string(),
            "Fetch recent channel message history with optional pagination.".to_string(),
        ));
    }
    if capabilities.can_send_embed {
        descs.push((
            tool_names::CHANNEL_SEND_EMBED.to_string(),
            "Send an embed message to the current channel.".to_string(),
        ));
    }
}

fn append_codespace_tool_description(descs: &mut Vec<(String, String)>) {
    descs.push((
        tool_names::CODESPACE.to_string(),
        "Sandboxed dev environment: create projects, write/read files, run tests, execute commands, and promote successful projects to skills. Actions: create_project, list_projects, write_file, read_file, run_tests, exec, run, promote, git_init, git, delete_project, status.".to_string(),
    ));
}

#[cfg(feature = "taste")]
fn append_taste_tool_descriptions(descs: &mut Vec<(String, String)>) {
    descs.push((
        tool_names::TASTE_EVALUATE.to_string(),
        "Evaluate text or UI artifact aesthetic quality on coherence, hierarchy, and intentionality axes. Returns scores [0.0-1.0] and improvement suggestions.".to_string(),
    ));
    descs.push((
        tool_names::TASTE_COMPARE.to_string(),
        "Record a preference comparison between two artifacts. Persists to store and updates Bradley-Terry ratings.".to_string(),
    ));
}

#[cfg(not(feature = "taste"))]
fn append_taste_tool_descriptions(_descs: &mut Vec<(String, String)>) {}

fn append_web_tool_descriptions(descs: &mut Vec<(String, String)>) {
    descs.push((
        tool_names::WEB_FETCH.to_string(),
        "Fetch a URL and extract readable text. Handles HTML, plain text, JSON. \
         Returns title, content, and content type. HTTPS only."
            .to_string(),
    ));
    descs.push((
        tool_names::WEB_SEARCH.to_string(),
        "Search the web via DuckDuckGo. Returns titles, URLs, and snippets. \
         No API key required. Use for factual lookups."
            .to_string(),
    ));
    descs.push((
        tool_names::WEB_SCRAPE.to_string(),
        "Fetch a URL and extract content with CSS selectors. More targeted than web_fetch. \
         Use when you need specific page sections."
            .to_string(),
    ));
    descs.push((
        tool_names::WEB_SUMMARIZE.to_string(),
        "Summarize text using extractive summarization. Selects important sentences by \
         position, keyword density, and cue phrases. No LLM call."
            .to_string(),
    ));
}

pub(crate) fn append_introspection_tool_descriptions(descs: &mut Vec<(String, String)>) {
    descs.push((
        tool_names::INTROSPECT_AFFECT.to_string(),
        "Query your affect detection for this turn (emotion, confidence, cause, desire). \
         Use when pre-injected affect may be inaccurate."
            .to_string(),
    ));
    descs.push((
        tool_names::INTROSPECT_RELATIONSHIP.to_string(),
        "Query relationship state with current user (trust, rapport, depth, notable events). \
         Use when tone needs explicit calibration."
            .to_string(),
    ));
    descs.push((
        tool_names::INTROSPECT_SELF_MODEL.to_string(),
        "Query your self-model: capability estimates, uncertainty register, calibration status. \
         Use when assessing whether you're qualified to answer."
            .to_string(),
    ));
    descs.push((
        tool_names::INTROSPECT_PRINCIPLES.to_string(),
        "Search distilled behavioral principles beyond what was pre-injected. \
         Use for novel situations where existing principles don't cover."
            .to_string(),
    ));
    descs.push((
        tool_names::INTROSPECT_EXPERIENCE.to_string(),
        "Search past similar interaction experiences for relevant patterns. \
         Use to check how similar situations were handled before."
            .to_string(),
    ));
    descs.push((
        tool_names::ADJUST_REASONING.to_string(),
        "Switch reasoning strategy mid-turn (standard/stepwise/verify_first/ask_clarify). \
         Max 1 per turn. Logged for strategy feedback."
            .to_string(),
    ));
    descs.push((
        tool_names::FLAG_UNCERTAINTY.to_string(),
        "Register explicit uncertainty in a domain. Updates self-model capability estimates. \
         Use when venturing into unfamiliar territory."
            .to_string(),
    ));
    descs.push((
        tool_names::ANNOTATE_TURN.to_string(),
        "Attach metadata to this turn for narrative tracking. \
         Use when a significant event occurs worth remembering."
            .to_string(),
    ));
    descs.push((
        tool_names::EVALUATE_CONSISTENCY.to_string(),
        "Check if a response draft is consistent with your persona specification. \
         Use before sending important or sensitive responses."
            .to_string(),
    ));
}

/// Append name/description pairs from dynamically loaded tools.
pub(crate) fn append_dynamic_tool_descriptions(
    descriptions: &mut Vec<(String, String)>,
    tools: &[Box<dyn Tool>],
) {
    descriptions.extend(
        tools
            .iter()
            .map(|tool| (tool.name().to_string(), tool.description().to_string())),
    );
}

pub(crate) fn append_mcp_tool_descriptions_with_security(
    descriptions: &mut Vec<(String, String)>,
    mcp_config: Option<&McpConfig>,
    security: &SecurityPolicy,
    mcp_tool_provider: &dyn McpToolProvider,
) {
    if let Some(config) = mcp_config {
        let mcp_tools = mcp_tool_provider.create_mcp_tools(config, security);
        append_dynamic_tool_descriptions(descriptions, &mcp_tools);
    }
}

/// Configuration for building the tool registry.
pub struct ToolRegistryConfig<'a> {
    /// Security policy for tool access control.
    pub security: &'a Arc<SecurityPolicy>,
    /// Shared memory backend for memory tools.
    pub memory: Arc<dyn Memory>,
    /// Composio API key, if available.
    pub composio_key: Option<&'a str>,
    /// Browser configuration (allowlist, session).
    pub browser: &'a crate::config::BrowserConfig,
    /// Per-tool enable/disable configuration.
    pub tools: &'a ToolsConfig,
    /// MCP server configuration, if any.
    pub mcp: Option<&'a McpConfig>,
    pub mcp_tool_provider: Arc<dyn McpToolProvider>,
    /// Taste engine configuration.
    pub taste: &'a crate::config::TasteConfig,
    /// Provider for the taste engine, if enabled.
    pub taste_provider: Option<Arc<dyn crate::core::providers::Provider>>,
    /// Model identifier for the taste engine.
    pub taste_model: &'a str,
    /// Channel capabilities for the active transport.
    pub channel_capabilities: Option<&'a ChannelCapabilities>,
    /// Codespace sandbox configuration.
    pub codespace: &'a crate::config::CodespaceConfig,
}

/// Build a [`ToolRegistry`] from explicit lower-layer tool configuration.
#[must_use]
pub fn build_tool_registry_from_parts(cfg: ToolRegistryConfig<'_>) -> Arc<ToolRegistry> {
    let tools = all_tools(cfg);
    let middleware = default_middleware_chain();
    let mut registry = ToolRegistry::new(middleware);
    for tool in tools {
        registry.register(tool);
    }
    Arc::new(registry)
}

/// Create full tool registry including memory tools and optional Composio
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn all_tools(cfg: ToolRegistryConfig<'_>) -> Vec<Box<dyn Tool>> {
    let ToolRegistryConfig {
        security,
        memory,
        composio_key,
        browser: browser_config,
        tools: tools_config,
        mcp: mcp_config,
        mcp_tool_provider,
        #[cfg(feature = "taste")]
            taste: taste_config,
        #[cfg(not(feature = "taste"))]
            taste: _,
        #[cfg(feature = "taste")]
        taste_provider,
        #[cfg(not(feature = "taste"))]
            taste_provider: _,
        #[cfg(feature = "taste")]
        taste_model,
        #[cfg(not(feature = "taste"))]
            taste_model: _,
        channel_capabilities,
        codespace: codespace_config,
    } = cfg;
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();

    append_core_tools(&mut tools, tools_config, &memory);
    append_browser_tools(&mut tools, browser_config, security);
    append_composio_tools(&mut tools, composio_key);

    append_mcp_tools(&mut tools, mcp_config, security, mcp_tool_provider.as_ref());
    append_delegation_tools(&mut tools);
    append_channel_tools(&mut tools, channel_capabilities);
    append_codespace_tools(&mut tools, codespace_config);

    #[cfg(feature = "taste")]
    {
        append_taste_tools(
            &mut tools,
            taste_config,
            taste_provider,
            taste_model,
            security,
        );
    }

    tools
}

fn append_core_tools(
    tools: &mut Vec<Box<dyn Tool>>,
    tools_config: &ToolsConfig,
    memory: &Arc<dyn Memory>,
) {
    if tools_config.shell.enabled {
        tools.push(Box::new(ShellTool::new()));
    }
    if tools_config.file_read.enabled {
        tools.push(Box::new(FileReadTool::new()));
    }
    if tools_config.file_write.enabled {
        tools.push(Box::new(FileWriteTool::new()));
    }
    if tools_config.file_delete.enabled {
        tools.push(Box::new(FileDeleteTool::new()));
    }
    if tools_config.memory_store.enabled {
        tools.push(Box::new(MemoryStoreTool::new(Arc::clone(memory))));
    }
    if tools_config.memory_recall.enabled {
        tools.push(Box::new(MemoryRecallTool::new(Arc::clone(memory))));
    }
    if tools_config.memory_lookup.enabled {
        tools.push(Box::new(MemoryLookupTool::new(Arc::clone(memory))));
    }
    if tools_config.memory_correct.enabled {
        tools.push(Box::new(MemoryCorrectTool::new(Arc::clone(memory))));
    }
    if tools_config.memory_forget.enabled {
        tools.push(Box::new(MemoryForgetTool::new(Arc::clone(memory))));
    }
    if tools_config.memory_governance.enabled {
        tools.push(Box::new(MemoryGovernanceTool::new(Arc::clone(memory))));
    }
}

fn append_browser_tools(
    tools: &mut Vec<Box<dyn Tool>>,
    browser_config: &crate::config::BrowserConfig,
    security: &Arc<SecurityPolicy>,
) {
    if !browser_config.enabled {
        return;
    }

    tools.push(Box::new(BrowserOpenTool::new(
        browser_config.allowed_domains.clone(),
    )));
    tools.push(Box::new(BrowserTool::new(
        Arc::clone(security),
        browser_config.allowed_domains.clone(),
        browser_config.session_name.clone(),
    )));
}

fn append_composio_tools(tools: &mut Vec<Box<dyn Tool>>, composio_key: Option<&str>) {
    if let Some(key) = composio_key
        && !key.is_empty()
    {
        tools.push(Box::new(ComposioTool::new(key)));
    }
}

fn append_delegation_tools(tools: &mut Vec<Box<dyn Tool>>) {
    tools.push(Box::new(DelegateTool::new()));
    tools.push(Box::new(SubagentSpawnTool::new()));
    tools.push(Box::new(SubagentOutputTool::new()));
    tools.push(Box::new(SubagentCancelTool::new()));
}

fn append_channel_tools(
    tools: &mut Vec<Box<dyn Tool>>,
    channel_capabilities: Option<&ChannelCapabilities>,
) {
    let Some(capabilities) = channel_capabilities else {
        return;
    };

    if capabilities.can_create_thread {
        tools.push(Box::new(ChannelCreateThreadTool::new()));
    }
    if capabilities.can_add_reaction {
        tools.push(Box::new(ChannelAddReactionTool::new()));
    }
    if capabilities.can_send_buttons || capabilities.can_send_embed {
        tools.push(Box::new(ChannelSendRichTool::new()));
    }
    if capabilities.can_fetch_history {
        tools.push(Box::new(ChannelGetHistoryTool::new()));
    }
    if capabilities.can_send_embed {
        tools.push(Box::new(ChannelSendEmbedTool::new()));
    }
}

fn append_codespace_tools(
    tools: &mut Vec<Box<dyn Tool>>,
    codespace_config: &crate::config::CodespaceConfig,
) {
    if codespace_config.enabled {
        tools.push(Box::new(super::CodespaceTool::new(
            codespace_config.clone(),
        )));
    }
}

#[cfg(feature = "taste")]
fn append_taste_tools(
    tools: &mut Vec<Box<dyn Tool>>,
    taste_config: &crate::config::TasteConfig,
    taste_provider: Option<Arc<dyn crate::core::providers::Provider>>,
    taste_model: &str,
    security: &Arc<SecurityPolicy>,
) {
    if taste_config.enabled {
        let Some(provider) = taste_provider else {
            append_unavailable_taste_tools(tools, "taste provider is unavailable");
            return;
        };
        let model = taste_model.to_string();
        match crate::core::taste::create_taste_engine(
            taste_config,
            provider,
            model,
            &security.workspace_dir,
        ) {
            Ok(engine) => {
                tools.push(Box::new(super::TasteEvaluateTool::new(Arc::clone(&engine))));
                tools.push(Box::new(super::TasteCompareTool::new(engine)));
            }
            Err(error) => {
                tracing::warn!(%error, "taste tools unavailable after backend initialization failure");
                append_unavailable_taste_tools(
                    tools,
                    format!("taste backend unavailable: {error}"),
                );
            }
        }
    }
}

pub(crate) fn append_mcp_tools(
    tools: &mut Vec<Box<dyn Tool>>,
    mcp_config: Option<&McpConfig>,
    security: &SecurityPolicy,
    mcp_tool_provider: &dyn McpToolProvider,
) {
    if let Some(config) = mcp_config {
        tools.extend(mcp_tool_provider.create_mcp_tools(config, security));
    }
}
