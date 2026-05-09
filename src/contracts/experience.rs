//! Core data types for the experience subsystem

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::contracts::scores::Confidence;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExperienceKind {
    /// Agent-initiated self-improvement task.
    SelfTask,
    /// Persona or state writeback accepted by the runtime.
    #[serde(alias = "evolution_change")]
    PersonaWriteback,
    /// Structured turn outcome with quantitative signals (ADR-0010 Phase 1).
    #[serde(alias = "plan_execution")]
    TurnInteraction,
    /// Codespace project activity (create, test, promote, etc.).
    CodespaceActivity,
}

impl ExperienceKind {
    /// Return the `snake_case` string label for this kind.
    pub(crate) fn kind_str(&self) -> &'static str {
        match self {
            Self::SelfTask => "self_task",
            Self::PersonaWriteback => "persona_writeback",
            Self::TurnInteraction => "turn_interaction",
            Self::CodespaceActivity => "codespace_activity",
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExperienceOutcome {
    /// Task completed successfully.
    Success,
    /// Task failed.
    Failure,
    /// Task partially completed.
    Partial,
    /// Outcome could not be determined.
    Unknown,
}

/// A single unit of experience captured from agent activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExperienceAtom {
    /// Unique identifier for this experience atom.
    pub id: String,
    /// Category of the experience.
    pub kind: ExperienceKind,
    /// Brief description of what happened.
    pub summary: String,
    /// Outcome of the activity.
    pub outcome: ExperienceOutcome,
    /// Extracted lesson or takeaway.
    pub lesson: String,
    /// RFC 3339 timestamp of occurrence.
    pub occurred_at: String,
    /// Confidence score in `[0.0, 1.0]`.
    pub confidence: Confidence,
}

impl ExperienceAtom {
    /// Create a new experience atom with default lesson and confidence.
    pub(crate) fn new(
        kind: ExperienceKind,
        summary: impl Into<String>,
        outcome: ExperienceOutcome,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            kind,
            summary: summary.into(),
            outcome,
            lesson: String::new(),
            occurred_at: Utc::now().to_rfc3339(),
            confidence: Confidence::new(0.7),
        }
    }

    /// Set the lesson field (builder pattern).
    #[must_use]
    pub(crate) fn with_lesson(mut self, lesson: impl Into<String>) -> Self {
        self.lesson = lesson.into();
        self
    }

    /// Set the confidence field (builder pattern).
    #[must_use]
    pub(crate) fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = Confidence::new(confidence);
        self
    }
}
