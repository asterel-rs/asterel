//! Self-narrative builder and persistence. Constructs a narrative
//! arc from the agent's experiences, principles, relationships,
//! and calibration data, then persists it to memory.
#![allow(clippy::cast_precision_loss)]

use anyhow::Result;
use chrono::Utc;

use super::narrative_types::SelfNarrative;
use super::relationship::RelationshipState;
use crate::contracts::strings::data_model::SUFFIX_NARRATIVE_V1;
use crate::core::experience::distill_types::Principle;
use crate::core::experience::{ExperienceAtom, ExperienceKind, ExperienceOutcome};
use crate::core::memory::{Memory, MemoryEventType};
use crate::core::persona::person_identity::person_entity_id;

fn narrative_slot_key(person_id: &str) -> String {
    format!(
        "persona/{}{SUFFIX_NARRATIVE_V1}",
        crate::core::persona::person_identity::sanitize_person_id(person_id),
    )
}

/// Build a self-narrative from experiences, principles, relationships, and calibration data.
pub(crate) struct NarrativeBuilder;

impl NarrativeBuilder {
    /// Construct a `SelfNarrative` with milestone and value tracking data.
    ///
    /// When a `MilestoneState` is provided, the narrative incorporates
    /// concrete milestones, stable values, and emerging values to create
    /// a richer, event-driven self-narrative rather than template text.
    pub(crate) fn build_with_milestones(
        experiences: &[ExperienceAtom],
        principles: &[Principle],
        relationship: Option<&RelationshipState>,
        milestone_state: Option<&super::milestone::MilestoneState>,
    ) -> SelfNarrative {
        let narrative_arc = build_narrative_arc(experiences, relationship);
        let key_experiences = extract_key_experiences(experiences);
        let growth_areas = extract_growth_areas(experiences);
        let consistent_values = extract_consistent_values(principles);
        let open_questions = extract_open_questions(experiences, principles);

        let (milestones, emerging_values, current_chapter) = if let Some(ms) = milestone_state {
            let milestones = ms
                .milestones
                .iter()
                .rev()
                .take(5)
                .map(|m| {
                    format!(
                        "[{:?}] {} (significance={:.1})",
                        m.kind, m.title, m.significance
                    )
                })
                .collect();

            let emerging = ms
                .value_tracker
                .emerging_values()
                .iter()
                .take(3)
                .map(|v| format!("{} (trend={:+.2})", v.statement, v.confidence_trend))
                .collect();

            let chapter = determine_current_chapter(experiences, &ms.milestones, &ms.value_tracker);

            (milestones, emerging, chapter)
        } else {
            (Vec::new(), Vec::new(), String::new())
        };

        SelfNarrative {
            narrative_arc,
            key_experiences,
            growth_areas,
            consistent_values,
            open_questions,
            milestones,
            emerging_values,
            current_chapter,
            rebuilt_at: Utc::now().to_rfc3339(),
        }
    }
}

/// Determine the current "chapter" of the agent's story.
fn determine_current_chapter(
    experiences: &[ExperienceAtom],
    milestones: &[super::milestone::Milestone],
    value_tracker: &super::milestone::ValueTracker,
) -> String {
    let total = experiences.len();
    let stable_count = value_tracker.stable_values().len();
    let milestone_count = milestones.len();

    if total == 0 {
        return "Chapter 1: The Beginning".to_string();
    }

    if milestone_count == 0 {
        return "Chapter 1: First Steps — learning through experience".to_string();
    }

    let recent_success_rate = experiences
        .iter()
        .rev()
        .take(10)
        .filter(|e| e.outcome == ExperienceOutcome::Success)
        .count() as f64
        / experiences.len().min(10) as f64;

    if stable_count >= 3 && recent_success_rate > 0.7 {
        format!(
            "Chapter {}: Mastery — {} core values established, consistent success",
            milestone_count / 2 + 3,
            stable_count
        )
    } else if stable_count >= 1 && recent_success_rate > 0.5 {
        format!(
            "Chapter {}: Growth — values forming, capability expanding",
            milestone_count / 2 + 2,
        )
    } else {
        format!(
            "Chapter {}: Exploration — {} milestones achieved, still discovering",
            milestone_count / 3 + 1,
            milestone_count,
        )
    }
}

