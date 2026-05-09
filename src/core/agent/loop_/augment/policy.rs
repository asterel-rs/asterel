//! Turn policy: governed policy selection functions (ADR-0010).
//!
//! Types are defined in `policy_types.rs` and re-exported here.
//! Provides affect-modulated policy adjustment.

#[path = "policy_types.rs"]
mod policy_types;

use num_traits::ToPrimitive;

// Re-export OutcomeScore for downstream use (test + future integration)
pub(crate) use policy_types::OutcomeScore;
pub(crate) use policy_types::{
    DomainTag, MemoryPolicy, PolicyDecision, ReasoningStrategy, SituationFeatures, TurnOutcome,
    TurnSignals,
};

use crate::core::affect::AffectLabel;

/// Modulate a base policy for the detected affect state and intensity.
///
/// Low-intensity (< 0.3) affect triggers only minor memory parameter tweaks.
/// High-intensity affect can override the reasoning strategy entirely:
///
/// | Label | Strategy override | Memory change |
/// |-------|------------------|---------------|
/// | Confused | Stepwise | +2–6 `retrieve_top_k` (intensity-proportional) |
/// | Frustrated | VerifyFirst | `min_facts` ≥ 2 |
/// | Anxious | AskClarify | `min_facts` ≥ 1, +3 `retrieve_top_k` |
/// | Angry | (unchanged) | `noise_budget` = 0 |
/// | Overwhelmed | Stepwise | `noise_budget` ≤ 1 |
/// | Others | (unchanged) | (unchanged) |
#[must_use]
pub(crate) fn modulate_policy_for_affect(
    base: &PolicyDecision,
    affect_label: AffectLabel,
    intensity: f32,
) -> PolicyDecision {
    let mut policy = base.clone();

    if intensity < 0.3 {
        match affect_label {
            AffectLabel::Confused | AffectLabel::Frustrated | AffectLabel::Anxious => {
                policy.memory.retrieve_top_k = policy.memory.retrieve_top_k.saturating_add(1);
            }
            AffectLabel::Angry => {
                policy.memory.noise_budget = policy.memory.noise_budget.min(1);
            }
            AffectLabel::Neutral
            | AffectLabel::Sad
            | AffectLabel::Excited
            | AffectLabel::Grateful
            | AffectLabel::Curious
            | AffectLabel::Overwhelmed => {}
        }
        return policy;
    }

    match affect_label {
        AffectLabel::Confused => {
            policy.reasoning = ReasoningStrategy::Stepwise;
            // Intensity-proportional top_k: +2 at low intensity, up to +6 at full
            let boost = bounded_intensity_steps(intensity, 2.0, 6.0, 6.0, 2);
            policy.memory.retrieve_top_k = policy.memory.retrieve_top_k.saturating_add(boost);
        }
        AffectLabel::Frustrated => {
            policy.reasoning = ReasoningStrategy::VerifyFirst;
            policy.memory.min_facts = policy.memory.min_facts.max(2);
        }
        AffectLabel::Anxious => {
            policy.reasoning = ReasoningStrategy::AskClarify;
            policy.memory.min_facts = policy.memory.min_facts.max(1);
            policy.memory.retrieve_top_k = policy.memory.retrieve_top_k.saturating_add(3);
        }
        AffectLabel::Angry => {
            policy.memory.noise_budget = 0;
        }
        AffectLabel::Overwhelmed => {
            policy.reasoning = ReasoningStrategy::Stepwise;
            policy.memory.noise_budget = policy.memory.noise_budget.min(1);
        }
        AffectLabel::Sad
        | AffectLabel::Neutral
        | AffectLabel::Excited
        | AffectLabel::Grateful
        | AffectLabel::Curious => {}
    }

    policy
}

// ── Situation extraction ─────────────────────────────────────────

/// Extract situation features from the user message and affect reading.
///
/// Phase 1 uses simple keyword heuristics. Later phases can use
/// classifiers or LLM-based extraction.
#[must_use]
pub(crate) fn extract_situation(
    user_message: &str,
    affect_label: AffectLabel,
    affect_confidence: f64,
) -> SituationFeatures {
    let lower = user_message.to_lowercase();

    let domain = if contains_technical_keywords(&lower) {
        DomainTag::Technical
    } else if contains_creative_keywords(&lower) {
        DomainTag::Creative
    } else if contains_personal_keywords(&lower) {
        DomainTag::Personal
    } else if contains_admin_keywords(&lower) {
        DomainTag::Administrative
    } else {
        DomainTag::General
    };

    let word_count = user_message.split_whitespace().count();
    let has_question = user_message.contains('?');
    let complexity: f32 = match (word_count, has_question) {
        (0..=5, false) => 0.1,
        (0..=10, _) => 0.3,
        (11..=50, false) => 0.4,
        (11..=50, true) | (_, false) => 0.6,
        (_, true) => 0.8,
    };

    SituationFeatures {
        domain,
        complexity,
        affect_label,
        affect_intensity: bounded_unit_ratio_to_f32(affect_confidence),
    }
}

/// Map a normalised intensity value to a discrete step count.
///
/// Formula: `round(intensity × scale)`, clamped to `[min, max]`.
/// `fallback` is returned on any numeric overflow (unreachable in practice).
fn bounded_intensity_steps(
    intensity: f32,
    min: f32,
    max: f32,
    scale: f32,
    fallback: usize,
) -> usize {
    intensity
        .mul_add(scale, 0.0)
        .round()
        .clamp(min, max)
        .to_usize()
        .unwrap_or(fallback)
}

