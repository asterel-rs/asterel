//! World model: tracks the agent's understanding of the external
//! environment, including active project context, tool reliability
//! records, and temporal awareness.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::contracts::strings::data_model::{
    SOURCE_PERSONA_WORLD_MODEL_UPDATE, SOURCE_PERSONA_WORLD_MODEL_WRITEBACK,
};
use crate::core::memory::{Memory, MemoryEventType};
use crate::core::persona::person_identity::{person_entity_id, sanitize_person_id};

/// Agent's understanding of the external environment.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct WorldModel {
    /// Currently active project context, if detected.
    pub active_project: Option<ProjectContext>,
    /// Per-tool success/failure reliability records.
    pub tool_reliability: Vec<ToolReliabilityRecord>,
    /// Temporal awareness for the current session.
    pub time_context: TimeContext,
}

/// Detected project metadata (language, framework, type).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProjectContext {
    /// Primary programming language.
    pub language: String,
    /// Framework in use, if any.
    pub framework: Option<String>,
    /// High-level project category (e.g. "web-service", "cli").
    pub project_type: String,
}

/// Success/failure statistics for a single tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ToolReliabilityRecord {
    /// Name of the tool being tracked.
    pub tool_name: String,
    /// Number of successful invocations.
    pub success_count: u32,
    /// Number of failed invocations.
    pub failure_count: u32,
    /// Average execution duration in milliseconds.
    pub avg_duration_ms: u64,
}

impl ToolReliabilityRecord {
    /// Compute the ratio of successes to total invocations (1.0 if none).
    pub(crate) fn success_rate(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            return 1.0;
        }
        f64::from(self.success_count) / f64::from(total)
    }
}

/// Temporal awareness: session start, turn count, time of day.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct TimeContext {
    /// RFC 3339 timestamp of the session start, if known.
    pub session_start: Option<String>,
    /// RFC 3339 timestamp of the previous completed turn, if known.
    #[serde(default)]
    pub last_turn_at: Option<String>,
    /// Number of turns completed in the current session.
    pub turn_count: u32,
    /// Coarse time-of-day label (e.g. "morning").
    pub time_of_day: Option<String>,
}

fn world_model_slot_key(person_id: &str) -> String {
    format!("persona/{}/world_model/v1", sanitize_person_id(person_id))
}

/// Load the world model from memory, returning a default if absent.
///
/// # Errors
///
/// Returns an error if the memory lookup or JSON parsing fails.
pub(crate) async fn load_world_model(mem: &dyn Memory, person_id: &str) -> Result<WorldModel> {
    let entity_id = person_entity_id(person_id);
    let slot_key = world_model_slot_key(person_id);
    let Some(slot) = mem.resolve_slot(&entity_id, &slot_key).await? else {
        return Ok(WorldModel::default());
    };
    serde_json::from_str::<WorldModel>(&slot.value)
        .with_context(|| format!("parse world model from slot key: {slot_key}"))
}

/// Persist the world model to memory.
///
/// # Errors
///
/// Returns an error if serialization or the memory write fails.
pub(crate) async fn persist_world_model(
    mem: &dyn Memory,
    person_id: &str,
    model: &WorldModel,
) -> Result<()> {
    super::persist_helper::persist_persona_slot(
        mem,
        person_entity_id(person_id),
        world_model_slot_key(person_id),
        MemoryEventType::FactUpdated,
        serde_json::to_string(model)?,
        0.8,
        0.5,
        SOURCE_PERSONA_WORLD_MODEL_UPDATE,
        SOURCE_PERSONA_WORLD_MODEL_WRITEBACK,
        None,
        person_id,
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;
    use crate::core::memory::{MarkdownMemory, Memory};

    fn tool_rec(name: &str, ok: u32, fail: u32, ms: u64) -> ToolReliabilityRecord {
        ToolReliabilityRecord {
            tool_name: name.to_string(),
            success_count: ok,
            failure_count: fail,
            avg_duration_ms: ms,
        }
    }

    #[test]
    fn default_world_model_is_empty() {
        let m = WorldModel::default();
        assert!(m.active_project.is_none());
        assert!(m.tool_reliability.is_empty());
        assert_eq!(m.time_context.turn_count, 0);
    }

    #[test]
    fn success_rate_edge_cases() {
        assert!((tool_rec("a", 0, 0, 0).success_rate() - 1.0).abs() < f64::EPSILON);
        assert!((tool_rec("a", 7, 3, 0).success_rate() - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn render_empty_model_returns_empty() {
        assert!(
            crate::core::persona::presenter::render_world_model_block(&WorldModel::default())
                .is_empty()
        );
    }

    #[test]
    fn render_with_project_and_tools() {
        let model = WorldModel {
            active_project: Some(ProjectContext {
                language: "Rust".into(),
                framework: Some("actix-web".into()),
                project_type: "web-service".into(),
            }),
            tool_reliability: vec![tool_rec("shell", 9, 1, 200)],
            ..WorldModel::default()
        };
        let block = crate::core::persona::presenter::render_world_model_block(&model);
        assert!(block.contains("[World Model]"));
        assert!(block.contains("Rust") && block.contains("actix-web"));
        assert!(block.contains("90%") && block.contains("200ms"));
    }

    #[tokio::test]
    async fn world_model_round_trip() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
        let expected = WorldModel {
            active_project: Some(ProjectContext {
                language: "Rust".into(),
                framework: None,
                project_type: "cli".into(),
            }),
            tool_reliability: vec![tool_rec("file_read", 5, 1, 45)],
            time_context: TimeContext {
                session_start: Some("2026-03-01T10:00:00Z".into()),
                last_turn_at: Some("2026-03-01T10:30:00Z".into()),
                turn_count: 12,
                time_of_day: Some("morning".into()),
            },
        };
        persist_world_model(mem.as_ref(), "person-test", &expected)
            .await
            .unwrap();
        let loaded = load_world_model(mem.as_ref(), "person-test").await.unwrap();
        let proj = loaded.active_project.as_ref().expect("project present");
        assert_eq!(proj.language, "Rust");
        assert_eq!(loaded.tool_reliability.len(), 1);
        assert_eq!(loaded.time_context.turn_count, 12);
    }

    #[tokio::test]
    async fn load_missing_returns_default() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
        let m = load_world_model(mem.as_ref(), "nonexistent").await.unwrap();
        assert!(m.active_project.is_none());
    }
}
