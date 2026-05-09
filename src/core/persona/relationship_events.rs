//! Notable relationship event detection from affect intensity,
//! outcome success, and trust/rapport thresholds.

use chrono::Utc;

use crate::contracts::affect::AffectLabel;
use crate::core::persona::relationship::{RelEvent, RelEventKind, RelationshipState};

/// Maximum number of notable events retained per relationship.
const MAX_NOTABLE_EVENTS: usize = 20;

/// Relationship event inputs for a completed turn.
///
/// `before` and `after` are both required so threshold-crossing events are
/// detected from the actual state transition rather than by re-projecting from
/// the already-updated state.
pub(crate) struct RelationshipEventInput<'a> {
    pub affect_label: AffectLabel,
    pub affect_intensity: f32,
    pub outcome_success: f32,
    pub before: &'a RelationshipState,
    pub after: &'a RelationshipState,
}

/// Detect a notable event from the current turn's affect and relationship state,
/// returning `Some(RelEvent)` if significance thresholds are crossed.
pub(crate) fn detect_notable_event(input: &RelationshipEventInput<'_>) -> Option<RelEvent> {
    let now = Utc::now().to_rfc3339();
    let RelationshipEventInput {
        affect_label,
        affect_intensity,
        outcome_success,
        before,
        after,
    } = *input;

    // High-intensity emotional moment (intensity > 0.7, non-neutral affect).
    if affect_intensity > 0.7 && affect_label != AffectLabel::Neutral {
        let summary = format!(
            "{affect_label:?} moment (intensity={affect_intensity:.2}, success={outcome_success:.2})"
        );
        return Some(RelEvent {
            kind: RelEventKind::EmotionalMoment,
            summary,
            timestamp: now,
            significance: significance_score(affect_intensity, outcome_success),
        });
    }

    // Trust crossing 0.7 threshold (upward) from the actual state transition.
    if before.trust_level < 0.7 && after.trust_level >= 0.7 {
        return Some(RelEvent {
            kind: RelEventKind::TrustSignal,
            summary: format!(
                "Trust level crossing 0.7 threshold (from {:.2} to {:.2})",
                before.trust_level, after.trust_level
            ),
            timestamp: now,
            significance: 0.8,
        });
    }

    // Strong negative feedback signal.
    if outcome_success < 0.3 && affect_intensity > 0.5 && is_negative_affect(affect_label) {
        return Some(RelEvent {
            kind: RelEventKind::NegativeFeedback,
            summary: format!("Negative signal (success={outcome_success:.2}, {affect_label:?})"),
            timestamp: now,
            significance: significance_score(affect_intensity, 1.0 - outcome_success),
        });
    }

    if after.unresolved_tension > 0.65 || after.repair_debt > 0.55 {
        return Some(RelEvent {
            kind: RelEventKind::Rupture,
            summary: format!(
                "Relationship rupture pressure (tension={:.2}, repair_debt={:.2})",
                after.unresolved_tension, after.repair_debt
            ),
            timestamp: now,
            significance: significance_score(after.unresolved_tension, after.repair_debt),
        });
    }

    let tension_repaired = before.unresolved_tension > after.unresolved_tension + 0.15;
    let debt_repaired = before.repair_debt > after.repair_debt + 0.15;
    if outcome_success > 0.8
        && after.repair_debt < 0.2
        && after.unresolved_tension < 0.2
        && (tension_repaired || debt_repaired)
        && is_positive_affect(affect_label)
    {
        return Some(RelEvent {
            kind: RelEventKind::Repair,
            summary: format!("Repair succeeded (success={outcome_success:.2}, {affect_label:?})"),
            timestamp: now,
            significance: significance_score(affect_intensity, outcome_success),
        });
    }

    // Strong positive feedback signal.
    if outcome_success >= 0.85 && affect_intensity > 0.5 && is_positive_affect(affect_label) {
        return Some(RelEvent {
            kind: RelEventKind::PositiveFeedback,
            summary: format!(
                "Strong positive signal (success={outcome_success:.2}, {affect_label:?})"
            ),
            timestamp: now,
            significance: significance_score(affect_intensity, outcome_success),
        });
    }

    None
}

