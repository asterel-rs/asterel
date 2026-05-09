//! Tool execution middleware — security, policy, and shared context.
//!
//! # Middleware chain order
//!
//! The default chain is assembled by [`default_middleware_chain`] and runs
//! in this fixed order for every tool invocation:
//!
//! ```text
//! 1. SecurityMiddleware          — autonomy-level gates, tool allowlist,
//!                                  group isolation, capability checks,
//!                                  approval-flow routing (PolicyEngine or
//!                                  raw-grant + autonomy fallback)
//! 2. HookMiddleware              — external shell hooks (pre/post, WP-G2)
//! 3. EntityRateLimitMiddleware   — per-entity, burst, conversation, and
//!                                  workspace rate caps
//! 4. AuditMiddleware             — structured tracing spans
//! 5. SemanticCompactionMiddleware — registry-driven meaning-preserving
//!                                  reduction for known output kinds
//! 6. ToolOutputCompactionMiddleware — generic head+tail compaction at
//!                                     8 000 chars for unknown / fallback raw
//! 7. OutputSizeLimitMiddleware   — hard ceiling at 256 KB / 4 000 lines
//! 8. ToolResultSanitizationMiddleware — wraps output in external-content
//!                                       markers to resist prompt injection
//! 9. SecretScrubMiddleware       — regex-based API-key and token redaction
//! 10. TaintMiddleware            — taint-label propagation and attachment
//! ```
//!
//! `before_execute` hooks run in list order (1 → 10).
//! `after_execute` hooks run in the same list order (1 → 10) after the tool
//! returns; output-shaping middleware therefore applies bottom-up relative
//! to the before-phase.
//!
//! # `ExecutionContext`
//!
//! [`ExecutionContext`] is the read-only (with interior-mutable exceptions)
//! bag of state threaded through every middleware and tool invocation within
//! a single request.  It carries the security policy, entity identity,
//! workspace path, permission store, approval broker, rate limiter, trust
//! tracker, taint context, and delegation budget — everything a middleware
//! or tool needs without reaching into global state.

pub(crate) mod hook_types;
pub(crate) mod hooks;
mod orchestrator;
mod policy;
mod security;
mod semantic_compaction;
pub mod taint;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

pub use orchestrator::{
    ApprovalResolution, ToolExecutionOrchestrator, approval_request_from_intent,
    enforce_process_spawn_guardrails, enforce_shell_command_guardrails,
    request_approval_with_cache,
};
pub use policy::{
    AuditMiddleware, EntityRateLimitMiddleware, MAX_TOOL_OUTPUT_BYTES, MAX_TOOL_OUTPUT_LINES,
    OutputSizeLimitMiddleware, SecretScrubMiddleware, ToolOutputCompactionMiddleware,
    ToolResultSanitizationMiddleware,
};
pub use security::SecurityMiddleware;
pub use semantic_compaction::{
    SEMANTIC_COMPACTION_CONFIDENCE_FLOOR, SEMANTIC_COMPACTION_THRESHOLD_CHARS,
    SemanticCompactionMiddleware, SemanticCompactionOutcome, SemanticFormatter,
    SemanticFormatterRegistry, classify_shell_command_output_kind,
};
use serde_json::Value;
pub use taint::TaintMiddleware;

pub use crate::core::tools::traits::ToolResultTextField;
pub use hook_types::HookConfigSet;
pub use hooks::{HookAbortSignal, HookMiddleware};

use crate::config::GroupIsolationLevel;
use crate::contracts::ids::EntityId;
use crate::contracts::observability::{NoopObserver, Observer};
use crate::core::memory::Memory;
use crate::core::tools::traits::{ActionIntent, ToolResult};
use crate::security::capability::CapabilitySet;
use crate::security::domain_trust::DomainTrustTracker;
use crate::security::policy::{AutonomyLevel, EntityRateLimiter, TenantPolicyContext};
use crate::security::taint::label::TaintSet;
use crate::security::{ApprovalBroker, PermissionStore, SecurityPolicy};

/// Process-wide [`DomainTrustTracker`] singleton shared across all sessions.
///
/// A single tracker is sufficient because trust scores are scoped by tool
/// name and do not carry per-session state.  Using a singleton avoids
/// allocating one tracker per session and allows cross-session trust signals
/// to accumulate over the lifetime of the process.
static GLOBAL_DOMAIN_TRUST_TRACKER: OnceLock<Arc<DomainTrustTracker>> = OnceLock::new();

