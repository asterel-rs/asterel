//! VAD-first rule-based affect detector.
//!
//! Computes continuous Valence-Arousal-Dominance coordinates from lexical and
//! structural features of the user's message, then derives a discrete affect
//! label via nearest-prototype matching in VAD space.
//!
//! # Why VAD-first?
//!
//! Discrete label matching ("does the message contain 'frustrated'?") fails on
//! indirect expressions and mixed signals. Computing continuous VAD coordinates
//! first captures the *magnitude* and *texture* of the emotional signal, then
//! the label assignment is a geometric operation (nearest prototype in 3D space).
//!
//! Explicit emotion keywords still matter — they apply a 40% distance discount
//! to the matching prototype, ensuring messages like "I'm so frustrated" are
//! classified correctly even when the VAD coordinates are ambiguous.
//!
//! # Pipeline position
//!
//! `RuleBasedDetector` is the synchronous fast path; `hybrid.rs` wraps it and
//! adds optional LLM disambiguation. All detectors share the [`AffectDetector`]
//! trait defined in `llm_detector.rs`.
//!
//! References: `[AFFECTIVE-COMPUTING]` Picard, 1997. See the public research
//! reference index in the docs site.

use super::types::{AffectLabel, AffectReading};
use crate::contracts::scores::Confidence;

/// Rule-based affect detector (v2 — VAD-first).
///
/// Computes continuous Valence-Arousal-Dominance coordinates from
/// lexical and structural features, then derives the discrete label
/// via nearest-prototype matching in VAD space.
pub(crate) struct RuleBasedDetector;

/// VAD prototypes for each affect label, derived from Russell's
/// circumplex model and Mehrabian's PAD framework.
const PROTOTYPES: &[(AffectLabel, f64, f64, f64)] = &[
    // (label, valence, arousal, dominance)
    (AffectLabel::Angry, -0.7, 0.8, 0.7),
    (AffectLabel::Frustrated, -0.5, 0.6, 0.4),
    (AffectLabel::Anxious, -0.4, 0.7, 0.2),
    (AffectLabel::Sad, -0.5, 0.2, 0.3),
    (AffectLabel::Confused, -0.2, 0.4, 0.2),
    (AffectLabel::Excited, 0.7, 0.8, 0.6),
    (AffectLabel::Grateful, 0.6, 0.3, 0.4),
    (AffectLabel::Curious, 0.2, 0.5, 0.3),
    (AffectLabel::Overwhelmed, -0.3, 0.6, 0.1),
    (AffectLabel::Neutral, 0.0, 0.3, 0.5),
];

impl RuleBasedDetector {
    pub(crate) fn new() -> Self {
        Self
    }

    /// Detect affect from a user message using VAD-first analysis.
    ///
    /// 1. Compute continuous V, A, D scores from text features.
    /// 2. Derive discrete label from nearest prototype in VAD space.
    /// 3. Compute confidence from prototype distance.
    #[allow(clippy::unused_self, clippy::cast_precision_loss)]
    pub(crate) fn detect(&self, user_message: &str) -> AffectReading {
        let lower = user_message.to_lowercase();
        let words: Vec<&str> = user_message.split_whitespace().collect();
        let word_count = words.len();

        if word_count == 0 {
            return AffectReading::neutral();
        }

        let question_marks = user_message.chars().filter(|c| *c == '?').count();
        let exclamation_marks = user_message.chars().filter(|c| *c == '!').count();
        let alpha_count = user_message
            .chars()
            .filter(|c| c.is_alphabetic())
            .count()
            .max(1);
        let upper_count = user_message.chars().filter(|c| c.is_uppercase()).count();
        let caps_ratio = upper_count as f64 / alpha_count as f64;

        // ── Valence [-1, 1] ─────────────────────────────────────────
        let positive_hits = count_hits(&lower, POSITIVE_WORDS);
        let negative_hits = count_hits(&lower, NEGATIVE_WORDS);
        let total_sentiment = (positive_hits + negative_hits).max(1) as f64;
        let sentiment_ratio = (positive_hits as f64 - negative_hits as f64) / total_sentiment;
        let excl_boost = if exclamation_marks > 0 && positive_hits > 0 {
            0.1
        } else if exclamation_marks > 0 && negative_hits > 0 {
            -0.1
        } else {
            0.0
        };
        let valence = (sentiment_ratio + excl_boost).clamp(-1.0, 1.0);

        // ── Arousal [0, 1] ──────────────────────────────────────────
        let caps_component = caps_ratio * 0.3;
        let punct_density =
            (question_marks + exclamation_marks) as f64 / (word_count as f64).max(1.0);
        let punct_component = punct_density.min(1.0) * 0.3;
        let urgency_hits = count_hits(&lower, URGENCY_WORDS);
        let urgency_component = (urgency_hits as f64 / 2.0).min(1.0) * 0.3;
        let emotion_arousal_hits = count_hits(&lower, HIGH_AROUSAL_EMOTION_WORDS);
        let emotion_component = (emotion_arousal_hits as f64 / 2.0).min(1.0) * 0.3;
        let arousal = (caps_component + punct_component + urgency_component + emotion_component)
            .clamp(0.0, 1.0);

        // ── Dominance [0, 1] ────────────────────────────────────────
        let imperative_score = if is_imperative_start(words.first().copied().unwrap_or("")) {
            0.4
        } else {
            0.0
        };
        let first_person = count_first_person(&lower, word_count);
        let hedging = count_hits(&lower, HEDGING_WORDS) as f64 / (word_count as f64).max(1.0);
        let dominance =
            (0.5 + imperative_score + first_person * 0.3 - hedging * 0.3).clamp(0.0, 1.0);

        // ── Label from nearest prototype ────────────────────────────
        let (label, distance) = nearest_prototype(valence, arousal, dominance, &lower);

        // Confidence: inversely proportional to distance from prototype.
        // Close to prototype → high confidence, far away → low.
        let confidence = (1.0 - distance / 2.0).clamp(0.2, 0.9);

        AffectReading {
            label,
            valence,
            arousal,
            dominance,
            confidence: Confidence::new(confidence),
        }
    }
}

