//! Session-scoped working memory buffer.
//!
//! [`WorkingMemoryView`] holds two lists of [`WorkingMemoryItem`] entries:
//!
//! - **carried** — items hydrated from persistent memory at session start
//!   via [`WorkingMemoryView::materialize_from_recall`].
//! - **accumulated** — items collected during the current session via
//!   [`WorkingMemoryView::add_item`] / [`WorkingMemoryView::add_pinned_item`].
//!
//! ## Eviction policy
//!
//! When the view exceeds its configured capacity, the lowest-importance
//! non-pinned item is evicted first. When all items are pinned and the
//! view exceeds a hard cap (`capacity × 2`), the lowest-importance pinned
//! item is force-evicted to prevent unbounded growth.
//!
//! At the end of a session, [`WorkingMemoryView::drain_accumulated`] drains
//! the accumulated buffer for persistence back into the memory backend.

use std::mem;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::contracts::ids::{EntityId, SessionId};
use crate::contracts::memory_domain::MemoryRecallEntry;

const DEFAULT_WORKING_MEMORY_CAPACITY: usize = 50;

/// How an item entered the working memory view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkingMemorySource {
    /// Recalled from persistent memory at session start.
    Recalled,
    /// Extracted from the current conversation.
    Conversation,
    /// Explicitly noted by the agent via memory tool.
    AgentNoted,
    /// Injected by the system (for example, a tool result).
    System,
}

/// A single item stored in the session working memory buffer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingMemoryItem {
    /// Unique item identifier within this view.
    pub item_id: String,
    /// Lookup key (for example, a slot key or topic).
    pub key: String,
    /// Textual content of the item.
    pub value: String,
    /// How this item entered working memory.
    pub source: WorkingMemorySource,
    /// Importance score in `[0.0, 1.0]`.
    pub importance: f64,
    /// Whether this item is pinned and exempt from eviction.
    pub pinned: bool,
    /// RFC 3339 timestamp when this item was added to the view.
    pub added_at: String,
}

/// A session-scoped working memory view.
#[derive(Debug)]
pub struct WorkingMemoryView {
    /// Session this view belongs to.
    session_id: SessionId,
    /// Entity this view is scoped to.
    entity_id: EntityId,
    /// Items carried forward from persistent memory.
    carried: Vec<WorkingMemoryItem>,
    /// Items accumulated during this session and not yet persisted.
    accumulated: Vec<WorkingMemoryItem>,
    /// Maximum capacity before eviction is attempted.
    capacity: usize,
    /// RFC 3339 timestamp when this view was materialized.
    materialized_at: String,
}

