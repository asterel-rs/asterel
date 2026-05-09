//! Core security policy: autonomy levels, command/path validation,
//! rate limiting, cost tracking, and tenant isolation.
//!
//! This module defines [`SecurityPolicy`] — the per-session configuration
//! struct that governs what the agent is allowed to do — and the supporting
//! types and trackers that enforce those limits at runtime.
//!
//! # Autonomy levels
//!
//! [`AutonomyLevel`] is the primary dial that controls agent behaviour.  The
//! three levels map to distinct tool-access patterns:
//!
//! | Level | Behaviour |
//! |-------|-----------|
//! | `ReadOnly` | No tool execution at all; `can_act()` returns `false`.  Used for pure-inference or display-only contexts. |
//! | `Supervised` (default) | Read-only tools (`file_read`, `memory_recall`, etc.) run without approval; write and high-risk tools are held for human approval via [`ApprovalBroker`].  The `PolicyEngine` returns `RequireApproval` for these in the absence of an explicit allow rule. |
//! | `Full` | All tools execute without approval (autonomous mode).  Intended for trusted CI/automation pipelines where no human is in the loop. |
//!
//! The level is set from config at session start and cannot be upgraded
//! at runtime — it can only be demoted by the domain trust tracker if the agent
//! exhibits anomalous behaviour (see `domain_trust`).
//!
//! # Tool access pipeline
//!
//! `SecurityPolicy` provides the autonomy level consumed by the `PolicyEngine`
//! as its final fallback tier.  The full decision chain is:
//!
//! ```text
//! PolicyEngine::evaluate(tool, args, has_grant, autonomy)
//!   1. Explicit deny rules  → Deny (hard block)
//!   2. Explicit ask rules   → RequireApproval → ApprovalBroker
//!   3. Explicit allow rules → Allow (bypass approval)
//!   4. Autonomy fallback    → ReadOnly=Deny | Supervised=Ask | Full=Allow
//!   5. Default              → Deny
//! ```
//!
//! # Rate limiting and cost tracking
//!
//! [`ActionTracker`] counts tool executions in a rolling one-hour window.
//! [`CostTracker`] accumulates estimated cost in cents with an automatic
//! day-boundary rollover.  Both limits are enforced by [`consume_action_cost`],
//! which records the action and returns an error if either budget is exceeded.
//! The limits are intentionally conservative by default (300 actions/hour,
//! 500 cents/day) and should be widened in config only for trusted, monitored
//! deployments.
//!
//! # Workspace containment
//!
//! `workspace_dir` and `workspace_only` restrict file operations to a single
//! directory tree.  Combined with `forbidden_paths`, this prevents the agent
//! from reading sensitive system files or writing outside its designated
//! workspace even when `AutonomyLevel::Full` is active.
//!
//! # Tenant isolation
//!
//! [`TenantPolicyContext`] enforces multi-tenant memory isolation.  When
//! `tenant_mode_enabled = true`, every memory recall must target an entity ID
//! that is equal to or hierarchically under the active `tenant_id`.
//! Default-scoped or empty entity IDs are rejected outright to prevent
//! accidental cross-tenant data leakage.  Tenant context is separate from the
//! tool-execution policy and is enforced by the memory layer, not here.
//!
//! [`ApprovalBroker`]: crate::security::ApprovalBroker
//! [`consume_action_cost`]: SecurityPolicy::consume_action_cost

mod command;
mod path;
mod trackers;
mod types;

use std::path::{Path, PathBuf};

pub use crate::contracts::tenant::{
    TENANT_DEFAULT_SCOPE_FALLBACK_DENIED_ERROR, TENANT_RECALL_CROSS_SCOPE_DENIED_ERROR,
    TenantPolicyContext,
};
pub use trackers::{ActionTracker, CostTracker, EntityRateLimiter, RateLimitError};
pub use types::{ActionPolicyVerdict, AutonomyLevel, ExternalActionExecution};

pub(crate) use crate::contracts::strings::verdicts::{
    ACTION_LIMIT_EXCEEDED_ERROR, COST_LIMIT_EXCEEDED_ERROR,
};

