//! Affect decay helpers: label bridge, composite relevance, and per-turn decay math.

use serde::{Deserialize, Serialize};

use crate::config::schema::EmotionDecayRates;
use crate::contracts::affect::{AffectLabel, AffectNodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DecayAffectLabel {
    Neutral,
    Confusion,
    Frustration,
    Anxiety,
    Sadness,
    Anger,
    Joy,
    Gratitude,
    Curiosity,
    Overload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct AffectOntologyBridge {
    pub raw_label: AffectLabel,
    pub decay_label: DecayAffectLabel,
    pub topology_nodes: Vec<(AffectNodeId, f32)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RelevanceContext {
    pub topic_overlap: f64,
    pub objective_overlap: f64,
    pub open_loop_overlap: f64,
    pub entity_continuity: f64,
    pub social_salience: f64,
}

impl Default for RelevanceContext {
    fn default() -> Self {
        Self {
            topic_overlap: 1.0,
            objective_overlap: 0.5,
            open_loop_overlap: 0.5,
            entity_continuity: 1.0,
            social_salience: 0.5,
        }
    }
}

impl RelevanceContext {
    pub(crate) fn from_label_transition(
        active_label: DecayAffectLabel,
        current_label: AffectLabel,
    ) -> Self {
        let current = bridge_affect_label(current_label);
        let same_family = current.decay_label == active_label;
        let current_negative = matches!(
            current.decay_label,
            DecayAffectLabel::Frustration
                | DecayAffectLabel::Anxiety
                | DecayAffectLabel::Sadness
                | DecayAffectLabel::Anger
                | DecayAffectLabel::Overload
                | DecayAffectLabel::Confusion
        );
        let active_negative = matches!(
            active_label,
            DecayAffectLabel::Frustration
                | DecayAffectLabel::Anxiety
                | DecayAffectLabel::Sadness
                | DecayAffectLabel::Anger
                | DecayAffectLabel::Overload
                | DecayAffectLabel::Confusion
        );

        Self {
            topic_overlap: if same_family {
                1.0
            } else if current.decay_label == DecayAffectLabel::Neutral {
                0.55
            } else {
                0.25
            },
            objective_overlap: if active_negative == current_negative {
                0.8
            } else {
                0.35
            },
            open_loop_overlap: if matches!(
                current.decay_label,
                DecayAffectLabel::Overload
                    | DecayAffectLabel::Anxiety
                    | DecayAffectLabel::Frustration
            ) && active_negative
            {
                0.85
            } else {
                0.4
            },
            entity_continuity: 1.0,
            social_salience: if matches!(
                current.decay_label,
                DecayAffectLabel::Anxiety
                    | DecayAffectLabel::Sadness
                    | DecayAffectLabel::Gratitude
                    | DecayAffectLabel::Overload
            ) {
                0.8
            } else {
                0.5
            },
        }
    }
}

pub(crate) fn bridge_affect_label(raw_label: AffectLabel) -> AffectOntologyBridge {
    let (decay_label, topology_nodes): (DecayAffectLabel, &[_]) = match raw_label {
        AffectLabel::Neutral => (DecayAffectLabel::Neutral, &[("guardedness", 0.15)]),
        AffectLabel::Confused => (
            DecayAffectLabel::Confusion,
            &[("guardedness", 0.5), ("curiosity", 0.35)],
        ),
        AffectLabel::Frustrated => (
            DecayAffectLabel::Frustration,
            &[("anger", 0.55), ("guardedness", 0.35)],
        ),
        AffectLabel::Anxious => (
            DecayAffectLabel::Anxiety,
            &[("anxiety", 0.7), ("guardedness", 0.4)],
        ),
        AffectLabel::Sad => (
            DecayAffectLabel::Sadness,
            &[("loneliness", 0.5), ("emptiness", 0.4)],
        ),
        AffectLabel::Angry => (
            DecayAffectLabel::Anger,
            &[("anger", 0.8), ("guardedness", 0.25)],
        ),
        AffectLabel::Excited => (DecayAffectLabel::Joy, &[("joy", 0.75), ("relief", 0.25)]),
        AffectLabel::Grateful => (
            DecayAffectLabel::Gratitude,
            &[("relief", 0.4), ("attachment", 0.4), ("joy", 0.3)],
        ),
        AffectLabel::Curious => (
            DecayAffectLabel::Curiosity,
            &[("curiosity", 0.8), ("longing", 0.2)],
        ),
        AffectLabel::Overwhelmed => (
            DecayAffectLabel::Overload,
            &[("guardedness", 0.5), ("anxiety", 0.45), ("emptiness", 0.2)],
        ),
    };

    AffectOntologyBridge {
        raw_label,
        decay_label,
        topology_nodes: topology_nodes
            .iter()
            .map(|(node, weight)| (AffectNodeId((*node).to_string()), *weight))
            .collect(),
    }
}

pub(crate) fn composite_relevance(context: &RelevanceContext) -> f64 {
    let weighted = context.topic_overlap * 0.35
        + context.objective_overlap * 0.20
        + context.open_loop_overlap * 0.20
        + context.entity_continuity * 0.15
        + context.social_salience * 0.10;
    weighted.clamp(0.1, 1.0)
}

pub(crate) fn decay_rate_for_decay_label(
    label: DecayAffectLabel,
    rates: &EmotionDecayRates,
) -> f64 {
    match label {
        DecayAffectLabel::Neutral => 0.50,
        DecayAffectLabel::Confusion => 0.16,
        DecayAffectLabel::Frustration => rates.frustration,
        DecayAffectLabel::Anxiety | DecayAffectLabel::Overload => rates.anxiety,
        DecayAffectLabel::Sadness => rates.sadness,
        DecayAffectLabel::Anger => rates.anger,
        DecayAffectLabel::Joy => rates.joy,
        DecayAffectLabel::Gratitude => rates.gratitude,
        DecayAffectLabel::Curiosity => rates.curiosity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_maps_excited_to_joy_and_topology_nodes() {
        let bridge = bridge_affect_label(AffectLabel::Excited);
        assert_eq!(bridge.decay_label, DecayAffectLabel::Joy);
        assert!(!bridge.topology_nodes.is_empty());
    }

    #[test]
    fn composite_relevance_respects_context_mix() {
        let strong = composite_relevance(&RelevanceContext {
            topic_overlap: 1.0,
            objective_overlap: 1.0,
            open_loop_overlap: 1.0,
            entity_continuity: 1.0,
            social_salience: 1.0,
        });
        let weak = composite_relevance(&RelevanceContext {
            topic_overlap: 0.1,
            objective_overlap: 0.1,
            open_loop_overlap: 0.1,
            entity_continuity: 1.0,
            social_salience: 0.1,
        });
        assert!(strong > weak);
    }
}
