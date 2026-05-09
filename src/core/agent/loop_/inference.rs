//! Post-turn inference pass: structured fact extraction from LLM output.
//!
//! # What this module does
//!
//! During the answer phase, the LLM may emit structured markers inside
//! its response:
//! - `INFERRED_CLAIM slot_key => value` — a new inferred fact about
//!   the current context (e.g. the user's programming language,
//!   active topic, timezone).
//! - `CONTRADICTION_EVENT slot_key => value` — an explicit signal that
//!   a previously stored fact has been invalidated.
//!
//! After the tool loop completes, `run_post_turn_inference_pass` scans
//! the raw assistant response for these markers, validates and
//! deduplicates them, then writes `MemoryInferenceEvent`s to the
//! memory backend.
//!
//! # Consistency checking
//!
//! Before writing new `InferredClaim` events, the module runs the
//! `SlotValueConsistencyService` to automatically detect and mark
//! contradictions with existing slot values.  This keeps the memory
//! store coherent without requiring the LLM to explicitly emit
//! `CONTRADICTION_EVENT` markers for every conflicting slot.
//!
//! # Provider selection
//!
//! This module does **not** call a provider.  It operates entirely on
//! the text already produced by the answer provider in the tool loop.
//! The `reflect_provider` (used in `reflect.rs`) is a separate call.

use std::sync::Arc;

use anyhow::{Context, Result};

use super::RuntimeMemoryWriteContext;
use crate::contracts::observability::{AutonomySignal, Observer};
use crate::core::agent::memory_excerpt::safe_memory_excerpt;
use crate::core::memory::consistency::{
    ConsistencyService, ContradictionDetector, SlotValueConsistencyService, SlotValueDetector,
};
use crate::core::memory::{
    Memory, MemoryInferenceEvent, MemoryLayer, MemoryProvenance, MemorySource, SourceKind,
};
use crate::security::writeback_guard::enforce_inference_write_policy;

/// Parse a `slot_key => value` payload from a single inference marker
/// line.  Returns `None` if the line is malformed, either component is
/// empty after trimming, or the slot key fails the validity check.
fn parse_inference_payload(line: &str) -> Option<(&str, String)> {
    let (slot_key, value) = line.split_once("=>")?;
    let slot_key = slot_key.trim();
    let value = value.trim();
    if slot_key.is_empty() || value.is_empty() {
        return None;
    }
    if !is_valid_slot_key(slot_key) {
        tracing::warn!(
            slot_key,
            "rejected inferred slot_key: invalid characters or length"
        );
        return None;
    }
    let value = safe_memory_excerpt(value, 360);
    if value.trim().is_empty() {
        return None;
    }
    Some((slot_key, value))
}

/// Return `true` if `slot_key` is safe to write to memory.
///
/// Keys are restricted to ASCII alphanumerics plus `.`, `_`, and `-`,
/// and capped at 128 characters.  This prevents injection of
/// arbitrary strings into the memory slot namespace by a rogue or
/// misaligned LLM response.
fn is_valid_slot_key(slot_key: &str) -> bool {
    slot_key.len() <= 128
        && slot_key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Scan `assistant_response` line-by-line and emit a
/// `MemoryInferenceEvent` for each recognized marker.
///
/// `INFERRED_CLAIM` events target the `Semantic` memory layer for
/// general context facts.  `CONTRADICTION_EVENT` events target
/// `Episodic` so they can be replayed during consistency reconciliation.
fn build_post_turn_inference_events(
    entity_id: &str,
    assistant_response: &str,
) -> Vec<MemoryInferenceEvent> {
    const INFERRED_PREFIX: &str = "INFERRED_CLAIM ";
    const CONTRADICTION_PREFIX: &str = "CONTRADICTION_EVENT ";

    assistant_response
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if let Some(payload) = line.strip_prefix(INFERRED_PREFIX) {
                let (slot_key, value) = parse_inference_payload(payload)?;
                return Some(
                    MemoryInferenceEvent::inferred_claim(entity_id, slot_key, value)
                        .with_layer(MemoryLayer::Semantic),
                );
            }
            if let Some(payload) = line.strip_prefix(CONTRADICTION_PREFIX) {
                let (slot_key, value) = parse_inference_payload(payload)?;
                return Some(
                    MemoryInferenceEvent::contradiction_marked(entity_id, slot_key, value)
                        .with_layer(MemoryLayer::Episodic),
                );
            }
            None
        })
        .collect()
}

/// Return the provenance reference string and source class for a
/// `MemoryInferenceEvent`.  Used to attach origin metadata to every
/// persisted memory event for audit and recall scoring.
fn inference_provenance_reference(event: &MemoryInferenceEvent) -> (&'static str, MemorySource) {
    match event {
        MemoryInferenceEvent::InferredClaim { .. } => {
            ("inference.post_turn.inferred_claim", MemorySource::Inferred)
        }
        MemoryInferenceEvent::ContradictionEvent { .. } => (
            "inference.post_turn.contradiction_event",
            MemorySource::System,
        ),
    }
}

/// Extract inferred claims and contradiction events from the
/// assistant response and persist them to memory.
///
/// # Errors
///
/// Returns an error if write-scope enforcement or memory persistence
/// fails.
pub(super) async fn run_post_turn_inference_pass(
    mem: &dyn Memory,
    write_context: &RuntimeMemoryWriteContext,
    assistant_response: &str,
    observer: &Arc<dyn Observer>,
) -> Result<()> {
    write_context.enforce_write_scope()?;
    let mut events =
        build_post_turn_inference_events(write_context.entity_id.as_str(), assistant_response);
    if events.is_empty() {
        return Ok(());
    }

    if events
        .iter()
        .any(|event| matches!(event, MemoryInferenceEvent::InferredClaim { .. }))
    {
        let detector: Arc<dyn ContradictionDetector> = Arc::new(SlotValueDetector::new());
        let consistency_service = SlotValueConsistencyService::new(detector);
        let contradiction_events = consistency_service
            .check_and_mark_contradictions(mem, write_context.entity_id.as_str(), &events)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "auto-contradiction detection failed, continuing without");
                Vec::new()
            });

        events.extend(contradiction_events.into_iter().map(|event| match event {
            MemoryInferenceEvent::ContradictionEvent { confidence, .. } => {
                event.with_confidence(confidence.get().min(0.85))
            }
            other @ MemoryInferenceEvent::InferredClaim { .. } => other,
        }));
    }

    for event in &events {
        if matches!(event, MemoryInferenceEvent::ContradictionEvent { .. }) {
            observer.emit_autonomy_signal(AutonomySignal::ContradictionDetected);
        }
    }

    for event in events {
        let (reference, source_class) = inference_provenance_reference(&event);
        let input = event
            .into_memory_event_input()
            .with_source_kind(SourceKind::Conversation)
            .with_source_ref(reference)
            .with_provenance(MemoryProvenance::source_reference(source_class, reference));
        enforce_inference_write_policy(&input).context("enforce inference write policy")?;
        mem.append_event(input)
            .await
            .context("append inferred memory event")?;
    }

    Ok(())
}
