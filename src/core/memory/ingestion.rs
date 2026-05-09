//! Memory ingestion pipeline: signal envelope construction and
//! write-path orchestration with deduplication and policy guards.

mod error;
mod pipeline;
mod signal_envelope;

#[cfg(test)]
use std::sync::Arc;

pub use error::{IngestionError, IngestionPipelineResult};
#[cfg(test)]
use pipeline::semantic_dedup_key;
pub use pipeline::{DefaultIngestPipeline, IngestionPipeline, IngestionResult};
pub use signal_envelope::SignalEnvelope;

#[cfg(test)]
use crate::core::memory::memory_types::{PrivacyLevel, SignalTier, SourceKind};
#[cfg(test)]
use crate::core::memory::traits::Memory;

#[cfg(test)]
mod tests;
