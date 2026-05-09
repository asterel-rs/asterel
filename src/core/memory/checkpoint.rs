//! Memory checkpoint and rollback.
//!
//! A checkpoint captures a snapshot of the memory state at a point in time.
//! Checkpoints enable rollback: restoring the memory to a prior state by
//! replaying events up to the checkpoint watermark.
//!
//! Checkpoints are lightweight — they record a watermark (event count),
//! not a full copy of the data. Rollback works by logically ignoring
//! events after the watermark.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::contracts::ids::EntityId;

/// A snapshot marker for memory state at a specific point.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryCheckpoint {
    /// Unique checkpoint identifier.
    pub checkpoint_id: String,
    /// Entity this checkpoint belongs to.
    pub entity_id: EntityId,
    /// Event count at the time of checkpoint creation.
    /// All events with `seq_id` <= this watermark are "in" the checkpoint.
    pub watermark: usize,
    /// Human-readable label for the checkpoint.
    pub label: String,
    /// Who created this checkpoint.
    pub created_by: String,
    /// RFC 3339 timestamp of checkpoint creation.
    pub created_at: String,
}

/// Reason for a rollback operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RollbackReason {
    /// Why the rollback is being performed.
    pub description: String,
    /// Who initiated the rollback.
    pub initiated_by: String,
    /// RFC 3339 timestamp.
    pub initiated_at: String,
}

/// Result of a rollback operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RollbackResult {
    /// The checkpoint that was rolled back to.
    pub checkpoint: MemoryCheckpoint,
    /// Number of events that were logically undone.
    pub events_rolled_back: usize,
    /// The rollback reason for audit.
    pub reason: RollbackReason,
    /// Whether the rollback was successful.
    pub success: bool,
}

/// A registry of checkpoints for an entity.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckpointRegistry {
    /// All checkpoints, ordered by watermark ascending.
    checkpoints: Vec<MemoryCheckpoint>,
}

impl CheckpointRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn create_checkpoint(
        entity_id: impl Into<EntityId>,
        watermark: usize,
        label: impl Into<String>,
        created_by: impl Into<String>,
    ) -> MemoryCheckpoint {
        MemoryCheckpoint {
            checkpoint_id: Uuid::new_v4().to_string(),
            entity_id: entity_id.into(),
            watermark,
            label: label.into(),
            created_by: created_by.into(),
            created_at: Utc::now().to_rfc3339(),
        }
    }

    pub fn register(&mut self, checkpoint: MemoryCheckpoint) {
        let insert_at = self
            .checkpoints
            .partition_point(|existing| existing.watermark <= checkpoint.watermark);
        self.checkpoints.insert(insert_at, checkpoint);
    }

    #[must_use]
    pub fn latest(&self) -> Option<&MemoryCheckpoint> {
        self.checkpoints.last()
    }

    #[must_use]
    pub fn find_by_id(&self, checkpoint_id: &str) -> Option<&MemoryCheckpoint> {
        self.checkpoints
            .iter()
            .find(|checkpoint| checkpoint.checkpoint_id == checkpoint_id)
    }

    #[must_use]
    pub fn find_by_label(&self, label: &str) -> Option<&MemoryCheckpoint> {
        self.checkpoints
            .iter()
            .rev()
            .find(|checkpoint| checkpoint.label == label)
    }

    #[must_use]
    pub fn checkpoints(&self) -> &[MemoryCheckpoint] {
        &self.checkpoints
    }

    pub fn remove_by_id(&mut self, checkpoint_id: &str) -> Option<MemoryCheckpoint> {
        let index = self
            .checkpoints
            .iter()
            .position(|checkpoint| checkpoint.checkpoint_id == checkpoint_id)?;
        Some(self.checkpoints.remove(index))
    }
}

