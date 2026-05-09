//! Milestone detection and value tracking for the agent's
//! development journey. Detects significant events (capability
//! breakthroughs, relationship depth, consistent values) and
//! persists them for event-driven narrative construction.

use anyhow::Result;
use chrono::Utc;
use num_traits::ToPrimitive;
use serde::{Deserialize, Serialize};

use crate::contracts::strings::data_model::SLOT_MILESTONES_V1;
use crate::core::experience::distill_types::Principle;
use crate::core::experience::{ExperienceAtom, ExperienceKind, ExperienceOutcome};
use crate::core::memory::{Memory, MemoryEventType};
use crate::core::persona::person_identity::{person_entity_id, sanitize_person_id};
use crate::core::persona::relationship::RelationshipState;

/// A significant event in the agent's development.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Milestone {
    pub kind: MilestoneKind,
    pub title: String,
    pub description: String,
    /// When this milestone was achieved.
    pub achieved_at: String,
    /// Significance score (0.0–1.0).
    pub significance: f64,
}

/// Categories of milestones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MilestoneKind {
    /// First time achieving something.
    FirstTime,
    /// Reaching a numeric threshold (100 interactions, etc.).
    Threshold,
    /// Demonstrating improvement over time.
    Growth,
    /// Successfully handling a difficult situation.
    Challenge,
    /// Significant relationship development.
    Relationship,
    /// A new value or principle solidified.
    ValueFormation,
}

/// Tracked values and their consistency over time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ValueTracker {
    pub values: Vec<TrackedValue>,
}

/// A single tracked value/principle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TrackedValue {
    /// The principle statement.
    pub statement: String,
    /// How many times this value has been observed.
    pub observation_count: u32,
    /// Confidence trend: positive = strengthening, negative = weakening.
    pub confidence_trend: f64,
    /// Historical confidence observations.
    pub confidence_history: Vec<f64>,
    /// Whether this value has remained stable.
    pub is_stable: bool,
}

/// Full milestone + value tracking state for persistence.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct MilestoneState {
    pub milestones: Vec<Milestone>,
    pub value_tracker: ValueTracker,
    /// Counters for threshold detection.
    pub counters: MilestoneCounters,
}

/// Internal counters for detecting threshold milestones.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct MilestoneCounters {
    pub total_experiences: u64,
    #[serde(default)]
    pub successful_turns: u64,
    pub total_interactions: u64,
    pub domains_mastered: Vec<String>,
    pub last_checked_at: String,
}

impl ValueTracker {
    /// Update tracked values from current principles.
    pub(crate) fn update_from_principles(&mut self, principles: &[Principle]) {
        for principle in principles {
            if principle.confidence < crate::contracts::scores::Confidence::new(0.5)
                || principle.validation_count < 2
            {
                continue;
            }

            if let Some(tracked) = self
                .values
                .iter_mut()
                .find(|v| v.statement == principle.statement)
            {
                tracked.observation_count = tracked.observation_count.saturating_add(1);
                tracked.confidence_history.push(principle.confidence.get());
                // Keep last 20 observations.
                if tracked.confidence_history.len() > 20 {
                    tracked.confidence_history.remove(0);
                }
                tracked.confidence_trend = compute_trend(&tracked.confidence_history);
                tracked.is_stable =
                    tracked.observation_count >= 5 && tracked.confidence_trend.abs() < 0.1;
            } else if self.values.len() < 30 {
                self.values.push(TrackedValue {
                    statement: principle.statement.clone(),
                    observation_count: 1,
                    confidence_trend: 0.0,
                    confidence_history: vec![principle.confidence.get()],
                    is_stable: false,
                });
            }
        }
    }

    /// Get values that have remained stable (core identity values).
    pub(crate) fn stable_values(&self) -> Vec<&TrackedValue> {
        self.values.iter().filter(|v| v.is_stable).collect()
    }

    /// Get values that are strengthening (emerging values).
    pub(crate) fn emerging_values(&self) -> Vec<&TrackedValue> {
        self.values
            .iter()
            .filter(|v| !v.is_stable && v.confidence_trend > 0.05 && v.observation_count >= 3)
            .collect()
    }
}

