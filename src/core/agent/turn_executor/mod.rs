//! Turn execution orchestration.
//!
//! [`TurnExecutor`] wires together history loading, the tool loop,
//! response finalization, and transcript persistence into a single
//! `execute` call. Higher-level surfaces (gateway WebSocket, Discord,
//! REST API) each construct a [`TurnExecutionRequest`] and call `execute`;
//! the shared implementation drives everything beneath.
//!
//! Post-turn side-effects (relationship updates, contract-gated turn summaries,
//! and working-memory flush) are spawned as background tasks so they do not block
//! the response path.

use std::future::Future;
use std::sync::Arc;

#[cfg(test)]
use std::sync::Mutex;

use anyhow::Result;

use super::naturalness_gate::RelationshipDistance;
use super::response_finalize::{
    NaturalnessFinalizationContext, ResponseFinalizationRequest,
    finalize_response_contextual_with_context, finalize_response_with_context,
    naturalness_affect_from_text,
};
use super::response_style::{ResponseMode, classify_response_mode};
use super::tool_loop::{
    AgentStateNotifier, LoopStopReason, ToolLoop, ToolLoopResult, ToolLoopRunParams,
};
use super::turn_contract::CompanionTurnContract;
use super::turn_enrichment::{
    PostTurnInput, PreTurnInput, affect_intensity, enrich_pre_turn, flush_working_memory,
    materialize_working_memory, run_post_turn_hooks,
};
use crate::config::LoopDetectionConfig;
use crate::contracts::ids::{EntityId, PersonId, SessionId};
use crate::contracts::observability::{Observer, ObserverEvent};
use crate::core::affect::AffectReading;
use crate::core::agent::response_audit::{ContractMismatchReason, ResponseContract};
use crate::core::agent::transcript::{load_provider_history_async, persist_tool_loop_turn_async};
use crate::core::memory::{Memory, WorkingMemoryView};
use crate::core::persona::soul_core::SelfAmendmentCandidateSink;
use crate::core::providers::InferenceOpts;
#[cfg(test)]
use crate::core::providers::ProviderResult;
use crate::core::providers::response::{ContentBlock, ProviderMessage};
use crate::core::providers::streaming::StreamSink;
use crate::core::providers::traits::Provider;
use crate::core::sessions::SessionOrchestrator;
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::registry::ToolRegistry;

mod turn_executor_metrics;
mod turn_executor_pipeline;

pub use turn_executor_pipeline::*;

#[cfg(test)]
mod tests;
