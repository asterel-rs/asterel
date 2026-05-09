//! Relationship state tracking: trust, rapport, affect trend,
//! and notable events per user. Persisted to and loaded from
//! memory with EMA-smoothed updates each turn.

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::contracts::affect::AffectLabel;
use crate::contracts::strings::data_model::{
    SOURCE_PERSONA_RELATIONSHIP_UPDATE, SOURCE_PERSONA_RELATIONSHIP_WRITEBACK,
};
use crate::core::memory::{Memory, MemoryEventType};
use crate::core::persona::person_identity::{person_entity_id, sanitize_person_id};

/// Per-user relationship state tracking trust, rapport, and dyadic realism axes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RelationshipState {
    /// Trust level in `[0.1, 0.95]`.
    pub trust_level: f32,
    /// Rapport score in `[0.1, 0.95]`.
    pub rapport: f32,
    /// How much deeper disclosure has been mutually earned `[0.0, 1.0]`.
    #[serde(default = "default_disclosure_depth")]
    pub disclosure_depth: f32,
    /// Attachment security / steadiness `[0.0, 1.0]`.
    #[serde(default = "default_attachment_security")]
    pub attachment_security: f32,
    /// Unresolved tension or rupture debt `[0.0, 1.0]`.
    #[serde(default)]
    pub unresolved_tension: f32,
    /// Repair work still owed after sharp misattunements `[0.0, 1.0]`.
    #[serde(default)]
    pub repair_debt: f32,
    /// EMA-smoothed affect trend in `[-1.0, 1.0]`.
    pub recent_affect_trend: f32,
    /// Total number of interactions with this user.
    pub interaction_count: u32,
    /// RFC 3339 timestamp of the last interaction.
    pub last_interaction: String,
    /// Notable events in the relationship history.
    pub notable_events: Vec<RelEvent>,
}

impl Default for RelationshipState {
    fn default() -> Self {
        Self {
            trust_level: 0.5,
            rapport: 0.5,
            disclosure_depth: 0.2,
            attachment_security: 0.5,
            unresolved_tension: 0.0,
            repair_debt: 0.0,
            recent_affect_trend: 0.0,
            interaction_count: 0,
            last_interaction: String::new(),
            notable_events: Vec::new(),
        }
    }
}

const fn default_disclosure_depth() -> f32 {
    0.2
}

const fn default_attachment_security() -> f32 {
    0.5
}

/// A notable event in the relationship timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RelEvent {
    /// Category of the event.
    pub kind: RelEventKind,
    /// Brief summary of what happened.
    pub summary: String,
    /// RFC 3339 timestamp of the event.
    pub timestamp: String,
    /// Significance score in `[0.0, 1.0]`.
    pub significance: f32,
}

/// Category of a notable relationship event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RelEventKind {
    /// User gave positive feedback.
    PositiveFeedback,
    /// User gave negative feedback.
    NegativeFeedback,
    /// User re-engaged with a previous topic.
    TopicReengagement,
    /// A moment of notable emotional expression.
    EmotionalMoment,
    /// User demonstrated increased trust.
    TrustSignal,
    /// Tension or rupture rose sharply.
    Rupture,
    /// Repair work visibly reduced debt/tension.
    Repair,
}

/// Affect label with intensity and significance for tagging events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EmotionalTag {
    /// Detected affect label.
    pub affect: AffectLabel,
    /// Intensity of the affect in `[0.0, 1.0]`.
    pub intensity: f32,
    /// Significance of this tag in `[0.0, 1.0]`.
    pub significance: f32,
}

fn relationship_slot_key(person_id: &str) -> String {
    format!("persona/{}/relationship/v1", sanitize_person_id(person_id))
}

/// Load the relationship state for a person from memory.
///
/// # Errors
///
/// Returns an error if the memory lookup or JSON parsing fails.
pub(crate) async fn load_relationship(
    mem: &dyn Memory,
    person_id: &str,
) -> Result<Option<RelationshipState>> {
    load_relationship_for_entity(mem, &person_entity_id(person_id), person_id).await
}