impl WorkingMemoryView {
    /// Create an empty working memory view.
    #[must_use]
    pub fn new(
        session_id: impl Into<SessionId>,
        entity_id: impl Into<EntityId>,
        capacity: usize,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            entity_id: entity_id.into(),
            carried: Vec::new(),
            accumulated: Vec::new(),
            capacity: normalize_capacity(capacity),
            materialized_at: Utc::now().to_rfc3339(),
        }
    }

    /// Materialize a working memory view from recalled persistent memory items.
    #[must_use]
    pub fn materialize_from_recall(
        session_id: impl Into<SessionId>,
        entity_id: impl Into<EntityId>,
        recalled_items: Vec<MemoryRecallEntry>,
        capacity: usize,
    ) -> Self {
        let materialized_at = Utc::now().to_rfc3339();
        let carried = recalled_items
            .into_iter()
            .map(|item| WorkingMemoryItem {
                item_id: Uuid::new_v4().to_string(),
                key: item.slot_key.to_string(),
                value: item.value,
                source: WorkingMemorySource::Recalled,
                importance: item.importance.get(),
                pinned: false,
                added_at: materialized_at.clone(),
            })
            .collect();

        let mut view = Self {
            session_id: session_id.into(),
            entity_id: entity_id.into(),
            carried,
            accumulated: Vec::new(),
            capacity: normalize_capacity(capacity),
            materialized_at,
        };

        view.evict_if_over_capacity();
        view
    }

    /// Add a non-pinned item to the accumulated session buffer.
    pub fn add_item(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        source: WorkingMemorySource,
        importance: f64,
    ) -> &WorkingMemoryItem {
        self.add_item_with_pin(key, value, source, importance, false)
    }

    /// Add a pinned item to the accumulated session buffer.
    pub fn add_pinned_item(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        source: WorkingMemorySource,
        importance: f64,
    ) -> &WorkingMemoryItem {
        self.add_item_with_pin(key, value, source, importance, true)
    }

    /// Evict the lowest-importance non-pinned item when the view exceeds capacity.
    fn evict_if_over_capacity(&mut self) {
        while self.len() > self.capacity {
            if !self.evict_one_lowest_non_pinned() {
                break;
            }
        }
    }

    /// Iterate over all carried and accumulated items.
    pub fn items(&self) -> impl Iterator<Item = &WorkingMemoryItem> {
        self.carried.iter().chain(self.accumulated.iter())
    }

    /// Find the first item with the provided key.
    #[must_use]
    pub fn find_by_key(&self, key: &str) -> Option<&WorkingMemoryItem> {
        self.items().find(|item| item.key == key)
    }

    /// Return the number of items currently held by the view.
    #[must_use]
    pub fn len(&self) -> usize {
        self.carried.len() + self.accumulated.len()
    }

    /// Return whether the view currently holds no items.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Drain all accumulated items for end-of-session persistence.
    pub fn drain_accumulated(&mut self) -> Vec<WorkingMemoryItem> {
        mem::take(&mut self.accumulated)
    }

    /// Return the owning session identifier.
    #[must_use]
    pub fn session_id(&self) -> &str {
        self.session_id.as_str()
    }

    /// Return the owning entity identifier.
    #[must_use]
    pub fn entity_id(&self) -> &str {
        self.entity_id.as_str()
    }

    /// Return when the view was materialized.
    #[must_use]
    pub fn materialized_at(&self) -> &str {
        &self.materialized_at
    }

    /// Internal implementation shared by [`add_item`] and [`add_pinned_item`].
    /// Evicts non-pinned items first; force-evicts pinned items only when
    /// the hard cap (`capacity × 2`) is reached.
    fn add_item_with_pin(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        source: WorkingMemorySource,
        importance: f64,
        pinned: bool,
    ) -> &WorkingMemoryItem {
        while self.len() >= self.capacity {
            if !self.evict_one_lowest_non_pinned() {
                if self.len() >= self.hard_cap() {
                    self.force_evict_lowest_pinned();
                } else {
                    break;
                }
            }
        }

        let index = self.accumulated.len();

        self.accumulated.push(WorkingMemoryItem {
            item_id: Uuid::new_v4().to_string(),
            key: key.into(),
            value: value.into(),
            source,
            importance: importance.clamp(0.0, 1.0),
            pinned,
            added_at: Utc::now().to_rfc3339(),
        });

        &self.accumulated[index]
    }

    /// Upper bound before pinned items may be force-evicted.
    const fn hard_cap(&self) -> usize {
        self.capacity.saturating_mul(2)
    }

    /// Remove the globally lowest-importance item regardless of pinned status.
    /// Called only when the hard cap is reached and no non-pinned item remains.
    fn force_evict_lowest_pinned(&mut self) {
        let candidate = self
            .carried
            .iter()
            .enumerate()
            .map(|(i, item)| (ItemList::Carried, i, item.importance))
            .chain(
                self.accumulated
                    .iter()
                    .enumerate()
                    .map(|(i, item)| (ItemList::Accumulated, i, item.importance)),
            )
            .min_by(|a, b| a.2.total_cmp(&b.2));

        if let Some((list, index, _)) = candidate {
            match list {
                ItemList::Carried => {
                    self.carried.remove(index);
                }
                ItemList::Accumulated => {
                    self.accumulated.remove(index);
                }
            }
        }
    }

    fn evict_one_lowest_non_pinned(&mut self) -> bool {
        let candidate = self
            .carried
            .iter()
            .enumerate()
            .filter(|(_, item)| !item.pinned)
            .map(|(index, item)| (ItemList::Carried, index, item.importance))
            .chain(
                self.accumulated
                    .iter()
                    .enumerate()
                    .filter(|(_, item)| !item.pinned)
                    .map(|(index, item)| (ItemList::Accumulated, index, item.importance)),
            )
            .min_by(|left, right| left.2.total_cmp(&right.2));

        let Some((list, index, _)) = candidate else {
            return false;
        };

        match list {
            ItemList::Carried => {
                self.carried.remove(index);
            }
            ItemList::Accumulated => {
                self.accumulated.remove(index);
            }
        }

        true
    }
}

