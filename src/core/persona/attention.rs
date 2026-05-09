//! Attention schema: computes ranked salience scores over topics
//! from memory, experience, principles, and affect signals.
#![allow(clippy::cast_precision_loss)]

use serde::{Deserialize, Serialize};

use crate::core::experience::distill_types::Principle;
use crate::core::memory::MemoryRecallEntry;

/// Top-N topics to surface in the attention focus block.
const ATTENTION_TOP_N: usize = 3;

/// An item with a computed salience score for attention ranking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SalienceEntry {
    /// The topic string this entry refers to.
    pub topic: String,
    /// Computed salience score (higher = more relevant).
    pub score: f64,
    /// Origin of this salience signal.
    pub source: SalienceSource,
}

/// Where the salience signal originates from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SalienceSource {
    /// Derived from a memory recall item.
    Memory,
    /// Derived from a past experience atom.
    Experience,
    /// Derived from a distilled principle.
    Principle,
    /// Derived from the current affect intensity.
    Affect,
}

impl SalienceSource {
    /// Return a zero-allocation `snake_case` label matching the `serde` output.
    /// Used by render helpers to avoid `serde_json::to_string` round-trips on
    /// the per-turn hotpath.
    #[must_use]
    pub(crate) const fn as_label(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Experience => "experience",
            Self::Principle => "principle",
            Self::Affect => "affect",
        }
    }
}

/// The attention schema: a ranked set of salient topics for the current turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct AttentionSchema {
    /// Ranked salience entries, highest score first.
    pub entries: Vec<SalienceEntry>,
}

impl AttentionSchema {
    /// Compute attention from grounding items, principles, and affect.
    pub(crate) fn compute(
        user_message: &str,
        grounding_items: &[MemoryRecallEntry],
        principles: &[Principle],
        affect_intensity: f64,
        rapport: f64,
    ) -> Self {
        let mut entries = Vec::with_capacity(ATTENTION_TOP_N + 4);
        // Pre-lowercase eligible (length > 3) words once per call to avoid
        // the O(principles × words) per-word allocation cascade that
        // `keyword_overlap_score` incurred when given raw words.
        let msg_words_lower = crate::utils::text::lowercase_words_over_len(user_message, 3);
        let msg_words_lower_refs: Vec<&str> = msg_words_lower.iter().map(String::as_str).collect();

        // Memory-grounded items contribute salience based on relevance score.
        for item in grounding_items.iter().take(10) {
            let keyword_boost = keyword_overlap_score(&msg_words_lower_refs, &item.value);
            let score = item.score * 0.4 + keyword_boost * 0.3 + item.confidence.get() * 0.3;
            if score > 0.2 {
                entries.push(SalienceEntry {
                    topic: truncate_topic(item.slot_key.as_str(), 60),
                    score: score.min(1.0),
                    source: SalienceSource::Memory,
                });
            }
        }

        // Principle-derived salience (high-confidence principles are salient).
        for principle in principles.iter().take(5) {
            let keyword_boost = keyword_overlap_score(&msg_words_lower_refs, &principle.statement);
            let score = principle.confidence.get() * 0.5 + keyword_boost * 0.5;
            if score > 0.3 {
                entries.push(SalienceEntry {
                    topic: truncate_topic(&principle.statement, 60),
                    score: score.min(1.0),
                    source: SalienceSource::Principle,
                });
            }
        }

        // Affect-based salience boost — high arousal pushes affect topics up.
        if affect_intensity > 0.5 {
            entries.push(SalienceEntry {
                topic: "current emotional state".to_string(),
                score: affect_intensity * 0.7 + rapport * 0.3,
                source: SalienceSource::Affect,
            });
        }

        // Sort by salience descending and keep top N.
        entries.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        entries.truncate(ATTENTION_TOP_N);

        Self { entries }
    }
}

use crate::utils::text::keyword_overlap_score;

fn truncate_topic(s: &str, max: usize) -> String {
    crate::utils::text::truncate_ellipsis(s, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_inputs_produce_empty_schema() {
        let schema = AttentionSchema::compute("hello", &[], &[], 0.2, 0.5);
        assert!(schema.entries.is_empty());
    }

    #[test]
    fn high_affect_adds_emotional_entry() {
        let schema = AttentionSchema::compute("test", &[], &[], 0.8, 0.6);
        assert!(!schema.entries.is_empty());
        assert!(schema.entries[0].topic.contains("emotional"));
    }

    #[test]
    fn entries_capped_at_top_n() {
        let items: Vec<MemoryRecallEntry> = (0..10)
            .map(|i| MemoryRecallEntry {
                entity_id: "e".into(),
                slot_key: format!("topic.{i}").into(),
                value: format!("relevant content about topic {i}"),
                source: crate::core::memory::MemorySource::ExplicitUser,
                confidence: crate::contracts::scores::Confidence::new(0.9),
                importance: crate::contracts::scores::Importance::new(0.7),
                privacy_level: crate::core::memory::PrivacyLevel::Private,
                score: 0.8,
                occurred_at: String::new(),
            })
            .collect();
        let schema = AttentionSchema::compute("relevant content topic", &items, &[], 0.2, 0.5);
        assert!(schema.entries.len() <= ATTENTION_TOP_N);
    }
}