/// Lexical keywords that directly signal a specific affect label.
/// When present, they reduce the Euclidean distance to the matching
/// prototype by 40%, ensuring explicit emotion words are respected.
const LABEL_KEYWORDS: &[(AffectLabel, &[&str])] = &[
    (
        AffectLabel::Angry,
        &["angry", "furious", "pissed", "outraged"],
    ),
    (AffectLabel::Frustrated, &["frustrated", "frustrating"]),
    (
        AffectLabel::Confused,
        &[
            "confused",
            "confusing",
            "don't understand",
            "can't understand",
        ],
    ),
    (AffectLabel::Sad, &["sad", "disappointed", "hopeless"]),
    (
        AffectLabel::Anxious,
        &["anxious", "worried", "nervous", "afraid"],
    ),
    (
        AffectLabel::Excited,
        &["excited", "thrilled", "amazing", "love it", "awesome"],
    ),
    (
        AffectLabel::Grateful,
        &["grateful", "thankful", "appreciate"],
    ),
    (
        AffectLabel::Curious,
        &["curious", "wondering", "interested", "how does"],
    ),
    (
        AffectLabel::Overwhelmed,
        &["overwhelmed", "too much", "overloaded", "can't keep up"],
    ),
];

/// Find the nearest prototype in VAD space using Euclidean distance,
/// with lexical affinity discounting for explicit emotion keywords.
fn nearest_prototype(v: f64, a: f64, d: f64, lower: &str) -> (AffectLabel, f64) {
    let mut best_label = AffectLabel::Neutral;
    let mut best_dist = f64::MAX;

    for &(label, pv, pa, pd) in PROTOTYPES {
        let mut dist = ((v - pv).powi(2) + (a - pa).powi(2) + (d - pd).powi(2)).sqrt();

        // Apply lexical affinity discount when user explicitly uses emotion words.
        for &(kw_label, keywords) in LABEL_KEYWORDS {
            if label == kw_label && keywords.iter().any(|kw| lower.contains(kw)) {
                dist *= 0.6;
                break;
            }
        }

        if dist < best_dist {
            best_dist = dist;
            best_label = label;
        }
    }

    (best_label, best_dist)
}

fn count_hits(lower: &str, words: &[&str]) -> usize {
    words.iter().filter(|w| lower.contains(**w)).count()
}

fn is_imperative_start(first_word: &str) -> bool {
    const IMPERATIVES: &[&str] = &[
        "do",
        "make",
        "fix",
        "run",
        "add",
        "remove",
        "delete",
        "update",
        "change",
        "set",
        "create",
        "build",
        "stop",
        "start",
        "show",
        "tell",
        "give",
        "help",
        "explain",
        "implement",
        "deploy",
        "install",
        "configure",
        "enable",
        "disable",
    ];
    let lower = first_word.to_lowercase();
    IMPERATIVES.iter().any(|w| *w == lower)
}

// Cast safety: first-person hit and word counts are bounded by message length.
#[allow(clippy::cast_precision_loss)]
fn count_first_person(lower: &str, word_count: usize) -> f64 {
    const FIRST_PERSON: &[&str] = &["i ", "i'm", "i've", "my ", "mine", "me "];
    let hits = FIRST_PERSON.iter().filter(|w| lower.contains(**w)).count();
    (hits as f64 / (word_count as f64).max(1.0)).min(1.0)
}

