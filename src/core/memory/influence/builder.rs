//! Context bundle builder for memory grounding.
//!
//! Classifies recalled memory items into fact/hint/noise tiers and
//! assembles a `ContextBundle` for prompt augmentation.

use std::collections::HashSet;

use super::types::{ContextBundle, GroundingEntry, GroundingTier};
use crate::contracts::ids::SlotKey;
use crate::core::memory::MemoryRecallEntry;

/// Classify recall items by confidence tier into a `ContextBundle`.
#[must_use]
pub fn build_context_bundle<S: std::hash::BuildHasher>(
    items: &[MemoryRecallEntry],
    contradicted_slots: &HashSet<SlotKey, S>,
) -> ContextBundle {
    let mut bundle = ContextBundle::default();
    for item in items {
        let tier = GroundingTier::from_confidence(item.confidence.get());
        let grounding_item = GroundingEntry {
            slot_key: item.slot_key.clone(),
            value: item.value.clone(),
            tier,
            confidence: item.confidence.get(),
            source: item.source,
            is_contradicted: contradicted_slots.contains(&item.slot_key),
            recall_score: item.score,
        };

        match tier {
            GroundingTier::Fact => bundle.facts.push(grounding_item),
            GroundingTier::Hint => bundle.hints.push(grounding_item),
            GroundingTier::Noise => bundle.noise.push(grounding_item),
        }
    }
    bundle
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::build_context_bundle;
    use crate::contracts::ids::{EntityId, SlotKey};
    use crate::core::memory::{MemoryRecallEntry, MemorySource, PrivacyLevel};

    fn recall_item(slot_key: &str, confidence: f64) -> MemoryRecallEntry {
        MemoryRecallEntry {
            entity_id: EntityId::new("default"),
            slot_key: SlotKey::new(slot_key),
            value: "value".to_string(),
            source: MemorySource::ExplicitUser,
            confidence: confidence.into(),
            importance: 0.6.into(),
            privacy_level: PrivacyLevel::Private,
            score: 0.8,
            occurred_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn build_context_bundle_distributes_items_by_confidence_tier() {
        let items = vec![
            recall_item("profile.name", 0.95),
            recall_item("preference.locale", 0.6),
            recall_item("misc.note", 0.2),
        ];

        let bundle = build_context_bundle(&items, &HashSet::new());

        assert_eq!(bundle.fact_count(), 1);
        assert_eq!(bundle.hint_count(), 1);
        assert_eq!(bundle.noise.len(), 1);
    }
}
