//! File ownership tracker: write confers ownership, owned file delete
//! is auto-approved (WP-I2).
//!
//! Tracks which files were created or modified by the agent in the
//! current session. Owned files can be deleted without operator approval.
//! Foreign file deletes still require approval.
//!
//! Design source: ecosystem survey 2026-04-03 (koda `FileTracker`).
//!
//! ## Wiring status — Phase I (complete)
//!
//! `record_create`/`record_modify` are called from `file_write` via the
//! process-global [`global_tracker`].  `is_owned()`, `owned_count()`, and
//! `remove()` are wired into `FileDeleteTool`: ownership gates auto-approval
//! of deletions, and `remove()` clears the record after a successful delete.
//! `OwnershipRecord` fields (`kind`, `acquired_at_turn`) are diagnostic — not
//! yet surfaced in any output — so the struct retains `#[allow(dead_code)]`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Tracks file ownership for the current session.
pub(crate) struct FileOwnershipTracker {
    /// Map of canonical file path → ownership info.
    owned: Mutex<HashMap<PathBuf, OwnershipRecord>>,
}

// TODO(phase-I): fields surfaced in session diagnostics — see module wiring status.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct OwnershipRecord {
    /// How ownership was acquired.
    kind: OwnershipKind,
    /// Turn number when ownership was acquired.
    acquired_at_turn: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OwnershipKind {
    /// File was created by the agent.
    Created,
    /// File was modified by the agent.
    Modified,
}

/// Access the process-global [`FileOwnershipTracker`] shared by `file_write`
/// and `file_delete`.  Initialised lazily on first call.
pub(super) fn global_tracker() -> &'static FileOwnershipTracker {
    static TRACKER: std::sync::OnceLock<FileOwnershipTracker> = std::sync::OnceLock::new();
    TRACKER.get_or_init(FileOwnershipTracker::new)
}

impl FileOwnershipTracker {
    pub(crate) fn new() -> Self {
        Self {
            owned: Mutex::new(HashMap::new()),
        }
    }

    /// Record that a file was created by the agent.
    pub(crate) fn record_create(&self, path: &Path, turn: u32) {
        if let Ok(canonical) =
            std::fs::canonicalize(path).or_else(|_| Ok::<_, std::io::Error>(path.to_path_buf()))
        {
            self.owned
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(
                    canonical,
                    OwnershipRecord {
                        kind: OwnershipKind::Created,
                        acquired_at_turn: turn,
                    },
                );
        }
    }

    /// Record that a file was modified by the agent.
    pub(crate) fn record_modify(&self, path: &Path, turn: u32) {
        if let Ok(canonical) = std::fs::canonicalize(path) {
            self.owned
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .entry(canonical)
                .or_insert(OwnershipRecord {
                    kind: OwnershipKind::Modified,
                    acquired_at_turn: turn,
                });
        }
    }

    /// Check if the agent owns a file (created or modified it this session).
    pub(crate) fn is_owned(&self, path: &Path) -> bool {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.owned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(&canonical)
    }

    /// Number of owned files.
    pub(crate) fn owned_count(&self) -> usize {
        self.owned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Remove ownership record (called from `file_delete` after confirmed deletion).
    pub(crate) fn remove(&self, path: &Path) {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        self.owned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&canonical);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn created_file_is_owned() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let tracker = FileOwnershipTracker::new();
        tracker.record_create(&file_path, 1);
        assert!(tracker.is_owned(&file_path));
    }

    #[test]
    fn untracked_file_is_not_owned() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("foreign.txt");
        std::fs::write(&file_path, "data").unwrap();

        let tracker = FileOwnershipTracker::new();
        assert!(!tracker.is_owned(&file_path));
    }

    #[test]
    fn remove_clears_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let tracker = FileOwnershipTracker::new();
        tracker.record_create(&file_path, 1);
        assert_eq!(tracker.owned_count(), 1);

        tracker.remove(&file_path);
        assert!(!tracker.is_owned(&file_path));
        assert_eq!(tracker.owned_count(), 0);
    }

    #[test]
    fn modify_does_not_overwrite_create() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "v1").unwrap();

        let tracker = FileOwnershipTracker::new();
        tracker.record_create(&file_path, 1);
        tracker.record_modify(&file_path, 2);
        // Should still show as owned (create takes precedence via or_insert)
        assert!(tracker.is_owned(&file_path));
        assert_eq!(tracker.owned_count(), 1);
    }
}
