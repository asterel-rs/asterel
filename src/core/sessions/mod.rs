//! Session management subsystem: lifecycle, compaction, storage, and transcript tooling.
//!
//! ## Session lifecycle
//!
//! Sessions move through the following states (see [`types::SessionState`]):
//!
//! ```text
//! Active ──► Compacted ──► Archived
//!             │
//!             └──► Active  (rehydrated on next turn)
//! ```
//!
//! - **Active**: accumulates `ChatMessage` turns in memory and the backing store.
//! - **Compacted**: the older portion of the transcript has been summarised to a single synthetic
//!   message to keep context within the model's token budget.
//! - **Archived**: the session is closed and retained for audit/retrieval only.
//!
//! ## Compaction pipeline
//!
//! Triggered automatically by [`orchestrator::SessionOrchestrator`] when the transcript exceeds
//! the configured token threshold (see [`types::CompactionConfig`]):
//!
//! 1. **Token check** — estimate token count; skip if below threshold.
//! 2. **Microcompaction** — drop or truncate low-value tool-result spans to reclaim headroom.
//! 3. **Summarisation** — call the LLM (or apply rule-based fallback) to produce a
//!    `[COMPACTED CONTEXT]` message representing earlier turns.
//! 4. **Rehydration** — replace the old prefix with the summary; session returns to Active.
//!
//! Compaction audit records (see [`compaction_audit`]) track every compaction event for
//! observability and regression testing.

pub mod cleanup;
pub mod compaction;
pub mod compaction_audit;
pub mod compaction_context;
pub mod orchestrator;
pub(crate) mod presenter;
pub mod store;
pub mod types;

pub use orchestrator::SessionOrchestrator;
pub use store::PostgresSessionStore;
pub use types::{
    ChatMessage, ChatMessagePart, ChatMessagePartInput, CompactionConfig, CompactionResult,
    MessagePartKind, MessageRole, Session, SessionConfig, SessionMetadata, SessionOwnerScope,
    SessionState, SessionTranscriptReadModel, TranscriptMessage, render_principal_owner_scope,
    render_tenant_owner_scope, render_tenant_principal_owner_scope,
};