/// Return the process-wide [`DomainTrustTracker`] singleton, creating it on first call.
pub fn global_trust_tracker() -> Arc<DomainTrustTracker> {
    Arc::clone(GLOBAL_DOMAIN_TRUST_TRACKER.get_or_init(|| Arc::new(DomainTrustTracker::new())))
}

/// Default confidence threshold for intent classification.
pub const DEFAULT_INTENT_CLASSIFIER_THRESHOLD: f32 = 0.85;
/// Root agent default direct-child delegation quota.
pub const DEFAULT_ROOT_CHILD_DELEGATION_QUOTA: u32 = 4;
/// Subagent default direct-child delegation quota.
pub const DEFAULT_SUBAGENT_CHILD_DELEGATION_QUOTA: u32 = 2;
/// Maximum allowed delegation depth. Root agents operate at depth 0.
pub const DEFAULT_MAX_DELEGATION_DEPTH: u8 = 2;

/// File names that must never be overwritten or deleted by file tools.
const CRITICAL_BOOTSTRAP_PROTECTED_TARGETS: [&str; 4] =
    ["SOUL.md", "CHARACTER.md", "USER.md", "AGENTS.md"];

// Tool name constants used for security classification and middleware routing.
//
// When adding a new tool:
// 1. Add a constant here.
// 2. Update `is_network_boundary_tool` if the tool crosses the network boundary,
//    and `is_external_action_tool` if it writes/posts/mutates external state.
// 3. Update `check_global_policy` in `middleware::security` if the tool
//    requires special handling under `ReadOnly` autonomy.
pub(crate) mod tool_names {
    pub(crate) const SHELL: &str = "shell";
    pub(crate) const FILE_READ: &str = "file_read";
    pub(crate) const FILE_WRITE: &str = "file_write";
    pub(crate) const FILE_DELETE: &str = "file_delete";
    pub(crate) const MEMORY_STORE: &str = "memory_store";
    pub(crate) const MEMORY_RECALL: &str = "memory_recall";
    pub(crate) const MEMORY_LOOKUP: &str = "memory_lookup";
    pub(crate) const MEMORY_CORRECT: &str = "memory_correct";
    pub(crate) const MEMORY_FORGET: &str = "memory_forget";
    pub(crate) const MEMORY_GOVERNANCE: &str = "memory_governance";
    pub(crate) const DELEGATE: &str = "delegate";
    pub(crate) const SUBAGENT_SPAWN: &str = "subagent_spawn";
    pub(crate) const SUBAGENT_OUTPUT: &str = "subagent_output";
    pub(crate) const SUBAGENT_CANCEL: &str = "subagent_cancel";
    pub(crate) const CODESPACE: &str = "codespace";

    // Network-boundary tools — blocked in network-isolated groups.
    pub(crate) const BROWSER: &str = "browser";
    pub(crate) const BROWSER_OPEN: &str = "browser_open";
    pub(crate) const WEB_FETCH: &str = "web_fetch";
    pub(crate) const WEB_SEARCH: &str = "web_search";
    pub(crate) const WEB_SCRAPE: &str = "web_scrape";
    pub(crate) const WEB_SUMMARIZE: &str = "web_summarize";
    pub(crate) const WEBSEARCH: &str = "websearch";
    pub(crate) const DUCKDUCKGO_SEARCH: &str = "duckduckgo_search";

    // External action tools — network-boundary AND require `ExternalActionExecution::Enabled`.
    pub(crate) const COMPOSIO: &str = "composio";

    /// Common prefix for all MCP-sourced tools.  Any tool whose name starts
    /// with this string is treated as an external action tool.
    pub(crate) const MCP_PREFIX: &str = "mcp_";

    pub(crate) const CHANNEL_CREATE_THREAD: &str = "channel_create_thread";
    pub(crate) const CHANNEL_ADD_REACTION: &str = "channel_add_reaction";
    pub(crate) const CHANNEL_SEND_RICH: &str = "channel_send_rich";
    pub(crate) const CHANNEL_GET_HISTORY: &str = "channel_get_history";
    pub(crate) const CHANNEL_SEND_EMBED: &str = "channel_send_embed";

