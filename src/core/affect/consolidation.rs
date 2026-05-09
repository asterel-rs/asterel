//! Affect memory consolidation: Bayesian cross-session learning from the
//! emotional history of interactions.
//!
//! # Purpose
//!
//! A companion that cannot remember *how past sessions felt* will repeat
//! the same emotional missteps. This module maintains a set of
//! [`EmotionalMemory`] records — each one a pattern (e.g., "discussing
//! deadlines") paired with a probability distribution over positive, negative,
//! and neutral affect outcomes.
//!
//! After each session, [`consolidate_session`] updates every memory record
//! with the session's aggregate sentiment via Bayesian update (weighted
//! average), then prunes records whose distribution is too uncertain
//! (high entropy) and identifies records stable enough to promote to
//! long-term memory.
//!
//! # Pipeline position
//!
//! ```text
//! session ends
//!     │
//!     ▼  compute_session_sentiment(valence_readings)
//! (positive_ratio, negative_ratio, neutral_ratio)
//!     │
//!     ▼  consolidate_session(existing_memories, ratios, config)
//! ConsolidationResult { updated, pruned_count, promotable }
//!     │
//!     ├── updated    → written back to persistent storage
//!     └── promotable → elevated to long-term character memory
//! ```
//!
//! # Design notes
//!
//! - The Bayesian update uses a **weighted average** rather than Bayes' rule
//!   directly, because we want the update magnitude bounded by
//!   `consolidation_max_single_session_shift`. This prevents a single dramatic
//!   session from swinging a stable memory record too far.
//!
//! - Entropy pruning removes records that have seen contradictory evidence
//!   (e.g., the same pattern triggered positive responses some days and
//!   negative others). Keeping such records would add noise, not signal.
//!
//! - Promotion requires `consolidation_min_sessions_for_promotion` sessions
//!   **and** a non-ambiguous distribution (dominant class ≥ 50%). This
//!   ensures long-term memory only receives reliable patterns.

use serde::{Deserialize, Serialize};

use crate::config::schema::AffectDecayConfig;

/// A single learned emotional association between a context pattern and its
/// affective outcome distribution.
///
/// The three `*_confidence` fields form a probability simplex that sums to 1.0.
/// They are updated by [`EmotionalMemory::bayesian_update`] as new session
/// evidence arrives.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EmotionalMemory {
    /// The context pattern this memory is associated with (e.g., a topic label
    /// or conversation type tag).
    pub pattern: String,
    /// Probability that interactions matching this pattern are positively valenced.
    pub positive_confidence: f64,
    /// Probability that interactions matching this pattern are negatively valenced.
    pub negative_confidence: f64,
    /// Probability that interactions matching this pattern are emotionally neutral.
    pub neutral_confidence: f64,
    /// Cumulative evidence weight (sum of all `evidence_strength` values seen).
    /// Used as the denominator in the weighted-average Bayesian update.
    pub weight: f64,
    /// Number of sessions this memory has been updated from.
    pub session_count: u32,
    /// Number of contradictory updates observed for this pattern.
    #[serde(default)]
    pub contradiction_count: u32,
    /// Recurrence strength for this pattern in `[0.0, 1.0+]`.
    #[serde(default = "default_recurrence_score")]
    pub recurrence_score: f64,
    /// Rolling salience estimate for the pattern in `[0.0, 1.0]`.
    #[serde(default)]
    pub salience_ema: f64,
}

impl EmotionalMemory {
    pub(crate) fn new(pattern: String, positive: f64, negative: f64, neutral: f64) -> Self {
        let total = positive + negative + neutral;
        let (positive_share, negative_share, neutral_share) = if total > 0.0 {
            (positive / total, negative / total, neutral / total)
        } else {
            (0.0, 0.0, 1.0)
        };
        Self {
            pattern,
            positive_confidence: positive_share,
            negative_confidence: negative_share,
            neutral_confidence: neutral_share,
            weight: 1.0,
            session_count: 1,
            contradiction_count: 0,
            recurrence_score: 1.0,
            salience_ema: dominant_salience(positive_share, negative_share, neutral_share),
        }
    }

