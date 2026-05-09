//! Self-task queueing: converts reflect-stage self-tasks into companion follow-ups.

use chrono::Utc;

use crate::core::memory::Memory;
use crate::core::persona::follow_up_queue::{PendingFollowUp, enqueue_follow_up};
use crate::security::writeback_guard::SelfTaskWriteback;

/// Convert reflect-stage self-task writebacks into persisted follow-up queue entries.
pub(super) async fn enqueue_reflect_self_tasks(
    mem: &dyn Memory,
    person_id: &str,
    self_tasks: &[SelfTaskWriteback],
) {
    let entity_id = crate::core::persona::person_identity::person_entity_id(person_id);
    for task in self_tasks {
        let task_id = format!(
            "reflect-follow-up-{}-{}",
            Utc::now().timestamp_millis(),
            sanitize_task_id_component(&task.title)
        );
        enqueue_follow_up(
            mem,
            &entity_id,
            person_id,
            PendingFollowUp {
                task_title: task.title.clone(),
                summary: task.instructions.clone(),
                created_at: Utc::now().to_rfc3339(),
                task_id,
            },
        )
        .await;
    }
}

fn sanitize_task_id_component(title: &str) -> String {
    let mut value = title
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
        .take(40)
        .collect::<String>()
        .to_ascii_lowercase();
    if value.is_empty() {
        value.push_str("task");
    }
    value
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::enqueue_reflect_self_tasks;
    use crate::core::memory::{MarkdownMemory, Memory};
    use crate::core::persona::follow_up_queue::load_pending_follow_ups;
    use crate::security::writeback_guard::SelfTaskWriteback;

    #[tokio::test]
    async fn reflect_self_tasks_become_follow_up_queue_entries() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
        let tasks = vec![SelfTaskWriteback {
            title: "Review queue".to_string(),
            instructions: "Check the remaining companion follow-ups".to_string(),
            expires_at: "2026-04-12T00:00:00Z".to_string(),
        }];

        enqueue_reflect_self_tasks(mem.as_ref(), "person-test", &tasks).await;

        let loaded = load_pending_follow_ups(mem.as_ref(), "person:person-test").await;
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].task_title, "Review queue");
    }
}