    #[cfg(feature = "taste")]
    pub(crate) const TASTE_EVALUATE: &str = "taste_evaluate";
    #[cfg(feature = "taste")]
    pub(crate) const TASTE_COMPARE: &str = "taste_compare";

    pub(crate) const INTROSPECT_AFFECT: &str = "introspect_affect";
    pub(crate) const INTROSPECT_RELATIONSHIP: &str = "introspect_relationship";
    pub(crate) const INTROSPECT_SELF_MODEL: &str = "introspect_self_model";
    pub(crate) const INTROSPECT_PRINCIPLES: &str = "introspect_principles";
    pub(crate) const INTROSPECT_EXPERIENCE: &str = "introspect_experience";
    pub(crate) const ADJUST_REASONING: &str = "adjust_reasoning";
    pub(crate) const FLAG_UNCERTAINTY: &str = "flag_uncertainty";
    pub(crate) const ANNOTATE_TURN: &str = "annotate_turn";
    pub(crate) const EVALUATE_CONSISTENCY: &str = "evaluate_consistency";
}

fn is_critical_bootstrap_target(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|filename| {
            CRITICAL_BOOTSTRAP_PROTECTED_TARGETS
                .iter()
                .any(|blocked| filename.eq_ignore_ascii_case(blocked))
        })
}

fn is_external_action_tool(tool_name: &str) -> bool {
    tool_name == tool_names::COMPOSIO
        || tool_name.starts_with(tool_names::MCP_PREFIX)
        || matches!(
            tool_name,
            tool_names::CHANNEL_CREATE_THREAD
                | tool_names::CHANNEL_ADD_REACTION
                | tool_names::CHANNEL_SEND_RICH
                | tool_names::CHANNEL_SEND_EMBED
        )
}

fn is_network_boundary_tool(tool_name: &str) -> bool {
    is_external_action_tool(tool_name)
        || matches!(
            tool_name,
            tool_names::BROWSER
                | tool_names::BROWSER_OPEN
                | tool_names::WEB_FETCH
                | tool_names::WEB_SEARCH
                | tool_names::WEB_SCRAPE
                | tool_names::WEBSEARCH
                | tool_names::DUCKDUCKGO_SEARCH
                | tool_names::CHANNEL_GET_HISTORY
        )
}

/// Structured record of a single tool execution for audit logging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolExecutionAuditRecord {
    /// Name of the tool that was executed.
    pub tool_name: String,
    /// Truncated summary of the tool's input arguments.
    pub args_summary: String,
    /// Whether the tool execution succeeded.
    pub success: bool,
    /// Brief text summary of the outcome.
    pub summary: String,
}

/// Sink for persisting tool execution audit records.
pub trait ToolExecutionAuditSink: Send + Sync {
    /// Persist a single tool execution audit record.
    fn record_tool_execution<'a>(
        &'a self,
        record: &'a ToolExecutionAuditRecord,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

    /// Persist a governance approval audit record.
    fn record_governance_approval<'a>(
        &'a self,
        _record: &'a crate::security::governance::GovernanceAuditRecord,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }
}