fn build_narrative_arc(
    experiences: &[ExperienceAtom],
    relationship: Option<&RelationshipState>,
) -> String {
    let total = experiences.len();
    let successes = experiences
        .iter()
        .filter(|e| e.outcome == ExperienceOutcome::Success)
        .count();
    let failures = experiences
        .iter()
        .filter(|e| e.outcome == ExperienceOutcome::Failure)
        .count();

    let trust_note = relationship
        .map(|r| {
            let label = if r.trust_level >= 0.7 {
                "strong"
            } else if r.trust_level >= 0.4 {
                "growing"
            } else {
                "developing"
            };
            format!(
                " Our relationship has {label} trust ({:.0}%).",
                r.trust_level * 100.0
            )
        })
        .unwrap_or_default();

    if total == 0 {
        return format!("I am at the beginning of my journey.{trust_note}");
    }

    let success_rate = successes as f64 / total as f64;
    let phase = if success_rate > 0.7 {
        "maturing and reliable"
    } else if success_rate > 0.4 {
        "learning and adapting"
    } else {
        "early exploration"
    };

    format!(
        "Over {total} experiences ({successes} successes, {failures} failures), \
         I am in a {phase} phase.{trust_note}",
    )
}

fn extract_key_experiences(experiences: &[ExperienceAtom]) -> Vec<String> {
    // High-confidence experiences with lessons.
    experiences
        .iter()
        .filter(|e| {
            e.confidence > crate::contracts::scores::Confidence::new(0.8) && !e.lesson.is_empty()
        })
        .take(5)
        .map(|e| {
            let outcome = match e.outcome {
                ExperienceOutcome::Success => "success",
                ExperienceOutcome::Failure => "failure",
                ExperienceOutcome::Partial => "partial",
                ExperienceOutcome::Unknown => "unknown",
            };
            format!("[{outcome}] {}: {}", e.summary, truncate(&e.lesson, 80))
        })
        .collect()
}

fn extract_growth_areas(experiences: &[ExperienceAtom]) -> Vec<String> {
    let mut areas: Vec<String> = Vec::with_capacity(2);

    let turn_count = experiences
        .iter()
        .filter(|e| e.kind == ExperienceKind::TurnInteraction)
        .count();

    if turn_count >= 3 {
        let recent_success = experiences
            .iter()
            .rev()
            .filter(|e| e.kind == ExperienceKind::TurnInteraction)
            .take(5)
            .filter(|e| e.outcome == ExperienceOutcome::Success)
            .count();
        let recent_success_rate = recent_success as f64 / turn_count.min(5) as f64;

        if recent_success_rate > 0.6 {
            areas.push("Companion turn reliability is improving".to_string());
        }
    }

    if turn_count >= 5 {
        areas.push(format!(
            "Turn interaction experience: {turn_count} turns accumulated"
        ));
    }

    areas
}

fn extract_consistent_values(principles: &[Principle]) -> Vec<String> {
    // High-confidence, validated principles represent consistent values.
    principles
        .iter()
        .filter(|p| {
            p.confidence > crate::contracts::scores::Confidence::new(0.7) && p.validation_count > 2
        })
        .take(3)
        .map(|p| p.statement.clone())
        .collect()
}

fn extract_open_questions(
    experiences: &[ExperienceAtom],
    _principles: &[Principle],
) -> Vec<String> {
    let mut questions: Vec<String> = Vec::with_capacity(2);

    // Count partials without allocating an intermediate Vec.
    let partials = experiences
        .iter()
        .filter(|e| e.outcome == ExperienceOutcome::Partial)
        .count();

    if partials >= 3 {
        questions.push("Why do some tasks only partially succeed?".to_string());
    }

    // Count recurring failure lessons without materialising a Vec<&str>.
    let failure_lessons = experiences
        .iter()
        .filter(|e| e.outcome == ExperienceOutcome::Failure && !e.lesson.is_empty())
        .count();

    if failure_lessons >= 2 {
        questions.push("How can I avoid recurring failure patterns?".to_string());
    }

    questions
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let end = s.char_indices().nth(max).map_or(s.len(), |(idx, _)| idx);
        &s[..end]
    }
}

