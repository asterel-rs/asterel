//! Two-phase heartbeat: lightweight skip/run check before full agent loop (WP-J2).
//!
//! Phase 1: A cheap LLM call returns a structured `{action: skip|run}` decision.
//! Phase 2: Only if `action == run`, the full agent loop fires.
//!
//! This separates the cost of checking for tasks from the cost of executing them.
//!
//! Design source: ecosystem survey 2026-04-03 (nanobot two-phase heartbeat).
//!
//! ## Wiring status — Phase J
//!
//! **Wired (2026-04-05):** `HeartbeatCheckResult` is constructed by `heartbeat_worker.rs`
//! from `engine.collect_tasks()` output; `should_run()` gates the full agent loop execution.

use serde::{Deserialize, Serialize};

/// Result of the Phase 1 heartbeat check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HeartbeatAction {
    /// Nothing to do — skip the full agent loop.
    Skip,
    /// Tasks detected — run the full agent loop.
    Run,
}

/// Structured heartbeat check response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct HeartbeatCheckResult {
    /// Whether to skip or run the full loop.
    pub action: HeartbeatAction,
    /// Brief description of pending tasks (only when `action == Run`).
    #[serde(default)]
    pub tasks: String,
}

impl HeartbeatCheckResult {
    /// Create a skip result.
    #[must_use]
    pub(crate) fn skip() -> Self {
        Self {
            action: HeartbeatAction::Skip,
            tasks: String::new(),
        }
    }

    /// Create a run result with a task description.
    #[must_use]
    pub(crate) fn run(tasks: &str) -> Self {
        Self {
            action: HeartbeatAction::Run,
            tasks: tasks.to_string(),
        }
    }

    /// Whether the full agent loop should fire.
    #[must_use]
    pub(crate) fn should_run(&self) -> bool {
        self.action == HeartbeatAction::Run
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_result() {
        let result = HeartbeatCheckResult::skip();
        assert!(!result.should_run());
    }

    #[test]
    fn run_result() {
        let result = HeartbeatCheckResult::run("check scheduled reports");
        assert!(result.should_run());
        assert_eq!(result.tasks, "check scheduled reports");
    }
}
