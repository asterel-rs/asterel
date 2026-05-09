//! Post-turn pipeline: the full execution path for one agent turn.
//!
//! This module owns the largest slice of the turn lifecycle: from
//! building the enriched prompt through running the tool loop, then
//! orchestrating every post-answer side-effect:
//!
//! 1. **Pre-answer enrichment** — context recall, style profile, self-model
//!    shadow, embodied state, augmentation blocks.
//! 2. **Tool loop** — the primary inference + tool-call cycle.
//! 3. **Response finalization** — format cleanup, reasoning stripping.
//! 4. **Post-answer pipeline**:
//!    - Metacognitive calibration logging.
//!    - Persona reflect/writeback (gated by calibration).
//!    - Relationship-state update (affect reading + turn outcome).
//!    - Embodied-state policy-modulation snapshot refresh.
//!    - Conversation state + fact ledger persistence.
//!    - Optional explainability block appended to visible response.
//!
//! All of the above is wrapped in `TurnCallAccounting` which enforces
//! a per-turn LLM call budget (answer + reflect ≤ `PERSONA_PER_TURN_CALL_BUDGET`).

use std::sync::Arc;

use anyhow::{Context, Result};

use super::augment;
use super::post_answer_handlers::{
    run_metacognitive_logging_if_enabled, run_persona_reflect_if_enabled,
    update_turn_embodied_state_if_enabled,
};
use super::pre_answer_enrichment::{
    PreAnswerEnrichment, PreAnswerSharedParams, build_pre_answer_enrichment,
};
use super::session::TurnExecutionSettings;
use super::types::{
    MainSessionTurnParams, RuntimeMemoryWriteContext, TurnCallAccounting, TurnExecutionOutcome,
    TurnPipelineContext,
};
use crate::config::Config;
use crate::contracts::observability::{AutonomySignal, Observer};
use crate::core::agent::response_finalize::{
    NaturalnessFinalizationContext, ResponseFinalizationRequest, finalize_response_with_context,
    naturalness_affect_from_text, naturalness_relationship_distance_from_state,
};
use crate::core::agent::response_style::classify_response_mode;
use crate::core::agent::tool_loop::LoopStopReason;
use crate::core::agent::tool_types::ToolCallRecord;
use crate::core::agent::turn_executor::{TurnExecutionPlan, execute_turn_plan};
use crate::core::persona::embodied_state::{
    max_tokens_factor_from_snapshot, temperature_delta_from_snapshot, top_p_delta_from_snapshot,
};
use crate::core::persona::person_identity::person_entity_id;
use crate::core::persona::relationship::load_relationship;
use crate::core::persona::relationship::update_relationship_after_turn;
use crate::core::persona::self_model::SelfModelShadow;
use crate::core::providers::response::ProviderMessage;
use crate::core::providers::{CliStreamSink, InferenceOpts, StreamSink};
use crate::core::tools::middleware::ExecutionContext;
use crate::security::SecurityPolicy;
use crate::security::policy::{AutonomyLevel, TenantPolicyContext};
use crate::utils::text::{strip_inference_markers, strip_internal_prompt_blocks, strip_reasoning};

/// Settings governing tool-loop execution for a turn.
#[derive(Clone, Copy)]
pub(super) struct ToolLoopExecutionSettings<'a> {
    /// Provider inference options (thinking level, top-p, etc.).
    pub(super) inference_options: Option<InferenceOpts>,
    /// Prior conversation history for multi-turn context.
    pub(super) conversation_history: &'a [ProviderMessage],
    /// Whether to expose reasoning traces in output.
    pub(super) show_reasoning: bool,
    /// Whether output must be buffered for naturalness verification.
    pub(super) naturalness_gate_enabled: bool,
}

/// Raw output from a single tool-loop run.
///
/// Carries the text that will become the visible response, token
/// usage if the provider reported it, tool records for post-turn
/// inference and relationship scoring, and flags used by response
/// finalization (`streaming_active`, `control_output`).
struct TurnGenerationOutput {
    text: String,
    tokens_used: Option<u64>,
    tool_calls: Vec<ToolCallRecord>,
    logprobs: Option<Vec<crate::core::providers::response::TokenLogprob>>,
    /// True when the answer provider streamed tokens; finalization
    /// must avoid double-printing already-streamed content.
    streaming_active: bool,
    /// Reserved for future non-finalized control outputs.
    control_output: bool,
}

