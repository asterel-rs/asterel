//! Agent subsystem: the core execution layer for `Asterel` turns.
//!
//! # Architecture
//!
//! The agent subsystem is organized into three responsibility layers:
//!
//! ## Turn execution pipeline (`loop_`)
//! The central orchestrator. Each user message passes through:
//! 1. **Pre-answer enrichment** — context recall, style guidance, augmentations.
//! 2. **Inference / tool loop** — the primary answer call plus iterative tool use.
//! 3. **Post-answer pipeline** — metacognitive logging, persona reflect/writeback,
//!    relationship update, embodied-state refresh, and memory persistence.
//!
//! ## Tool execution layer (`tool_loop`, `tool_execution`, `tool_types`)
//! `ToolLoop` drives the iterative tool-call cycle between the LLM and the
//! registered tool registry. Loop detection, rate-limiting, and approval
//! brokering all live here.
//!
//! ## Presentation and filtering (`presenter`, `response_style`, `result_filter`)
//! Post-inference text processing: response-mode classification, finalization,
//! audit, and caller-side filtering.
//!
//! ## Supporting modules
//! - `checkpoint` — mid-turn snapshot persistence for crash recovery.
//! - `session_control` — external signals (pause, abort) for supervised sessions.
//! - `turn_enrichment` — pre-turn prompt/context assembly plus post-turn
//!   relationship and memory writeback helpers shared by transport execution.
//! - `turn_executor` — the adapter that wires `TurnEnrichment` results into a
//!   full `TurnExecutionRequest` and hands off to the loop.

pub(crate) mod checkpoint;
pub mod loop_;
pub(crate) mod memory_excerpt;
pub(crate) mod naturalness_gate;
pub(crate) mod presenter;
pub(crate) mod response_audit;
mod response_finalize;
mod response_fix;
pub(crate) mod response_style;
pub(crate) mod result_filter;
pub(crate) mod session_control;
mod tool_execution;
pub mod tool_loop;
pub(crate) mod tool_protocol;
mod tool_types;
pub mod transcript;
pub mod turn_contract;
pub mod turn_enrichment;
pub mod turn_executor;

pub use loop_::{RunContext, RunRequest, run};
pub(crate) use response_finalize::{
    NaturalnessFinalizationContext, ResponseFinalizationRequest,
    finalize_response_contextual_with_context, naturalness_relationship_distance_from_state,
    naturalness_relationship_surface_from_contract,
};
pub use tool_loop::{
    AgentStateNotifier, LoopDetectionEvent, LoopDetectionKind, LoopDetectionSeverity,
    LoopStopReason, ToolCallRecord, ToolLoop, ToolLoopResult, ToolLoopRunParams,
    augment_prompt_with_trust_boundary,
};
pub use turn_contract::CompanionTurnContract;
pub use turn_enrichment::{
    PostTurnInput, PreTurnEnrichment, PreTurnInput, affect_intensity, enrich_pre_turn,
    run_post_turn_hooks,
};
pub use turn_executor::{
    TurnExecutionOutcome, TurnExecutionRequest, TurnExecutor, TurnHistoryAdapter, TurnRunAdapter,
    TurnTranscriptAdapter,
};
