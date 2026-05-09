//! Session-level turn execution with verify-repair escalation.
//!
//! This module is the boundary between the top-level `run` entry point
//! and the per-turn post-turn pipeline.  Its primary responsibility is
//! **resilience**: wrapping each turn attempt in an exponential-backoff
//! retry loop that distinguishes transient provider failures (network
//! timeouts, 429 rate limits) from permanent ones (quota exhaustion,
//! 4xx errors) and only retries where recovery is plausible.
//!
//! # Session lifecycle
//!
//! ```text
//! execute_main_session_turn_with_metrics()
//!     │  (builds TurnPipelineContext, resolves person_id)
//!     ▼
//! execute_main_session_turn_with_policy_outcome()
//!     │  ┌──────────────── retry loop ─────────────────────┐
//!     │  │  attempt N                                       │
//!     │  ▼                                                  │
//!     │  execute_main_session_turn_with_accounting()        │
//!     │  │  ├─ Ok(outcome)  ──────────────────────────────►│ return
//!     │  │  └─ Err(e)                                      │
//!     │  │       ├─ analyze_verify_failure(e)              │
//!     │  │       ├─ decide_verify_repair_escalation(...)   │
//!     │  │       │    ├─ escalate? ──► emit event, bail    │
//!     │  │       │    └─ retry?   ──► exponential backoff  │
//!     │  └──────────────────────────────────────────────────┘
//! ```
//!
//! The hard cap `effective_cap = min(config_cap, 10)` is a safety net
//! that prevents infinite retries even if the configuration is
//! misconfigured.

use std::sync::Arc;

use anyhow::Result;

use super::session_posturn::execute_main_session_turn_with_accounting;
use super::types::{
    MainSessionTurnParams, RuntimeMemoryWriteContext, TurnExecutionOutcome, TurnParams,
    TurnPipelineContext,
};
use super::verify_repair::{
    VerifyRepairCaps, analyze_verify_failure, decide_verify_repair_escalation,
    emit_verify_repair_escalation_event,
};
use crate::config::Config;
use crate::contracts::observability::NoopObserver;
use crate::contracts::observability::Observer;
use crate::core::memory::Memory;
use crate::core::persona::person_identity::resolve_person_id;
use crate::core::providers::ThinkingLevel;
use crate::core::providers::response::ProviderMessage;
use crate::core::subagents::NoopSkillMetadataProvider;
use crate::security::policy::{EntityRateLimiter, TenantPolicyContext};
use crate::security::{PermissionStore, SecurityPolicy};

/// Per-turn execution settings controlling conversation history,
/// thinking mode, and ephemeral behavior.
#[derive(Clone, Copy)]
pub(super) struct TurnExecutionSettings<'a> {
    /// Prior conversation messages for multi-turn context.
    pub(super) conversation_history: &'a [ProviderMessage],
    /// Extended thinking level for the inference call.
    pub(super) thinking_level: ThinkingLevel,
    /// Whether to expose reasoning traces in output.
    pub(super) show_reasoning: bool,
    /// When true, skip memory recall and auto-save to prevent
    /// cross-session contamination (used by single-message `-m`
    /// mode).
    pub(super) ephemeral: bool,
}

#[cfg(test)]
pub(super) async fn run_main_turn(
    config: &Config,
    security: &SecurityPolicy,
    mem: Arc<dyn Memory>,
    params: &MainSessionTurnParams<'_>,
    user_message: &str,
    observer: &Arc<dyn Observer>,
) -> Result<String> {
    execute_main_session_turn_with_metrics(
        config,
        security,
        mem,
        params,
        user_message,
        observer,
        TurnExecutionSettings {
            conversation_history: &[],
            thinking_level: ThinkingLevel::Off,
            show_reasoning: false,
            ephemeral: false,
        },
    )
    .await
    .map(|outcome| outcome.response)
}

/// Execute one main-session turn with metrics collection and
/// verify-repair escalation.
///
/// # Errors
///
/// Returns an error if turn execution or verify-repair exhausts
/// its retry budget.
pub(super) async fn execute_main_session_turn_with_metrics(
    config: &Config,
    security: &SecurityPolicy,
    mem: Arc<dyn Memory>,
    params: &MainSessionTurnParams<'_>,
    user_message: &str,
    observer: &Arc<dyn Observer>,
    settings: TurnExecutionSettings<'_>,
) -> Result<TurnExecutionOutcome> {
    let ctx = TurnPipelineContext {
        config,
        security,
        mem,
        params,
        observer,
    };
    execute_main_session_turn_with_policy_outcome(
        &ctx,
        user_message,
        RuntimeMemoryWriteContext::main_session_person(params.person_id),
        settings,
    )
    .await
}

