//! Curiosity signal computation from attention diversity and affect.
//!
//! Drives the agent's exploratory behaviour when uncertainty is high
//! and the user's emotional state is non-negative.
//!
//! References: [CURIOSITY-ICM] Pathak et al., 2017 — Intrinsic Curiosity
//! Module. See the public research reference index in the docs site.
#![allow(clippy::cast_precision_loss)]

use serde::{Deserialize, Serialize};

use crate::contracts::affect::AffectLabel;
use crate::core::persona::attention::AttentionSchema;

/// A curiosity signal indicating the agent's drive to explore a topic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CuriositySignal {
    /// Overall curiosity score in `[0.0, 1.0]`.
    pub curiosity_score: f64,
    /// Topics that triggered curiosity.
    pub trigger_topics: Vec<String>,
    /// Suggested explorations the agent might proactively pursue.
    pub suggested_explorations: Vec<String>,
}

/// Compute a curiosity signal from the attention schema and affect state.
///
/// Curiosity is triggered when:
/// - Uncertainty is high (attention has diverse topics)
/// - Complexity is moderate (not trivially simple, not overwhelmingly complex)
/// - User affect is non-negative (don't be curious when the user is upset)
pub(crate) fn compute_curiosity(
    attention: &AttentionSchema,
    complexity: f64,
    affect_label: AffectLabel,
    affect_intensity: f64,
    threshold: f64,
) -> Option<CuriositySignal> {
    // Don't trigger curiosity on negative affect.
    if is_negative_affect(affect_label) {
        return None;
    }

    // Diversity factor: more diverse attention → higher uncertainty → more curiosity.
    let diversity = if attention.entries.is_empty() {
        0.0
    } else {
        let avg_score =
            attention.entries.iter().map(|e| e.score).sum::<f64>() / attention.entries.len() as f64;
        // Lower average = more spread out = more uncertain.
        1.0 - avg_score
    };

    // Complexity sweet spot: curiosity peaks at moderate complexity.
    let complexity_factor = 1.0 - (complexity - 0.5).abs() * 2.0;
    let complexity_factor = complexity_factor.max(0.0);

    // Arousal contribution: neutral-to-excited affect boosts curiosity.
    let arousal_factor = if affect_label == AffectLabel::Excited {
        affect_intensity * 0.3
    } else {
        0.0
    };

    let curiosity_score =
        (diversity * 0.4 + complexity_factor * 0.4 + arousal_factor * 0.2).clamp(0.0, 1.0);

    if curiosity_score < threshold {
        return None;
    }

    let trigger_topics: Vec<String> = attention
        .entries
        .iter()
        .filter(|e| e.score < 0.6) // lower-confidence items are curiosity targets
        .map(|e| e.topic.clone())
        .collect();

    let suggested_explorations = trigger_topics
        .iter()
        .take(2)
        .map(|t| format!("Explore further: {t}"))
        .collect();

    Some(CuriositySignal {
        curiosity_score,
        trigger_topics,
        suggested_explorations,
    })
}

fn is_negative_affect(label: AffectLabel) -> bool {
    matches!(
        label,
        AffectLabel::Angry | AffectLabel::Frustrated | AffectLabel::Sad | AffectLabel::Anxious
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::persona::attention::{AttentionSchema, SalienceEntry, SalienceSource};

    fn test_attention() -> AttentionSchema {
        AttentionSchema {
            entries: vec![
                SalienceEntry {
                    topic: "machine learning".into(),
                    score: 0.4,
                    source: SalienceSource::Memory,
                },
                SalienceEntry {
                    topic: "data pipelines".into(),
                    score: 0.3,
                    source: SalienceSource::Experience,
                },
            ],
        }
    }

    #[test]
    fn no_curiosity_on_negative_affect() {
        let signal = compute_curiosity(&test_attention(), 0.5, AffectLabel::Angry, 0.8, 0.3);
        assert!(signal.is_none());
    }

    #[test]
    fn curiosity_triggers_on_moderate_complexity_and_diversity() {
        let signal = compute_curiosity(&test_attention(), 0.5, AffectLabel::Neutral, 0.3, 0.2);
        assert!(signal.is_some());
        let signal = signal.unwrap();
        assert!(signal.curiosity_score >= 0.2);
    }

    #[test]
    fn high_threshold_suppresses_curiosity() {
        let signal = compute_curiosity(&test_attention(), 0.5, AffectLabel::Neutral, 0.3, 0.95);
        assert!(signal.is_none());
    }

    #[test]
    fn empty_attention_produces_low_curiosity() {
        let schema = AttentionSchema::default();
        let signal = compute_curiosity(&schema, 0.5, AffectLabel::Neutral, 0.3, 0.6);
        // Low diversity from empty attention → low score.
        assert!(signal.is_none());
    }
}
