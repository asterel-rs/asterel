//! Embodied state snapshot: persists the agent's resource
//! pressure, runtime capacity, interaction stability, and
//! coherence indices with bounded temperature adjustments.

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::embodied_computation::{
    build_signal_inputs, derive_embodied_state, normalized_temperature_delta_max,
};
use crate::config::PersonaConfig;
use crate::contracts::ids::PersonId;
use crate::core::memory::{Memory, MemoryEventType};
use crate::core::persona::continuity_gate::{ROLLBACK_DRILL_SLOT_KEY, RollbackDrillResult};
use crate::core::persona::metacognition::{
    CALIBRATION_SNAPSHOT_SLOT_KEY, CalibrationGateDecision, CalibrationSnapshot,
};
use crate::core::persona::person_identity::person_entity_id;
use crate::core::persona::self_model::SelfModelShadow;

const EMBODIED_STATE_SCHEMA_VERSION: u32 = 1;
/// Memory slot key for the persisted embodied state snapshot.
pub(crate) use crate::contracts::strings::data_model::SLOT_EMBODIED_STATE_V1 as EMBODIED_STATE_SLOT_KEY;

const fn default_max_tokens_factor() -> f64 {
    1.0
}

/// Persisted snapshot of the agent's embodied state and modulations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct EmbodiedStateSnapshot {
    /// Schema version for forward compatibility.
    pub schema_version: u32,
    /// Person ID this snapshot belongs to.
    pub person_id: PersonId,
    /// Resource pressure index in `[0.0, 1.0]`.
    pub resource_pressure_index: f64,
    /// Runtime capacity index in `[0.0, 1.0]`.
    pub runtime_capacity_index: f64,
    /// Interaction stability index in `[0.0, 1.0]`.
    pub interaction_stability_index: f64,
    /// Coherence index in `[0.0, 1.0]`.
    pub coherence_index: f64,
    /// Unclamped temperature delta.
    pub requested_temperature_delta: f64,
    /// Temperature delta after clamping.
    pub applied_temperature_delta: f64,
    /// Top-p sampling delta.
    #[serde(default)]
    pub top_p_delta: f64,
    /// Max-tokens multiplier.
    #[serde(default = "default_max_tokens_factor")]
    pub max_tokens_factor: f64,
    /// Optional reasoning strategy override.
    #[serde(default)]
    pub recommended_reasoning_strategy: Option<String>,
    /// Label explaining the current modulation regime.
    pub modulation_reason: String,
    /// Calibration gate status as a string.
    pub calibration_status: String,
    /// Rollback drill status, if available.
    #[serde(default)]
    pub rollback_status: Option<String>,
    /// RFC 3339 timestamp of this snapshot.
    pub updated_at: String,
}

/// Extract the temperature delta from a snapshot (0.0 if disabled).
#[must_use]
pub(crate) fn temperature_delta_from_snapshot(
    persona: &PersonaConfig,
    snapshot: Option<&EmbodiedStateSnapshot>,
) -> f64 {
    if !persona.enable_embodied_state_policy_modulation {
        return 0.0;
    }

    let max_abs = normalized_temperature_delta_max(persona);
    snapshot.map_or(0.0, |state| {
        state.applied_temperature_delta.clamp(-max_abs, max_abs)
    })
}

/// Extract the `top_p` delta from a snapshot (0.0 if disabled or absent).
#[must_use]
pub(crate) fn top_p_delta_from_snapshot(
    persona: &PersonaConfig,
    snapshot: Option<&EmbodiedStateSnapshot>,
) -> f64 {
    if !persona.enable_embodied_state_policy_modulation {
        return 0.0;
    }
    snapshot.map_or(0.0, |state| state.top_p_delta.clamp(-0.15, 0.0))
}

/// Extract the `max_tokens` multiplier from a snapshot (1.0 if disabled or absent).
#[must_use]
pub(crate) fn max_tokens_factor_from_snapshot(
    persona: &PersonaConfig,
    snapshot: Option<&EmbodiedStateSnapshot>,
) -> f64 {
    if !persona.enable_embodied_state_policy_modulation {
        return 1.0;
    }
    snapshot.map_or(1.0, |state| state.max_tokens_factor.clamp(0.7, 1.0))
}

/// Extract the recommended reasoning strategy override, if any.
#[must_use]
pub(crate) fn reasoning_strategy_override_from_snapshot(
    persona: &PersonaConfig,
    snapshot: Option<&EmbodiedStateSnapshot>,
) -> Option<String> {
    if !persona.enable_embodied_state_policy_modulation {
        return None;
    }
    snapshot.and_then(|state| state.recommended_reasoning_strategy.clone())
}

