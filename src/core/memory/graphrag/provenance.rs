//! Provenance types: evidence snippets and snippet sets derived from recall.
//!
//! An [`EvidenceSnippet`] is a compact reference to a single recall entry
//! (entity, slot, summary, and quality scores). An [`EvidenceSnippetSet`]
//! groups multiple snippets and provides deduplication and ranking helpers.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::contracts::ids::{EntityId, EvidenceId, SlotKey};
use crate::core::memory::MemoryRecallEntry;

/// Compact evidence reference derived from existing recall items.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceSnippet {
    pub evidence_id: EvidenceId,
    pub entity_id: EntityId,
    pub slot_key: SlotKey,
    pub summary: String,
    pub score: f64,
    pub confidence: f64,
    pub importance: f64,
    pub occurred_at: String,
}

impl EvidenceSnippet {
    #[must_use]
    pub fn short_label(&self) -> String {
        format!("{} ({})", self.slot_key, self.evidence_id)
    }
}

impl From<&MemoryRecallEntry> for EvidenceSnippet {
    fn from(item: &MemoryRecallEntry) -> Self {
        let entity_id = item.entity_id.clone();
        let slot_key = item.slot_key.clone();
        Self {
            evidence_id: format!("{entity_id}::{slot_key}::{}", item.occurred_at).into(),
            entity_id,
            slot_key,
            summary: item.value.clone(),
            score: item.score,
            confidence: item.confidence.get(),
            importance: item.importance.get(),
            occurred_at: item.occurred_at.clone(),
        }
    }
}

/// Evidence helper that deduplicates recall hits while keeping the
/// highest-scoring snippets first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EvidenceSnippetSet {
    pub items: Vec<EvidenceSnippet>,
}

impl EvidenceSnippetSet {
    #[must_use]
    pub fn from_recall_items(items: &[MemoryRecallEntry], limit: usize) -> Self {
        let mut snippets: Vec<EvidenceSnippet> = items.iter().map(EvidenceSnippet::from).collect();
        snippets.sort_by(|lhs, rhs| rhs.score.total_cmp(&lhs.score));

        let mut seen = HashSet::new();
        snippets.retain(|snippet| seen.insert(snippet.evidence_id.clone()));
        snippets.truncate(limit);

        Self { items: snippets }
    }
}

#[cfg(test)]
mod tests {
    use crate::contracts::ids::{EntityId, SlotKey};
    use crate::contracts::scores::{Confidence, Importance};
    use crate::core::memory::{MemoryRecallEntry, MemorySource, PrivacyLevel};

    use super::*;

    fn recall_item(slot_key: &str, score: f64, occurred_at: &str) -> MemoryRecallEntry {
        MemoryRecallEntry {
            entity_id: EntityId::new("entity-1"),
            slot_key: SlotKey::new(slot_key),
            value: format!("value for {slot_key}"),
            source: MemorySource::ExplicitUser,
            confidence: Confidence::new(0.9),
            importance: Importance::new(0.7),
            privacy_level: PrivacyLevel::Private,
            score,
            occurred_at: occurred_at.to_string(),
        }
    }

    #[test]
    fn evidence_set_deduplicates_and_sorts() {
        let items = vec![
            recall_item("support_notes", 0.4, "2026-01-01T00:00:00Z"),
            recall_item("support_notes", 0.9, "2026-01-01T00:00:00Z"),
            recall_item("launch_faq", 0.7, "2026-01-02T00:00:00Z"),
        ];

        let snippets = EvidenceSnippetSet::from_recall_items(&items, 5);

        assert_eq!(snippets.items.len(), 2);
        assert_eq!(snippets.items[0].slot_key, SlotKey::new("support_notes"));
        assert!(snippets.items[0].score >= snippets.items[1].score);
    }
}
