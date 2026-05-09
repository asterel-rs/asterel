//! Data types for memory grounding: tiers, items, and the composite
//! `ContextBundle` that groups recalled facts, hints, and noise.

use serde::{Deserialize, Serialize};

use crate::contracts::ids::SlotKey;
use crate::core::memory::MemorySource;

/// Confidence tier used to classify recalled memory items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroundingTier {
    /// High-confidence item (>= 0.8).
    Fact,
    /// Medium-confidence item (>= 0.4, < 0.8).
    Hint,
    /// Low-confidence item (< 0.4).
    Noise,
}

impl GroundingTier {
    /// Classify a confidence score into a grounding tier.
    #[must_use]
    pub fn from_confidence(confidence: f64) -> Self {
        if confidence >= 0.8 {
            Self::Fact
        } else if confidence >= 0.4 {
            Self::Hint
        } else {
            Self::Noise
        }
    }
}

/// A single recalled memory item with grounding metadata.
#[derive(Debug, Clone)]
pub struct GroundingEntry {
    /// The slot key this item belongs to.
    pub slot_key: SlotKey,
    /// The stored value.
    pub value: String,
    /// Confidence tier classification.
    pub tier: GroundingTier,
    /// Raw confidence score (0.0-1.0).
    pub confidence: f64,
    /// How this fact was originally sourced.
    pub source: MemorySource,
    /// Whether this item has been marked as contradicted.
    pub is_contradicted: bool,
    /// Recall relevance score from the retrieval query (0.0-1.0).
    pub recall_score: f64,
}

/// Grouped set of recalled memory items split by grounding tier.
#[derive(Debug, Clone, Default)]
pub struct ContextBundle {
    /// High-confidence items used as grounded facts.
    pub facts: Vec<GroundingEntry>,
    /// Medium-confidence items used as hints.
    pub hints: Vec<GroundingEntry>,
    /// Low-confidence items (typically excluded from prompts).
    pub noise: Vec<GroundingEntry>,
}

impl ContextBundle {
    /// Returns `true` if all tiers are empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.facts.is_empty() && self.hints.is_empty() && self.noise.is_empty()
    }

    /// Number of items classified as facts.
    #[must_use]
    pub fn fact_count(&self) -> usize {
        self.facts.len()
    }

    /// Number of items classified as hints.
    #[must_use]
    pub fn hint_count(&self) -> usize {
        self.hints.len()
    }

    /// Returns `true` when all non-noise items have recall scores below the threshold,
    /// indicating weak retrieval quality for this turn.
    #[must_use]
    pub fn all_low_relevance(&self, threshold: f64) -> bool {
        let mut count = 0usize;
        let all_low = self
            .facts
            .iter()
            .chain(self.hints.iter())
            .inspect(|_| count += 1)
            .all(|item| item.recall_score < threshold);
        count > 0 && all_low
    }
}

#[cfg(test)]
mod tests {
    use super::{ContextBundle, GroundingTier};

    #[test]
    fn grounding_tier_from_high_confidence_is_fact() {
        assert_eq!(GroundingTier::from_confidence(0.9), GroundingTier::Fact);
    }

    #[test]
    fn grounding_tier_from_medium_confidence_is_hint() {
        assert_eq!(GroundingTier::from_confidence(0.6), GroundingTier::Hint);
    }

    #[test]
    fn grounding_tier_from_low_confidence_is_noise() {
        assert_eq!(GroundingTier::from_confidence(0.2), GroundingTier::Noise);
    }

    #[test]
    fn default_context_bundle_is_empty() {
        assert!(ContextBundle::default().is_empty());
    }

    #[test]
    fn all_low_relevance_true_when_all_below_threshold() {
        let mut bundle = ContextBundle::default();
        bundle.facts.push(super::GroundingEntry {
            slot_key: "k".into(),
            value: "v".to_string(),
            tier: GroundingTier::Fact,
            confidence: 0.9,
            source: crate::core::memory::MemorySource::ExplicitUser,
            is_contradicted: false,
            recall_score: 0.3,
        });
        assert!(bundle.all_low_relevance(0.5));
    }

    #[test]
    fn all_low_relevance_false_when_some_above_threshold() {
        let mut bundle = ContextBundle::default();
        bundle.facts.push(super::GroundingEntry {
            slot_key: "k".into(),
            value: "v".to_string(),
            tier: GroundingTier::Fact,
            confidence: 0.9,
            source: crate::core::memory::MemorySource::ExplicitUser,
            is_contradicted: false,
            recall_score: 0.7,
        });
        assert!(!bundle.all_low_relevance(0.5));
    }

    #[test]
    fn all_low_relevance_false_when_empty() {
        let bundle = ContextBundle::default();
        assert!(!bundle.all_low_relevance(0.5));
    }
}