/// # Errors
/// Returns an error when memory lookup fails unexpectedly.
pub(crate) async fn load_embodied_state_snapshot(
    mem: &dyn Memory,
    person_id: &str,
) -> Result<Option<EmbodiedStateSnapshot>> {
    let entity_id = person_entity_id(person_id);
    let Some(slot) = mem
        .resolve_slot(&entity_id, EMBODIED_STATE_SLOT_KEY)
        .await?
    else {
        return Ok(None);
    };

    match serde_json::from_str::<EmbodiedStateSnapshot>(&slot.value) {
        Ok(parsed) => Ok(Some(parsed)),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "failed to parse embodied-state snapshot; resetting modulation state"
            );
            Ok(None)
        }
    }
}

/// # Errors
/// Returns an error when memory lookup or persistence fails unexpectedly.
pub(crate) async fn update_embodied_state_snapshot(
    mem: &dyn Memory,
    persona: &PersonaConfig,
    person_id: &str,
    self_model_shadow: Option<&SelfModelShadow>,
    calibration_gate: Option<&CalibrationGateDecision>,
) -> Result<Option<EmbodiedStateSnapshot>> {
    if !persona.enable_embodied_state_policy_modulation {
        return Ok(None);
    }

    let calibration_snapshot = load_calibration_snapshot(mem, person_id).await?;
    let rollback_result = load_rollback_result(mem, person_id).await?;
    let signals = build_signal_inputs(
        self_model_shadow,
        calibration_gate,
        calibration_snapshot.as_ref(),
        rollback_result.as_ref(),
    );
    let derived = derive_embodied_state(persona, signals);
    let updated_at = Utc::now().to_rfc3339();

    let snapshot = EmbodiedStateSnapshot {
        schema_version: EMBODIED_STATE_SCHEMA_VERSION,
        person_id: PersonId::new(person_id),
        resource_pressure_index: derived.resource_pressure_index,
        runtime_capacity_index: derived.runtime_capacity_index,
        interaction_stability_index: derived.interaction_stability_index,
        coherence_index: derived.coherence_index,
        requested_temperature_delta: derived.requested_temperature_delta,
        applied_temperature_delta: derived.applied_temperature_delta,
        top_p_delta: derived.top_p_delta,
        max_tokens_factor: derived.max_tokens_factor,
        recommended_reasoning_strategy: derived.recommended_reasoning_strategy,
        modulation_reason: derived.modulation_reason.to_string(),
        calibration_status: signals.calibration_status.as_str().to_string(),
        rollback_status: rollback_result.map(|result| result.status),
        updated_at,
    };

    persist_embodied_state_snapshot(mem, person_id, &snapshot).await?;
    Ok(Some(snapshot))
}

async fn load_calibration_snapshot(
    mem: &dyn Memory,
    person_id: &str,
) -> Result<Option<CalibrationSnapshot>> {
    let entity_id = person_entity_id(person_id);
    let Some(slot) = mem
        .resolve_slot(&entity_id, CALIBRATION_SNAPSHOT_SLOT_KEY)
        .await?
    else {
        return Ok(None);
    };
    match serde_json::from_str::<CalibrationSnapshot>(&slot.value) {
        Ok(parsed) => Ok(Some(parsed)),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "failed to parse calibration snapshot while updating embodied state"
            );
            Ok(None)
        }
    }
}

async fn load_rollback_result(
    mem: &dyn Memory,
    person_id: &str,
) -> Result<Option<RollbackDrillResult>> {
    let entity_id = person_entity_id(person_id);
    let Some(slot) = mem
        .resolve_slot(&entity_id, ROLLBACK_DRILL_SLOT_KEY)
        .await?
    else {
        return Ok(None);
    };
    match serde_json::from_str::<RollbackDrillResult>(&slot.value) {
        Ok(parsed) => Ok(Some(parsed)),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "failed to parse rollback drill snapshot while updating embodied state"
            );
            Ok(None)
        }
    }
}