/// Clamp a `f64` to `[0.0, 1.0]` and downcast to `f32`.
///
/// Non-finite inputs (NaN, ±inf) are mapped to 0.0 to prevent
/// downstream clamping anomalies.
fn bounded_unit_ratio_to_f32(value: f64) -> f32 {
    let bounded = if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    bounded.to_f32().unwrap_or(0.0)
}

fn contains_technical_keywords(text: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "code",
        "error",
        "bug",
        "compile",
        "build",
        "deploy",
        "api",
        "function",
        "module",
        "test",
        "debug",
        "fix",
        "implement",
        "refactor",
        "database",
        "server",
        "config",
    ];
    KEYWORDS.iter().any(|kw| text.contains(kw))
}

fn contains_creative_keywords(text: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "write",
        "story",
        "poem",
        "design",
        "imagine",
        "brainstorm",
        "idea",
        "creative",
    ];
    KEYWORDS.iter().any(|kw| text.contains(kw))
}

fn contains_personal_keywords(text: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "feeling", "emotion", "stress", "worry", "anxious", "happy", "sad", "angry", "advice",
    ];
    KEYWORDS.iter().any(|kw| text.contains(kw))
}

fn contains_admin_keywords(text: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "schedule", "meeting", "email", "remind", "organize", "calendar", "todo",
    ];
    KEYWORDS.iter().any(|kw| text.contains(kw))
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modulate_policy_confused_high_intensity_switches_to_stepwise() {
        let base = PolicyDecision::default();
        let modulated = modulate_policy_for_affect(&base, AffectLabel::Confused, 0.9);

        assert_eq!(modulated.reasoning, ReasoningStrategy::Stepwise);
        // Intensity 0.9 → boost = round(0.9*6) = 5, clamped to [2,6]
        assert_eq!(
            modulated.memory.retrieve_top_k,
            base.memory.retrieve_top_k + 5
        );
    }

    #[test]
    fn modulate_policy_confused_low_intensity_small_boost() {
        let base = PolicyDecision::default();
        let modulated = modulate_policy_for_affect(&base, AffectLabel::Confused, 0.35);

        assert_eq!(modulated.reasoning, ReasoningStrategy::Stepwise);
        // Intensity 0.35 → boost = round(0.35*6) = 2, clamped to [2,6]
        assert_eq!(
            modulated.memory.retrieve_top_k,
            base.memory.retrieve_top_k + 2
        );
    }

    #[test]
    fn modulate_policy_frustrated_switches_to_verify_first() {
        let base = PolicyDecision::default();
        let modulated = modulate_policy_for_affect(&base, AffectLabel::Frustrated, 0.8);

        assert_eq!(modulated.reasoning, ReasoningStrategy::VerifyFirst);
        assert_eq!(modulated.memory.min_facts, 2);
    }

    #[test]
    fn modulate_policy_neutral_returns_base_unchanged() {
        let base = PolicyDecision::default();
        let modulated = modulate_policy_for_affect(&base, AffectLabel::Neutral, 0.9);

        assert_eq!(modulated.reasoning, base.reasoning);
        assert_eq!(modulated.memory.retrieve_top_k, base.memory.retrieve_top_k);
        assert_eq!(modulated.memory.min_facts, base.memory.min_facts);
        assert_eq!(modulated.memory.noise_budget, base.memory.noise_budget);
    }

    #[test]
    fn modulate_policy_low_intensity_minimal_change() {
        let base = PolicyDecision::default();
        let modulated = modulate_policy_for_affect(&base, AffectLabel::Confused, 0.2);

        assert_eq!(modulated.reasoning, base.reasoning);
        assert_eq!(
            modulated.memory.retrieve_top_k,
            base.memory.retrieve_top_k + 1
        );
    }

    #[test]
    fn extract_situation_detects_technical_domain() {
        let situation = extract_situation(
            "Can you fix this compile error in my code?",
            AffectLabel::Neutral,
            0.8,
        );
        assert_eq!(situation.domain, DomainTag::Technical);
    }

    #[test]
    fn extract_situation_detects_creative_domain() {
        let situation =
            extract_situation("Write me a story about a dragon", AffectLabel::Excited, 0.6);
        assert_eq!(situation.domain, DomainTag::Creative);
    }

    #[test]
    fn extract_situation_detects_personal_domain() {
        let situation = extract_situation(
            "I've been feeling really anxious lately",
            AffectLabel::Anxious,
            0.7,
        );
        assert_eq!(situation.domain, DomainTag::Personal);
    }

    #[test]
    fn extract_situation_defaults_to_general() {
        let situation = extract_situation("hello", AffectLabel::Neutral, 0.9);
        assert_eq!(situation.domain, DomainTag::General);
    }

    #[test]
    fn extract_situation_complexity_increases_with_length_and_questions() {
        let short = extract_situation("hi", AffectLabel::Neutral, 0.9);
        let long_question = extract_situation(
            "Can you explain how the memory influence system works and how it \
             integrates with the augmentation pipeline for grounding?",
            AffectLabel::Neutral,
            0.9,
        );
        assert!(long_question.complexity > short.complexity);
    }
}