/// Compute a simple trend from a time series using linear regression slope.
fn compute_trend(values: &[f64]) -> f64 {
    let n = values.len();
    if n < 2 {
        return 0.0;
    }
    let n_f = n.to_f64().unwrap_or(0.0);
    let sum_x: f64 = (0..n).map(|i| i.to_f64().unwrap_or(0.0)).sum();
    let sum_y: f64 = values.iter().sum();
    let cross_sum: f64 = values
        .iter()
        .enumerate()
        .map(|(i, y)| i.to_f64().unwrap_or(0.0) * y)
        .sum();
    let sum_xx: f64 = (0..n)
        .map(|i| {
            let x = i.to_f64().unwrap_or(0.0);
            x * x
        })
        .sum();

    let denom = n_f * sum_xx - sum_x * sum_x;
    if denom.abs() < f64::EPSILON {
        return 0.0;
    }
    (n_f * cross_sum - sum_x * sum_y) / denom
}

/// Detect new milestones from experiences, principles, and relationship.
pub(crate) fn detect_milestones(
    state: &mut MilestoneState,
    experiences: &[ExperienceAtom],
    principles: &[Principle],
    relationship: Option<&RelationshipState>,
) -> Vec<Milestone> {
    let mut new_milestones = Vec::new();
    let now = Utc::now().to_rfc3339();

    let total_exp = experiences.len().to_u64().unwrap_or(u64::MAX);
    let successes = experiences
        .iter()
        .filter(|e| e.outcome == ExperienceOutcome::Success)
        .count()
        .to_u64()
        .unwrap_or(u64::MAX);
    let turn_successes = experiences
        .iter()
        .filter(|e| {
            e.kind == ExperienceKind::TurnInteraction && e.outcome == ExperienceOutcome::Success
        })
        .count()
        .to_u64()
        .unwrap_or(u64::MAX);

    detect_threshold_milestones(state, &mut new_milestones, total_exp, successes, &now);
    detect_first_time_milestones(state, &mut new_milestones, turn_successes, &now);
    detect_growth_milestones(
        &state.milestones,
        &mut new_milestones,
        experiences,
        total_exp,
        &now,
    );

    // ── Value formation milestones ──────────────────────────────
    state.value_tracker.update_from_principles(principles);
    detect_value_milestones(
        &state.value_tracker,
        &state.milestones,
        &mut new_milestones,
        &now,
    );

    // ── Relationship milestones ─────────────────────────────────
    if let Some(rel) = relationship
        && rel.trust_level >= 0.8
        && rel.interaction_count >= 20
        && !has_milestone(&state.milestones, "Deep trust established")
    {
        let trust_percentage = rel.trust_level * 100.0;
        let interaction_count = rel.interaction_count;
        new_milestones.push(Milestone {
            kind: MilestoneKind::Relationship,
            title: "Deep trust established".to_string(),
            description: format!(
                "Trust level reached {trust_percentage:.0}% over {interaction_count} interactions."
            ),
            achieved_at: now.clone(),
            significance: 0.85,
        });
    }

    state.milestones.extend(new_milestones.iter().cloned());
    new_milestones
}

fn detect_threshold_milestones(
    state: &mut MilestoneState,
    out: &mut Vec<Milestone>,
    total_exp: u64,
    successes: u64,
    now: &str,
) {
    let thresholds: &[(u64, &str)] = &[
        (10, "First 10 experiences"),
        (50, "50 experiences milestone"),
        (100, "Century: 100 experiences"),
        (500, "500 experiences — deep experience"),
    ];
    for (threshold, title) in thresholds {
        if total_exp >= *threshold
            && state.counters.total_experiences < *threshold
            && !has_milestone(&state.milestones, title)
        {
            out.push(Milestone {
                kind: MilestoneKind::Threshold,
                title: (*title).to_string(),
                description: format!(
                    "Accumulated {total_exp} experiences with {successes} successes."
                ),
                achieved_at: now.to_string(),
                significance: (threshold.to_f64().unwrap_or(500.0) / 500.0).clamp(0.3, 0.9),
            });
        }
    }
    state.counters.total_experiences = total_exp;
}

fn detect_first_time_milestones(
    state: &mut MilestoneState,
    out: &mut Vec<Milestone>,
    turn_successes: u64,
    now: &str,
) {
    if turn_successes >= 1
        && state.counters.successful_turns == 0
        && !has_milestone(&state.milestones, "First successful companion turn")
    {
        out.push(Milestone {
            kind: MilestoneKind::FirstTime,
            title: "First successful companion turn".to_string(),
            description:
                "Handled a grounded companion conversation turn successfully for the first time."
                    .to_string(),
            achieved_at: now.to_string(),
            significance: 0.7,
        });
    }
    state.counters.successful_turns = turn_successes;
}

