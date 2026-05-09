//! Runtime checkpoint for crash recovery (WP-H2).
//!
//! Persists in-flight turn state before each tool execution so that
//! a crash mid-turn can be recovered on restart. Interrupted tools
//! are replayed with error markers.
//!
//! Design source: ecosystem survey 2026-04-03 (nanobot runtime checkpoint).
//!
//! ## Wiring status — phase-H
//!
//! **Blocked by:** crash-recovery replay loop in `AgentSession::start`.
//! **Entry point:** `AgentSession::start` calls `load_from_file` to detect a prior
//! crashed turn; the replay loop calls `is_completed` to skip already-done calls and
//! `interrupted_tools` to inject error markers for pending ones.
//! `is_completed`, `interrupted_tools`, and `load_from_file` carry
//! `#[allow(dead_code)]` until that wiring lands.

use serde::{Deserialize, Serialize};

use crate::contracts::ids::SessionId;

/// Snapshot of an in-flight turn that can be restored after a crash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TurnCheckpoint {
    /// Session ID this checkpoint belongs to.
    pub session_id: SessionId,
    /// The assistant message content that was being constructed.
    pub assistant_message: String,
    /// Tool calls that have already completed in this turn.
    pub completed_tool_results: Vec<CompletedToolResult>,
    /// Tool call IDs that were dispatched but not yet completed.
    pub pending_tool_call_ids: Vec<String>,
    /// Monotonic checkpoint counter for deduplication.
    pub sequence: u64,
}

/// A tool result that was completed before the crash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CompletedToolResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub output: String,
    pub success: bool,
}

impl TurnCheckpoint {
    /// Create a new checkpoint for a turn.
    pub(crate) fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.into(),
            assistant_message: String::new(),
            completed_tool_results: Vec::new(),
            pending_tool_call_ids: Vec::new(),
            sequence: 0,
        }
    }

    /// Record a tool call as dispatched (pending).
    pub(crate) fn mark_dispatched(&mut self, tool_call_id: &str) {
        self.pending_tool_call_ids.push(tool_call_id.to_string());
        self.sequence += 1;
    }

    /// Record a tool call as completed.
    pub(crate) fn mark_completed(&mut self, result: CompletedToolResult) {
        self.pending_tool_call_ids
            .retain(|id| id != &result.tool_call_id);
        self.completed_tool_results.push(result);
        self.sequence += 1;
    }

    /// Check if a tool call ID has already been completed (for dedup on replay).
    #[allow(dead_code)]
    pub(crate) fn is_completed(&self, tool_call_id: &str) -> bool {
        self.completed_tool_results
            .iter()
            .any(|r| r.tool_call_id == tool_call_id)
    }

    /// Get pending tool calls that were interrupted.
    pub(crate) fn interrupted_tools(&self) -> &[String] {
        &self.pending_tool_call_ids
    }

    /// Serialize to JSON for persistence.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub(crate) fn save_to_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Load from a checkpoint file. Returns `None` if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but is malformed.
    pub(crate) fn load_from_file(path: &std::path::Path) -> anyhow::Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let json = std::fs::read_to_string(path)?;
        let checkpoint: Self = serde_json::from_str(&json)?;
        Ok(Some(checkpoint))
    }

    /// Remove the checkpoint file after successful turn completion.
    pub(crate) fn clear_file(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_checkpoint_is_empty() {
        let cp = TurnCheckpoint::new("sess-1");
        assert_eq!(cp.session_id.as_str(), "sess-1");
        assert!(cp.completed_tool_results.is_empty());
        assert!(cp.pending_tool_call_ids.is_empty());
        assert_eq!(cp.sequence, 0);
    }

    #[test]
    fn dispatch_and_complete_cycle() {
        let mut cp = TurnCheckpoint::new("sess-1");
        cp.mark_dispatched("call-1");
        cp.mark_dispatched("call-2");
        assert_eq!(cp.pending_tool_call_ids.len(), 2);

        cp.mark_completed(CompletedToolResult {
            tool_call_id: "call-1".to_string(),
            tool_name: "shell".to_string(),
            output: "ok".to_string(),
            success: true,
        });
        assert_eq!(cp.pending_tool_call_ids.len(), 1);
        assert!(cp.is_completed("call-1"));
        assert!(!cp.is_completed("call-2"));
    }

    #[test]
    fn interrupted_tools_reports_pending() {
        let mut cp = TurnCheckpoint::new("sess-1");
        cp.mark_dispatched("call-1");
        cp.mark_dispatched("call-2");
        cp.mark_completed(CompletedToolResult {
            tool_call_id: "call-1".to_string(),
            tool_name: "file_read".to_string(),
            output: "data".to_string(),
            success: true,
        });
        assert_eq!(cp.interrupted_tools(), &["call-2"]);
    }

    #[test]
    fn file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("checkpoint.json");

        let mut cp = TurnCheckpoint::new("sess-1");
        cp.assistant_message = "working on it".to_string();
        cp.mark_dispatched("call-1");
        cp.save_to_file(&path).unwrap();

        let loaded = TurnCheckpoint::load_from_file(&path)
            .unwrap()
            .expect("should exist");
        assert_eq!(loaded.session_id.as_str(), "sess-1");
        assert_eq!(loaded.assistant_message, "working on it");
        assert_eq!(loaded.pending_tool_call_ids, vec!["call-1"]);

        TurnCheckpoint::clear_file(&path);
        assert!(TurnCheckpoint::load_from_file(&path).unwrap().is_none());
    }
}
