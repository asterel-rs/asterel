//! Post-answer handler dispatch: embodied state updates, persona
//! reflection writeback, and metacognitive calibration logging.

use std::sync::Arc;

use anyhow::{Context, Result};

use super::augment;
use super::reflect::run_persona_reflect_writeback;
use super::types::{TurnCallAccounting, TurnPipelineContext};
use crate::config::Config;
use crate::core::agent::tool_types::ToolCallRecord;
use crate::core::memory::Memory;
use crate::core::persona::embodied_state::update_embodied_state_snapshot;
use crate::core::persona::metacognition::{
    CalibrationGateDecision, predicted_success_from_self_model, record_metacognitive_turn,
};
use crate::core::persona::self_model::SelfModelShadow;

/// Update the embodied-state policy modulation snapshot after a
/// completed turn, if persona and embodied state are enabled.
pub(super) async fn update_turn_embodied_state_if_enabled(
    config: &Config,
    mem: &dyn Memory,
    person_id: &str,
    self_model_shadow: Option<&SelfModelShadow>,
    calibration_gate: Option<&CalibrationGateDecision>,
    ephemeral: bool,
) {
    if ephemeral
        || !config.persona.enabled_main_session
        || !config.persona.enable_embodied_state_policy_modulation
    {
        return;
    }

    match update_embodied_state_snapshot(
        mem,
        &config.persona,
        person_id,
        self_model_shadow,
        calibration_gate,
    )
    .await
    {
        Ok(Some(snapshot)) => {
            tracing::debug!(
                person_id,
                resource_pressure_index = snapshot.resource_pressure_index,
                runtime_capacity_index = snapshot.runtime_capacity_index,
                interaction_stability_index = snapshot.interaction_stability_index,
                coherence_index = snapshot.coherence_index,
                applied_temperature_delta = snapshot.applied_temperature_delta,
                top_p_delta = snapshot.top_p_delta,
                max_tokens_factor = snapshot.max_tokens_factor,
                reasoning_override = ?snapshot.recommended_reasoning_strategy,
                modulation_reason = %snapshot.modulation_reason,
                "updated embodied-state policy modulation snapshot"
            );
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(
                error = %error,
                "embodied-state update failed; continuing without modulation refresh"
            );
        }
    }
}

/// Record a metacognitive calibration sample comparing predicted
/// and observed success, returning the calibration gate decision.
pub(super) async fn run_metacognitive_logging_if_enabled(
    config: &Config,
    mem: &dyn Memory,
    person_id: &str,
    user_message: &str,
    response: &str,
    self_model_shadow: Option<&SelfModelShadow>,
    tool_calls: &[ToolCallRecord],
) -> Option<CalibrationGateDecision> {
    if !config.persona.enabled_main_session || !config.persona.enable_metacognitive_logging {
        return None;
    }

    let predicted_success = predicted_success_from_self_model(self_model_shadow);
    let turn_outcome =
        augment::policy::TurnOutcome::from_turn_signals(&augment::policy::TurnSignals {
            user_message,
            assistant_answer: response,
            tool_calls,
        });
    let observed_success = f64::from(turn_outcome.success.value());
    let continuity_score = self_model_shadow.map(|shadow| shadow.continuity_score);

    match record_metacognitive_turn(
        mem,
        &config.persona,
        person_id,
        predicted_success,
        observed_success,
        continuity_score,
    )
    .await
    {
        Ok(decision) => {
            tracing::debug!(
                status = ?decision.status,
                sample_count = decision.sample_count,
                mean_error = decision.mean_error,
                p95_error = decision.p95_error,
                "metacognitive calibration snapshot updated"
            );
            Some(decision)
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                "metacognitive logging failed; continuing without calibration gate decision"
            );
            None
        }
    }
}

/// Run persona reflect/writeback if persona mode is enabled and
/// the calibration gate permits it.
///
/// # Errors
///
/// Returns an error if rate-limit or call-budget enforcement fails.
pub(super) async fn run_persona_reflect_if_enabled(
    ctx: &TurnPipelineContext<'_>,
    user_message: &str,
    response: &str,
    accounting: &mut TurnCallAccounting,
    calibration_gate: Option<&CalibrationGateDecision>,
) -> Result<()> {
    if !ctx.config.persona.enabled_main_session {
        return Ok(());
    }

    if let Some(decision) = calibration_gate
        && !decision.allows_reflect_writeback()
    {
        tracing::warn!(
            sample_count = decision.sample_count,
            mean_error = decision.mean_error,
            p95_error = decision.p95_error,
            "persona reflect/writeback skipped by calibration gate"
        );
        return Ok(());
    }

    ctx.security
        .consume_action_cost(0)
        .map_err(anyhow::Error::msg)
        .context("consume rate limit for persona reflect")?;
    accounting
        .consume_reflect_call()
        .context("consume reflect call budget")?;

    if let Err(error) = run_persona_reflect_writeback(
        ctx.config,
        Arc::clone(&ctx.mem),
        ctx.params.reflect_provider,
        ctx.params.model_name,
        ctx.params.person_id,
        user_message,
        response,
    )
    .await
    {
        tracing::warn!(error = %error, "persona reflect/writeback failed; answer path preserved");
    }

    Ok(())
}