/// Remove oldest events when the list exceeds `MAX_NOTABLE_EVENTS`.
pub(crate) fn prune_old_events(events: &mut Vec<RelEvent>) {
    if events.len() > MAX_NOTABLE_EVENTS {
        // Sort by significance descending, keep top N.
        events.sort_by(|a, b| {
            b.significance
                .partial_cmp(&a.significance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        events.truncate(MAX_NOTABLE_EVENTS);
    }
}

/// Compute a significance score from affect intensity and outcome extremity.
pub(crate) fn significance_score(affect_intensity: f32, outcome_extremity: f32) -> f32 {
    (affect_intensity * 0.5 + outcome_extremity * 0.5).clamp(0.0, 1.0)
}

fn is_positive_affect(label: AffectLabel) -> bool {
    matches!(
        label,
        AffectLabel::Excited | AffectLabel::Grateful | AffectLabel::Curious
    )
}

fn is_negative_affect(label: AffectLabel) -> bool {
    matches!(
        label,
        AffectLabel::Angry
            | AffectLabel::Frustrated
            | AffectLabel::Sad
            | AffectLabel::Anxious
            | AffectLabel::Overwhelmed
    )
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_NOTABLE_EVENTS, RelationshipEventInput, detect_notable_event, prune_old_events,
        significance_score,
    };
    use crate::contracts::affect::AffectLabel;
    use crate::core::persona::relationship::{RelEvent, RelEventKind, RelationshipState};

    fn default_state() -> RelationshipState {
        RelationshipState::default()
    }

    fn event_input<'a>(
        affect_label: AffectLabel,
        affect_intensity: f32,
        outcome_success: f32,
        before: &'a RelationshipState,
        after: &'a RelationshipState,
    ) -> RelationshipEventInput<'a> {
        RelationshipEventInput {
            affect_label,
            affect_intensity,
            outcome_success,
            before,
            after,
        }
    }

    #[test]
    fn detect_emotional_moment_high_intensity() {
        let before = default_state();
        let after = default_state();
        let event =
            detect_notable_event(&event_input(AffectLabel::Angry, 0.9, 0.3, &before, &after));
        assert!(event.is_some());
        let event = event.unwrap();
        assert!(matches!(event.kind, RelEventKind::EmotionalMoment));
        assert!(event.significance > 0.0);
    }

    #[test]
    fn no_event_for_neutral_low_intensity() {
        let before = default_state();
        let after = default_state();
        let event = detect_notable_event(&event_input(
            AffectLabel::Neutral,
            0.3,
            0.6,
            &before,
            &after,
        ));
        assert!(event.is_none());
    }

    #[test]
    fn detect_trust_threshold_crossing() {
        let mut before = default_state();
        before.trust_level = 0.69;
        let mut after = before.clone();
        after.trust_level = 0.71;
        let event = detect_notable_event(&event_input(
            AffectLabel::Neutral,
            0.3,
            0.9,
            &before,
            &after,
        ));
        assert!(event.is_some());
        let event = event.unwrap();
        assert!(matches!(event.kind, RelEventKind::TrustSignal));
    }

    #[test]
    fn no_trust_event_without_actual_threshold_crossing() {
        let mut before = default_state();
        before.trust_level = 0.69;
        let mut after = before.clone();
        after.trust_level = 0.69;
        let event = detect_notable_event(&event_input(
            AffectLabel::Neutral,
            0.3,
            0.9,
            &before,
            &after,
        ));
        assert!(event.is_none());
    }

    #[test]
    fn detect_positive_feedback() {
        // Use intensity=0.6 (below 0.7 EmotionalMoment threshold, above 0.5 PositiveFeedback threshold).
        let before = default_state();
        let after = default_state();
        let event = detect_notable_event(&event_input(
            AffectLabel::Excited,
            0.6,
            0.9,
            &before,
            &after,
        ));
        assert!(event.is_some());
        let event = event.unwrap();
        assert!(matches!(event.kind, RelEventKind::PositiveFeedback));
    }

    #[test]
    fn detect_negative_feedback() {
        let before = default_state();
        let after = default_state();
        let event = detect_notable_event(&event_input(
            AffectLabel::Frustrated,
            0.7,
            0.1,
            &before,
            &after,
        ));
        assert!(event.is_some());
        let event = event.unwrap();
        assert!(matches!(event.kind, RelEventKind::NegativeFeedback));
    }

    #[test]
    fn repair_requires_actual_debt_or_tension_reduction() {
        let mut before = default_state();
        before.repair_debt = 0.36;
        before.unresolved_tension = 0.25;
        let mut after = before.clone();
        after.repair_debt = 0.12;
        after.unresolved_tension = 0.1;
        let event = detect_notable_event(&event_input(
            AffectLabel::Grateful,
            0.6,
            0.9,
            &before,
            &after,
        ));
        assert!(event.is_some());
        assert!(matches!(event.unwrap().kind, RelEventKind::Repair));

        let event = detect_notable_event(&event_input(
            AffectLabel::Grateful,
            0.6,
            0.9,
            &after,
            &after,
        ));
        assert!(!matches!(event.unwrap().kind, RelEventKind::Repair));
    }

    #[test]
    fn prune_keeps_max_events_by_significance() {
        let mut events: Vec<RelEvent> = (0..25)
            .map(|i| RelEvent {
                kind: RelEventKind::EmotionalMoment,
                summary: format!("event {i}"),
                timestamp: String::new(),
                significance: u16::try_from(i).map_or(f32::from(u16::MAX), f32::from) / 25.0,
            })
            .collect();
        prune_old_events(&mut events);
        assert_eq!(events.len(), MAX_NOTABLE_EVENTS);
        // Most significant events retained.
        assert!(events[0].significance >= events[events.len() - 1].significance);
    }

    #[test]
    fn significance_score_clamps() {
        assert!((significance_score(1.0, 1.0) - 1.0).abs() < f32::EPSILON);
        assert!((significance_score(0.0, 0.0) - 0.0).abs() < f32::EPSILON);
    }
}