/// Resolve the effective inference temperature for this turn.
///
/// Stacks three sources in order:
/// 1. Style-profile temperature (from the persona's learned preferences).
/// 2. Affect/taste overlay delta (emotion-driven modulation).
/// 3. Embodied-state delta (resource pressure / coherence index).
///
/// The combined value is then clamped to the autonomy band defined in
/// the config, preventing the LLM from operating outside the
/// operator-configured safety envelope.
fn compute_clamped_temperature(
    config: &Config,
    params: &MainSessionTurnParams<'_>,
    enrichment: &PreAnswerEnrichment,
    effective_autonomy_lvl: AutonomyLevel,
) -> f64 {
    let requested_temperature = enrichment
        .style_profile
        .as_ref()
        .map_or(params.temperature, |profile| profile.temperature);
    let embodied_temperature_delta =
        temperature_delta_from_snapshot(&config.persona, enrichment.embodied_state.as_ref());
    let requested_temperature = (requested_temperature
        + enrichment.style_overlay_temperature_delta
        + embodied_temperature_delta)
        .max(0.0);
    clamp_temperature_for_turn(config, requested_temperature, effective_autonomy_lvl)
}

/// Assemble provider-level inference options from enrichment data.
///
/// `top_p` and `max_tokens_factor` are derived from the embodied-state
/// snapshot and are only set when they deviate from the provider's
/// default (i.e. delta is above floating-point epsilon).  Passing
/// `None` signals the provider to use its built-in default.
fn build_inference_options(
    config: &Config,
    enrichment: &PreAnswerEnrichment,
    thinking_level: crate::core::providers::ThinkingLevel,
) -> InferenceOpts {
    let top_p_delta =
        top_p_delta_from_snapshot(&config.persona, enrichment.embodied_state.as_ref());
    let top_p = if top_p_delta.abs() > f64::EPSILON {
        Some((1.0 + top_p_delta).clamp(0.85, 1.0))
    } else {
        None
    };

    let factor =
        max_tokens_factor_from_snapshot(&config.persona, enrichment.embodied_state.as_ref());
    let max_tokens_factor = if (factor - 1.0).abs() > f64::EPSILON {
        Some(factor.clamp(0.7, 1.0))
    } else {
        None
    };

    InferenceOpts {
        thinking_level,
        top_p,
        max_tokens_factor,
    }
}

/// Inputs forwarded from the answer phase into the post-answer pipeline.
///
/// `sanitized_response` has reasoning traces stripped so that
/// reflection and persistence operate on the final user-visible text.
/// `ephemeral` suppresses all persistence side-effects for single-shot
/// `-m` mode invocations.
struct PostAnswerInput<'a> {
    user_message: &'a str,
    sanitized_response: &'a str,
    tool_calls: &'a [ToolCallRecord],
    self_model_shadow: Option<&'a SelfModelShadow>,
    ephemeral: bool,
    logprobs: Option<Vec<crate::core::providers::response::TokenLogprob>>,
}