/// Immutable (except for interior-mutable fields) context threaded through
/// every middleware and tool invocation within a single agent request.
///
/// A new `ExecutionContext` is typically cloned from a per-surface template
/// with `entity_id`, `turn_number`, and session-specific overrides applied.
/// The `security`, `rate_limiter`, and `permission_store` fields are
/// `Arc`-wrapped so they can be shared cheaply across concurrent calls.
#[derive(Clone)]
pub struct ExecutionContext {
    /// Security policy governing what the agent is allowed to do.
    pub security: Arc<SecurityPolicy>,
    /// Autonomy level for this invocation — drives the approval-flow branch.
    pub autonomy_level: AutonomyLevel,
    /// Identifier for the requesting entity (user, channel, etc.).
    pub entity_id: EntityId,
    /// Current conversation turn number.
    pub turn_number: u32,
    /// Shared memory backend for memory-aware tools and runtime event recording.
    pub memory: Option<Arc<dyn Memory>>,
    /// Runtime observer for structured turn/tool events.
    pub observer: Arc<dyn Observer>,
    /// Session identifier for the current execution, when the surface provides one.
    pub session_id: Option<String>,
    /// Per-turn system prompt propagated to delegated subagents when present.
    pub delegation_system_prompt: Option<String>,
    /// Root workspace directory for file operations.
    pub workspace_dir: PathBuf,
    /// Whitelist of tool names this context may invoke, if set.
    pub allowed_tools: Option<HashSet<String>>,
    /// Persistent per-tool permission grants, if available.
    pub permission_store: Option<Arc<PermissionStore>>,
    /// Per-entity rate limiter for tool invocations.
    pub rate_limiter: Arc<EntityRateLimiter>,
    /// Multi-tenant policy context for quota enforcement.
    pub tenant_context: TenantPolicyContext,
    /// Interactive approval broker for gated actions.
    pub approval_broker: Option<Arc<dyn ApprovalBroker>>,
    /// Channel-specific action broker for reactions, threads, etc.
    pub channel_action_broker: Option<Arc<dyn crate::core::tools::channel::ChannelActionBroker>>,
    /// Name of the originating channel (e.g. `"discord"`).
    pub source_channel: Option<String>,
    /// Channel-specific identifier for the source conversation.
    pub source_channel_id: Option<String>,
    /// Audit sink for recording tool execution outcomes.
    pub execution_audit_sink: Option<Arc<dyn ToolExecutionAuditSink>>,
    /// Trust tracker for recording per-domain success/violation signals.
    pub trust_tracker: Option<Arc<DomainTrustTracker>>,
    /// Owned subagent runtime for delegation tools on this surface.
    pub subagent_manager: Option<Arc<crate::core::subagents::SubagentOrchestrator>>,
    /// Routing group for multi-group isolation.
    pub routing_group: Option<String>,
    /// Process-level isolation for the routing group.
    pub process_isolation: GroupIsolationLevel,
    /// Network-level isolation for the routing group.
    pub network_isolation: GroupIsolationLevel,
    /// Optional intent classifier for risk assessment.
    pub intent_classifier: Option<Arc<dyn crate::security::intent_classifier::IntentClassifier>>,
    /// Confidence threshold for the intent classifier.
    pub intent_classifier_threshold: f32,
    /// Capabilities granted to this execution context.
    /// `None` means all capabilities are granted (no restriction).
    pub granted_capabilities: Option<CapabilitySet>,
    /// Current taint labels propagated from previous tool executions.
    pub taint_context: Option<TaintSet>,
    /// Cognitive context for introspective tools (present only in persona-enabled sessions).
    pub(crate) cognitive_context:
        Option<Arc<crate::core::tools::cognitive_context::CognitiveContext>>,
    /// Required capabilities for the current tool invocation.
    /// Set by the tool registry before middleware runs.
    pub current_tool_capabilities: Vec<crate::security::capability::Capability>,
    /// Current delegation nesting depth. Root agents operate at depth 0.
    pub delegation_depth: u8,
    /// Maximum allowed delegation depth for nested child agents.
    pub max_delegation_depth: u8,
    /// Direct-child quota granted to each newly spawned child agent.
    pub child_delegation_quota: u32,
    /// Remaining direct-child delegation budget for this execution context.
    pub remaining_child_delegations: Arc<AtomicU32>,
    /// Optional policy engine for rule-based tool approval decisions.
    /// When present, consulted before the raw grant / autonomy fallback.
    pub policy_engine: Option<Arc<crate::security::tool_policy::PolicyEngine>>,
    /// In-session memory access log for audit / diagnostic purposes (WP-I1).
    /// Wired by the runtime when a session-scoped log is desired.
    pub memory_access_log: Option<Arc<crate::core::memory::governance::MemoryAccessLog>>,
}