fn detect_growth_milestones(
    existing: &[Milestone],
    out: &mut Vec<Milestone>,
    experiences: &[ExperienceAtom],
    total_exp: u64,
    now: &str,
) {
    if total_exp < 20 {
        return;
    }
    let recent_success_rate = experiences
        .iter()
        .rev()
        .take(10)
        .filter(|e| e.outcome == ExperienceOutcome::Success)
        .count()
        .to_f64()
        .unwrap_or(0.0)
        / 10.0;

    let older_success_rate = if experiences.len() >= 20 {
        experiences
            .iter()
            .take(10)
            .filter(|e| e.outcome == ExperienceOutcome::Success)
            .count()
            .to_f64()
            .unwrap_or(0.0)
            / 10.0
    } else {
        0.5
    };

    if recent_success_rate > older_success_rate + 0.2
        && !has_milestone(existing, "Measurable improvement")
    {
        out.push(Milestone {
            kind: MilestoneKind::Growth,
            title: "Measurable improvement".to_string(),
            description: format!(
                "Success rate improved from {:.0}% to {:.0}%.",
                older_success_rate * 100.0,
                recent_success_rate * 100.0
            ),
            achieved_at: now.to_string(),
            significance: 0.8,
        });
    }
}

fn detect_value_milestones(
    tracker: &ValueTracker,
    existing: &[Milestone],
    out: &mut Vec<Milestone>,
    now: &str,
) {
    for value in &tracker.values {
        let statement = value.statement.as_str();
        let truncated_title = truncate(statement, 40);
        let title = format!("Value solidified: {truncated_title}");
        if value.is_stable && value.observation_count >= 5 && !has_milestone(existing, &title) {
            let truncated_statement = truncate(statement, 60);
            let observation_count = value.observation_count;
            out.push(Milestone {
                kind: MilestoneKind::ValueFormation,
                title: format!("Value solidified: {truncated_title}"),
                description: format!(
                    "The principle '{truncated_statement}' has been consistently upheld across \
                     {observation_count} observations."
                ),
                achieved_at: now.to_string(),
                significance: 0.6,
            });
        }
    }
}

fn has_milestone(milestones: &[Milestone], title: &str) -> bool {
    milestones.iter().any(|m| m.title == title)
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let end = s.char_indices().nth(max).map_or(s.len(), |(idx, _)| idx);
        &s[..end]
    }
}

// ── Persistence ─────────────────────────────────────────────────

fn milestone_slot_key(person_id: &str) -> String {
    let slot_suffix = SLOT_MILESTONES_V1
        .trim_start_matches("persona.")
        .replace('.', "/");
    let sanitized_person_id = sanitize_person_id(person_id);
    format!("persona/{sanitized_person_id}/{slot_suffix}")
}

/// Load the milestone state from memory, returning a default if absent.
///
/// # Errors
///
/// Returns an error if the memory lookup fails.
pub(crate) async fn load_milestone_state(
    mem: &dyn Memory,
    person_id: &str,
) -> Result<MilestoneState> {
    let entity_id = person_entity_id(person_id);
    let slot_key = milestone_slot_key(person_id);
    match mem.resolve_slot(&entity_id, &slot_key).await? {
        Some(slot) => match serde_json::from_str::<MilestoneState>(&slot.value) {
            Ok(state) => Ok(state),
            Err(error) => {
                tracing::warn!(%error, "failed to parse milestone state; resetting");
                Ok(MilestoneState::default())
            }
        },
        None => Ok(MilestoneState::default()),
    }
}

