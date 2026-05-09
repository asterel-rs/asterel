//! Type definitions for writeback guard payloads: state headers,
//! style profiles, memory inferences, self-tasks, and verdicts.

use crate::contracts::ids::SlotKey;

/// Immutable fields that writebacks must echo back unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImmutableStateHeader {
    /// Schema version for forward compatibility.
    pub schema_version: u32,
    /// Hash of the identity principles document.
    pub identity_principles_hash: String,
    /// Safety posture level (e.g., "strict", "relaxed").
    pub safety_posture: String,
}

/// Mutable state header fields written back by the LLM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateHeaderWriteback {
    /// The agent's current high-level objective.
    pub current_objective: String,
    /// Unresolved tasks or questions.
    pub open_loops: Vec<String>,
    /// Planned next actions.
    pub next_actions: Vec<String>,
    /// Promises or invariants the agent is maintaining.
    pub commitments: Vec<String>,
    /// Brief summary of the recent conversation context.
    pub recent_context_summary: String,
    /// RFC 3339 timestamp of the last update.
    pub last_updated_at: String,
}

/// Validated writeback payload produced by the LLM.
#[derive(Debug, Clone, PartialEq)]
pub struct WritebackPayload {
    /// Mutable state header fields.
    pub state_header: StateHeaderWriteback,
    /// New memory entries to append.
    pub memory_append: Vec<String>,
    /// Self-assigned tasks with expiry deadlines.
    pub self_tasks: Vec<SelfTaskWriteback>,
    /// Optional style profile adjustments.
    pub style_profile: Option<StyleWriteback>,
    /// Inferences about the conversation or environment.
    pub memory_inferences: Vec<MemoryInferenceEntry>,
    /// Inferences about the user (stored under user `entity_id`).
    pub user_inferences: Vec<MemoryInferenceEntry>,
}

/// A single key-value inference to store in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryInferenceEntry {
    /// Dot-separated slot key (e.g., "user.preference.lang").
    pub slot_key: SlotKey,
    /// The inferred value to store.
    pub value: String,
}

/// A self-assigned task with a bounded expiry horizon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfTaskWriteback {
    /// Short title for the task.
    pub title: String,
    /// Detailed instructions for the task.
    pub instructions: String,
    /// RFC 3339 expiry timestamp (bounded by max horizon).
    pub expires_at: String,
}

/// Style profile adjustments within safe ranges.
#[derive(Debug, Clone, PartialEq)]
pub struct StyleWriteback {
    /// Formality score (bounded by `STYLE_SCORE_MIN..=STYLE_SCORE_MAX`).
    pub formality: u8,
    /// Verbosity score (bounded by `STYLE_SCORE_MIN..=STYLE_SCORE_MAX`).
    pub verbosity: u8,
    /// Temperature value (bounded by safe float range).
    pub temperature: f64,
}

/// Outcome of writeback validation: accepted payload or rejection.
#[derive(Debug, Clone, PartialEq)]
pub enum WritebackVerdict {
    /// Payload passed all validation checks.
    Accepted(Box<WritebackPayload>),
    /// Payload was rejected for the given reason.
    Rejected {
        /// Human-readable rejection reason (sanitized).
        reason: String,
    },
}

/// Allowed slot specification derived from the companion turn contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedWritebackSlot {
    /// Slot key or prefix pattern (`*` suffix means prefix match).
    pub slot: String,
    /// Source rationale for observability and rejection context.
    pub source_rationale: String,
}

/// Thin plan metadata forwarded from reflect pre-processing into the guard.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WritebackPlanMetadata {
    /// Allowed writeback slot keys/patterns for this turn.
    pub allowed_slots: Vec<AllowedWritebackSlot>,
}