impl ExecutionContext {
    /// Build a minimal context from a security policy with safe defaults.
    ///
    /// No memory, session ID, approval broker, or rate limiter overrides are
    /// set.  Intended as a base that callers extend with surface-specific
    /// values (see [`Self::runtime_root`]).
    #[must_use]
    pub fn from_security(security: Arc<SecurityPolicy>) -> Self {
        Self {
            workspace_dir: security.workspace_dir.clone(),
            autonomy_level: security.autonomy,
            security,
            entity_id: EntityId::new("default"),
            turn_number: 0,
            memory: None,
            observer: Arc::new(NoopObserver),
            session_id: None,
            delegation_system_prompt: None,
            allowed_tools: None,
            permission_store: None,
            rate_limiter: Arc::new(EntityRateLimiter::new(100, 20)),
            tenant_context: TenantPolicyContext::disabled(),
            approval_broker: None,
            channel_action_broker: None,
            source_channel: None,
            source_channel_id: None,
            execution_audit_sink: None,
            trust_tracker: None,
            subagent_manager: None,
            routing_group: None,
            process_isolation: GroupIsolationLevel::Shared,
            network_isolation: GroupIsolationLevel::Shared,
            intent_classifier: None,
            intent_classifier_threshold: DEFAULT_INTENT_CLASSIFIER_THRESHOLD,
            granted_capabilities: None,
            taint_context: None,
            cognitive_context: None,
            current_tool_capabilities: Vec::new(),
            delegation_depth: 0,
            max_delegation_depth: DEFAULT_MAX_DELEGATION_DEPTH,
            child_delegation_quota: DEFAULT_SUBAGENT_CHILD_DELEGATION_QUOTA,
            remaining_child_delegations: Arc::new(AtomicU32::new(
                DEFAULT_ROOT_CHILD_DELEGATION_QUOTA,
            )),
            policy_engine: None,
            memory_access_log: None,
        }
    }

    /// Build a fully-wired root context for runtime surfaces (agent, gateway, channels).
    ///
    /// Compared to [`Self::from_security`], this additionally:
    /// - auto-loads `PolicyEngine` from `{workspace_dir}/policy.toml`,
    ///   falling back to an empty engine if the file is absent or malformed.
    /// - wires in the process-wide [`DomainTrustTracker`] singleton.
    #[must_use]
    pub fn runtime_root(
        security: Arc<SecurityPolicy>,
        workspace_dir: PathBuf,
        rate_limiter: Arc<EntityRateLimiter>,
        permission_store: Option<Arc<PermissionStore>>,
        tenant_context: TenantPolicyContext,
    ) -> Self {
        let mut ctx = Self::from_security(security);

        // Auto-load PolicyEngine from {workspace_dir}/policy.toml
        let policy_engine =
            crate::security::tool_policy::PolicyEngine::load_from_workspace(&workspace_dir)
                .unwrap_or_else(|e| {
                    tracing::warn!(%e, "failed to load policy.toml, using empty policy engine");
                    crate::security::tool_policy::PolicyEngine::empty()
                });
        ctx.policy_engine = Some(Arc::new(policy_engine));

        // Use the process-wide DomainTrustTracker singleton
        ctx.trust_tracker = Some(global_trust_tracker());

        ctx.workspace_dir = workspace_dir;
        ctx.rate_limiter = rate_limiter;
        ctx.permission_store = permission_store;
        ctx.tenant_context = tenant_context;
        ctx.memory_access_log = Some(Arc::new(
            crate::core::memory::governance::MemoryAccessLog::new(),
        ));
        ctx
    }

    /// Read the remaining direct-child delegation budget.
    #[must_use]
    pub fn remaining_child_delegations(&self) -> u32 {
        self.remaining_child_delegations.load(Ordering::Relaxed)
    }

    /// Atomically decrement the child delegation budget, returning `true` if a
    /// slot was available and consumed.  Returns `false` when the budget is
    /// exhausted without modifying the counter.
    #[must_use]
    pub fn try_consume_child_delegation_slot(&self) -> bool {
        let mut current = self.remaining_child_delegations.load(Ordering::Relaxed);
        loop {
            if current == 0 {
                return false;
            }
            match self.remaining_child_delegations.compare_exchange(
                current,
                current - 1,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(next) => current = next,
            }
        }
    }
}

#[cfg(test)]
impl ExecutionContext {
    #[must_use]
    pub fn test_default(security: Arc<SecurityPolicy>) -> Self {
        let mut ctx = Self::from_security(security);
        ctx.entity_id = "test:default".into();
        ctx.memory = None;
        ctx.session_id = None;
        ctx.delegation_system_prompt = None;
        ctx
    }

    #[must_use]
    pub fn with_autonomy(mut self, level: AutonomyLevel) -> Self {
        self.autonomy_level = level;
        self
    }

    #[must_use]
    pub fn with_entity(mut self, id: &str) -> Self {
        self.entity_id = id.into();
        self
    }

