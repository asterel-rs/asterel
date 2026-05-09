//! Pure computation functions for deriving embodied state from
//! self-model, calibration, and continuity signals. Produces
//! resource pressure, capacity, stability, and coherence indices.

use crate::config::PersonaConfig;
use crate::core::persona::continuity_gate::RollbackDrillResult;
use crate::core::persona::metacognition::{
    CalibrationGateDecision, CalibrationGateStatus, CalibrationSnapshot,
};
use crate::core::persona::self_model::SelfModelShadow;

const MAX_ABS_TEMPERATURE_DELTA_CAP: f64 = 0.25;
const DEFAULT_SIGNAL_MIDPOINT: f64 = 0.5;
const DEFAULT_UNCERTAINTY_COUNT: usize = 2;

#[derive(Debug, Clone, Copy)]
struct EmbodiedPenalties {
    calibration_penalty: f64,
    rollback_penalty: f64,
    rollback_bonus: f64,
}

#[derive(Debug, Clone, Copy)]
struct EmbodiedIndices {
    pressure: f64,
    capacity: f64,
    stability: f64,
    coherence: f64,
}

#[derive(Debug, Clone, Copy)]
struct EmbodiedModulation<'a> {
    requested_temperature_delta: f64,
    applied_temperature_delta: f64,
    top_p_delta: f64,
    max_tokens_factor: f64,
    modulation_reason: &'a str,
}

/// Raw signals fed into the embodied state derivation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct EmbodiedSignalInputs {
    /// EMA-smoothed general capability success rate.
    pub capability_success_ema: f64,
    /// Identity continuity score.
    pub continuity_score: f64,
    /// Number of active uncertainty items.
    pub uncertainty_count: usize,
    /// Mean calibration error.
    pub mean_error: f64,
    /// 95th percentile calibration error.
    pub p95_error: f64,
    /// Current calibration gate status.
    pub calibration_status: CalibrationGateStatus,
    /// Whether the last rollback drill failed.
    pub rollback_failed: bool,
    /// Whether the last rollback drill passed.
    pub rollback_passed: bool,
}

/// Derived embodied state: indices and parameter modulations.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EmbodiedDerivedState {
    /// Resource pressure index in `[0.0, 1.0]` (higher = more pressure).
    pub resource_pressure_index: f64,
    /// Runtime capacity index in `[0.0, 1.0]`.
    pub runtime_capacity_index: f64,
    /// Interaction stability index in `[0.0, 1.0]`.
    pub interaction_stability_index: f64,
    /// Coherence index in `[0.0, 1.0]`.
    pub coherence_index: f64,
    /// Unclamped temperature delta before applying bounds.
    pub requested_temperature_delta: f64,
    /// Temperature delta after clamping to configured max.
    pub applied_temperature_delta: f64,
    /// Top-p sampling delta (negative under pressure).
    pub top_p_delta: f64,
    /// Max-tokens multiplier in `[0.7, 1.0]`.
    pub max_tokens_factor: f64,
    /// Override reasoning strategy (e.g. "`VerifyFirst`") if needed.
    pub recommended_reasoning_strategy: Option<String>,
    /// Static label explaining the modulation regime.
    pub modulation_reason: &'static str,
}

/// Clamp the configured max temperature delta to the hard cap.
pub(crate) fn normalized_temperature_delta_max(persona: &PersonaConfig) -> f64 {
    persona
        .embodied_temperature_delta_max
        .abs()
        .clamp(0.0, MAX_ABS_TEMPERATURE_DELTA_CAP)
}

