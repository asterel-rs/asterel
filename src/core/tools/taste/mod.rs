//! Taste evaluation tools — aesthetic quality assessment and preference learning.
//!
//! # Overview
//!
//! The taste subsystem exposes two tools that let the agent interact with the
//! `TasteEngine` to score and learn from artifacts:
//!
//! | Tool | Purpose |
//! |------|---------|
//! | [`evaluate::TasteEvaluateTool`] | Score a single artifact on taste axes and produce suggestions. |
//! | [`compare::TasteCompareTool`] | Record a pairwise preference comparison to refine the taste model. |
//!
//! # Taste engine integration
//!
//! Both tools accept an `Arc<dyn TasteEngine>` at construction time. The
//! engine is responsible for evaluation logic and comparison storage; tools
//! are pure dispatchers that parse arguments, call the engine, and format
//! the response. The engine is injected rather than hard-coded so test
//! harnesses can substitute mock engines without network or disk access.
//!
//! # Artifact kinds
//!
//! The taste system currently supports two artifact kinds:
//! * `text` — prose or structured text with an optional `TextFormat` hint
//!   (`plain`, `markdown`, or `html`).
//! * `ui` — a description of a user interface component or layout.

pub mod compare;
pub mod evaluate;

pub use compare::TasteCompareTool;
pub use evaluate::TasteEvaluateTool;
