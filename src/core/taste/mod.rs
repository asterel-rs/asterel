//! Taste engine module — LLM-based aesthetic evaluation for text and UI artifacts.
//!
//! The taste engine provides structured critique of artifacts across three aesthetic axes:
//! coherence, hierarchy, and intentionality. It supports pair comparisons for preference learning
//! and stores ratings in a persistent backend.

pub(crate) mod adapter;
pub(crate) mod critic;
pub mod engine;
pub(crate) mod influence;
pub(crate) mod learner;
pub(crate) mod modes;
pub(crate) mod presenter;
pub(crate) mod store;
pub mod types;
pub(crate) mod value_profile;
pub(crate) mod value_signals;

pub use engine::{TasteEngine, create_taste_engine};
pub use types::*;