/// Load the relationship state for a person from memory for a specific entity.
///
/// This is intended for tenant-scoped callers that already resolved their
/// scoped entity identifier.
pub(crate) async fn load_relationship_for_entity(
    mem: &dyn Memory,
    entity_id: &str,
    person_id: &str,
) -> Result<Option<RelationshipState>> {
    let slot_key = relationship_slot_key(person_id);
    let Some(slot) = mem.resolve_slot(entity_id, &slot_key).await? else {
        return Ok(None);
    };

    let parsed = serde_json::from_str::<RelationshipState>(&slot.value)
        .with_context(|| format!("parse relationship state from slot key: {slot_key}"))?;
    Ok(Some(parsed))
}

/// Persist the relationship state for a person to memory.
///
/// # Errors
///
/// Returns an error if serialization or the memory write fails.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn persist_relationship(
    mem: &dyn Memory,
    person_id: &str,
    state: &RelationshipState,
) -> Result<()> {
    persist_relationship_for_entity(mem, &person_entity_id(person_id), person_id, state).await
}

/// Persist the relationship state for a person to a caller-provided entity.
///
/// This is intended for transport/runtime callers that already resolved a
/// scoped entity identifier.
pub(crate) async fn persist_relationship_for_entity(
    mem: &dyn Memory,
    entity_id: &str,
    person_id: &str,
    state: &RelationshipState,
) -> Result<()> {
    super::persist_helper::persist_persona_slot(
        mem,
        entity_id,
        relationship_slot_key(person_id),
        MemoryEventType::FactUpdated,
        serde_json::to_string(state)?,
        0.9,
        0.7,
        SOURCE_PERSONA_RELATIONSHIP_UPDATE,
        SOURCE_PERSONA_RELATIONSHIP_WRITEBACK,
        None,
        person_id,
    )
    .await
}

// ── Non-linear Trust & Rapport Dynamics ────────────────────

/// Compute non-linear trust delta: logarithmic growth (slow near
/// ceiling), exponential decay (fast from high trust).
fn compute_trust_update(current: f32, outcome_success: f32, recent_negative: bool) -> f32 {
    let delta = outcome_success - 0.5;
    if delta >= 0.0 {
        // Logarithmic growth: gain slows as trust approaches ceiling
        let gain = delta * 0.03 * (1.0 - current).powf(0.6);
        // If recent negative feedback, sigmoid-dampen recovery
        if recent_negative {
            let dampen = 1.0 / (1.0 + (-(current - 0.5) * 6.0).exp());
            gain * (1.0 - 0.5 * dampen)
        } else {
            gain
        }
    } else {
        // Exponential decay: high trust collapses faster
        let loss = delta.abs() * 0.04 * current.powf(1.5);
        -loss
    }
}

/// Compute non-linear rapport delta: asymmetric growth/decay.
fn compute_rapport_update(current: f32, trend: f32) -> f32 {
    if trend >= 0.0 {
        // Diminishing returns near ceiling
        trend * 0.015 * (1.0 - current).powf(0.4)
    } else {
        // Faster decay
        trend * 0.02 * current.powf(0.8)
    }
}

fn update_disclosure_depth(current: f32, trust: f32, rapport: f32, repair_debt: f32) -> f32 {
    let target = (trust * 0.5 + rapport * 0.5 - repair_debt * 0.4).clamp(0.0, 1.0);
    (current * 0.85 + target * 0.15).clamp(0.0, 1.0)
}

fn update_attachment_security(current: f32, trust: f32, unresolved_tension: f32) -> f32 {
    let target = (trust * 0.8 + (1.0 - unresolved_tension) * 0.2).clamp(0.0, 1.0);
    (current * 0.8 + target * 0.2).clamp(0.0, 1.0)
}

fn update_unresolved_tension(current: f32, affect_delta: f32, outcome_success: f32) -> f32 {
    let mut next = current * 0.9;
    if affect_delta < -0.2 || outcome_success < 0.35 {
        next += ((-affect_delta).max(0.0) * 0.2) + (0.35 - outcome_success).max(0.0) * 0.25;
    } else {
        next -= outcome_success * 0.08;
    }
    next.clamp(0.0, 1.0)
}

fn update_repair_debt(current: f32, unresolved_tension: f32, outcome_success: f32) -> f32 {
    let mut next = current * 0.92;
    if unresolved_tension > 0.45 && outcome_success < 0.45 {
        next += 0.15;
    } else if outcome_success > 0.75 {
        next -= 0.12;
    }
    next.clamp(0.0, 1.0)
}

// ── Affect Signal Mapping ──────────────────────────────────

