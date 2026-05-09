//! Metacognitive calibration: logs per-turn predicted vs observed
//! success, computes calibration error, and gates further persona
//! adaptation when the agent's self-assessment is poorly calibrated.

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::PersonaConfig;
use crate::contracts::ids::PersonId;
use crate::contracts::strings::data_model::SLOT_METACOGNITION_TURN_PREFIX;
use crate::core::memory::{Memory, MemoryEventType};
use crate::core::persona::person_identity::person_entity_id;
use crate::core::persona::self_model::SelfModelShadow;

const METACOGNITION_SCHEMA_VERSION: u32 = 1;
const TURN_LOG_SLOT_PREFIX: &str = SLOT_METACOGNITION_TURN_PREFIX;
/// Memory slot key for the calibration snapshot.
pub(crate) use crate::contracts::strings::data_model::SLOT_METACOGNITION_CALIBRATION_V1 as CALIBRATION_SNAPSHOT_SLOT_KEY;
const DEFAULT_PREDICTED_SUCCESS: f64 = 0.5;
const MIN_CALIBRATION_WINDOW: usize = 1;

/// Per-turn log of predicted vs observed success for calibration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct MetacognitiveTurnLog {
    /// Schema version for forward compatibility.
    pub schema_version: u32,
    /// Person ID this log entry belongs to.
    pub person_id: PersonId,
    /// Agent's predicted success probability before the turn.
    pub predicted_success: f64,
    /// Actual observed success after the turn.
    pub observed_success: f64,
    /// Absolute difference between predicted and observed.
    pub calibration_error: f64,
    /// Continuity score at the time of this turn.
    pub continuity_score: Option<f64>,
    /// Classified error category when the turn had low success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_category: Option<super::error_taxonomy::ErrorCategory>,
    /// Confidence in the error classification.
    #[serde(default)]
    pub error_classification_confidence: f64,
    /// RFC 3339 timestamp of this turn.
    pub occurred_at: String,
}

/// Sliding-window calibration snapshot for gate evaluation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct CalibrationSnapshot {
    /// Schema version for forward compatibility.
    pub schema_version: u32,
    /// Person ID this snapshot belongs to.
    pub person_id: PersonId,
    /// Recent calibration errors in the sliding window.
    pub error_window: Vec<f64>,
    /// Number of samples in the window.
    pub sample_count: usize,
    /// Mean calibration error across the window.
    pub mean_error: f64,
    /// 95th percentile calibration error.
    pub p95_error: f64,
    /// Gate evaluation status.
    pub gate_status: CalibrationGateStatus,
    /// Configured minimum samples for gate evaluation.
    pub gate_min_samples: usize,
    /// Configured maximum mean error threshold.
    pub gate_mean_error_max: f64,
    /// Configured maximum p95 error threshold.
    pub gate_p95_error_max: f64,
    /// RFC 3339 timestamp of this snapshot.
    pub updated_at: String,
}

/// Status of the calibration gate evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CalibrationGateStatus {
    /// Gate is disabled in configuration.
    Disabled,
    /// Not enough samples to evaluate.
    InsufficientSamples,
    /// Calibration error is within thresholds.
    Passed,
    /// Calibration error exceeds thresholds.
    Blocked,
}

impl CalibrationGateStatus {
    /// Return the status as a static string label.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::InsufficientSamples => "insufficient_samples",
            Self::Passed => "passed",
            Self::Blocked => "blocked",
        }
    }
}

impl std::fmt::Display for CalibrationGateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Full decision from the calibration gate evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CalibrationGateDecision {
    /// Gate outcome.
    pub(crate) status: CalibrationGateStatus,
    /// Number of samples used for evaluation.
    pub(crate) sample_count: usize,
    /// Mean calibration error.
    pub(crate) mean_error: f64,
    /// 95th percentile calibration error.
    pub(crate) p95_error: f64,
}

impl CalibrationGateDecision {
    /// Whether this decision permits reflect writeback.
    #[must_use]
    pub(crate) fn allows_reflect_writeback(self) -> bool {
        !matches!(self.status, CalibrationGateStatus::Blocked)
    }
}

/// Extract predicted success probability from a self-model shadow.
#[must_use]
pub(crate) fn predicted_success_from_self_model(
    self_model_shadow: Option<&SelfModelShadow>,
) -> f64 {
    self_model_shadow
        .and_then(|shadow| shadow.capability_estimates.first())
        .map_or(DEFAULT_PREDICTED_SUCCESS, |capability| {
            capability.success_ema.clamp(0.0, 1.0)
        })
}

