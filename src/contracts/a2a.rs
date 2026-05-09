//! Agent-to-Agent (A2A) protocol contracts.
//!
//! A2A is the protocol by which one agent delegates a task to another agent
//! and tracks its asynchronous completion. Because agent execution is
//! non-blocking, the delegating agent polls task state rather than waiting
//! for an immediate return value.
//!
//! The central state machine is [`A2aTaskState`].

/// Outbound text part for A2A messages.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct A2aOutboundPart {
    #[serde(rename = "type")]
    pub part_type: String,
    pub text: String,
}

/// Outbound A2A message envelope.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct A2aOutboundMessage {
    pub role: String,
    pub parts: Vec<A2aOutboundPart>,
}

/// Stored A2A task record.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct A2aTask {
    pub id: String,
    pub conversation_id: String,
    pub state: A2aTaskState,
    pub response: Option<A2aOutboundMessage>,
    #[serde(default)]
    pub error: Option<String>,
    /// Unix-epoch seconds when the task was created (used for TTL eviction).
    #[serde(default)]
    pub created_at: u64,
    /// Tenant that owns this task. Used to enforce cross-tenant isolation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    /// Auth principal that created this task. Used to enforce ownership on task APIs.
    #[serde(default, skip_serializing, skip_deserializing)]
    pub owner_principal: Option<String>,
}

/// Maximum number of A2A tasks kept in memory before eviction.
pub const A2A_MAX_TASKS: usize = 10_000;

/// Tasks older than this (in seconds) are eligible for eviction.
pub const A2A_TASK_TTL_SECS: u64 = 3600;

/// Hard TTL for non-terminal A2A tasks (4 hours). Prevents indefinite resource pinning.
pub const A2A_TASK_HARD_TTL_SECS: u64 = 14_400;

/// Lifecycle state of an asynchronous A2A task.
///
/// Valid state transitions (other transitions are invalid):
///
/// ```text
/// Submitted → Working → Completed
///                     ↘ Failed
/// Submitted → Canceled
/// Working   → Canceled
/// ```
///
/// - `Submitted` — the task has been accepted by the remote agent and is
///   queued for execution. The delegating agent can still cancel at this
///   stage.
/// - `Working` — the remote agent has begun executing the task. Cancellation
///   is best-effort; the agent may or may not honor it.
/// - `Completed` — the task finished successfully. The result payload is
///   available for retrieval.
/// - `Failed` — the task terminated with an error. The error detail is
///   stored alongside the task record. The delegating agent should decide
///   whether to retry, escalate, or surface the failure to the user.
/// - `Canceled` — the task was explicitly canceled before completion.
///   No result is available.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum A2aTaskState {
    Submitted,
    Working,
    Completed,
    Failed,
    Canceled,
}