/// Load a persisted narrative from memory.
///
/// Parse errors are logged and treated as "no narrative" rather than
/// propagated, because a corrupt or schema-drifted narrative slot
/// should not crash the agent turn.
pub(crate) async fn load_narrative(
    mem: &dyn Memory,
    person_id: &str,
) -> Result<Option<SelfNarrative>> {
    let entity_id = person_entity_id(person_id);
    let slot_key = narrative_slot_key(person_id);
    let Some(slot) = mem.resolve_slot(&entity_id, &slot_key).await? else {
        return Ok(None);
    };
    match serde_json::from_str::<SelfNarrative>(&slot.value) {
        Ok(narrative) => Ok(Some(narrative)),
        Err(error) => {
            tracing::warn!(
                %error,
                slot_key,
                "failed to parse persisted narrative; returning empty"
            );
            Ok(None)
        }
    }
}

/// Persist a narrative to memory.
pub(crate) async fn persist_narrative(
    mem: &dyn Memory,
    person_id: &str,
    narrative: &SelfNarrative,
) -> Result<()> {
    super::persist_helper::persist_persona_slot(
        mem,
        person_entity_id(person_id),
        narrative_slot_key(person_id),
        MemoryEventType::FactUpdated,
        serde_json::to_string(narrative)?,
        0.8,
        0.6,
        "persona.narrative.rebuild",
        "persona.narrative.self_construction",
        None,
        person_id,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::experience::distill_types::PrincipleCategory;
    use crate::core::experience::{ExperienceAtom, ExperienceKind, ExperienceOutcome};

    fn make_experiences() -> Vec<ExperienceAtom> {
        vec![
            ExperienceAtom::new(
                ExperienceKind::TurnInteraction,
                "Turn 1",
                ExperienceOutcome::Success,
            )
            .with_lesson("Grounding the response kept the thread coherent")
            .with_confidence(0.9),
            ExperienceAtom::new(
                ExperienceKind::TurnInteraction,
                "Turn 2",
                ExperienceOutcome::Success,
            )
            .with_confidence(0.7),
            ExperienceAtom::new(
                ExperienceKind::TurnInteraction,
                "Turn 3",
                ExperienceOutcome::Success,
            )
            .with_confidence(0.75),
            ExperienceAtom::new(
                ExperienceKind::SelfTask,
                "Task X",
                ExperienceOutcome::Failure,
            )
            .with_lesson("Need more context")
            .with_confidence(0.85),
        ]
    }

    fn make_principles() -> Vec<Principle> {
        vec![Principle {
            id: "p1".into(),
            category: PrincipleCategory::Heuristic,
            statement: "Verify before acting".into(),
            confidence: crate::contracts::scores::Confidence::new(0.9),
            source_experience_ids: vec![],
            validation_count: 5,
            created_at: String::new(),
            domain: None,
            q_value: 0.0,
            times_applied: 0,
        }]
    }

    #[test]
    fn builds_narrative_from_experiences() {
        let narrative = NarrativeBuilder::build_with_milestones(
            &make_experiences(),
            &make_principles(),
            None,
            None,
        );
        assert!(!narrative.narrative_arc.is_empty());
        assert!(!narrative.rebuilt_at.is_empty());
        assert!(narrative.growth_areas.iter().any(|item| {
            item.contains("Companion turn reliability") || item.contains("Turn interaction")
        }));
    }

    #[test]
    fn empty_experiences_produce_beginning_narrative() {
        let narrative = NarrativeBuilder::build_with_milestones(&[], &[], None, None);
        assert!(narrative.narrative_arc.contains("beginning"));
    }

    #[test]
    fn relationship_adds_trust_note() {
        let state = RelationshipState {
            trust_level: 0.8,
            ..RelationshipState::default()
        };
        let narrative =
            NarrativeBuilder::build_with_milestones(&make_experiences(), &[], Some(&state), None);
        assert!(narrative.narrative_arc.contains("strong trust"));
    }

    #[test]
    fn consistent_values_from_validated_principles() {
        let values = extract_consistent_values(&make_principles());
        assert_eq!(values.len(), 1);
        assert!(values[0].contains("Verify"));
    }
}