    /// Update this memory record with new session evidence using a weighted
    /// Bayesian average.
    ///
    /// The update formula for each dimension `d` is:
    /// ```text
    /// updated_d = (old_d × weight + new_d × strength) / (weight + strength)
    /// ```
    /// After blending, each dimension is clamped to move at most `max_shift`
    /// from its current value (set via `consolidation_max_single_session_shift`
    /// in config). This prevents a single highly emotional session from
    /// completely overwriting a stable long-term record.
    ///
    /// The three dimensions are re-normalised to sum to 1.0 after clamping.
    pub(crate) fn bayesian_update(
        &mut self,
        new_positive: f64,
        new_negative: f64,
        new_neutral: f64,
        evidence_strength: f64,
        max_shift: f64,
        salience: f64,
    ) {
        let total = new_positive + new_negative + new_neutral;
        if total <= 0.0 {
            return;
        }

        let new_p = new_positive / total;
        let new_negative_confidence = new_negative / total;
        let new_neutral_confidence = new_neutral / total;

        let weight = self.weight;
        let strength = evidence_strength;

        let updated_p =
            (self.positive_confidence * weight + new_p * strength) / (weight + strength);
        let updated_negative_confidence = (self.negative_confidence * weight
            + new_negative_confidence * strength)
            / (weight + strength);
        let updated_neutral_confidence = (self.neutral_confidence * weight
            + new_neutral_confidence * strength)
            / (weight + strength);

        let previous_dominant = dominant_label(
            self.positive_confidence,
            self.negative_confidence,
            self.neutral_confidence,
        );
        self.positive_confidence = clamp_shift(self.positive_confidence, updated_p, max_shift);
        self.negative_confidence = clamp_shift(
            self.negative_confidence,
            updated_negative_confidence,
            max_shift,
        );
        self.neutral_confidence = clamp_shift(
            self.neutral_confidence,
            updated_neutral_confidence,
            max_shift,
        );

        let sum = self.positive_confidence + self.negative_confidence + self.neutral_confidence;
        if sum > 0.0 {
            self.positive_confidence /= sum;
            self.negative_confidence /= sum;
            self.neutral_confidence /= sum;
        }

        let next_dominant = dominant_label(
            self.positive_confidence,
            self.negative_confidence,
            self.neutral_confidence,
        );
        if previous_dominant != next_dominant {
            self.contradiction_count = self.contradiction_count.saturating_add(1);
        }
        self.recurrence_score += evidence_strength.clamp(0.0, 1.0);
        self.salience_ema = self.salience_ema * 0.75 + salience.clamp(0.0, 1.0) * 0.25;

        self.weight += strength;
        self.session_count += 1;
    }

    /// Shannon entropy of the affect outcome distribution (bits).
    ///
    /// Maximum entropy (≈1.585 bits for three classes) indicates maximum
    /// uncertainty — the pattern predicts nothing. Minimum entropy (0.0) means
    /// one class has probability 1.0. Used as the pruning criterion: records
    /// above the `consolidation_entropy_prune_threshold` are discarded.
    pub(crate) fn entropy(&self) -> f64 {
        [
            self.positive_confidence,
            self.negative_confidence,
            self.neutral_confidence,
        ]
        .into_iter()
        .filter(|probability| *probability > 0.0)
        .map(|probability| -probability * probability.log2())
        .sum()
    }

    /// Whether this memory should be pruned from the working set.
    ///
    /// Returns `true` when entropy exceeds `entropy_threshold`, meaning the
    /// record has seen too much contradictory evidence to be useful.
    pub(crate) fn should_prune(&self, entropy_threshold: f64) -> bool {
        self.entropy() > entropy_threshold
            || (self.contradiction_count >= 3 && self.salience_ema < 0.35)
    }

    /// Whether this memory is ready for promotion to long-term storage.
    ///
    /// Requires both sufficient session count *and* a non-ambiguous
    /// distribution (dominant class ≥ 50%). Ambiguous memories are never
    /// promoted even if they have been updated many times.
    pub(crate) fn is_promotable(&self, min_sessions: u32) -> bool {
        self.session_count >= min_sessions
            && !self.is_ambiguous()
            && self.salience_ema >= 0.35
            && self.contradiction_count < 3
    }

    /// Returns `true` when no single class holds ≥ 50% probability —
    /// i.e., the distribution does not clearly favour any outcome.
    fn is_ambiguous(&self) -> bool {
        self.positive_confidence
            .max(self.negative_confidence)
            .max(self.neutral_confidence)
            < 0.50
    }
}

/// Clamp the shift of a probability value to at most `max_delta` in either
/// direction, then clamp the result to \[0.0, 1.0\].
fn clamp_shift(old: f64, new: f64, max_delta: f64) -> f64 {
    let delta = (new - old).clamp(-max_delta, max_delta);
    (old + delta).clamp(0.0, 1.0)
}