fn sanitize_path_component(input: &str) -> String {
    let trimmed = input.trim().trim_matches('.');
    if trimmed.is_empty() {
        return "anonymous".to_string();
    }
    trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn checkpoint_registry_path(workspace_dir: &Path, entity_id: &str) -> PathBuf {
    let safe_id = sanitize_path_component(entity_id);
    workspace_dir
        .join("state")
        .join("checkpoints")
        .join(format!("{safe_id}.json"))
}

/// Plan a rollback to a specific checkpoint.
///
/// This does NOT execute the rollback — it computes how many events
/// would be rolled back and returns a `RollbackResult` with `success=false`
/// (indicating it's a plan, not an execution).
///
/// # Errors
/// Returns an error if the checkpoint is not found in the registry.
pub fn plan_rollback(
    registry: &CheckpointRegistry,
    checkpoint_id: &str,
    current_event_count: usize,
    reason: RollbackReason,
) -> Result<RollbackResult> {
    let checkpoint = registry
        .find_by_id(checkpoint_id)
        .cloned()
        .with_context(|| format!("checkpoint not found: {checkpoint_id}"))?;

    let events_rolled_back = current_event_count.saturating_sub(checkpoint.watermark);
    let already_at_checkpoint = current_event_count <= checkpoint.watermark;

    Ok(RollbackResult {
        checkpoint,
        events_rolled_back,
        reason,
        success: already_at_checkpoint,
    })
}

/// Save a checkpoint registry to a workspace file.
///
/// # Errors
/// Returns an error if the file cannot be written.
pub fn save_checkpoint_registry(
    workspace_dir: &Path,
    entity_id: &str,
    registry: &CheckpointRegistry,
) -> Result<()> {
    let path = checkpoint_registry_path(workspace_dir, entity_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create checkpoint directory: {}", parent.display()))?;
    }

    let payload = serde_json::to_vec_pretty(registry).context("serialize checkpoint registry")?;
    let tmp_path = path.with_extension("tmp");

    fs::write(&tmp_path, payload)
        .with_context(|| format!("write checkpoint temp file: {}", tmp_path.display()))?;
    fs::rename(&tmp_path, &path).with_context(|| {
        format!(
            "rename checkpoint temp file from {} to {}",
            tmp_path.display(),
            path.display()
        )
    })?;

    Ok(())
}

/// Load a checkpoint registry from a workspace file.
///
/// Returns an empty registry if the file does not exist.
///
/// # Errors
/// Returns an error if the file exists but cannot be parsed.
pub fn load_checkpoint_registry(
    workspace_dir: &Path,
    entity_id: &str,
) -> Result<CheckpointRegistry> {
    let path = checkpoint_registry_path(workspace_dir, entity_id);
    if !path.exists() {
        return Ok(CheckpointRegistry::new());
    }

    let payload =
        fs::read(&path).with_context(|| format!("read checkpoint registry: {}", path.display()))?;
    let registry = serde_json::from_slice(&payload)
        .with_context(|| format!("parse checkpoint registry: {}", path.display()))?;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{
        CheckpointRegistry, MemoryCheckpoint, RollbackReason, load_checkpoint_registry,
        plan_rollback, save_checkpoint_registry,
    };

    fn checkpoint(entity_id: &str, watermark: usize, label: &str) -> MemoryCheckpoint {
        MemoryCheckpoint {
            checkpoint_id: format!("checkpoint-{label}-{watermark}"),
            entity_id: entity_id.into(),
            watermark,
            label: label.to_string(),
            created_by: "tester".to_string(),
            created_at: "2026-03-17T00:00:00Z".to_string(),
        }
    }

    fn rollback_reason() -> RollbackReason {
        RollbackReason {
            description: "operator rollback drill".to_string(),
            initiated_by: "tester".to_string(),
            initiated_at: "2026-03-17T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn create_checkpoint_generates_unique_id() {
        let first = CheckpointRegistry::create_checkpoint("entity-a", 3, "before", "tester");
        let second = CheckpointRegistry::create_checkpoint("entity-a", 3, "before", "tester");

        assert_ne!(first.checkpoint_id, second.checkpoint_id);
    }

    #[test]
    fn registry_maintains_watermark_order() {
        let mut registry = CheckpointRegistry::new();
        registry.register(checkpoint("entity-a", 10, "ten"));
        registry.register(checkpoint("entity-a", 2, "two"));
        registry.register(checkpoint("entity-a", 6, "six"));

        let watermarks: Vec<_> = registry
            .checkpoints()
            .iter()
            .map(|checkpoint| checkpoint.watermark)
            .collect();
        assert_eq!(watermarks, vec![2, 6, 10]);
    }

    #[test]
    fn latest_returns_highest_watermark() {
        let mut registry = CheckpointRegistry::new();
        registry.register(checkpoint("entity-a", 4, "early"));
        registry.register(checkpoint("entity-a", 12, "latest"));

        assert_eq!(
            registry.latest().map(|checkpoint| checkpoint.watermark),
            Some(12)
        );
    }

    #[test]
    fn find_by_id_returns_correct_checkpoint() {
        let mut registry = CheckpointRegistry::new();
        registry.register(checkpoint("entity-a", 4, "alpha"));
        registry.register(checkpoint("entity-a", 9, "beta"));

        let found = registry.find_by_id("checkpoint-beta-9");
        assert_eq!(
            found.map(|checkpoint| checkpoint.label.as_str()),
            Some("beta")
        );
    }

    #[test]
    fn find_by_label_returns_most_recent_match() {
        let mut registry = CheckpointRegistry::new();
        registry.register(checkpoint("entity-a", 1, "release"));
        registry.register(checkpoint("entity-a", 8, "release"));
        registry.register(checkpoint("entity-a", 5, "other"));

        let found = registry.find_by_label("release");
        assert_eq!(found.map(|checkpoint| checkpoint.watermark), Some(8));
    }

    #[test]
    fn remove_by_id_removes_and_returns() {
        let mut registry = CheckpointRegistry::new();
        registry.register(checkpoint("entity-a", 4, "alpha"));
        registry.register(checkpoint("entity-a", 9, "beta"));

        let removed = registry.remove_by_id("checkpoint-alpha-4");

        assert_eq!(
            removed.map(|checkpoint| checkpoint.label),
            Some("alpha".to_string())
        );
        assert!(registry.find_by_id("checkpoint-alpha-4").is_none());
        assert_eq!(registry.checkpoints().len(), 1);
    }

    #[test]
    fn plan_rollback_calculates_correct_event_count() {
        let mut registry = CheckpointRegistry::new();
        registry.register(checkpoint("entity-a", 7, "stable"));

        let plan = plan_rollback(&registry, "checkpoint-stable-7", 11, rollback_reason())
            .expect("rollback plan should succeed");

        assert_eq!(plan.events_rolled_back, 4);
        assert!(!plan.success);
        assert_eq!(plan.checkpoint.watermark, 7);
    }

    #[test]
    fn plan_rollback_returns_error_for_missing_checkpoint() {
        let registry = CheckpointRegistry::new();

        let error = plan_rollback(&registry, "missing", 11, rollback_reason())
            .expect_err("missing checkpoint should error");

        assert!(error.to_string().contains("checkpoint not found"));
    }

    #[test]
    fn save_and_load_registry_roundtrip() {
        let temp_dir = TempDir::new().expect("temp dir");
        let mut registry = CheckpointRegistry::new();
        registry.register(checkpoint("entity-a", 2, "first"));
        registry.register(checkpoint("entity-a", 6, "second"));

        save_checkpoint_registry(temp_dir.path(), "entity-a", &registry)
            .expect("save should succeed");
        let loaded =
            load_checkpoint_registry(temp_dir.path(), "entity-a").expect("load should succeed");

        assert_eq!(loaded, registry);
    }

    #[test]
    fn load_missing_registry_returns_empty() {
        let temp_dir = TempDir::new().expect("temp dir");

        let loaded = load_checkpoint_registry(temp_dir.path(), "entity-a")
            .expect("missing file should return empty registry");

        assert!(loaded.checkpoints().is_empty());
    }

    #[test]
    fn plan_rollback_marks_success_when_already_at_checkpoint() {
        let mut registry = CheckpointRegistry::new();
        registry.register(checkpoint("entity-a", 7, "stable"));

        let plan = plan_rollback(&registry, "checkpoint-stable-7", 7, rollback_reason())
            .expect("rollback plan should succeed");

        assert_eq!(plan.events_rolled_back, 0);
        assert!(plan.success);
    }

    #[test]
    fn sanitize_strips_path_traversal() {
        let result = super::sanitize_path_component("../../etc/passwd");
        assert!(!result.contains('/'));
        assert!(!result.contains(".."));
    }

    #[test]
    fn sanitize_normalizes_slashes_and_special_chars() {
        assert_eq!(
            super::sanitize_path_component("tenant/entity:foo"),
            "tenant_entity_foo"
        );
    }

    #[test]
    fn sanitize_falls_back_for_empty_input() {
        assert_eq!(super::sanitize_path_component(""), "anonymous");
        assert_eq!(super::sanitize_path_component("..."), "anonymous");
        assert_eq!(super::sanitize_path_component("  "), "anonymous");
    }

    #[test]
    fn sanitize_preserves_safe_characters() {
        assert_eq!(
            super::sanitize_path_component("agent-1_test"),
            "agent-1_test"
        );
    }
}