    #[must_use]
    pub fn with_workspace(mut self, dir: PathBuf) -> Self {
        self.workspace_dir = dir;
        self
    }

    #[must_use]
    pub fn with_allowed_tools(mut self, tools: HashSet<String>) -> Self {
        self.allowed_tools = Some(tools);
        self
    }

    #[must_use]
    pub fn with_source_channel(mut self, channel: &str) -> Self {
        self.source_channel = Some(channel.to_string());
        self
    }

    #[must_use]
    pub fn with_delegation_limits(
        mut self,
        depth: u8,
        max_depth: u8,
        child_quota: u32,
        remaining: u32,
    ) -> Self {
        self.delegation_depth = depth;
        self.max_delegation_depth = max_depth;
        self.child_delegation_quota = child_quota;
        self.remaining_child_delegations = Arc::new(AtomicU32::new(remaining));
        self
    }

    #[must_use]
    pub fn with_process_isolation(mut self, level: GroupIsolationLevel) -> Self {
        self.process_isolation = level;
        self
    }

    #[must_use]
    pub fn with_network_isolation(mut self, level: GroupIsolationLevel) -> Self {
        self.network_isolation = level;
        self
    }
}

/// Decision returned by a middleware's `before_execute` hook.
#[derive(Debug)]
pub enum MiddlewareDecision {
    /// Allow execution to continue to the next middleware or the tool itself.
    Continue,
    /// Abort execution and surface the reason as a failed [`ToolResult`].
    Block(String),
    /// Pause execution and route the intent through the approval flow.
    ///
    /// The orchestrator will call [`ApprovalBroker::request_approval`] with a
    /// request derived from the enclosed [`ActionIntent`].  If approved,
    /// execution resumes; if denied, a blocked result is returned.
    RequireApproval(ActionIntent),
}

/// Two-phase middleware hook invoked around every tool execution.
///
/// `before_execute` runs in list order before the tool.  It may block or
/// require approval.  `after_execute` runs in list order after the tool and
/// may mutate the result (truncation, scrubbing, taint annotation).
///
/// Implementations must be `Send + Sync` and `Debug`-able (for diagnostics).
pub trait ToolMiddleware: Send + Sync + std::fmt::Debug {
    /// Stable middleware type name for diagnostics and ordering assertions.
    fn middleware_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Whether this middleware's post-processing hook should run for results
    /// produced before the tool body executes (for example security blocks or
    /// approval denials). Pure result shaping middleware should keep the
    /// default `true`; side-effecting post-tool hooks should return `false` so
    /// blocked calls are not reported as completed tool executions.
    fn runs_after_pre_execution_return(&self) -> bool {
        true
    }

    /// Inspect the call and return a routing decision.
    ///
    /// # Errors
    ///
    /// Returns `Err` only for infrastructure failures (e.g. I/O errors inside
    /// a hook subprocess).  Policy decisions are expressed via
    /// [`MiddlewareDecision::Block`], not errors.
    fn before_execute<'a>(
        &'a self,
        tool_name: &'a str,
        args: &'a Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<MiddlewareDecision>> + Send + 'a>>;

    /// Post-process or annotate the tool result.
    ///
    /// Called in list order after the tool returns.  Mutations are cumulative:
    /// later middleware sees changes made by earlier middleware.
    fn after_execute<'a>(
        &'a self,
        tool_name: &'a str,
        result: &'a mut ToolResult,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

/// Assemble the default middleware pipeline.
///
/// The chain order is fixed and documented in the module-level doc comment.
/// Surfaces that need a different set can build their own chain and pass it
/// to [`ToolRegistry::new`].
#[must_use]
pub fn default_middleware_chain() -> Vec<Arc<dyn ToolMiddleware>> {
    vec![
        Arc::new(SecurityMiddleware),
        Arc::new(HookMiddleware::new(
            HookConfigSet::default(),
            HookAbortSignal::new(),
        )),
        Arc::new(EntityRateLimitMiddleware),
        Arc::new(AuditMiddleware),
        Arc::new(SemanticCompactionMiddleware::default()),
        Arc::new(ToolOutputCompactionMiddleware),
        Arc::new(OutputSizeLimitMiddleware),
        Arc::new(ToolResultSanitizationMiddleware),
        Arc::new(SecretScrubMiddleware),
        Arc::new(TaintMiddleware),
    ]
}