const fn default_recurrence_score() -> f64 {
    1.0
}

fn dominant_label(positive: f64, negative: f64, neutral: f64) -> &'static str {
    if positive >= negative && positive >= neutral {
        "positive"
    } else if negative >= positive && negative >= neutral {
        "negative"
    } else {
        "neutral"
    }
}

fn dominant_salience(positive: f64, negative: f64, neutral: f64) -> f64 {
    positive.max(negative).max(neutral)
}

/// The outcome of a single consolidation pass over the memory set.
#[derive(Debug, Clone)]
pub(crate) struct ConsolidationResult {
    /// All memory records that survived pruning, with updated distributions.
    pub updated: Vec<EmotionalMemory>,
    /// Subset of `updated` records that are ready for long-term promotion.
    pub promotable: Vec<EmotionalMemory>,
}

/// Run a full consolidation pass: update all memories with the session's
/// aggregate sentiment, prune high-entropy records, and collect promotable ones.
///
/// Call this once per session boundary. The `existing_memories` vec is
/// updated in-place (records are removed when pruned).
///
/// `session_*_ratio` values should sum to 1.0 and are produced by
/// [`compute_session_sentiment`].
pub(crate) fn consolidate_session(
    existing_memories: &mut Vec<EmotionalMemory>,
    session_positive_ratio: f64,
    session_negative_ratio: f64,
    session_neutral_ratio: f64,
    config: &AffectDecayConfig,
) -> ConsolidationResult {
    let evidence_strength = 1.0;
    let salience = dominant_salience(
        session_positive_ratio,
        session_negative_ratio,
        session_neutral_ratio,
    );

    for memory in existing_memories.iter_mut() {
        memory.bayesian_update(
            session_positive_ratio,
            session_negative_ratio,
            session_neutral_ratio,
            evidence_strength,
            config.consolidation_max_single_session_shift,
            salience,
        );
    }

    existing_memories
        .retain(|memory| !memory.should_prune(config.consolidation_entropy_prune_threshold));

    let promotable = existing_memories
        .iter()
        .filter(|memory| memory.is_promotable(config.consolidation_min_sessions_for_promotion))
        .cloned()
        .collect();

    ConsolidationResult {
        updated: existing_memories.clone(),
        promotable,
    }
}

/// Aggregate a session's valence readings into positive/negative/neutral ratios.
///
/// Thresholds: valence > 0.1 → positive, valence < −0.1 → negative, else neutral.
///
/// Returns `(positive_ratio, negative_ratio, neutral_ratio)` that sum to 1.0.
/// Returns `(0.0, 0.0, 1.0)` for an empty slice (no evidence → treat as neutral).
pub(crate) fn compute_session_sentiment(valence_readings: &[f64]) -> (f64, f64, f64) {
    if valence_readings.is_empty() {
        return (0.0, 0.0, 1.0);
    }

    let mut positive = 0u32;
    let mut negative = 0u32;
    let mut neutral = 0u32;

    for &value in valence_readings {
        if value > 0.1 {
            positive += 1;
        } else if value < -0.1 {
            negative += 1;
        } else {
            neutral += 1;
        }
    }

    let total_count = u32::try_from(valence_readings.len()).unwrap_or(u32::MAX);
    let total = f64::from(total_count);
    (
        f64::from(positive) / total,
        f64::from(negative) / total,
        f64::from(neutral) / total,
    )
}

const _: EmotionalMemory = EmotionalMemory {
    pattern: String::new(),
    positive_confidence: 0.0,
    negative_confidence: 0.0,
    neutral_confidence: 1.0,
    weight: 0.0,
    session_count: 0,
    contradiction_count: 0,
    recurrence_score: 1.0,
    salience_ema: 0.0,
};

const _: ConsolidationResult = ConsolidationResult {
    updated: Vec::new(),
    promotable: Vec::new(),
};

const _: fn(String, f64, f64, f64) -> EmotionalMemory = EmotionalMemory::new;
const _: fn(&mut EmotionalMemory, f64, f64, f64, f64, f64, f64) = EmotionalMemory::bayesian_update;
const _: fn(&EmotionalMemory) -> f64 = EmotionalMemory::entropy;
const _: fn(&EmotionalMemory, f64) -> bool = EmotionalMemory::should_prune;
const _: fn(&EmotionalMemory, u32) -> bool = EmotionalMemory::is_promotable;
const _: fn(f64, f64, f64) -> f64 = clamp_shift;
const _: fn(&mut Vec<EmotionalMemory>, f64, f64, f64, &AffectDecayConfig) -> ConsolidationResult =
    consolidate_session;