async fn run_post_answer_pipeline(
    ctx: &TurnPipelineContext<'_>,
    write_context: &RuntimeMemoryWriteContext,
    input: &PostAnswerInput<'_>,
    accounting: &mut TurnCallAccounting,
) -> Result<Option<String>> {
    let calibration_gate = run_metacognitive_logging_if_enabled(
        ctx.config,
        ctx.mem.as_ref(),
        ctx.params.person_id,
        input.user_message,
        input.sanitized_response,
        input.self_model_shadow,
        input.tool_calls,
    )
    .await;

    run_persona_reflect_if_enabled(
        ctx,
        input.user_message,
        input.sanitized_response,
        accounting,
        calibration_gate.as_ref(),
    )
    .await?;

    // ── Relationship update ─────────────────────────────────────────
    if ctx.config.persona.enabled_main_session && !input.ephemeral {
        let reading = crate::core::affect::RuleBasedDetector::new().detect(input.user_message);
        let turn_outcome =
            augment::policy::TurnOutcome::from_turn_signals(&augment::policy::TurnSignals {
                user_message: input.user_message,
                assistant_answer: input.sanitized_response,
                tool_calls: input.tool_calls,
            });
        if let Err(error) = update_relationship_after_turn(
            ctx.mem.as_ref(),
            ctx.params.person_id,
            reading.label,
            // Cast safety: detector confidence is normalized to [0.0, 1.0] before f32 conversion.
            #[allow(clippy::cast_possible_truncation)]
            {
                reading.confidence.get() as f32
            },
            turn_outcome.success.value(),
        )
        .await
        {
            tracing::warn!(%error, "post-turn relationship update failed");
        }
    }

    update_turn_embodied_state_if_enabled(
        ctx.config,
        ctx.mem.as_ref(),
        ctx.params.person_id,
        input.self_model_shadow,
        calibration_gate.as_ref(),
        input.ephemeral,
    )
    .await;

    if !input.ephemeral {
        let _reason_trace = save_response_and_consolidate(
            ctx,
            write_context,
            Some(ctx.params.person_id),
            input.user_message,
            input.sanitized_response,
            input.logprobs.as_deref(),
        )
        .await;
    }

    // User facts: extract and persist stable facts from user message (P-1).
    if !input.ephemeral {
        let facts = crate::core::persona::user_facts::extract_user_facts(input.user_message);
        for (suffix, value) in facts {
            if let Err(error) = crate::core::persona::user_facts::persist_user_fact(
                ctx.mem.as_ref(),
                ctx.params.person_id,
                &suffix,
                &value,
            )
            .await
            {
                tracing::debug!(%error, "user fact persist failed");
            }
        }
    }

    Ok(None)
}

/// Execute a full turn: pre-answer enrichment, tool loop,
/// post-answer pipeline, and persistence, wrapped in call budget
/// accounting.
///
/// # Errors
///
/// Returns an error if write-scope enforcement, call-budget
/// consumption, or tool-loop execution fails.
pub(super) async fn execute_main_session_turn_with_accounting(
    ctx: &TurnPipelineContext<'_>,
    user_message: &str,
    write_context: &RuntimeMemoryWriteContext,
    settings: TurnExecutionSettings<'_>,
) -> Result<TurnExecutionOutcome> {
    ctx.observer
        .emit_autonomy_signal(AutonomySignal::IntentCreated);
    let mut accounting =
        TurnCallAccounting::for_persona_mode(ctx.config.persona.enabled_main_session);
    write_context.enforce_write_scope()?;

    if !settings.ephemeral {
        save_user_message_if_enabled(ctx.config, ctx.mem.as_ref(), write_context, user_message)
            .await;
    }

    let enrichment = build_pre_answer_enrichment(
        ctx.config,
        ctx.security,
        ctx.mem.as_ref(),
        pre_answer_shared_params(ctx),
        write_context,
        user_message,
        settings.ephemeral,
    )
    .await;
    enforce_intent_policy(ctx.security, ctx.observer)?;
    accounting
        .consume_answer_call()
        .context("consume answer call budget")?;

    let effective_autonomy_lvl = ctx.config.autonomy.effective_autonomy_lvl();
    let clamped_temperature =
        compute_clamped_temperature(ctx.config, ctx.params, &enrichment, effective_autonomy_lvl);
    let inference_options = Some(build_inference_options(
        ctx.config,
        &enrichment,
        settings.thinking_level,
    ));
    let exec_ctx = build_main_session_execution_context(
        ctx.config,
        ctx.security,
        &ctx.mem,
        ctx.params,
        effective_autonomy_lvl,
    );

    let effective_system_prompt =
        build_effective_system_prompt(ctx.params.system_prompt, &enrichment.system_prompt_addendum);

    let turn_output = execute_turn_with_tool_loop(
        ctx.params,
        clamped_temperature,
        &exec_ctx,
        &enrichment.enriched_message,
        &effective_system_prompt,
        ToolLoopExecutionSettings {
            inference_options,
            conversation_history: settings.conversation_history,
            show_reasoning: settings.show_reasoning,
            naturalness_gate_enabled: ctx.config.persona.enable_naturalness_gate,
        },
    )
    .await?;

    let raw_response = finalize_main_session_raw_response(
        ctx,
        &turn_output,
        user_message,
        settings.conversation_history,
    )
    .await;
    let tokens_used = turn_output.tokens_used;
    let tool_calls = turn_output.tool_calls;
    let logprobs = turn_output.logprobs;

    let sanitized_response = strip_reasoning(&raw_response);
    let sanitized_for_display = strip_inference_markers(&sanitized_response);
    let mut visible_response = if settings.show_reasoning {
        strip_internal_prompt_blocks(&raw_response)
    } else {
        sanitized_for_display
    };

    if let Some(explanation_block) = run_post_answer_pipeline(
        ctx,
        write_context,
        &PostAnswerInput {
            user_message,
            sanitized_response: &sanitized_response,
            tool_calls: &tool_calls,
            self_model_shadow: enrichment.self_model_shadow.as_ref(),
            ephemeral: settings.ephemeral,
            logprobs,
        },
        &mut accounting,
    )
    .await?
    {
        visible_response.push_str(&explanation_block);
    }

    Ok(TurnExecutionOutcome {
        response: visible_response,
        tokens_used,
        accounting,
    })
}

