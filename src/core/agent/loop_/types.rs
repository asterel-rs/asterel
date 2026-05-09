//! Shared types for the agent turn loop.
//!
//! This module is the single source of truth for all data structures
//! threaded through the per-turn execution pipeline:
//!
//! - [`TurnCallAccounting`] — enforces the per-turn LLM call budget
//!   (answer call + optional reflect call).
//! - [`TurnExecutionOutcome`] — the value returned to the session layer
//!   after a successful turn.
//! - [`TurnPipelineContext`] — a cheap view over the session-wide
//!   dependencies (config, security, memory, params, observer)
//!   passed by reference into every pipeline stage.
//! - [`MainSessionTurnParams`] — all provider, model, and tooling
//!   parameters for the main interactive session.
//! - [`TurnParams`] — the public API type for integration callers
//!   (channel adapters, test harnesses).
//! - [`RuntimeMemoryWriteContext`] — entity + tenant-policy scope for
//!   all memory writes within a turn.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::broadcast;

use crate::config::Config;
use crate::config::LoopDetectionConfig;
use crate::contracts::ids::EntityId;
use crate::contracts::observability::Observer;
use crate::core::memory::Memory;
use crate::core::persona::person_identity::person_entity_id;
use crate::core::providers::{Provider, StreamSink};
use crate::core::subagents::{SkillMetadataProvider, SubagentOrchestrator};
use crate::core::tools::{ToolExecutionAuditSink, ToolRegistry};
use crate::security::policy::{EntityRateLimiter, TenantPolicyContext};
use crate::security::{ApprovalBroker, PermissionStore, SecurityPolicy};

/// Maximum LLM calls per turn when persona mode is active.
pub(super) const PERSONA_PER_TURN_CALL_BUDGET: u8 = 2;

/// Tracks the number of LLM calls consumed in a single turn
/// against the per-turn budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TurnCallAccounting {
    /// Maximum allowed calls this turn.
    pub(super) budget_limit: u8,
    /// Answer-phase calls consumed so far.
    pub(super) answer_calls: u8,
    /// Reflect-phase calls consumed so far.
    pub(super) reflect_calls: u8,
}

impl TurnCallAccounting {
    /// Create accounting with the appropriate budget for persona mode.
    pub(super) fn for_persona_mode(enabled: bool) -> Self {
        Self {
            budget_limit: if enabled {
                PERSONA_PER_TURN_CALL_BUDGET
            } else {
                1
            },
            answer_calls: 0,
            reflect_calls: 0,
        }
    }

    /// Total LLM calls consumed (answer + reflect).
    pub(super) fn total_calls(self) -> u8 {
        self.answer_calls + self.reflect_calls
    }

    /// Record one answer-phase call, failing if the budget is exceeded.
    ///
    /// # Errors
    ///
    /// Returns an error when the per-turn call budget is exceeded.
    pub(super) fn consume_answer_call(&mut self) -> Result<()> {
        self.answer_calls = self.answer_calls.saturating_add(1);
        self.ensure_budget()
    }

    /// Record one reflect-phase call, failing if the budget is exceeded.
    ///
    /// # Errors
    ///
    /// Returns an error when the per-turn call budget is exceeded.
    pub(super) fn consume_reflect_call(&mut self) -> Result<()> {
        self.reflect_calls = self.reflect_calls.saturating_add(1);
        self.ensure_budget()
    }

    fn ensure_budget(self) -> Result<()> {
        if self.total_calls() > self.budget_limit {
            anyhow::bail!(
                "persona per-turn call budget exceeded: consumed={} budget={}",
                self.total_calls(),
                self.budget_limit
            );
        }
        Ok(())
    }
}

/// Result of executing one turn: the response text, token usage,
/// and call accounting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TurnExecutionOutcome {
    /// The assistant's final response text.
    pub(super) response: String,
    /// Total tokens consumed, if the provider reported usage.
    pub(super) tokens_used: Option<u64>,
    /// Call budget accounting for this turn.
    pub(super) accounting: TurnCallAccounting,
}

/// Shared context threaded through the turn execution pipeline.
pub(super) struct TurnPipelineContext<'a> {
    /// Application configuration.
    pub(super) config: &'a Config,
    /// Active security policy for tool/action gating.
    pub(super) security: &'a SecurityPolicy,
    /// Memory backend for context recall and persistence.
    pub(super) mem: Arc<dyn Memory>,
    /// Turn-level parameters (providers, model, tools, etc.).
    pub(super) params: &'a MainSessionTurnParams<'a>,
    /// Runtime observability observer for event recording.
    pub(super) observer: &'a Arc<dyn Observer>,
}

