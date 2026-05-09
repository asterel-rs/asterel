//! Taint tracking for tool execution outputs.
//!
//! Labels tool outputs with contamination markers that propagate
//! through the middleware pipeline, enabling downstream consumers
//! to reason about data provenance and trust levels.

pub mod label;
pub mod propagation;
