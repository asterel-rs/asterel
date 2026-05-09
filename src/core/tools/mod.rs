//! Tools subsystem — the agent's interface to the outside world.
//!
//! # Architecture
//!
//! Every capability the agent can exercise is implemented as a [`Tool`].
//! Calling a tool always goes through the [`ToolRegistry`], which runs the
//! request through the middleware chain before (and after) the tool itself
//! executes.  The canonical ordering of that chain is:
//!
//! ```text
//! SecurityMiddleware          ← autonomy-level, allowlist, isolation, capability checks
//! HookMiddleware              ← external shell hooks (WP-G2)
//! EntityRateLimitMiddleware   ← per-entity and global action-rate caps
//! AuditMiddleware             ← tracing spans for every invocation
//! SemanticCompactionMiddleware ← meaning-preserving reduction for known output kinds
//! ToolOutputCompactionMiddleware ← generic head+tail compaction fallback
//! OutputSizeLimitMiddleware   ← hard byte/line truncation ceiling
//! ToolResultSanitizationMiddleware ← prompt-injection defence (external-content markers)
//! SecretScrubMiddleware       ← API-key and token redaction
//! TaintMiddleware             ← taint-label propagation
//!          ↓
//! [Tool::execute]
//! ```
//!
//! # Security contract
//!
//! Tools that touch the filesystem (`file_read`, `file_write`) or the shell
//! (`shell`) enforce sandbox boundaries — workspace confinement, symlink
//! rejection, hard-link rejection, and environment stripping — inside the
//! tool implementation itself, in addition to the `SecurityMiddleware` checks
//! that run first.  Neither layer is sufficient on its own.
//!
//! # Adding a tool
//!
//! 1. Implement [`Tool`] in a new submodule.
//! 2. Add its name constant to [`middleware::tool_names`].
//! 3. Register it in [`factory`].
//! 4. Update [`middleware::security`]'s classification helpers if it touches
//!    the network, filesystem, or a protected bootstrap file.

pub mod browser;
pub mod browser_open;
pub mod channel;
/// Sandboxed `codespace` development environment tool.
pub mod codespace;
pub mod cognitive_context;
pub mod composio;
pub mod delegate;
/// Factory functions for assembling the default tool sets.
pub mod factory;
pub mod file_delete;
pub mod file_read;
pub(crate) mod file_tracker;
pub mod file_write;
pub mod introspection;
/// Memory tools: `store`, `recall`, `lookup`, `forget`, `correct`, governance.
pub mod memory;
/// Middleware chain and shared [`ExecutionContext`].
pub mod middleware;
/// [`ToolRegistry`]: stores tools and dispatches executions through the middleware pipeline.
pub mod registry;
mod schema_helpers;
pub mod shell;
pub mod subagent;
#[cfg(feature = "taste")]
pub mod taste;
pub mod traits;
pub mod web;

pub use browser::BrowserTool;
pub use browser_open::BrowserOpenTool;
pub use channel::{
    ChannelActionBroker, ChannelAddReactionTool, ChannelCreateThreadTool, ChannelGetHistoryTool,
    ChannelSendEmbedTool, ChannelSendRichTool,
};
pub use codespace::CodespaceTool;
pub use composio::ComposioTool;
pub use delegate::DelegateTool;
#[cfg(test)]
pub(crate) use factory::append_dynamic_tool_descriptions;
pub use factory::{
    ToolRegistryConfig, all_tools, build_tool_registry_from_parts, default_action_operator,
    default_tools, tool_desc_with_security, tool_desc_with_security_and_mcp_provider,
    tool_descriptions, tool_descriptions_with_mcp_provider,
};
pub use file_delete::FileDeleteTool;
pub use file_read::FileReadTool;
pub use file_write::FileWriteTool;
pub use introspection::{
    AdjustReasoningTool, AnnotateTurnTool, EvaluateConsistencyTool, FlagUncertaintyTool,
    IntrospectAffectTool, IntrospectExperienceTool, IntrospectPrinciplesTool,
    IntrospectRelationshipTool, IntrospectSelfModelTool,
};
pub use memory::{
    MemoryCorrectTool, MemoryForgetTool, MemoryGovernanceTool, MemoryLookupTool, MemoryRecallTool,
    MemoryStoreTool,
};
pub use middleware::{
    DEFAULT_MAX_DELEGATION_DEPTH, DEFAULT_ROOT_CHILD_DELEGATION_QUOTA,
    DEFAULT_SUBAGENT_CHILD_DELEGATION_QUOTA, ExecutionContext, MiddlewareDecision,
    ToolExecutionAuditRecord, ToolExecutionAuditSink, ToolMiddleware, default_middleware_chain,
};
pub use registry::ToolRegistry;
pub use shell::ShellTool;
pub use subagent::{SubagentCancelTool, SubagentOutputTool, SubagentSpawnTool};
#[cfg(feature = "taste")]
pub use taste::{TasteCompareTool, TasteEvaluateTool};
pub use traits::{
    ActionIntent, ActionOperator, ActionResult, AttachmentSource, McpToolProvider,
    NoopMcpToolProvider, NoopOperator, OutputAttachment, Tool, ToolResult,
    ToolResultCompactionTarget, ToolResultSemanticMetadata, ToolResultSemanticStats,
    ToolResultSemanticStreamMode, ToolResultTextField, ToolSpec,
};

#[cfg(test)]
mod tests;
