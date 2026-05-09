//! Consistency service that detects and marks contradictions in memory.
//!
//! Orchestrates contradiction detection across inferred events, tagging
//! contradicted claims before they are persisted.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::detector::ContradictionDetector;
use crate::core::memory::{Memory, MemoryInferenceEvent};

/// Service that checks inferred events for contradictions and emits
/// contradiction-marking events.
pub(crate) trait ConsistencyService: Send + Sync {
    /// Detect contradictions and return events marking them.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying memory query fails.
    fn check_and_mark_contradictions<'a>(
        &'a self,
        mem: &'a dyn Memory,
        entity_id: &'a str,
        inferred_events: &'a [MemoryInferenceEvent],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<MemoryInferenceEvent>>> + Send + 'a>>;
}

/// Consistency service backed by slot-value contradiction detection.
pub(crate) struct SlotValueConsistencyService {
    detector: Arc<dyn ContradictionDetector>,
}

impl SlotValueConsistencyService {
    /// Create a consistency service wrapping the given detector.
    pub(crate) fn new(detector: Arc<dyn ContradictionDetector>) -> Self {
        Self { detector }
    }
}

impl ConsistencyService for SlotValueConsistencyService {
    fn check_and_mark_contradictions<'a>(
        &'a self,
        mem: &'a dyn Memory,
        entity_id: &'a str,
        inferred_events: &'a [MemoryInferenceEvent],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<MemoryInferenceEvent>>> + Send + 'a>> {
        Box::pin(async move {
            let findings = self
                .detector
                .detect_contradictions(mem, entity_id, inferred_events)
                .await?;

            let events = findings
                .into_iter()
                .map(|finding| {
                    MemoryInferenceEvent::contradiction_marked(
                        entity_id,
                        finding.slot_key.as_str(),
                        &finding.new_value,
                    )
                    .with_confidence(finding.contradiction_confidence.get())
                })
                .collect();

            Ok(events)
        })
    }
}