/// Per-session parameters for the main session turn pipeline.
pub(super) struct MainSessionTurnParams<'a> {
    /// Provider used for the primary answer inference.
    pub(super) answer_provider: &'a dyn Provider,
    /// Provider used for the reflect/post-answer inference.
    pub(super) reflect_provider: &'a dyn Provider,
    /// Optional shared auxiliary provider for augmentor-side LLM helpers.
    pub(super) augmentor_provider: Option<Arc<dyn Provider>>,
    /// Optional sink for streaming token events.
    pub(super) stream_sink: Option<Arc<dyn StreamSink>>,
    /// Broadcast sender for interactive input.
    pub(super) interactive_input_tx: Option<broadcast::Sender<String>>,
    /// Broker for human-in-the-loop tool-call approval.
    pub(super) approval_broker: Option<Arc<dyn ApprovalBroker>>,
    /// Audit sink for tool execution records.
    pub(super) execution_audit_sink: Option<Arc<dyn ToolExecutionAuditSink>>,
    /// Person identity for memory scoping.
    pub(super) person_id: &'a str,
    /// System prompt injected into every inference call.
    pub(super) system_prompt: &'a str,
    /// Model name for inference.
    pub(super) model_name: &'a str,
    /// Sampling temperature for inference.
    pub(super) temperature: f64,
    /// Tool registry with all registered tools.
    pub(super) registry: Arc<ToolRegistry>,
    /// Hard cap on tool loop iterations per turn.
    pub(super) max_tool_iterations: u32,
    /// Loop-detection configuration shared with transport-facing turns.
    pub(super) loop_detection: LoopDetectionConfig,
    /// Per-entity rate limiter for action throttling.
    pub(super) rate_limiter: Arc<EntityRateLimiter>,
    /// Persistent permission store for tool approvals.
    pub(super) permission_store: Arc<PermissionStore>,
    /// Owned subagent runtime for delegation tools in the main session.
    pub(super) subagent_manager: Arc<SubagentOrchestrator>,
    /// Provider for resolving skill metadata used to build turn hint blocks
    /// injected into the enriched prompt during pre-answer enrichment.
    pub(super) skill_metadata_provider: Arc<dyn SkillMetadataProvider>,
}

/// Parameters for executing a single integration turn (gateway /
/// channel callers).
pub struct TurnParams<'a> {
    /// Application configuration.
    pub config: &'a Config,
    /// Active security policy.
    pub security: &'a SecurityPolicy,
    /// Memory backend.
    pub mem: Arc<dyn Memory>,
    /// Provider for the primary answer inference.
    pub answer_provider: &'a dyn Provider,
    /// Provider for reflect/post-answer inference.
    pub reflect_provider: &'a dyn Provider,
    /// System prompt for the turn.
    pub system_prompt: &'a str,
    /// Model name for inference.
    pub model_name: &'a str,
    /// Sampling temperature.
    pub temperature: f64,
    /// Entity ID for memory scoping.
    pub entity_id: &'a str,
    /// Tenant-level policy context for multi-tenant enforcement.
    pub policy_context: TenantPolicyContext,
    /// The user's input message for this turn.
    pub user_message: &'a str,
}

/// Entity-scoped context for memory writes, with tenant policy
/// enforcement.
#[derive(Debug, Clone)]
pub(crate) struct RuntimeMemoryWriteContext {
    /// Entity ID that memory writes are scoped to.
    pub(crate) entity_id: EntityId,
    /// Tenant policy context for write-scope enforcement.
    pub(crate) policy_context: TenantPolicyContext,
}

impl RuntimeMemoryWriteContext {
    /// Create a write context for a main-session person entity.
    pub(super) fn main_session_person(person_id: &str) -> Self {
        Self {
            entity_id: EntityId::new(person_entity_id(person_id)),
            policy_context: TenantPolicyContext::disabled(),
        }
    }

    /// Create a write context for an arbitrary entity with a tenant
    /// policy.
    pub(super) fn for_entity_with_policy(
        entity_id: impl AsRef<str>,
        policy_context: TenantPolicyContext,
    ) -> Self {
        Self {
            entity_id: EntityId::new(entity_id.as_ref()),
            policy_context,
        }
    }

    /// Verify the entity ID is within the tenant's allowed write
    /// scope.
    ///
    /// # Errors
    ///
    /// Returns an error if the entity ID violates the tenant policy.
    pub(crate) fn enforce_write_scope(&self) -> Result<()> {
        self.policy_context
            .enforce_recall_scope(self.entity_id.as_str())
            .map_err(anyhow::Error::msg)
    }
}
