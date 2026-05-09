//! Shared helpers for persisting persona state to memory.

use anyhow::Result;

use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryLayer, MemoryProvenance, MemorySource,
    PrivacyLevel, SourceKind,
};
use crate::security::writeback_guard::enforce_persona_long_term_write_policy;

/// Persist a persona slot with full policy enforcement.
///
/// Constructs a `MemoryEventInput` with `SourceKind::Manual`,
/// `MemoryProvenance`, and calls `enforce_persona_long_term_write_policy`.
///
/// # Errors
///
/// Returns an error if policy enforcement or memory append fails.
#[allow(clippy::too_many_arguments)] // Persona persistence requires all context params together
pub(crate) async fn persist_persona_slot(
    mem: &dyn Memory,
    entity_id: impl AsRef<str>,
    slot_key: impl AsRef<str>,
    event_type: MemoryEventType,
    payload: String,
    confidence: f64,
    importance: f64,
    source_ref: impl Into<String>,
    provenance_label: &str,
    occurred_at: Option<String>,
    person_id: &str,
) -> Result<()> {
    let slot_key = slot_key.as_ref().to_string();
    let mut input = MemoryEventInput::new(
        entity_id,
        &slot_key,
        event_type,
        payload,
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_confidence(confidence)
    .with_importance(importance)
    .with_layer(persona_slot_layer(&slot_key))
    .with_source_kind(SourceKind::Manual)
    .with_source_ref(source_ref)
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::System,
        provenance_label,
    ));
    if let Some(at) = occurred_at {
        input = input.with_occurred_at(at);
    }
    enforce_persona_long_term_write_policy(&input, person_id)?;
    mem.append_event(input).await?;
    Ok(())
}

fn persona_slot_layer(slot_key: &str) -> MemoryLayer {
    if slot_key.contains("/user_facts/")
        || slot_key.contains("/user_knowledge/")
        || slot_key.contains("/world_model/")
        || slot_key.contains("follow_up_queue")
        || slot_key.starts_with("inferred.")
    {
        MemoryLayer::Semantic
    } else {
        MemoryLayer::Identity
    }
}

#[cfg(test)]
mod tests {
    use super::persona_slot_layer;
    use crate::core::memory::MemoryLayer;

    #[test]
    fn persona_slot_layer_marks_identity_slots_as_identity() {
        assert_eq!(
            persona_slot_layer("persona/alice/style_profile/v1"),
            MemoryLayer::Identity
        );
        assert_eq!(
            persona_slot_layer("persona/alice/big_five/v1"),
            MemoryLayer::Identity
        );
    }

    #[test]
    fn persona_slot_layer_marks_user_context_slots_as_semantic() {
        assert_eq!(
            persona_slot_layer("persona/alice/user_facts/name"),
            MemoryLayer::Semantic
        );
        assert_eq!(
            persona_slot_layer("persona/alice/world_model/v1"),
            MemoryLayer::Semantic
        );
    }
}