/// # Errors
/// Returns an error when memory persistence fails.
pub(crate) async fn record_metacognitive_turn(
    mem: &dyn Memory,
    persona: &PersonaConfig,
    person_id: &str,
    predicted_success: f64,
    observed_success: f64,
    continuity_score: Option<f64>,
) -> Result<CalibrationGateDecision> {
    record_metacognitive_turn_with_error(
        mem,
        persona,
        person_id,
        predicted_success,
        observed_success,
        continuity_score,
        None,
    )
    .await
}

/// Record a metacognitive turn with optional error classification.
///
/// When a `ClassifiedError` is provided, the error category and
/// confidence are persisted alongside the calibration data, enabling
/// pattern analysis across turns.
///
/// # Errors
/// Returns an error when memory persistence fails.
pub(crate) async fn record_metacognitive_turn_with_error(
    mem: &dyn Memory,
    persona: &PersonaConfig,
    person_id: &str,
    predicted_success: f64,
    observed_success: f64,
    continuity_score: Option<f64>,
    classified_error: Option<&super::error_taxonomy::ClassifiedError>,
) -> Result<CalibrationGateDecision> {
    let occurred_at = Utc::now().to_rfc3339();
    let predicted_success = predicted_success.clamp(0.0, 1.0);
    let observed_success = observed_success.clamp(0.0, 1.0);
    let calibration_error = (predicted_success - observed_success).abs().clamp(0.0, 1.0);

    let turn_log = MetacognitiveTurnLog {
        schema_version: METACOGNITION_SCHEMA_VERSION,
        person_id: PersonId::new(person_id),
        predicted_success,
        observed_success,
        calibration_error,
        continuity_score: continuity_score.map(|score| score.clamp(0.0, 1.0)),
        error_category: classified_error.map(|e| e.category),
        error_classification_confidence: classified_error.map_or(0.0, |e| e.confidence.get()),
        occurred_at: occurred_at.clone(),
    };
    persist_turn_log(mem, person_id, &turn_log).await?;

    let prior_snapshot = load_calibration_snapshot(mem, person_id).await?;
    let window_size = persona
        .calibration_gate_window_size
        .max(MIN_CALIBRATION_WINDOW);
    let mut error_window = prior_snapshot.map_or_else(Vec::new, |snapshot| snapshot.error_window);
    error_window.push(calibration_error);
    if error_window.len() > window_size {
        let overflow = error_window.len() - window_size;
        error_window.drain(0..overflow);
    }

    let sample_count = error_window.len();
    let mean_error = mean(&error_window);
    let p95_error = p95(&error_window);
    let status = evaluate_gate_status(persona, sample_count, mean_error, p95_error);

    let snapshot = CalibrationSnapshot {
        schema_version: METACOGNITION_SCHEMA_VERSION,
        person_id: PersonId::new(person_id),
        error_window,
        sample_count,
        mean_error,
        p95_error,
        gate_status: status,
        gate_min_samples: persona.calibration_gate_min_samples.max(1),
        gate_mean_error_max: persona.calibration_gate_mean_error_max,
        gate_p95_error_max: persona.calibration_gate_p95_error_max,
        updated_at: occurred_at,
    };
    persist_calibration_snapshot(mem, person_id, &snapshot).await?;

    Ok(CalibrationGateDecision {
        status,
        sample_count,
        mean_error,
        p95_error,
    })
}

fn evaluate_gate_status(
    persona: &PersonaConfig,
    sample_count: usize,
    mean_error: f64,
    p95_error: f64,
) -> CalibrationGateStatus {
    if !persona.enable_calibration_gate {
        return CalibrationGateStatus::Disabled;
    }

    let min_samples = persona.calibration_gate_min_samples.max(1);
    if sample_count < min_samples {
        return CalibrationGateStatus::InsufficientSamples;
    }

    if mean_error <= persona.calibration_gate_mean_error_max
        && p95_error <= persona.calibration_gate_p95_error_max
    {
        CalibrationGateStatus::Passed
    } else {
        CalibrationGateStatus::Blocked
    }
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let sum: f64 = values.iter().copied().sum();
    let count = u32::try_from(values.len()).unwrap_or(u32::MAX).max(1);
    (sum / f64::from(count)).clamp(0.0, 1.0)
}

fn p95(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);

    let count = sorted.len();
    let rank = (count * 95).div_ceil(100);
    let index = rank.saturating_sub(1).min(count.saturating_sub(1));
    sorted[index].clamp(0.0, 1.0)
}