/// Assemble signal inputs from optional self-model and calibration data.
pub(crate) fn build_signal_inputs(
    self_model_shadow: Option<&SelfModelShadow>,
    calibration_gate: Option<&CalibrationGateDecision>,
    calibration_snapshot: Option<&CalibrationSnapshot>,
    rollback_result: Option<&RollbackDrillResult>,
) -> EmbodiedSignalInputs {
    let capability_success_ema = self_model_shadow
        .and_then(|shadow| shadow.capability_estimates.first())
        .map_or(DEFAULT_SIGNAL_MIDPOINT, |cap| {
            cap.success_ema.clamp(0.0, 1.0)
        });
    let continuity_score = self_model_shadow.map_or(DEFAULT_SIGNAL_MIDPOINT, |shadow| {
        shadow.continuity_score.clamp(0.0, 1.0)
    });
    let uncertainty_count = self_model_shadow.map_or(DEFAULT_UNCERTAINTY_COUNT, |shadow| {
        shadow.uncertainty_register.len()
    });

    let (mean_error, p95_error, calibration_status) = if let Some(decision) = calibration_gate {
        (
            decision.mean_error.clamp(0.0, 1.0),
            decision.p95_error.clamp(0.0, 1.0),
            decision.status,
        )
    } else if let Some(snapshot) = calibration_snapshot {
        (
            snapshot.mean_error.clamp(0.0, 1.0),
            snapshot.p95_error.clamp(0.0, 1.0),
            snapshot.gate_status,
        )
    } else {
        (
            DEFAULT_SIGNAL_MIDPOINT,
            DEFAULT_SIGNAL_MIDPOINT,
            CalibrationGateStatus::Disabled,
        )
    };

    let rollback_failed = rollback_result.is_some_and(|result| result.status.starts_with("failed"));
    let rollback_passed = rollback_result.is_some_and(|result| result.status == "passed");

    EmbodiedSignalInputs {
        capability_success_ema,
        continuity_score,
        uncertainty_count,
        mean_error,
        p95_error,
        calibration_status,
        rollback_failed,
        rollback_passed,
    }
}

/// Derive all embodied indices and parameter modulations from signals.
pub(crate) fn derive_embodied_state(
    persona: &PersonaConfig,
    signals: EmbodiedSignalInputs,
) -> EmbodiedDerivedState {
    let penalties = embodied_penalties(signals);
    let indices = embodied_indices(signals, penalties);
    let modulation = embodied_modulation(persona, signals, indices);

    EmbodiedDerivedState {
        resource_pressure_index: indices.pressure,
        runtime_capacity_index: indices.capacity,
        interaction_stability_index: indices.stability,
        coherence_index: indices.coherence,
        requested_temperature_delta: modulation.requested_temperature_delta,
        applied_temperature_delta: modulation.applied_temperature_delta,
        top_p_delta: modulation.top_p_delta,
        max_tokens_factor: modulation.max_tokens_factor,
        recommended_reasoning_strategy: recommended_reasoning_strategy(indices),
        modulation_reason: modulation.modulation_reason,
    }
}

fn embodied_penalties(signals: EmbodiedSignalInputs) -> EmbodiedPenalties {
    // Calibration gate penalty table:
    //   Blocked              → +0.30  (high; gate is actively blocking)
    //   InsufficientSamples  → +0.10  (moderate; not enough data yet)
    //   Passed               → +0.00  (no penalty)
    //   Disabled             → +0.05  (small constant; calibration not monitored)
    let calibration_penalty = match signals.calibration_status {
        CalibrationGateStatus::Blocked => 0.30,
        CalibrationGateStatus::InsufficientSamples => 0.10,
        CalibrationGateStatus::Passed => 0.0,
        CalibrationGateStatus::Disabled => 0.05,
    };

    EmbodiedPenalties {
        calibration_penalty,
        rollback_penalty: if signals.rollback_failed { 0.20 } else { 0.0 },
        rollback_bonus: if signals.rollback_passed { 0.08 } else { 0.0 },
    }
}

