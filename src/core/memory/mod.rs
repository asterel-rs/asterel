//! Memory subsystem facade.
//!
//! This is the companion's long-term knowledge store. It exposes:
//!
//! - **Backend selection**: `Postgres` (production) or `Markdown` (local/dev),
//!   constructed via [`factory::create_memory`].
//! - **Core trait**: the [`Memory`] supertrait composes [`MemoryReader`],
//!   [`MemoryWriter`], and [`MemoryGovernance`] (ISP).
//! - **Recall pipeline**: backend-ranked slot-keyed retrieval. `Postgres`
//!   combines vector/keyword search, MMR diversity reranking, and optional
//!   GraphRAG PPR boosting; `Markdown` uses its local projection keyword path.
//! - **Ingestion pipeline**: signal envelope normalization, dedup, and
//!   write-policy enforcement before appending events.
//! - **Consolidation**: session-to-semantic distillation through the exported
//!   rule-based post-turn path. Optional LLM-assisted consolidation code exists
//!   separately but is not wired into the live post-turn facade.
//! - **Hygiene**: scheduled archival, TTL pruning, and sleep-phase
//!   consolidation for the `Postgres` backend.
//! - **Governance**: optional in-process access logging, archive policy types,
//!   and backend-specific forget semantics (soft / hard / tombstone).

/// Per-backend capability matrix for forget semantics.
mod capability;
pub mod checkpoint;
/// Token and payload chunking helpers for large-value ingestion.
pub mod chunker;
/// Shared enum-to-string codec for memory backends (layer, tier, source, privacy).
pub mod codec;
/// Consistency checking: contradiction detection and repair.
pub mod consistency;
/// Session-to-semantic consolidation jobs and scheduling contracts.
pub mod consolidation;
/// Embedding provider trait and HTTP clients (`OpenAI`, `Cohere`, `Voyage`, etc.).
pub mod embeddings;
/// Emotion inference for memory events (valence, arousal, label).
pub mod emotional_context;
/// Backend factory: constructs the live `Memory` instance from config.
mod factory;
/// Session-scoped access logging, status lifecycle, and archive policy (WP-I3).
pub(crate) mod governance;
/// GraphRAG: knowledge-graph grounding, PPR activation, entity resolution.
pub mod graphrag;
/// Memory hygiene scheduler: archival, TTL pruning, and sleep-phase consolidation.
pub mod hygiene;
/// Entity and slot key sanitization (trim, char normalization).
pub(crate) mod identifier;
/// Context-influence rendering: surfaces relevant memories into the prompt window.
pub mod influence;
/// Ingestion pipeline: signal envelope normalization, dedup, and write-policy guards.
pub mod ingestion;
/// `Markdown`-backed memory: append-only flat-file store for local/dev use.
pub mod markdown;
/// Canonical memory domain types shared across backends.
pub(crate) mod memory_types;
#[cfg(feature = "postgres")]
/// `PostgreSQL`-backed memory: vector search, belief slots, graph projection.
pub mod postgres;
/// Generic recall-and-deserialize helper (typed slot-prefix queries).
pub(crate) mod recall_helpers;
/// Post-retrieval reranking: MMR diversity, LLM reranking, PPR blending.
pub mod reranking;
/// Tool execution event builder (wraps tool results as memory events).
pub mod tool_event;
/// Core memory trait: `Memory` supertrait + `MemoryReader` / `MemoryWriter` / `MemoryGovernance`.
pub(crate) mod traits;
/// Vector math: cosine similarity, byte serialization, hybrid merge (weighted + RRF).
pub mod vector;
/// Session-scoped working memory view: materialized from recall, evicted by importance.
pub mod working;

pub use crate::contracts::memory_error::{MemoryError, MemoryResult};
/// Backend capability matrix helpers.
pub use capability::{
    backend_capability_matrix, capability_matrix_for_backend, capability_matrix_for_memory,
    ensure_forget_mode_supported,
};
pub use checkpoint::{CheckpointRegistry, MemoryCheckpoint, RollbackReason, RollbackResult};
/// Rule-based consolidation contracts and one-shot execution helpers.
pub use consolidation::{
    CONSOLIDATION_SLOT_KEY, ConsolidationDisposition, ConsolidationInput, ConsolidationOutput,
    ConsolidationWorkerPhase, ConsolidationWorkerStatus, consolidation_worker_statuses,
    enqueue_consolidation_task, run_consolidation, schedule_durable_memory_consolidation,
};
pub use emotional_context::{EmotionalContext, infer_emotion_from_text};
/// Factory helpers for memory backend construction and event persistence.
pub use factory::{create_memory, persist_inference_events};
/// Ingestion pipeline types.
pub use ingestion::{
    DefaultIngestPipeline, IngestionError, IngestionPipeline, IngestionPipelineResult,
    IngestionResult, SignalEnvelope,
};
/// Markdown memory backend.
pub use markdown::MarkdownMemory;
/// Core memory domain types.
pub use memory_types::{
    BeliefSlot, CapabilitySupport, ForgetArtifact, ForgetArtifactCheck, ForgetMode,
    ForgetObservation, ForgetOutcome, ForgetRequirement, ForgetStatus, GraphEdge, GraphEntity,
    GraphEntityType, GraphRelationType, MemoryCapMatrix, MemoryCategory, MemoryEntry, MemoryEvent,
    MemoryEventInput, MemoryEventType, MemoryInferenceEvent, MemoryLayer, MemoryProvenance,
    MemoryRecallEntry, MemorySource, NodeTier, PrivacyLevel, RecallQuery, SignalTier, SourceKind,
};
#[cfg(feature = "postgres")]
/// `PostgreSQL` memory backend.
pub use postgres::PostgresMemory;
pub use tool_event::build_tool_execution_event;
/// Core memory contract and sub-traits (ISP: Interface Segregation).
pub use traits::{Memory, MemoryGovernance, MemoryReader, MemoryWriter};
pub use working::{WorkingMemoryItem, WorkingMemorySource, WorkingMemoryView};