fn affect_signal(label: AffectLabel) -> f32 {
    match label {
        AffectLabel::Neutral => 0.0,
        AffectLabel::Confused => -0.3,
        AffectLabel::Frustrated => -0.5,
        AffectLabel::Anxious => -0.4,
        AffectLabel::Sad => -0.35,
        AffectLabel::Angry => -0.7,
        AffectLabel::Excited => 0.5,
        AffectLabel::Grateful => 0.6,
        AffectLabel::Curious => 0.3,
        AffectLabel::Overwhelmed => -0.45,
    }
}

/// Compute affect signal directly from valence (continuous VAD).
///
/// When continuous valence is available, this is preferred over the
/// discrete label mapping since it preserves finer-grained affect
/// information. The value is clamped to [-1, 1] for safety.
// Cast safety: valence is clamped to [-1.0, 1.0] before conversion to f32.
#[allow(clippy::cast_possible_truncation)]
pub(crate) fn affect_signal_from_valence(valence: f64) -> f32 {
    if valence.is_nan() {
        0.0
    } else {
        valence.clamp(-1.0, 1.0) as f32
    }
}

/// Update the relationship state after a turn.
///
/// This performs a read-modify-write cycle on the relationship slot.
/// The memory backend's `append_event` is atomic (last-writer-wins),
/// but two concurrent callers could read the same state and one
/// update would be silently lost.  This is acceptable because the
/// function is only called once per turn from the single-threaded
/// agent loop.
pub(crate) async fn update_relationship_after_turn(
    mem: &dyn Memory,
    person_id: &str,
    affect_label: AffectLabel,
    affect_intensity: f32,
    outcome_success: f32,
) -> Result<RelationshipState> {
    update_relationship_after_turn_for_entity(
        mem,
        &person_entity_id(person_id),
        person_id,
        affect_label,
        affect_intensity,
        outcome_success,
    )
    .await
}

/// Update relationship state using an explicit scoped entity identifier.
pub(crate) async fn update_relationship_after_turn_for_entity(
    mem: &dyn Memory,
    entity_id: &str,
    person_id: &str,
    affect_label: AffectLabel,
    affect_intensity: f32,
    outcome_success: f32,
) -> Result<RelationshipState> {
    let mut state = load_relationship_for_entity(mem, entity_id, person_id)
        .await?
        .unwrap_or_default();
    let before_state = state.clone();

    let bounded_intensity = clamp_unit_f32(affect_intensity);
    let outcome_success = clamp_unit_f32(outcome_success);
    let emotional_tag = EmotionalTag {
        affect: affect_label,
        intensity: bounded_intensity,
        significance: ((1.0 - (outcome_success - 0.5).abs()) * bounded_intensity).clamp(0.0, 1.0),
    };
    tracing::debug!(
        affect = ?emotional_tag.affect,
        intensity = emotional_tag.intensity,
        significance = emotional_tag.significance,
        "relationship emotional tag observed"
    );

    let discrete_valence = f64::from(affect_signal(affect_label));
    let next_affect_delta = affect_signal_from_valence(discrete_valence) * bounded_intensity;
    state.recent_affect_trend =
        (state.recent_affect_trend * 0.7 + next_affect_delta * 0.3).clamp(-1.0, 1.0);

    // Trust: non-linear dynamics (logarithmic growth, exponential decay)
    let recent_negative = state.notable_events.iter().rev().take(5).any(|e| {
        matches!(
            e.kind,
            RelEventKind::NegativeFeedback | RelEventKind::Rupture
        )
    });
    let trust_delta = compute_trust_update(state.trust_level, outcome_success, recent_negative);
    state.trust_level = (state.trust_level + trust_delta).clamp(0.1, 0.95);

    // Rapport: asymmetric non-linear dynamics (diminishing growth, faster decay)
    let rapport_delta = compute_rapport_update(state.rapport, state.recent_affect_trend);
    state.rapport = (state.rapport + rapport_delta).clamp(0.1, 0.95);

    state.unresolved_tension =
        update_unresolved_tension(state.unresolved_tension, next_affect_delta, outcome_success);
    state.repair_debt =
        update_repair_debt(state.repair_debt, state.unresolved_tension, outcome_success);
    state.disclosure_depth = update_disclosure_depth(
        state.disclosure_depth,
        state.trust_level,
        state.rapport,
        state.repair_debt,
    );
    state.attachment_security = update_attachment_security(
        state.attachment_security,
        state.trust_level,
        state.unresolved_tension,
    );

    state.interaction_count = state.interaction_count.saturating_add(1);
    state.last_interaction = Utc::now().to_rfc3339();

    if let Some(event) = super::relationship_events::detect_notable_event(
        &super::relationship_events::RelationshipEventInput {
            affect_label,
            affect_intensity: bounded_intensity,
            outcome_success,
            before: &before_state,
            after: &state,
        },
    ) {
        state.notable_events.push(event);
        super::relationship_events::prune_old_events(&mut state.notable_events);
    }

    persist_relationship_for_entity(mem, entity_id, person_id, &state).await?;
    Ok(state)
}