async fn main_session_naturalness_relationship_distance(
    ctx: &TurnPipelineContext<'_>,
) -> crate::core::agent::naturalness_gate::RelationshipDistance {
    if !ctx.config.persona.enable_naturalness_gate {
        return crate::core::agent::naturalness_gate::RelationshipDistance::Unknown;
    }

    let relationship = load_relationship(ctx.mem.as_ref(), ctx.params.person_id)
        .await
        .ok()
        .flatten();
    naturalness_relationship_distance_from_state(
        relationship.as_ref(),
        crate::core::agent::response_finalize::NaturalnessRelationshipSurface::Private,
    )
}

async fn finalize_main_session_raw_response(
    ctx: &TurnPipelineContext<'_>,
    output: &TurnGenerationOutput,
    user_message: &str,
    conversation_history: &[ProviderMessage],
) -> String {
    let relationship_distance = main_session_naturalness_relationship_distance(ctx).await;
    finalize_raw_response(
        output,
        user_message,
        conversation_history,
        ctx.config.persona.enable_response_finalization,
        ctx.config.persona.enable_naturalness_gate,
        relationship_distance,
    )
}

fn finalize_raw_response(
    output: &TurnGenerationOutput,
    user_message: &str,
    conversation_history: &[ProviderMessage],
    response_finalization_enabled: bool,
    naturalness_gate_enabled: bool,
    relationship_distance: crate::core::agent::naturalness_gate::RelationshipDistance,
) -> String {
    let output_mode = classify_response_mode(user_message);
    if !response_finalization_enabled && !naturalness_gate_enabled {
        tracing::debug!(
            target: "core::agent::loop_::session_posturn",
            surface = "main_session",
            output_mode = ?output_mode,
            "response finalization disabled"
        );
        return output.text.clone();
    }

    debug_assert!(
        !output.control_output,
        "main-session finalization must not receive user-facing control output"
    );
    let finalization = finalize_response_with_context(
        ResponseFinalizationRequest::user_facing(
            &output.text,
            output_mode,
            output.streaming_active,
            None,
            naturalness_gate_enabled,
        ),
        NaturalnessFinalizationContext {
            conversation_history,
            user_affect: naturalness_affect_from_text(user_message),
            relationship_distance,
        },
    );
    if !finalization.applied_actions.is_empty()
        || !finalization.preserved
        || finalization.contract_mismatch_reason.is_some()
        || !finalization.micro_rewrite_reason_codes.is_empty()
    {
        tracing::debug!(
            target: "core::agent::loop_::session_posturn",
            surface = "main_session",
            output_mode = ?output_mode,
            before_score = finalization.before_score,
            after_score = finalization.after_score,
            preserved = finalization.preserved,
            actions = ?finalization.applied_actions,
            contract_mismatch_reason = finalization.contract_mismatch_reason.map(crate::core::agent::response_audit::ContractMismatchReason::code),
            micro_rewrite_reason_codes = ?finalization.micro_rewrite_reason_codes,
            "response finalization evaluated"
        );
    }
    finalization.final_text
}

/// Merge the base system prompt with the pre-answer-enrichment addendum.
///
/// The addendum contains the self-contract block and/or the self-model
/// shadow block.  When the addendum is empty the base prompt is
/// returned as-is to avoid unnecessary allocations.
fn build_effective_system_prompt(base_prompt: &str, system_prompt_addendum: &str) -> String {
    if system_prompt_addendum.is_empty() {
        base_prompt.to_string()
    } else {
        format!("{base_prompt}\n\n{system_prompt_addendum}")
    }
}