const _: fn(&[f64]) -> (f64, f64, f64) = compute_session_sentiment;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emotional_memory_entropy_uniform_is_max() {
        let memory = EmotionalMemory::new("test".to_string(), 1.0, 1.0, 1.0);
        assert!((memory.entropy() - 3.0_f64.log2()).abs() < 0.01);
    }

    #[test]
    fn emotional_memory_entropy_certain_is_zero() {
        let memory = EmotionalMemory::new("test".to_string(), 1.0, 0.0, 0.0);
        assert!(memory.entropy().abs() < 0.01);
    }

    #[test]
    fn bayesian_update_shifts_toward_evidence() {
        let mut memory = EmotionalMemory::new("test".to_string(), 0.8, 0.1, 0.1);
        let initial_positive = memory.positive_confidence;

        memory.bayesian_update(0.1, 0.8, 0.1, 1.0, 0.15, 0.8);

        assert!(
            memory.positive_confidence < initial_positive,
            "positive should decrease after negative evidence"
        );
    }

    #[test]
    fn bayesian_update_capped_by_max_shift() {
        let mut memory = EmotionalMemory::new("test".to_string(), 0.9, 0.05, 0.05);
        let initial_positive = memory.positive_confidence;

        memory.bayesian_update(0.0, 1.0, 0.0, 10.0, 0.15, 1.0);

        let shift = (initial_positive - memory.positive_confidence).abs();
        assert!(shift <= 0.16, "shift should be capped: got {shift}");
    }

    #[test]
    fn high_entropy_memory_is_pruned() {
        let memory = EmotionalMemory::new("test".to_string(), 1.0, 1.0, 1.0);
        assert!(memory.should_prune(1.4));
    }

    #[test]
    fn low_entropy_memory_is_not_pruned() {
        let memory = EmotionalMemory::new("test".to_string(), 0.9, 0.05, 0.05);
        assert!(!memory.should_prune(1.4));
    }

    #[test]
    fn promotable_requires_min_sessions() {
        let mut memory = EmotionalMemory::new("test".to_string(), 0.8, 0.1, 0.1);
        assert!(
            !memory.is_promotable(3),
            "1 session should not be promotable"
        );
        memory.session_count = 3;
        assert!(memory.is_promotable(3), "3 sessions should be promotable");
    }

    #[test]
    fn consolidate_session_prunes_and_promotes() {
        let config = AffectDecayConfig::default();
        let mut memories = vec![
            EmotionalMemory {
                pattern: "generally positive".to_string(),
                positive_confidence: 0.8,
                negative_confidence: 0.1,
                neutral_confidence: 0.1,
                weight: 3.0,
                session_count: 3,
                contradiction_count: 0,
                recurrence_score: 3.0,
                salience_ema: 0.8,
            },
            EmotionalMemory::new("contradictory".to_string(), 1.0, 1.0, 1.0),
        ];

        let result = consolidate_session(&mut memories, 0.7, 0.2, 0.1, &config);

        assert_eq!(memories.len(), 1, "contradictory should be pruned");
        assert_eq!(
            result.updated.len(),
            1,
            "one memory should remain after pruning"
        );
        assert!(
            !result.promotable.is_empty(),
            "positive pattern should be promotable"
        );
    }

    #[test]
    fn compute_session_sentiment_ratios() {
        let readings = vec![0.5, 0.3, -0.5, 0.0, 0.2];
        let (positive, negative, neutral) = compute_session_sentiment(&readings);
        assert!((positive - 0.6).abs() < 0.01);
        assert!((negative - 0.2).abs() < 0.01);
        assert!((neutral - 0.2).abs() < 0.01);
    }

    #[test]
    fn compute_session_sentiment_empty_is_neutral() {
        let (positive, negative, neutral) = compute_session_sentiment(&[]);
        assert!(positive.abs() < f64::EPSILON);
        assert!(negative.abs() < f64::EPSILON);
        assert!((neutral - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn contradictory_low_salience_memory_is_prunable() {
        let memory = EmotionalMemory {
            pattern: "mixed".to_string(),
            positive_confidence: 0.34,
            negative_confidence: 0.33,
            neutral_confidence: 0.33,
            weight: 4.0,
            session_count: 4,
            contradiction_count: 3,
            recurrence_score: 4.0,
            salience_ema: 0.2,
        };
        assert!(memory.should_prune(1.6));
    }
}