fn clamp_unit_f32(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::core::memory::{MarkdownMemory, Memory};

    #[test]
    fn default_relationship_state_has_neutral_values() {
        let state = RelationshipState::default();
        assert!((state.trust_level - 0.5).abs() < f32::EPSILON);
        assert!((state.rapport - 0.5).abs() < f32::EPSILON);
        assert!((state.disclosure_depth - 0.2).abs() < f32::EPSILON);
        assert!((state.attachment_security - 0.5).abs() < f32::EPSILON);
        assert!(state.unresolved_tension.abs() < f32::EPSILON);
        assert!(state.repair_debt.abs() < f32::EPSILON);
        assert!((state.recent_affect_trend - 0.0).abs() < f32::EPSILON);
        assert_eq!(state.interaction_count, 0);
        assert!(state.notable_events.is_empty());
    }

    #[tokio::test]
    async fn update_relationship_increments_interaction_count() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        let state = update_relationship_after_turn(
            mem.as_ref(),
            "person-test",
            AffectLabel::Neutral,
            0.5,
            0.7,
        )
        .await
        .expect("relationship update should pass");
        assert_eq!(state.interaction_count, 1);
        assert!(state.disclosure_depth >= 0.0);
    }

    #[tokio::test]
    async fn affect_trend_shifts_with_negative_affect() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        let state = update_relationship_after_turn(
            mem.as_ref(),
            "person-test",
            AffectLabel::Angry,
            1.0,
            0.3,
        )
        .await
        .expect("relationship update should pass");
        assert!(state.recent_affect_trend < 0.0);
    }

    #[tokio::test]
    async fn trust_increases_with_success() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        let state = update_relationship_after_turn(
            mem.as_ref(),
            "person-test",
            AffectLabel::Neutral,
            0.5,
            0.9,
        )
        .await
        .expect("relationship update should pass");
        // Default trust is 0.5, success 0.9 should push it above 0.5
        assert!(
            state.trust_level > 0.5,
            "trust should increase with success, got {}",
            state.trust_level
        );
    }

    #[tokio::test]
    async fn trust_decreases_with_failure() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        let state = update_relationship_after_turn(
            mem.as_ref(),
            "person-test",
            AffectLabel::Neutral,
            0.5,
            0.1,
        )
        .await
        .expect("relationship update should pass");
        assert!(
            state.trust_level < 0.5,
            "trust should decrease with failure, got {}",
            state.trust_level
        );
    }

    #[test]
    fn render_relationship_context_block_format() {
        let state = RelationshipState {
            trust_level: 0.72,
            rapport: 0.65,
            interaction_count: 47,
            ..RelationshipState::default()
        };
        let block = crate::core::persona::presenter::render_relationship_context_block(&state);
        assert!(block.contains("[Relationship Context]"));
        assert!(block.contains("trust_level=0.72 (high)"));
        assert!(block.contains("rapport=0.65 (moderate)"));
        assert!(block.contains("interactions=47"));
    }

    #[tokio::test]
    async fn relationship_state_round_trip() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        let expected = RelationshipState {
            trust_level: 0.6,
            rapport: 0.55,
            disclosure_depth: 0.33,
            attachment_security: 0.58,
            unresolved_tension: 0.12,
            repair_debt: 0.05,
            recent_affect_trend: 0.15,
            interaction_count: 3,
            last_interaction: "2026-02-26T10:00:00Z".to_string(),
            notable_events: vec![RelEvent {
                kind: RelEventKind::TrustSignal,
                summary: "Accepted follow-up recommendation".to_string(),
                timestamp: "2026-02-26T10:00:00Z".to_string(),
                significance: 0.7,
            }],
        };

        persist_relationship(mem.as_ref(), "person-test", &expected)
            .await
            .expect("relationship persistence should pass");
        let loaded = load_relationship(mem.as_ref(), "person-test")
            .await
            .expect("relationship load should pass")
            .expect("relationship should exist");

        assert!((loaded.trust_level - expected.trust_level).abs() < f32::EPSILON);
        assert!((loaded.rapport - expected.rapport).abs() < f32::EPSILON);
        assert!((loaded.disclosure_depth - expected.disclosure_depth).abs() < f32::EPSILON);
        assert!((loaded.attachment_security - expected.attachment_security).abs() < f32::EPSILON);
        assert!((loaded.unresolved_tension - expected.unresolved_tension).abs() < f32::EPSILON);
        assert!((loaded.repair_debt - expected.repair_debt).abs() < f32::EPSILON);
        assert!((loaded.recent_affect_trend - expected.recent_affect_trend).abs() < f32::EPSILON);
        assert_eq!(loaded.interaction_count, expected.interaction_count);
        assert_eq!(loaded.last_interaction, expected.last_interaction);
        assert_eq!(loaded.notable_events.len(), expected.notable_events.len());
    }

    #[tokio::test]
    async fn load_for_entity_respects_entity_scope() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        let expected = RelationshipState {
            trust_level: 0.6,
            ..RelationshipState::default()
        };

        persist_relationship(mem.as_ref(), "person-test", &expected)
            .await
            .expect("relationship persistence should pass");

        let loaded = load_relationship_for_entity(
            mem.as_ref(),
            &person_entity_id("person-test"),
            "person-test",
        )
        .await
        .expect("relationship load should pass");
        assert!(loaded.is_some());

        let missing = load_relationship_for_entity(
            mem.as_ref(),
            &person_entity_id("other-entity"),
            "person-test",
        )
        .await
        .expect("relationship load should pass");
        assert!(missing.is_none());
    }

    #[test]
    fn trust_logarithmic_growth_slows_near_ceiling() {
        let low = super::compute_trust_update(0.3, 0.8, false);
        let high = super::compute_trust_update(0.8, 0.8, false);
        assert!(
            low > high,
            "growth should slow near ceiling: low={low}, high={high}"
        );
    }

    #[test]
    fn trust_exponential_decay_from_high() {
        let low_base = super::compute_trust_update(0.3, 0.2, false);
        let high_base = super::compute_trust_update(0.8, 0.2, false);
        assert!(
            high_base.abs() > low_base.abs(),
            "decay should be faster from high trust: high={}, low={}",
            high_base.abs(),
            low_base.abs(),
        );
    }

    #[test]
    fn trust_repair_slowdown_after_negative() {
        let normal = super::compute_trust_update(0.5, 0.7, false);
        let damaged = super::compute_trust_update(0.5, 0.7, true);
        assert!(
            damaged < normal,
            "recovery should be slower after negative: normal={normal}, damaged={damaged}"
        );
    }

    #[test]
    fn trust_repair_slowdown_after_rupture() {
        let state = RelationshipState {
            notable_events: vec![RelEvent {
                kind: RelEventKind::Rupture,
                summary: "Sharp misattunement".to_string(),
                timestamp: "2026-02-26T10:00:00Z".to_string(),
                significance: 0.9,
            }],
            ..RelationshipState::default()
        };
        let recent_disturbance = state.notable_events.iter().rev().take(5).any(|e| {
            matches!(
                e.kind,
                RelEventKind::NegativeFeedback | RelEventKind::Rupture
            )
        });

        let normal = super::compute_trust_update(0.5, 0.7, false);
        let damaged = super::compute_trust_update(0.5, 0.7, recent_disturbance);
        assert!(damaged < normal);
    }

    #[test]
    fn affect_signal_from_valence_sanitizes_non_finite_values() {
        assert_eq!(affect_signal_from_valence(f64::NAN), 0.0);
        assert_eq!(affect_signal_from_valence(f64::INFINITY), 1.0);
        assert_eq!(affect_signal_from_valence(f64::NEG_INFINITY), -1.0);
    }

    #[test]
    fn rapport_asymmetric_update() {
        let positive = super::compute_rapport_update(0.5, 0.3);
        let negative = super::compute_rapport_update(0.5, -0.3);
        // Negative should have larger magnitude (faster decay)
        assert!(
            negative.abs() > positive.abs(),
            "decay should be faster than growth: pos={positive}, neg={negative}"
        );
    }
}