async fn persist_embodied_state_snapshot(
    mem: &dyn Memory,
    person_id: &str,
    snapshot: &EmbodiedStateSnapshot,
) -> Result<()> {
    super::persist_helper::persist_persona_slot(
        mem,
        person_entity_id(person_id),
        EMBODIED_STATE_SLOT_KEY,
        MemoryEventType::SummaryCompacted,
        serde_json::to_string(snapshot).context("serialize embodied-state snapshot")?,
        0.9,
        0.6,
        format!("persona-embodied-state:{}", snapshot.updated_at),
        "persona.embodied_state.policy_modulation",
        Some(snapshot.updated_at.clone()),
        person_id,
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::{
        EMBODIED_STATE_SLOT_KEY, temperature_delta_from_snapshot, update_embodied_state_snapshot,
    };
    use crate::config::PersonaConfig;
    use crate::core::memory::{MarkdownMemory, Memory};
    use crate::core::persona::embodied_computation::{EmbodiedSignalInputs, derive_embodied_state};
    use crate::core::persona::metacognition::{CalibrationGateDecision, CalibrationGateStatus};
    use crate::core::persona::self_model::{CapabilityEstimate, SelfModelShadow};

    fn test_persona_config() -> PersonaConfig {
        PersonaConfig {
            enable_embodied_state_policy_modulation: true,
            embodied_temperature_delta_max: 0.10,
            ..PersonaConfig::default()
        }
    }

    fn sample_shadow(capability_success_ema: f64, continuity_score: f64) -> SelfModelShadow {
        SelfModelShadow {
            schema_version: 1,
            self_id: "person-test".to_string(),
            active_goal: "goal".to_string(),
            capability_estimates: vec![CapabilityEstimate {
                domain: "general".to_string(),
                success_ema: capability_success_ema,
                sample_size: 8,
            }],
            uncertainty_register: vec![],
            continuity_score,
            updated_at: "2026-02-28T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn embodied_state_cools_down_under_high_pressure() {
        let persona = test_persona_config();
        let derived = derive_embodied_state(
            &persona,
            EmbodiedSignalInputs {
                capability_success_ema: 0.2,
                continuity_score: 0.3,
                uncertainty_count: 6,
                mean_error: 0.9,
                p95_error: 0.95,
                calibration_status: CalibrationGateStatus::Blocked,
                rollback_failed: true,
                rollback_passed: false,
            },
        );
        assert!(derived.applied_temperature_delta < 0.0);
        assert!(derived.applied_temperature_delta >= -0.10);
        assert_eq!(derived.modulation_reason, "pressure_cooldown");
        // New modulations under high pressure
        assert!(
            derived.top_p_delta < 0.0,
            "top_p should decrease under pressure"
        );
        assert!(
            derived.max_tokens_factor < 1.0,
            "max_tokens should be reduced under pressure"
        );
    }

    #[test]
    fn embodied_state_boosts_temperature_when_stable_and_capable() {
        let persona = test_persona_config();
        let derived = derive_embodied_state(
            &persona,
            EmbodiedSignalInputs {
                capability_success_ema: 0.9,
                continuity_score: 0.9,
                uncertainty_count: 0,
                mean_error: 0.1,
                p95_error: 0.1,
                calibration_status: CalibrationGateStatus::Passed,
                rollback_failed: false,
                rollback_passed: true,
            },
        );
        assert!(derived.applied_temperature_delta > 0.0);
        assert!(derived.applied_temperature_delta <= 0.10);
        assert_eq!(derived.modulation_reason, "stable_capacity_boost");
        // No pressure modulations when stable
        assert!(
            (derived.top_p_delta).abs() < f64::EPSILON,
            "top_p should be neutral when stable"
        );
        assert!(
            (derived.max_tokens_factor - 1.0).abs() < f64::EPSILON,
            "max_tokens_factor should be 1.0 when stable"
        );
        assert!(
            derived.recommended_reasoning_strategy.is_none(),
            "no reasoning override when stable"
        );
    }

    #[test]
    fn embodied_state_recommends_verify_first_at_extreme_pressure() {
        let persona = test_persona_config();
        let derived = derive_embodied_state(
            &persona,
            EmbodiedSignalInputs {
                capability_success_ema: 0.05,
                continuity_score: 0.1,
                uncertainty_count: 6,
                mean_error: 0.95,
                p95_error: 0.99,
                calibration_status: CalibrationGateStatus::Blocked,
                rollback_failed: true,
                rollback_passed: false,
            },
        );
        assert!(derived.resource_pressure_index >= 0.85);
        assert_eq!(
            derived.recommended_reasoning_strategy.as_deref(),
            Some("VerifyFirst")
        );
        assert!(derived.top_p_delta <= -0.05);
        assert!(derived.max_tokens_factor <= 0.85);
    }

    #[tokio::test]
    async fn update_embodied_state_persists_latest_snapshot() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
        let persona = test_persona_config();
        let shadow = sample_shadow(0.72, 0.78);
        let decision = CalibrationGateDecision {
            status: CalibrationGateStatus::Passed,
            sample_count: 8,
            mean_error: 0.22,
            p95_error: 0.28,
        };

        let snapshot = update_embodied_state_snapshot(
            mem.as_ref(),
            &persona,
            "person-test",
            Some(&shadow),
            Some(&decision),
        )
        .await
        .expect("update embodied state should succeed")
        .expect("snapshot should be produced");
        assert!((0.0..=1.0).contains(&snapshot.resource_pressure_index));
        assert!((0.0..=1.0).contains(&snapshot.runtime_capacity_index));

        let slot = mem
            .resolve_slot("person:person-test", EMBODIED_STATE_SLOT_KEY)
            .await
            .expect("resolve slot")
            .expect("slot should exist");
        assert!(slot.value.contains("\"schema_version\":1"));

        let applied_delta = temperature_delta_from_snapshot(&persona, Some(&snapshot));
        assert!(applied_delta.abs() <= 0.10);
    }
}
