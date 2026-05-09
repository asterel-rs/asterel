//! Agent turn loop: orchestrates context building, inference,
//! session management, and post-turn reflection.
//!
//! # Turn lifecycle
//!
//! ```text
//! run() ──► run_session()
//!               │
//!               ▼
//!   execute_main_session_turn_with_metrics()        [session.rs]
//!               │  ┌─ verify/repair retry loop ─────────────┐
//!               ▼  ▼                                         │
//!   execute_main_session_turn_with_accounting()  [session_posturn.rs]
//!               │
//!       ┌───────┴────────────────────────────────────────┐
//!       │  1. build_pre_answer_enrichment()              │  [pre_answer_enrichment.rs]
//!       │     • memory recall / context contract         │
//!       │     • style profile, self-model shadow         │
//!       │     • augmentation blocks                      │
//!       │  2. execute_turn_with_tool_loop()              │  [session_posturn.rs]
//!       │     • ToolLoop (answer provider + tools)       │  [tool_loop/]
//!       │  3. run_post_answer_pipeline()                 │  [session_posturn.rs]
//!       │     a. run_metacognitive_logging_if_enabled()  │  [post_answer_handlers.rs]
//!       │     b. run_persona_reflect_if_enabled()        │  [post_answer_handlers.rs]
//!       │        └─► run_persona_reflect_writeback()     │  [reflect.rs]
//!       │     c. update_relationship_after_turn()        │
//!       │     d. update_turn_embodied_state_if_enabled() │  [post_answer_handlers.rs]
//!       │     e. save_response_and_consolidate()         │  [session_persistence.rs]
//!       └────────────────────────────────────────────────┘
//! ```
//!
//! The `verify_repair` module wraps step (2)+(3) in a retry loop that
//! backs off on transient provider failures before escalating.

pub(crate) mod augment;
pub(crate) mod context;
mod context_contract;
mod conversation_state;
mod inference;
mod post_answer_handlers;
mod pre_answer_enrichment;
mod reflect;
mod run;
mod self_task_queue;
mod session;
mod session_persistence;
mod session_posturn;
mod types;
mod verify_repair;

// ── Public API re-exports ────────────────────────────────────────
#[cfg(test)]
use std::sync::Arc;

pub use context::{
    ContextBudget, ContextRuntimeMetadata, build_context_contract_for_integration,
    build_context_contract_with_runtime_metadata_for_integration, build_context_for_integration,
    context_budget_for_model, render_provider_history_block, seed_context_contract,
};
pub use context_contract::{
    ContextFragment, ContextFragmentKind, ContextFragmentTrust, ContextUpdateMode,
    TurnContextContract, TurnContextUpdate,
};
pub use run::{RunContext, RunRequest, run};
// ── Test-only re-exports (visible to tests via super::*) ─────────
#[cfg(test)]
use session::TurnExecutionSettings;
#[cfg(test)]
use session::run_main_turn;
pub use session::{run_main_turn_policy_test, run_main_turn_test};
#[cfg(test)]
use session_posturn::execute_main_session_turn_with_accounting;
#[cfg(test)]
use types::MainSessionTurnParams;
#[cfg(test)]
use types::PERSONA_PER_TURN_CALL_BUDGET;
pub(super) use types::RuntimeMemoryWriteContext;
pub use types::TurnParams;
#[cfg(test)]
use types::TurnPipelineContext;

// ── Test-only crate imports ──────────────────────────────────────
#[cfg(test)]
use crate::config::Config;
#[cfg(test)]
use crate::contracts::observability::NoopObserver;
#[cfg(test)]
use crate::contracts::observability::Observer;
#[cfg(test)]
use crate::core::memory::{Memory, MemorySource};
#[cfg(test)]
use crate::core::providers::Provider;

#[cfg(test)]
mod tests;