#[derive(Debug, Clone, Copy)]
enum ItemList {
    Carried,
    Accumulated,
}

const fn normalize_capacity(capacity: usize) -> usize {
    if capacity == 0 {
        DEFAULT_WORKING_MEMORY_CAPACITY
    } else {
        capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::contracts::ids::{EntityId, SlotKey};
    use crate::contracts::memory::{MemorySource, PrivacyLevel};
    use crate::contracts::scores::{Confidence, Importance};

    fn recall_entry(slot_key: &str, value: &str, importance: f64) -> MemoryRecallEntry {
        MemoryRecallEntry {
            entity_id: EntityId::new("entity-1"),
            slot_key: SlotKey::new(slot_key),
            value: value.to_owned(),
            source: MemorySource::ExplicitUser,
            confidence: Confidence::new(0.9),
            importance: Importance::new(importance),
            privacy_level: PrivacyLevel::Private,
            score: 0.95,
            occurred_at: "2026-03-17T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn new_view_is_empty() {
        let view = WorkingMemoryView::new("session-1", "entity-1", 0);

        assert_eq!(view.session_id(), "session-1");
        assert_eq!(view.entity_id(), "entity-1");
        assert!(view.is_empty());
        assert_eq!(view.len(), 0);
        assert_eq!(view.capacity, DEFAULT_WORKING_MEMORY_CAPACITY);
        assert!(view.carried.is_empty());
        assert!(view.accumulated.is_empty());
        assert!(!view.materialized_at().is_empty());
    }

    #[test]
    fn materialize_from_recall_populates_carried() {
        let recalled = vec![
            recall_entry("profile.name", "Aster", 0.8),
            recall_entry("profile.role", "Operator", 0.6),
        ];
        let view =
            WorkingMemoryView::materialize_from_recall("session-1", "entity-1", recalled, 10);

        assert_eq!(view.carried.len(), 2);
        assert!(view.accumulated.is_empty());
        assert_eq!(view.carried[0].key, "profile.name");
        assert_eq!(view.carried[0].value, "Aster");
        assert_eq!(view.carried[0].source, WorkingMemorySource::Recalled);
        assert!(!view.carried[0].item_id.is_empty());
        assert_eq!(view.carried[0].added_at, view.materialized_at());
    }

    #[test]
    fn add_item_goes_to_accumulated() {
        let mut view = WorkingMemoryView::new("session-1", "entity-1", 5);

        let item = view.add_item(
            "conversation.topic",
            "memory redesign",
            WorkingMemorySource::Conversation,
            0.7,
        );

        assert_eq!(item.key, "conversation.topic");
        assert_eq!(item.value, "memory redesign");
        assert_eq!(item.source, WorkingMemorySource::Conversation);
        assert!(!item.pinned);
        assert!(view.carried.is_empty());
        assert_eq!(view.accumulated.len(), 1);
        assert_eq!(view.len(), 1);
    }

    #[test]
    fn eviction_removes_lowest_importance_non_pinned() {
        let recalled = vec![
            recall_entry("low", "drop me", 0.1),
            recall_entry("high", "keep me", 0.9),
        ];
        let mut view =
            WorkingMemoryView::materialize_from_recall("session-1", "entity-1", recalled, 2);

        view.add_item("new", "fresh", WorkingMemorySource::Conversation, 0.6);

        assert_eq!(view.len(), 2);
        assert!(view.find_by_key("low").is_none());
        assert!(view.find_by_key("high").is_some());
        assert!(view.find_by_key("new").is_some());
    }

    #[test]
    fn pinned_items_survive_eviction() {
        let mut view = WorkingMemoryView::new("session-1", "entity-1", 2);
        view.add_pinned_item("pinned", "keep", WorkingMemorySource::System, 0.1);
        view.add_item("other", "existing", WorkingMemorySource::Conversation, 0.3);
        view.add_item("fresh", "new", WorkingMemorySource::Conversation, 0.2);

        assert!(view.find_by_key("pinned").is_some());
        assert!(view.find_by_key("fresh").is_some());
        assert!(view.find_by_key("other").is_none());
    }

    #[test]
    fn drain_accumulated_returns_and_clears() {
        let mut view = WorkingMemoryView::new("session-1", "entity-1", 4);
        view.add_item("one", "1", WorkingMemorySource::Conversation, 0.4);
        view.add_item("two", "2", WorkingMemorySource::AgentNoted, 0.7);

        let drained = view.drain_accumulated();

        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].key, "one");
        assert_eq!(drained[1].key, "two");
        assert!(view.accumulated.is_empty());
        assert!(view.is_empty());
    }

    #[test]
    fn find_by_key_searches_both_lists() {
        let recalled = vec![recall_entry("carried", "from recall", 0.8)];
        let mut view =
            WorkingMemoryView::materialize_from_recall("session-1", "entity-1", recalled, 5);
        view.add_item(
            "accumulated",
            "from session",
            WorkingMemorySource::Conversation,
            0.5,
        );

        assert_eq!(
            view.find_by_key("carried").map(|item| item.value.as_str()),
            Some("from recall")
        );
        assert_eq!(
            view.find_by_key("accumulated")
                .map(|item| item.value.as_str()),
            Some("from session")
        );
        assert!(view.find_by_key("missing").is_none());
    }

    #[test]
    fn items_iterator_covers_both_lists() {
        let recalled = vec![recall_entry("carried", "from recall", 0.8)];
        let mut view =
            WorkingMemoryView::materialize_from_recall("session-1", "entity-1", recalled, 5);
        view.add_item(
            "accumulated",
            "from session",
            WorkingMemorySource::Conversation,
            0.5,
        );

        let keys = view
            .items()
            .map(|item| item.key.as_str())
            .collect::<Vec<_>>();

        assert_eq!(keys, vec!["carried", "accumulated"]);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn serde_roundtrip_for_item_and_source() {
        let source = WorkingMemorySource::AgentNoted;
        let source_json = serde_json::to_string(&source).unwrap();
        assert_eq!(source_json, "\"agent_noted\"");
        let parsed_source: WorkingMemorySource = serde_json::from_str(&source_json).unwrap();
        assert_eq!(parsed_source, source);

        let item = WorkingMemoryItem {
            item_id: "item-1".to_owned(),
            key: "topic".to_owned(),
            value: "working memory".to_owned(),
            source: WorkingMemorySource::System,
            importance: 0.75,
            pinned: true,
            added_at: "2026-03-17T00:00:00Z".to_owned(),
        };
        let json = serde_json::to_string(&item).unwrap();
        let parsed_item: WorkingMemoryItem = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed_item.item_id, item.item_id);
        assert_eq!(parsed_item.key, item.key);
        assert_eq!(parsed_item.value, item.value);
        assert_eq!(parsed_item.source, item.source);
        assert_eq!(parsed_item.importance, item.importance);
        assert_eq!(parsed_item.pinned, item.pinned);
        assert_eq!(parsed_item.added_at, item.added_at);
    }

    #[test]
    fn hard_cap_prevents_unbounded_growth_when_all_pinned() {
        let mut view = WorkingMemoryView::new("s1", "e1", 3);
        for i in 0..10 {
            view.add_pinned_item(
                format!("pin-{i}"),
                format!("val-{i}"),
                WorkingMemorySource::System,
                0.9,
            );
        }
        assert!(
            view.len() <= 6,
            "hard cap (capacity * 2 = 6) should prevent unbounded growth, got {}",
            view.len()
        );
    }
}
