//! Pending follow-up queue: persists deferred tasks and reminders across
//! sessions via a single memory slot (`persona.writeback.follow_up_queue.v1`).
//!
//! Items are enqueued by appending to the current slot value; `clear_follow_ups`
//! replaces the slot with an empty `FollowUpQueue`.  Both operations silently
//! swallow serialisation errors (a warning is logged) to avoid crashing the
//! pipeline on non-critical state.  `load_pending_follow_ups` returns an empty
//! vec on missing or unparseable slots rather than propagating an error.

use serde::{Deserialize, Serialize};

use crate::core::memory::{Memory, MemoryEventType};

pub const FOLLOW_UP_QUEUE_SLOT_KEY: &str = "persona.writeback.follow_up_queue.v1";
const MAX_PENDING_FOLLOW_UPS: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingFollowUp {
    pub task_title: String,
    pub summary: String,
    pub created_at: String,
    pub task_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowUpQueue {
    pub items: Vec<PendingFollowUp>,
}

pub async fn load_pending_follow_ups(mem: &dyn Memory, entity_id: &str) -> Vec<PendingFollowUp> {
    let Ok(Some(slot)) = mem.resolve_slot(entity_id, FOLLOW_UP_QUEUE_SLOT_KEY).await else {
        return Vec::new();
    };

    match serde_json::from_str::<FollowUpQueue>(&slot.value) {
        Ok(queue) => queue.items,
        Err(_) => Vec::new(),
    }
}

pub async fn enqueue_follow_up(
    mem: &dyn Memory,
    entity_id: &str,
    person_id: &str,
    follow_up: PendingFollowUp,
) {
    let mut items = load_pending_follow_ups(mem, entity_id).await;
    let source_ref = format!("persona.follow_up_queue.enqueue:{}", follow_up.task_id);
    let occurred_at = Some(follow_up.created_at.clone());
    items.push(follow_up);
    if items.len() > MAX_PENDING_FOLLOW_UPS {
        let remove_count = items.len() - MAX_PENDING_FOLLOW_UPS;
        items.drain(..remove_count);
    }

    let payload = match serde_json::to_string(&FollowUpQueue { items }) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(%error, "failed to serialize pending follow-up queue");
            return;
        }
    };

    if let Err(error) = super::persist_helper::persist_persona_slot(
        mem,
        entity_id,
        FOLLOW_UP_QUEUE_SLOT_KEY,
        MemoryEventType::SummaryCompacted,
        payload,
        0.85,
        0.55,
        source_ref,
        "persona.follow_up_queue.writeback",
        occurred_at,
        person_id,
    )
    .await
    {
        tracing::warn!(%error, "failed to persist pending follow-up queue");
    }
}

pub async fn clear_follow_ups(mem: &dyn Memory, entity_id: &str, person_id: &str) {
    let payload = match serde_json::to_string(&FollowUpQueue::default()) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(%error, "failed to serialize empty follow-up queue");
            return;
        }
    };

    if let Err(error) = super::persist_helper::persist_persona_slot(
        mem,
        entity_id,
        FOLLOW_UP_QUEUE_SLOT_KEY,
        MemoryEventType::SummaryCompacted,
        payload,
        0.9,
        0.4,
        "persona.follow_up_queue.clear",
        "persona.follow_up_queue.writeback",
        None,
        person_id,
    )
    .await
    {
        tracing::warn!(%error, "failed to clear pending follow-up queue");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::core::memory::{
        MarkdownMemory, Memory, MemoryEventInput, MemorySource, PrivacyLevel,
    };

    fn sample_follow_up() -> PendingFollowUp {
        PendingFollowUp {
            task_title: "Draft release note".to_string(),
            summary: "Prepared highlights and risk notes for review".to_string(),
            created_at: "2026-03-08T10:00:00Z".to_string(),
            task_id: "task-123".to_string(),
        }
    }

    #[tokio::test]
    async fn load_returns_empty_when_slot_missing() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        let loaded = load_pending_follow_ups(mem.as_ref(), "person:person-test").await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn enqueue_then_load_round_trips_items() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        enqueue_follow_up(
            mem.as_ref(),
            "person:person-test",
            "person-test",
            sample_follow_up(),
        )
        .await;

        let loaded = load_pending_follow_ups(mem.as_ref(), "person:person-test").await;
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].task_id, "task-123");
    }

    #[tokio::test]
    async fn enqueue_caps_queue_to_recent_items() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        for index in 0..25 {
            let mut follow_up = sample_follow_up();
            follow_up.task_id = format!("task-{index:02}");
            enqueue_follow_up(mem.as_ref(), "person:person-test", "person-test", follow_up).await;
        }

        let loaded = load_pending_follow_ups(mem.as_ref(), "person:person-test").await;
        assert_eq!(loaded.len(), MAX_PENDING_FOLLOW_UPS);
        assert_eq!(loaded[0].task_id, "task-05");
        assert_eq!(loaded[MAX_PENDING_FOLLOW_UPS - 1].task_id, "task-24");
    }

    #[tokio::test]
    async fn clear_removes_pending_items() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        enqueue_follow_up(
            mem.as_ref(),
            "person:person-test",
            "person-test",
            sample_follow_up(),
        )
        .await;
        clear_follow_ups(mem.as_ref(), "person:person-test", "person-test").await;

        let loaded = load_pending_follow_ups(mem.as_ref(), "person:person-test").await;
        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn load_returns_empty_for_invalid_json() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));

        let input = MemoryEventInput::new(
            "person:person-test",
            FOLLOW_UP_QUEUE_SLOT_KEY,
            MemoryEventType::SummaryCompacted,
            "{invalid-json",
            MemorySource::System,
            PrivacyLevel::Private,
        );
        mem.append_event(input)
            .await
            .expect("invalid json slot seed should persist");

        let loaded = load_pending_follow_ups(mem.as_ref(), "person:person-test").await;
        assert!(loaded.is_empty());
    }

    #[test]
    fn render_formats_pending_follow_up_list() {
        let block = crate::core::persona::presenter::render_follow_up_block(&[sample_follow_up()]);
        assert!(block.starts_with("[Pending Follow-ups]"));
        assert!(
            block.contains("- Draft release note: Prepared highlights and risk notes for review")
        );
    }
}