/// Persist the milestone state to memory.
///
/// # Errors
///
/// Returns an error if serialization or the memory write fails.
pub(crate) async fn persist_milestone_state(
    mem: &dyn Memory,
    person_id: &str,
    state: &MilestoneState,
) -> Result<()> {
    super::persist_helper::persist_persona_slot(
        mem,
        person_entity_id(person_id),
        milestone_slot_key(person_id),
        MemoryEventType::FactUpdated,
        serde_json::to_string(state)?,
        0.9,
        0.8,
        "persona.milestone.update",
        "persona.milestone.writeback",
        None,
        person_id,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{
        Milestone, MilestoneKind, MilestoneState, ValueTracker, compute_trend, detect_milestones,
    };
    use crate::core::experience::distill_types::{Principle, PrincipleCategory};
    use crate::core::experience::{ExperienceAtom, ExperienceKind, ExperienceOutcome};
    use crate::core::persona::relationship::RelationshipState;

    fn make_experiences(count: usize, outcome: ExperienceOutcome) -> Vec<ExperienceAtom> {
        (0..count)
            .map(|i| {
                ExperienceAtom::new(
                    ExperienceKind::TurnInteraction,
                    format!("experience {i}"),
                    outcome,
                )
                .with_confidence(0.7)
            })
            .collect()
    }

    fn make_principle(statement: &str, confidence: f64, validation_count: u32) -> Principle {
        Principle {
            id: uuid::Uuid::new_v4().to_string(),
            category: PrincipleCategory::Heuristic,
            statement: statement.to_string(),
            confidence: crate::contracts::scores::Confidence::new(confidence),
            source_experience_ids: vec![],
            validation_count,
            created_at: String::new(),
            domain: None,
            q_value: 0.0,
            times_applied: 0,
        }
    }

    #[test]
    fn no_milestones_on_empty_data() {
        let mut state = MilestoneState::default();
        let new = detect_milestones(&mut state, &[], &[], None);
        assert!(new.is_empty());
    }

    #[test]
    fn threshold_milestone_at_10() {
        let mut state = MilestoneState::default();
        let experiences = make_experiences(10, ExperienceOutcome::Success);
        let new = detect_milestones(&mut state, &experiences, &[], None);
        assert!(new.iter().any(|m| m.title.contains("First 10")));
    }

    #[test]
    fn threshold_not_repeated() {
        let mut state = MilestoneState::default();
        let experiences = make_experiences(15, ExperienceOutcome::Success);
        let _ = detect_milestones(&mut state, &experiences, &[], None);
        // Second call should not re-trigger.
        let new = detect_milestones(&mut state, &experiences, &[], None);
        assert!(
            !new.iter().any(|m| m.title.contains("First 10")),
            "milestone should not repeat"
        );
    }

    #[test]
    fn first_companion_turn_success_detected() {
        let mut state = MilestoneState::default();
        let experiences = vec![
            ExperienceAtom::new(
                ExperienceKind::TurnInteraction,
                "first companion turn",
                ExperienceOutcome::Success,
            )
            .with_confidence(0.8),
        ];
        let new = detect_milestones(&mut state, &experiences, &[], None);
        assert!(
            new.iter()
                .any(|m| m.title.contains("First successful companion turn"))
        );
    }

    #[test]
    fn value_tracker_detects_stability() {
        let mut tracker = ValueTracker::default();
        let principle = make_principle("Always verify before acting", 0.85, 5);

        // Observe the same principle multiple times.
        for _ in 0..6 {
            tracker.update_from_principles(std::slice::from_ref(&principle));
        }

        assert!(!tracker.stable_values().is_empty());
        assert!(tracker.stable_values()[0].statement.contains("verify"));
    }

    #[test]
    fn compute_trend_positive() {
        let values = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let trend = compute_trend(&values);
        assert!(trend > 0.0, "trend should be positive: {trend}");
    }

    #[test]
    fn compute_trend_flat() {
        let values = vec![0.5, 0.5, 0.5, 0.5];
        let trend = compute_trend(&values);
        assert!(trend.abs() < 0.01, "trend should be ~0: {trend}");
    }

    #[test]
    fn compute_trend_negative() {
        let values = vec![0.9, 0.7, 0.5, 0.3, 0.1];
        let trend = compute_trend(&values);
        assert!(trend < 0.0, "trend should be negative: {trend}");
    }

    #[test]
    fn relationship_milestone_on_high_trust() {
        let mut state = MilestoneState::default();
        let rel = RelationshipState {
            trust_level: 0.85,
            interaction_count: 25,
            ..RelationshipState::default()
        };
        let new = detect_milestones(&mut state, &[], &[], Some(&rel));
        assert!(new.iter().any(|m| m.title.contains("Deep trust")));
    }

    #[test]
    fn serde_round_trip() {
        let state = MilestoneState {
            milestones: vec![Milestone {
                kind: MilestoneKind::FirstTime,
                title: "test".to_string(),
                description: "test milestone".to_string(),
                achieved_at: "2026-03-01T00:00:00Z".to_string(),
                significance: 0.7,
            }],
            ..MilestoneState::default()
        };
        let json = serde_json::to_string(&state).unwrap();
        let restored: MilestoneState = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.milestones.len(), 1);
    }
}
