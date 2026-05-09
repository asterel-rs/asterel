//! Input normalization for memory event ingress.
//!
//! Validates and clamps entity IDs, slot keys, confidence/importance
//! scores, and provenance before events enter the write path.

use super::{MemoryEventInput, MemoryProvenance, MemorySource};
use crate::contracts::memory_error::{MemoryError, MemoryResult};
use crate::contracts::scores::{Confidence, Importance};

/// Validate and normalize all fields of a `MemoryEventInput` before
/// it enters the write path.
///
/// # Errors
///
/// Returns an error if entity ID, slot key, confidence, importance,
/// or provenance fails validation.
pub(super) fn normalize_memory_event_input(input: &mut MemoryEventInput) -> MemoryResult<()> {
    input.entity_id =
        crate::contracts::ids::EntityId::new(normalize_entity_id(input.entity_id.as_str())?);
    input.slot_key =
        crate::contracts::ids::SlotKey::new(normalize_slot_key(input.slot_key.as_str())?);
    input.confidence = normalize_confidence(input.confidence, "memory_event_input.confidence")?;
    input.importance = normalize_importance(input.importance, "memory_event_input.importance")?;
    if let Some(provenance) = &input.provenance {
        validate_provenance(input.source, provenance)?;
    }
    Ok(())
}

fn normalize_confidence(score: Confidence, field: &str) -> MemoryResult<Confidence> {
    if !score.get().is_finite() {
        return Err(MemoryError::validation(format!("{field} must be finite")));
    }
    Ok(Confidence::new(score.get()))
}

fn normalize_importance(score: Importance, field: &str) -> MemoryResult<Importance> {
    if !score.get().is_finite() {
        return Err(MemoryError::validation(format!("{field} must be finite")));
    }
    Ok(Importance::new(score.get()))
}

fn normalize_entity_id(raw: &str) -> MemoryResult<String> {
    let normalized = normalize_identifier(raw, false);
    if normalized.is_empty() {
        return Err(MemoryError::validation(
            "memory_event_input.entity_id must not be empty",
        ));
    }
    if normalized.len() > 128 {
        return Err(MemoryError::validation(
            "memory_event_input.entity_id must be <= 128 chars",
        ));
    }
    Ok(normalized)
}

fn normalize_slot_key(raw: &str) -> MemoryResult<String> {
    let normalized = normalize_identifier(raw, true);
    if normalized.is_empty() {
        return Err(MemoryError::validation(
            "memory_event_input.slot_key must not be empty",
        ));
    }
    if normalized.len() > 256 {
        return Err(MemoryError::validation(
            "memory_event_input.slot_key must be <= 256 chars",
        ));
    }
    if !is_valid_slot_key_pattern(&normalized) {
        return Err(MemoryError::validation(
            "memory_event_input.slot_key must match taxonomy pattern",
        ));
    }
    Ok(normalized)
}

fn is_valid_slot_key_pattern(slot_key: &str) -> bool {
    let mut chars = slot_key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphanumeric() {
        return false;
    }

    chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ':' | '/'))
}

use crate::core::memory::identifier::normalize_identifier;

fn validate_provenance(source: MemorySource, provenance: &MemoryProvenance) -> MemoryResult<()> {
    if provenance.source_class != source {
        return Err(MemoryError::validation(
            "memory_event_input.provenance.source_class must match memory_event_input.source",
        ));
    }

    if provenance.reference.trim().is_empty() {
        return Err(MemoryError::validation(
            "memory_event_input.provenance.reference must not be empty",
        ));
    }

    if provenance.reference.len() > 256 {
        return Err(MemoryError::validation(
            "memory_event_input.provenance.reference must be <= 256 chars",
        ));
    }

    if let Some(uri) = &provenance.evidence_uri
        && uri.trim().is_empty()
    {
        return Err(MemoryError::validation(
            "memory_event_input.provenance.evidence_uri must not be empty",
        ));
    }

    Ok(())
}