/// Security policy enforced on all tool executions.
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    /// Current autonomy level (`ReadOnly`, Supervised, Full).
    pub autonomy: AutonomyLevel,
    /// Whether external action execution is allowed.
    pub external_action_execution: ExternalActionExecution,
    /// Root directory for workspace containment checks.
    pub workspace_dir: PathBuf,
    /// If true, restrict file operations to the workspace directory.
    pub workspace_only: bool,
    /// Shell commands allowed by the security policy.
    pub allowed_commands: Vec<String>,
    /// Filesystem paths blocked by the security policy.
    pub forbidden_paths: Vec<String>,
    /// Maximum tool actions permitted per rolling hour window.
    pub max_actions_per_hour: u32,
    /// Maximum cost in cents permitted per calendar day.
    pub max_cost_per_day_cents: u32,
    /// Sliding-window action tracker for rate limiting.
    pub tracker: ActionTracker,
    /// Daily cost accumulator with automatic day rollover.
    pub cost_tracker: CostTracker,
}

impl Default for SecurityPolicy {
    fn default() -> Self {
        Self {
            autonomy: AutonomyLevel::Supervised,
            external_action_execution: ExternalActionExecution::Disabled,
            workspace_dir: PathBuf::from("."),
            workspace_only: true,
            allowed_commands: crate::contracts::security::default_allowed_commands(),
            forbidden_paths: crate::contracts::security::default_forbidden_paths(),
            max_actions_per_hour: 300,
            max_cost_per_day_cents: 500,
            tracker: ActionTracker::new(),
            cost_tracker: CostTracker::new(),
        }
    }
}

impl SecurityPolicy {
    /// Check if autonomy level permits any action at all
    #[must_use]
    pub fn can_act(&self) -> bool {
        self.autonomy != AutonomyLevel::ReadOnly
    }

    /// Record an action and check if the rate limit has been exceeded.
    /// Returns `true` if the action is allowed, `false` if rate-limited.
    #[must_use]
    pub fn record_action(&self) -> bool {
        let count = self.tracker.record();
        count <= self.max_actions_per_hour as usize
    }

    /// Check if the rate limit would be exceeded without recording.
    #[must_use]
    pub fn is_rate_limited(&self) -> bool {
        self.tracker.count_active() >= self.max_actions_per_hour as usize
    }

    /// # Errors
    ///
    /// Returns an error when action or cost budgets are exceeded.
    pub fn consume_action_cost(&self, estimated_cost_cents: u32) -> Result<(), &'static str> {
        if !self.record_action() {
            return Err(ACTION_LIMIT_EXCEEDED_ERROR);
        }

        if !self
            .cost_tracker
            .record(estimated_cost_cents, self.max_cost_per_day_cents)
        {
            return Err(COST_LIMIT_EXCEEDED_ERROR);
        }

        Ok(())
    }

    /// Build from config sections
    #[must_use]
    pub fn from_config(
        autonomy_config: &crate::config::AutonomyConfig,
        workspace_dir: &Path,
    ) -> Self {
        Self::from_config_runtime(
            autonomy_config,
            &crate::config::RuntimeConfig::default(),
            workspace_dir,
        )
    }

    /// Build from autonomy, runtime config, and workspace path.
    #[must_use]
    pub fn from_config_runtime(
        autonomy_config: &crate::config::AutonomyConfig,
        runtime_config: &crate::config::RuntimeConfig,
        workspace_dir: &Path,
    ) -> Self {
        let workspace_only = runtime_config.resolved_workspace_only(autonomy_config.workspace_only);
        Self {
            autonomy: autonomy_config.effective_autonomy_lvl(),
            external_action_execution: autonomy_config.external_action_execution,
            workspace_dir: workspace_dir.to_path_buf(),
            workspace_only,
            allowed_commands: autonomy_config.allowed_commands.clone(),
            forbidden_paths: autonomy_config.forbidden_paths.clone(),
            max_actions_per_hour: autonomy_config.max_actions_per_hour,
            max_cost_per_day_cents: autonomy_config.max_cost_per_day_cents,
            tracker: ActionTracker::new(),
            cost_tracker: CostTracker::new(),
        }
    }
}

#[cfg(test)]
mod tests;