/// Execute one turn with the full verify-repair retry loop.
///
/// `write_context` scopes all memory writes to the correct entity and
/// tenant policy.  `settings` controls the multi-turn conversation
/// history, extended thinking, and ephemeral mode.
///
/// # Errors
///
/// Returns an error if all retry attempts are exhausted or if a
/// non-retryable failure is detected (quota, 4xx, policy limit).
async fn execute_main_session_turn_with_policy_outcome(
    ctx: &TurnPipelineContext<'_>,
    user_message: &str,
    write_context: RuntimeMemoryWriteContext,
    settings: TurnExecutionSettings<'_>,
) -> Result<TurnExecutionOutcome> {
    // Use config-driven cap with a hard ceiling of 10 as absolute safety net.
    let caps = VerifyRepairCaps::from_config(ctx.config);
    let effective_cap = caps.max_attempts.min(10);
    let mut attempts = 0_u32;
    let mut repair_depth = 0_u32;

    loop {
        attempts = attempts.saturating_add(1);
        match execute_main_session_turn_with_accounting(ctx, user_message, &write_context, settings)
            .await
        {
            Ok(outcome) => return Ok(outcome),
            Err(error) => {
                let analysis = analyze_verify_failure(&error);
                if let Some(escalation) =
                    decide_verify_repair_escalation(caps, attempts, repair_depth, analysis, &error)
                {
                    if let Err(event_error) = emit_verify_repair_escalation_event(
                        ctx.mem.as_ref(),
                        write_context.entity_id.as_str(),
                        &escalation,
                    )
                    .await
                    {
                        tracing::warn!(
                            error = %event_error,
                            "verify/repair escalation event write failed"
                        );
                    }
                    anyhow::bail!(escalation.contract_message());
                }

                // Hard safety cap: prevent infinite retry even if config allows it.
                if attempts >= effective_cap {
                    anyhow::bail!("verify/repair cap reached ({effective_cap} attempts): {error}");
                }

                repair_depth = repair_depth.saturating_add(1);
                let backoff_ms = ctx
                    .config
                    .reliability
                    .provider_backoff_ms
                    .saturating_mul(1_u64.checked_shl(repair_depth.min(6)).unwrap_or(64));
                tracing::warn!(
                    attempt = attempts,
                    repair_depth,
                    backoff_ms,
                    failure_class = analysis.failure_class,
                    retryable = analysis.retryable,
                    error = %error,
                    "verify/repair retrying turn"
                );
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
            }
        }
    }
}

/// # Errors
///
/// Returns an error when integration turn execution fails.
pub async fn run_main_turn_test(params: TurnParams<'_>) -> Result<String> {
    run_main_turn_policy_test(TurnParams {
        entity_id: "default",
        policy_context: TenantPolicyContext::disabled(),
        ..params
    })
    .await
}

/// # Errors
///
/// Returns an error when model inference, tool execution, or policy-enforced
/// turn processing fails.
pub async fn run_main_turn_policy_test(params: TurnParams<'_>) -> Result<String> {
    let TurnParams {
        config,
        security,
        mem,
        answer_provider,
        reflect_provider,
        system_prompt,
        model_name,
        temperature,
        entity_id,
        policy_context,
        user_message,
    } = params;
    let observer: Arc<dyn Observer> = Arc::new(NoopObserver);
    let security_arc = Arc::new(security.clone());
    let registry = super::run::init_tools(config, &security_arc, &mem, None);
    let person_id = resolve_person_id(config);
    let params = MainSessionTurnParams {
        answer_provider,
        reflect_provider,
        augmentor_provider: None,
        stream_sink: None,
        interactive_input_tx: None,
        approval_broker: None,
        execution_audit_sink: None,
        person_id: &person_id,
        system_prompt,
        model_name,
        temperature,
        registry,
        max_tool_iterations: config.autonomy.max_tool_loop_iterations,
        loop_detection: config.tools.loop_detection.clone(),
        rate_limiter: Arc::new(EntityRateLimiter::new_with_scopes(
            config.autonomy.max_actions_per_hour,
            config.autonomy.max_actions_per_entity_per_hour,
            config.autonomy.max_actions_per_conversation_per_hour,
            config.autonomy.max_actions_per_workspace_per_hour,
            config.autonomy.burst_max_per_entity,
            config.autonomy.burst_window_secs,
        )),
        permission_store: Arc::new(PermissionStore::load(&config.workspace_dir)),
        subagent_manager: Arc::new(crate::core::subagents::SubagentOrchestrator::new()),
        skill_metadata_provider: Arc::new(NoopSkillMetadataProvider::new()),
    };

    let ctx = TurnPipelineContext {
        config,
        security,
        mem,
        params: &params,
        observer: &observer,
    };
    execute_main_session_turn_with_policy_outcome(
        &ctx,
        user_message,
        RuntimeMemoryWriteContext::for_entity_with_policy(entity_id, policy_context),
        TurnExecutionSettings {
            conversation_history: &[],
            thinking_level: ThinkingLevel::Off,
            show_reasoning: false,
            ephemeral: false,
        },
    )
    .await
    .map(|outcome| outcome.response)
}