const POSITIVE_WORDS: &[&str] = &[
    "amazing",
    "awesome",
    "great",
    "fantastic",
    "love",
    "perfect",
    "excellent",
    "incredible",
    "wonderful",
    "good",
    "nice",
    "thanks",
    "thank",
    "appreciate",
    "helpful",
    "cool",
];

const NEGATIVE_WORDS: &[&str] = &[
    "angry",
    "furious",
    "pissed",
    "hate",
    "stupid",
    "terrible",
    "worst",
    "ridiculous",
    "unacceptable",
    "frustrated",
    "annoying",
    "annoyed",
    "broken",
    "fails",
    "failing",
    "confused",
    "unclear",
    "worried",
    "anxious",
    "nervous",
    "afraid",
    "sad",
    "disappointed",
    "hopeless",
    "bad",
    "wrong",
    "error",
    "bug",
    "issue",
    "problem",
    "don't understand",
    "can't understand",
    "makes no sense",
    "no idea",
];

const URGENCY_WORDS: &[&str] = &[
    "urgent",
    "asap",
    "quickly",
    "hurry",
    "deadline",
    "immediately",
    "now",
    "critical",
    "emergency",
    "important",
    "must",
    "need",
];

const HIGH_AROUSAL_EMOTION_WORDS: &[&str] = &[
    "frustrated",
    "furious",
    "angry",
    "hate",
    "terrible",
    "ridiculous",
    "unacceptable",
    "pissed",
    "upset",
    "annoyed",
    "annoying",
    "outraged",
];

const HEDGING_WORDS: &[&str] = &[
    "maybe", "perhaps", "possibly", "might", "could", "i think", "not sure", "i guess", "sort of",
    "kind of",
];

#[cfg(test)]
mod tests {
    use super::RuleBasedDetector;
    use crate::core::affect::types::AffectLabel;

    #[test]
    fn detect_neutral_message() {
        let detector = RuleBasedDetector::new();
        let reading = detector.detect("Please help me with this task.");
        assert_eq!(reading.label, AffectLabel::Neutral);
    }

    #[test]
    fn detect_frustrated_message() {
        let detector = RuleBasedDetector::new();
        let reading = detector.detect("I'm so frustrated, nothing works!");
        assert_eq!(reading.label, AffectLabel::Frustrated);
    }

    #[test]
    fn detect_confused_message() {
        let detector = RuleBasedDetector::new();
        let reading = detector.detect("I don't understand, what do you mean?");
        assert_eq!(reading.label, AffectLabel::Confused);
    }

    #[test]
    fn detect_angry_shouting() {
        let detector = RuleBasedDetector::new();
        let reading = detector.detect("THIS IS RIDICULOUS!!");
        assert_eq!(reading.label, AffectLabel::Angry);
    }

    #[test]
    fn detect_excited_message() {
        let detector = RuleBasedDetector::new();
        let reading = detector.detect("This is amazing! I love it!");
        assert_eq!(reading.label, AffectLabel::Excited);
    }

    #[test]
    fn normal_question_is_neutral() {
        let detector = RuleBasedDetector::new();
        let reading = detector.detect("Can you help me with my project?");
        assert_eq!(reading.label, AffectLabel::Neutral);
    }

    #[test]
    fn vad_dominance_is_computed() {
        let detector = RuleBasedDetector::new();
        let reading = detector.detect("Fix this bug right now!");
        // Imperative start + urgency → higher dominance
        assert!(
            reading.dominance > 0.5,
            "expected dominance > 0.5, got {}",
            reading.dominance
        );
    }

    #[test]
    fn vad_hedging_lowers_dominance() {
        let detector = RuleBasedDetector::new();
        let reading = detector.detect("I think maybe perhaps we could sort of try this");
        // Heavy hedging → lower dominance
        assert!(
            reading.dominance < 0.5,
            "expected dominance < 0.5, got {}",
            reading.dominance
        );
    }

    #[test]
    fn vad_valence_positive_for_praise() {
        let detector = RuleBasedDetector::new();
        let reading = detector.detect("This is a great and wonderful solution, thanks!");
        assert!(
            reading.valence > 0.0,
            "expected positive valence, got {}",
            reading.valence
        );
    }

    #[test]
    fn vad_arousal_high_for_caps_and_excl() {
        let detector = RuleBasedDetector::new();
        let reading = detector.detect("THIS IS URGENT!! FIX IT NOW!!");
        assert!(
            reading.arousal > 0.4,
            "expected high arousal, got {}",
            reading.arousal
        );
    }

    #[test]
    fn empty_message_is_neutral() {
        let detector = RuleBasedDetector::new();
        let reading = detector.detect("");
        assert_eq!(reading.label, AffectLabel::Neutral);
    }
}