async fn execute_turn_with_tool_loop(
    params: &MainSessionTurnParams<'_>,
    clamped_temperature: f64,
    ctx: &ExecutionContext,
    enriched: &str,
    effective_system_prompt: &str,
    settings: ToolLoopExecutionSettings<'_>,
) -> Result<TurnGenerationOutput> {
    let stream_sink: Option<Arc<dyn StreamSink>> = if settings.naturalness_gate_enabled {
        None
    } else {
        Some(match &params.stream_sink {
            Some(sink) => Arc::clone(sink),
            None => Arc::new(CliStreamSink::new_with_reasoning_and_usage(
                settings.show_reasoning,
                cli_usage_footer_enabled(),
            )),
        })
    };
    let tool_result = execute_turn_plan(TurnExecutionPlan {
        registry: Arc::clone(&params.registry),
        max_iterations: params.max_tool_iterations,
        loop_detection: params.loop_detection.clone(),
        history: crate::core::agent::TurnHistoryAdapter {
            session_manager: None,
            channel_name: "main_session",
            session_key: None,
            tenant_id: None,
            max_tokens: 0,
            fallback_history: settings.conversation_history,
        },
        provider: params.answer_provider,
        system_prompt: effective_system_prompt.to_string(),
        user_message: enriched,
        response_finalization_enabled: false,
        response_contract: None,
        image_content: &[],
        model: params.model_name,
        temperature: clamped_temperature,
        inference_options: settings.inference_options,
        ctx,
        stream_sink,
        state_notifier: None,
        transcript: crate::core::agent::TurnTranscriptAdapter {
            session_manager: None,
            user_message: enriched,
            log_target: "core::agent::loop_::session_posturn",
        },
    })
    .await
    .context("run agent tool loop")?
    .result;
    let streaming_active = tool_result.streaming_delivered;
    tracing::debug!(
        entity_id = %ctx.entity_id,
        iterations = tool_result.iterations,
        stop_reason = ?tool_result.stop_reason,
        "main session tool loop completed"
    );
    handle_tool_loop_stop_reason(&tool_result.stop_reason, tool_result.iterations)?;
    Ok(TurnGenerationOutput {
        text: tool_result.final_text,
        tokens_used: tool_result.tokens_used,
        tool_calls: tool_result.tool_calls,
        logprobs: tool_result.logprobs,
        streaming_active,
        control_output: false,
    })
}

fn pre_answer_shared_params<'a>(ctx: &'a TurnPipelineContext<'a>) -> PreAnswerSharedParams<'a> {
    PreAnswerSharedParams {
        person_id: ctx.params.person_id,
        model_name: ctx.params.model_name,
        skill_metadata_provider: ctx.params.skill_metadata_provider.as_ref(),
        augmentor_provider: ctx.params.augmentor_provider.clone(),
        observer: Arc::clone(ctx.observer),
    }
}

/// Build the `ExecutionContext` passed into the tool loop.
///
/// Roots the context under the person entity, attaches the rate limiter,
/// permission store, approval broker, audit sink, and subagent manager
/// from session params.  The autonomy level is resolved from config
/// rather than a hardcoded default so the operator can tune it.
fn build_main_session_execution_context(
    config: &Config,
    security: &SecurityPolicy,
    memory: &Arc<dyn crate::core::memory::Memory>,
    params: &MainSessionTurnParams<'_>,
    effective_autonomy_lvl: AutonomyLevel,
) -> ExecutionContext {
    let mut ctx = ExecutionContext::runtime_root(
        Arc::new(security.clone()),
        config.workspace_dir.clone(),
        Arc::clone(&params.rate_limiter),
        Some(Arc::clone(&params.permission_store)),
        TenantPolicyContext::disabled(),
    );
    ctx.autonomy_level = effective_autonomy_lvl;
    ctx.entity_id = crate::contracts::ids::EntityId::new(person_entity_id(params.person_id));
    ctx.memory = Some(Arc::clone(memory));
    ctx.approval_broker = params.approval_broker.as_ref().map(Arc::clone);
    ctx.execution_audit_sink = params.execution_audit_sink.as_ref().map(Arc::clone);
    ctx.subagent_manager = Some(Arc::clone(&params.subagent_manager));
    ctx
}

