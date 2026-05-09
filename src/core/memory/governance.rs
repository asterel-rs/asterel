//! Memory governance: access logging, status lifecycle, and archive policy (WP-I3).
//!
//! Provides an in-process audit trail for memory reads and writes, plus
//! a configurable lifecycle policy for archiving and deleting entries.
//!
//! ## Key types
//!
//! - [`MemoryState`] — lifecycle state of a memory entry:
//!   `Active → Paused → Archived → Deleted`.
//! - [`MemoryAccessLog`] — thread-safe session-scoped log of all
//!   access records and state transitions.
//! - [`ArchivePolicy`] — days-until-archive / days-until-delete thresholds;
//!   `Paused` entries are exempt from auto-archiving by default.
//!
//! ## Design note
//!
//! These types are currently in-process only (no persistence). The access
//! log is cleared between sessions. Persistent governance audit trails are
//! tracked via the `deletion_ledger` table in the `PostgreSQL` backend.
//!
//! Design source: ecosystem survey 2026-04-03 (`mem0` `OpenMemory`
//! memory status history, access log, archive policy).
//!
//! ## Wiring status — Phase I
//!
//! **Partial wiring (2026-04-05):** memory tools record into
//! `ExecutionContext.memory_access_log` when a caller supplies one. The default
//! runtime context does not yet create a persistent session log, and the removed
//! `memory_pin` tool is no longer part of the live tool set. `ArchivePolicy`
//! retains `#[allow(dead_code)]` until the compaction/archive pass lands.

use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::contracts::ids::EntityId;

/// Memory lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryState {
    Active,
    Paused,
    Archived,
    Deleted,
}

/// Record of a memory access (read or write).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MemoryAccessRecord {
    /// Which entity performed the access.
    pub entity_id: EntityId,
    /// What type of access occurred.
    pub access_type: MemoryAccessType,
    /// When the access occurred.
    pub timestamp: DateTime<Utc>,
    /// Optional memory slot key that was accessed.
    pub slot_key: Option<String>,
}

/// Type of memory access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryAccessType {
    Read,
    Write,
    Delete,
    Search,
}

/// Record of a memory state transition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MemoryStatusTransition {
    /// Previous state.
    pub from: MemoryState,
    /// New state.
    pub to: MemoryState,
    /// Who triggered the transition.
    pub changed_by: String,
    /// When the transition occurred.
    pub timestamp: DateTime<Utc>,
}

/// Configurable archive policy for memory entries.
// TODO(phase-I): applied in memory backend's scheduled compaction/archive pass.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ArchivePolicy {
    /// Days after last access before auto-archiving. 0 = disabled.
    pub days_until_archive: u32,
    /// Days after archiving before permanent deletion. 0 = never delete.
    pub days_until_delete: u32,
    /// Whether paused memories are exempt from auto-archiving.
    pub exempt_paused: bool,
}

impl Default for ArchivePolicy {
    fn default() -> Self {
        Self {
            days_until_archive: 90,
            days_until_delete: 0, // never auto-delete
            exempt_paused: true,
        }
    }
}

/// In-memory access log for the current session. Thread-safe via internal mutexes.
pub struct MemoryAccessLog {
    records: Mutex<Vec<MemoryAccessRecord>>,
    transitions: Mutex<Vec<MemoryStatusTransition>>,
}

impl MemoryAccessLog {
    /// Record a memory access.
    pub(crate) fn record_access(
        &self,
        entity_id: &str,
        access_type: MemoryAccessType,
        slot_key: Option<&str>,
    ) {
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        records.push(MemoryAccessRecord {
            entity_id: entity_id.into(),
            access_type,
            timestamp: Utc::now(),
            slot_key: slot_key.map(ToString::to_string),
        });
    }

    /// Record a state transition.
    pub(crate) fn record_transition(&self, from: MemoryState, to: MemoryState, changed_by: &str) {
        let mut transitions = self
            .transitions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        transitions.push(MemoryStatusTransition {
            from,
            to,
            changed_by: changed_by.to_string(),
            timestamp: Utc::now(),
        });
    }
}

/// Construction and diagnostic accessors — currently used by tests and any
/// caller that explicitly wires `ExecutionContext.memory_access_log`.
/// `new()` still has no default runtime call site; it will move to one when
/// session state owns a live memory access log.
/// The query methods are used in tests and will serve diagnostics / admin endpoints later.
#[allow(dead_code)]
impl MemoryAccessLog {
    pub(crate) fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            transitions: Mutex::new(Vec::new()),
        }
    }

    /// Number of recorded accesses.
    pub(crate) fn access_count(&self) -> usize {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Number of recorded transitions.
    pub(crate) fn transition_count(&self) -> usize {
        self.transitions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Get all access records for an entity (cloned).
    pub(crate) fn accesses_for(&self, entity_id: &str) -> Vec<MemoryAccessRecord> {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|r| r.entity_id.as_str() == entity_id)
            .cloned()
            .collect()
    }

    /// Get all recorded state transitions (cloned).
    pub(crate) fn transitions(&self) -> Vec<MemoryStatusTransition> {
        self.transitions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_query_access() {
        let log = MemoryAccessLog::new();
        log.record_access(
            "person:local",
            MemoryAccessType::Read,
            Some("persona.identity"),
        );
        log.record_access(
            "person:local",
            MemoryAccessType::Write,
            Some("persona.mood"),
        );
        log.record_access("person:remote", MemoryAccessType::Search, None);

        assert_eq!(log.access_count(), 3);
        let local = log.accesses_for("person:local");
        assert_eq!(local.len(), 2);
        assert_eq!(local[0].access_type, MemoryAccessType::Read);
        assert_eq!(local[0].slot_key.as_deref(), Some("persona.identity"));
        assert_eq!(local[1].access_type, MemoryAccessType::Write);

        let remote = log.accesses_for("person:remote");
        assert_eq!(remote[0].access_type, MemoryAccessType::Search);
        assert!(remote[0].slot_key.is_none());
    }

    #[test]
    fn record_state_transition() {
        let log = MemoryAccessLog::new();
        log.record_transition(MemoryState::Active, MemoryState::Archived, "system");
        assert_eq!(log.transition_count(), 1);
        let transitions = log.transitions();
        let t = &transitions[0];
        assert_eq!(t.from, MemoryState::Active);
        assert_eq!(t.to, MemoryState::Archived);
        assert_eq!(t.changed_by, "system");
    }

    #[test]
    fn default_archive_policy() {
        let policy = ArchivePolicy::default();
        assert_eq!(policy.days_until_archive, 90);
        assert_eq!(policy.days_until_delete, 0);
        assert!(policy.exempt_paused);
    }
}
