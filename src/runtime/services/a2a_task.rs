//! A2A task lifecycle service.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

use crate::contracts::a2a::{
    A2A_MAX_TASKS, A2A_TASK_HARD_TTL_SECS, A2A_TASK_TTL_SECS, A2aOutboundMessage, A2aTask,
    A2aTaskState,
};

pub type A2aTaskStore = Arc<RwLock<HashMap<String, A2aTask>>>;

#[must_use]
pub fn new_a2a_task_store() -> A2aTaskStore {
    Arc::new(RwLock::new(HashMap::new()))
}

#[allow(clippy::implicit_hasher)]
#[allow(clippy::missing_errors_doc)]
pub async fn register_task(
    tasks: &RwLock<HashMap<String, A2aTask>>,
    conversation_id: &str,
    tenant_id: Option<&String>,
    owner_principal: Option<&String>,
) -> Result<String, &'static str> {
    let task_id = Uuid::new_v4().to_string();
    let mut tasks = tasks.write().await;
    evict_stale_tasks(&mut tasks);
    if tasks.len() >= A2A_MAX_TASKS {
        return Err("capacity_exceeded");
    }
    tasks.insert(
        task_id.clone(),
        A2aTask {
            id: task_id.clone(),
            conversation_id: conversation_id.to_string(),
            state: A2aTaskState::Working,
            response: None,
            error: None,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            tenant_id: tenant_id.cloned(),
            owner_principal: owner_principal.cloned(),
        },
    );
    Ok(task_id)
}

/// Mark a task as completed.
#[allow(clippy::implicit_hasher)]
pub async fn complete_task(
    tasks: &RwLock<HashMap<String, A2aTask>>,
    task_id: &str,
    response: A2aOutboundMessage,
) {
    let mut tasks = tasks.write().await;
    if let Some(task) = tasks.get_mut(task_id) {
        task.state = A2aTaskState::Completed;
        task.response = Some(response);
        task.error = None;
    }
}

/// Mark a task as failed.
#[allow(clippy::implicit_hasher)]
pub async fn fail_task(tasks: &RwLock<HashMap<String, A2aTask>>, task_id: &str) {
    let mut tasks = tasks.write().await;
    if let Some(task) = tasks.get_mut(task_id) {
        task.state = A2aTaskState::Failed;
        task.error = Some("LLM request failed".to_string());
        task.response = None;
    }
}

/// Cancel a task.
#[allow(clippy::implicit_hasher)]
pub async fn cancel_task(tasks: &RwLock<HashMap<String, A2aTask>>, task_id: &str) {
    let mut tasks = tasks.write().await;
    if let Some(task) = tasks.get_mut(task_id) {
        task.state = A2aTaskState::Canceled;
        task.error = None;
    }
}

/// Evict expired and overflow tasks from the A2A task map.
#[allow(clippy::implicit_hasher)] // Concrete HashMap is sufficient here.
pub fn evict_stale_tasks(tasks: &mut HashMap<String, A2aTask>) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    tasks.retain(|_, task| {
        let age = now.saturating_sub(task.created_at);
        let terminal = matches!(
            task.state,
            A2aTaskState::Completed | A2aTaskState::Failed | A2aTaskState::Canceled
        );
        if terminal {
            age <= A2A_TASK_TTL_SECS
        } else {
            age <= A2A_TASK_HARD_TTL_SECS
        }
    });

    if tasks.len() > A2A_MAX_TASKS {
        let mut terminal_ids: Vec<(String, u64)> = tasks
            .iter()
            .filter(|(_, task)| {
                matches!(
                    task.state,
                    A2aTaskState::Completed | A2aTaskState::Failed | A2aTaskState::Canceled
                )
            })
            .map(|(id, task)| (id.clone(), task.created_at))
            .collect();
        terminal_ids.sort_by_key(|(_, created_at)| *created_at);
        let to_remove = tasks.len().saturating_sub(A2A_MAX_TASKS);
        for (id, _) in terminal_ids.into_iter().take(to_remove) {
            tasks.remove(&id);
        }
    }

    if tasks.len() > A2A_MAX_TASKS {
        let mut all_ids: Vec<(String, u64)> = tasks
            .iter()
            .map(|(id, task)| (id.clone(), task.created_at))
            .collect();
        all_ids.sort_by_key(|(_, created_at)| *created_at);
        let to_remove = tasks.len().saturating_sub(A2A_MAX_TASKS);
        for (id, _) in all_ids.into_iter().take(to_remove) {
            tasks.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_task(id: &str, state: A2aTaskState, age_secs: u64) -> (String, A2aTask) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        (
            id.to_string(),
            A2aTask {
                id: id.to_string(),
                conversation_id: "conv-1".to_string(),
                state,
                response: None,
                error: None,
                created_at: now.saturating_sub(age_secs),
                tenant_id: None,
                owner_principal: None,
            },
        )
    }

    #[test]
    fn evict_removes_expired_terminal_tasks() {
        let mut tasks = HashMap::new();
        tasks.extend([
            make_task("t1", A2aTaskState::Completed, A2A_TASK_TTL_SECS + 10),
            make_task("t2", A2aTaskState::Failed, A2A_TASK_TTL_SECS + 10),
            make_task("t3", A2aTaskState::Working, 60),
        ]);

        evict_stale_tasks(&mut tasks);
        assert_eq!(tasks.len(), 1);
        assert!(tasks.contains_key("t3"));
    }

    #[test]
    fn evict_removes_non_terminal_tasks_past_hard_ttl() {
        let mut tasks = HashMap::new();
        tasks.extend([
            make_task("t1", A2aTaskState::Working, A2A_TASK_HARD_TTL_SECS + 10),
            make_task("t2", A2aTaskState::Submitted, A2A_TASK_HARD_TTL_SECS + 10),
            make_task("t3", A2aTaskState::Working, 60),
        ]);

        evict_stale_tasks(&mut tasks);
        assert_eq!(tasks.len(), 1);
        assert!(tasks.contains_key("t3"));
    }

    #[test]
    fn evict_capacity_overflow_removes_terminal_first() {
        let mut tasks = HashMap::new();
        for i in 0..A2A_MAX_TASKS {
            let (id, task) = make_task(&format!("w{i}"), A2aTaskState::Working, 120);
            tasks.insert(id, task);
        }
        let (id, task) = make_task("extra", A2aTaskState::Completed, 60);
        tasks.insert(id, task);

        evict_stale_tasks(&mut tasks);
        assert!(tasks.len() <= A2A_MAX_TASKS);
        assert!(!tasks.contains_key("extra"));
    }
}
