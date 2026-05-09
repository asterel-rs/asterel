//! Core agent execution domain.
//!
//! This module tree contains the provider abstraction, tool loop, persona,
//! memory backends, and session/state orchestration.

/// Agent loop and turn execution.
/// Affect recognition and style adaptation.
pub mod affect;
pub mod agent;
/// Conversation-level interactive commands (`/think`, `/new`, `/help`, etc.).
pub mod conversation_commands;
/// Evaluation helpers and deterministic utilities.
pub mod eval;
pub mod experience;
/// Memory ingestion, storage, and recall interfaces.
pub mod memory;
/// Persona identity and style modeling.
pub mod persona;
/// Model/provider abstraction and implementations.
pub mod providers;
/// Session persistence and compaction.
pub mod sessions;
/// Subagent runtime and coordination.
pub mod subagents;
#[cfg(feature = "taste")]
/// Optional taste-evaluation subsystem.
pub mod taste;
/// Tool definitions, registry, and execution middleware.
pub mod tools;