/// Gate the turn against the security policy's action cost budget.
///
/// A cost of 0 is used here because the intent-check is a policy
/// signal only — the actual tool-call costs are deducted inside the
/// tool loop.  Emits `IntentPolicyAllowed` or `IntentPolicyDenied` to
/// the observer so the supervising surface can react immediately.
fn enforce_intent_policy(security: &SecurityPolicy, observer: &Arc<dyn Observer>) -> Result<()> {
    match security.consume_action_cost(0) {
        Ok(()) => {
            observer.emit_autonomy_signal(AutonomySignal::IntentPolicyAllowed);
            Ok(())
        }
        Err(error) => {
            observer.emit_autonomy_signal(AutonomySignal::IntentPolicyDenied);
            Err(anyhow::Error::msg(error))
        }
    }
}

fn clamp_temperature_for_turn(
    config: &Config,
    requested_temperature: f64,
    effective_autonomy_lvl: AutonomyLevel,
) -> f64 {
    let clamped_temperature = config.autonomy.clamp_temperature(requested_temperature);
    if (requested_temperature - clamped_temperature).abs() > f64::EPSILON {
        let band = config.autonomy.selected_temp_band();
        tracing::info!(
            autonomy_level = ?effective_autonomy_lvl,
            requested_temperature,
            clamped_temperature,
            band_min = band.min,
            band_max = band.max,
            "temperature clamped to autonomy band"
        );
    }

    clamped_temperature
}

/// Translate a `LoopStopReason` into a success or structured error.
///
/// `Completed` and soft stops (`MaxIterations`, `RateLimited`,
/// `ApprovalDenied`) are treated as non-fatal — the partial response
/// is surfaced to the user rather than discarded.  Only `Error`
/// propagates as a hard failure.
fn handle_tool_loop_stop_reason(stop_reason: &LoopStopReason, iterations: u32) -> Result<()> {
    match stop_reason {
        LoopStopReason::Completed => Ok(()),
        LoopStopReason::MaxIterations => {
            tracing::warn!(iterations, "tool loop hit max iterations");
            Ok(())
        }
        LoopStopReason::RateLimited => {
            tracing::warn!("tool loop halted by rate limiter");
            Ok(())
        }
        LoopStopReason::ApprovalDenied => {
            tracing::warn!("tool loop halted by approval requirement");
            Ok(())
        }
        LoopStopReason::Error(message) => anyhow::bail!("tool loop failed: {message}"),
    }
}

/// Return `true` if the `ASTEREL_SHOW_USAGE` environment variable
/// is set to a truthy value.  When enabled the `CliStreamSink` prints
/// a token-usage footer after each response, useful for debugging
/// provider costs during development.
fn cli_usage_footer_enabled() -> bool {
    std::env::var("ASTEREL_SHOW_USAGE")
        .ok()
        .is_some_and(|value| {
            let normalized = value.trim();
            normalized == "1"
                || normalized.eq_ignore_ascii_case("true")
                || normalized.eq_ignore_ascii_case("yes")
                || normalized.eq_ignore_ascii_case("on")
        })
}

// Persistence functions extracted to session_persistence.rs
use super::session_persistence::{save_response_and_consolidate, save_user_message_if_enabled};

#[cfg(test)]
mod tests {
    use super::{TurnGenerationOutput, finalize_raw_response};
    use crate::core::agent::naturalness_gate::RelationshipDistance;

    #[test]
    fn main_session_finalization_threads_affect_to_naturalness_context() {
        let raw_text = "了解しました\n- **重要**: A\n- B\n- C\n- D";
        let output = TurnGenerationOutput {
            text: raw_text.to_string(),
            tokens_used: None,
            tool_calls: Vec::new(),
            logprobs: None,
            streaming_active: false,
            control_output: false,
        };

        let finalized = finalize_raw_response(
            &output,
            "I'm tired, anxious, overwhelmed, and can't keep up",
            &[],
            true,
            true,
            RelationshipDistance::Unknown,
        );

        assert_eq!(finalized, raw_text);
    }
}