async fn persist_turn_log(
    mem: &dyn Memory,
    person_id: &str,
    turn_log: &MetacognitiveTurnLog,
) -> Result<()> {
    let entity_id = person_entity_id(person_id);
    let slot_key = format!("{TURN_LOG_SLOT_PREFIX}.{}", Uuid::new_v4().simple());
    let payload = serde_json::to_string(turn_log).context("serialize metacognitive turn log")?;

    super::persist_helper::persist_persona_slot(
        mem,
        entity_id,
        slot_key,
        MemoryEventType::SummaryCompacted,
        payload,
        0.9,
        0.6,
        format!("persona-metacognition-turn:{}", turn_log.occurred_at),
        "persona.metacognition.turn",
        Some(turn_log.occurred_at.clone()),
        person_id,
    )
    .await
}

async fn load_calibration_snapshot(
    mem: &dyn Memory,
    person_id: &str,
) -> Result<Option<CalibrationSnapshot>> {
    let entity_id = person_entity_id(person_id);
    let Some(slot) = mem
        .resolve_slot(&entity_id, CALIBRATION_SNAPSHOT_SLOT_KEY)
        .await
        .context("resolve metacognitive calibration slot")?
    else {
        return Ok(None);
    };

    match serde_json::from_str::<CalibrationSnapshot>(&slot.value) {
        Ok(parsed) => Ok(Some(parsed)),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "failed to parse calibration snapshot; resetting metacognitive window"
            );
            Ok(None)
        }
    }
}

async fn persist_calibration_snapshot(
    mem: &dyn Memory,
    person_id: &str,
    snapshot: &CalibrationSnapshot,
) -> Result<()> {
    let entity_id = person_entity_id(person_id);
    let payload =
        serde_json::to_string(snapshot).context("serialize metacognitive calibration snapshot")?;

    super::persist_helper::persist_persona_slot(
        mem,
        entity_id,
        CALIBRATION_SNAPSHOT_SLOT_KEY,
        MemoryEventType::SummaryCompacted,
        payload,
        0.95,
        0.7,
        format!("persona-metacognition-calibration:{}", snapshot.updated_at),
        "persona.metacognition.calibration",
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
        CALIBRATION_SNAPSHOT_SLOT_KEY, CalibrationGateStatus, CalibrationSnapshot,
        predicted_success_from_self_model, record_metacognitive_turn,
    };
    use crate::config::PersonaConfig;
    use crate::core::memory::{MarkdownMemory, Memory};
    use crate::core::persona::self_model::{CapabilityEstimate, SelfModelShadow};

    fn test_config() -> PersonaConfig {
        PersonaConfig {
            calibration_gate_window_size: 3,
            calibration_gate_min_samples: 1,
            calibration_gate_mean_error_max: 0.15,
            calibration_gate_p95_error_max: 0.15,
            ..PersonaConfig::default()
        }
    }

    #[test]
    fn predicted_success_defaults_without_shadow() {
        assert!((predicted_success_from_self_model(None) - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn predicted_success_uses_first_capability_estimate() {
        let shadow = SelfModelShadow {
            schema_version: 1,
            self_id: "person-test".to_string(),
            active_goal: "goal".to_string(),
            capability_estimates: vec![CapabilityEstimate {
                domain: "general".to_string(),
                success_ema: 0.82,
                sample_size: 10,
            }],
            uncertainty_register: Vec::new(),
            continuity_score: 0.8,
            updated_at: "2026-02-28T00:00:00Z".to_string(),
        };
        assert!((predicted_success_from_self_model(Some(&shadow)) - 0.82).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn record_metacognitive_turn_persists_snapshot_and_blocks_when_threshold_exceeded() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
        let decision = record_metacognitive_turn(
            mem.as_ref(),
            &test_config(),
            "person-test",
            0.95,
            0.10,
            Some(0.70),
        )
        .await
        .expect("metacognitive turn should persist");

        assert_eq!(decision.status, CalibrationGateStatus::Blocked);
        assert!(!decision.allows_reflect_writeback());
        assert_eq!(decision.sample_count, 1);
        assert!((decision.mean_error - 0.85).abs() < f64::EPSILON);
        assert!((decision.p95_error - 0.85).abs() < f64::EPSILON);

        let slot = mem
            .resolve_slot("person:person-test", CALIBRATION_SNAPSHOT_SLOT_KEY)
            .await
            .expect("resolve snapshot slot should succeed")
            .expect("snapshot slot should exist");
        let parsed: CalibrationSnapshot =
            serde_json::from_str(&slot.value).expect("snapshot should parse");
        assert_eq!(parsed.sample_count, 1);
        assert!((parsed.mean_error - 0.85).abs() < f64::EPSILON);
        assert!((parsed.p95_error - 0.85).abs() < f64::EPSILON);
        assert_eq!(parsed.gate_status, CalibrationGateStatus::Blocked);
    }
}