fn embodied_indices(
    signals: EmbodiedSignalInputs,
    penalties: EmbodiedPenalties,
) -> EmbodiedIndices {
    // Normalise uncertainty count to [0, 1] over a 6-item reference scale.
    let uncertainty_count_u32 = u32::try_from(signals.uncertainty_count).unwrap_or(u32::MAX);
    let uncertainty_ratio = (f64::from(uncertainty_count_u32) / 6.0).clamp(0.0, 1.0);
    // Weighted blend of mean (60%) and P95 (40%) calibration error.
    let error_mix = (signals.mean_error * 0.6 + signals.p95_error * 0.4).clamp(0.0, 1.0);

    // resource_pressure = 0.20 + error_mix×0.45 + uncertainty×0.20
    //                     + calibration_penalty + rollback_penalty
    //                     - capability_ema×0.15
    let resource_pressure_index = (0.20
        + error_mix * 0.45
        + uncertainty_ratio * 0.20
        + penalties.calibration_penalty
        + penalties.rollback_penalty
        - signals.capability_success_ema * 0.15)
        .clamp(0.0, 1.0);
    // runtime_capacity = 0.15 + capability_ema×0.70 + (1-error_mix)×0.15 - uncertainty×0.10
    let runtime_capacity_index =
        (0.15 + signals.capability_success_ema * 0.70 + (1.0 - error_mix) * 0.15
            - uncertainty_ratio * 0.10)
            .clamp(0.0, 1.0);
    // interaction_stability = 0.20 + continuity×0.55 + rollback_bonus
    //                         - rollback_penalty - uncertainty×0.15
    let interaction_stability_index =
        (0.20 + signals.continuity_score * 0.55 + penalties.rollback_bonus
            - penalties.rollback_penalty
            - uncertainty_ratio * 0.15)
            .clamp(0.0, 1.0);
    // coherence = continuity×0.55 + (1-error_mix)×0.45
    let coherence_index =
        (signals.continuity_score * 0.55 + (1.0 - error_mix) * 0.45).clamp(0.0, 1.0);

    EmbodiedIndices {
        pressure: resource_pressure_index,
        capacity: runtime_capacity_index,
        stability: interaction_stability_index,
        coherence: coherence_index,
    }
}

fn embodied_modulation<'a>(
    persona: &PersonaConfig,
    signals: EmbodiedSignalInputs,
    indices: EmbodiedIndices,
) -> EmbodiedModulation<'a> {
    let max_abs = normalized_temperature_delta_max(persona);
    // Temperature regime selection:
    //   pressure ≥ 0.75, Blocked calibration, or failed rollback → cooldown
    //     delta = -(0.02 + (pressure - 0.70)×0.25)
    //   capacity ≥ 0.70 AND stability ≥ 0.65 AND coherence ≥ 0.70 → boost
    //     delta = 0.01 + (mean_confidence - 0.70)×0.20
    //   otherwise → neutral (0.0)
    let (requested_temperature_delta, modulation_reason) = if indices.pressure >= 0.75
        || matches!(signals.calibration_status, CalibrationGateStatus::Blocked)
        || signals.rollback_failed
    {
        let pressure_over = (indices.pressure - 0.70).max(0.0);
        (-(0.02 + pressure_over * 0.25), "pressure_cooldown")
    } else if indices.capacity >= 0.70 && indices.stability >= 0.65 && indices.coherence >= 0.70 {
        let confidence =
            ((indices.capacity + indices.stability + indices.coherence) / 3.0 - 0.70).max(0.0);
        (0.01 + confidence * 0.20, "stable_capacity_boost")
    } else {
        (0.0, "neutral_band")
    };
    // Clamp to the configured max absolute delta (hard cap: ±0.25).
    let applied_temperature_delta = requested_temperature_delta.clamp(-max_abs, max_abs);

    // top_p_delta: tightens sampling under pressure (pressure ≥ 0.75).
    //   delta = -(0.05 + (pressure - 0.75)×0.25), clamped severity to [0, 0.4]
    let top_p_delta = if indices.pressure >= 0.75 {
        let severity = (indices.pressure - 0.75).clamp(0.0, 0.4);
        -(0.05 + severity * 0.25)
    } else {
        0.0
    };

    // max_tokens_factor: reduces token budget under pressure (pressure ≥ 0.65).
    //   factor = 1.0 - (pressure - 0.65)×0.86, floor at 0.7
    let max_tokens_factor = if indices.pressure >= 0.65 {
        let severity = (indices.pressure - 0.65).clamp(0.0, 0.35);
        (1.0 - severity * 0.86).clamp(0.7, 1.0)
    } else {
        1.0
    };

    EmbodiedModulation {
        requested_temperature_delta,
        applied_temperature_delta,
        top_p_delta,
        max_tokens_factor,
        modulation_reason,
    }
}

fn recommended_reasoning_strategy(indices: EmbodiedIndices) -> Option<String> {
    if indices.pressure >= 0.85 {
        Some("VerifyFirst".to_string())
    } else {
        None
    }
}
