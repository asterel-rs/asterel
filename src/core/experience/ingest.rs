//! Experience ingestion: persists `ExperienceAtom` records to the
//! procedural memory layer with system provenance.

use std::future::Future;
use std::pin::Pin;

use crate::contracts::experience::ExperienceAtom;
use crate::contracts::strings::data_model::{
    SOURCE_EXPERIENCE_ATOM_INGESTION, SOURCE_REF_EXPERIENCE_INGEST,
};
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryLayer, MemoryProvenance, MemorySource,
    PrivacyLevel, SourceKind,
};

/// Persist an `ExperienceAtom` to the procedural memory layer.
///
/// Creates an `experience.{kind}.{id}` slot with system provenance.
///
/// # Errors
///
/// Returns an error if serialisation or the memory append fails.
pub(crate) fn persist_experience_atom<'a>(
    mem: &'a dyn Memory,
    entity_id: &'a str,
    atom: &'a ExperienceAtom,
) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
    Box::pin(async move {
        let slot_key = format!("experience.{}.{}", atom.kind.kind_str(), atom.id);
        let value = serde_json::to_string(atom)
            .map_err(|e| anyhow::anyhow!("serialize experience atom: {e}"))?;

        let input = MemoryEventInput::new(
            entity_id,
            &slot_key,
            MemoryEventType::FactAdded,
            value,
            MemorySource::System,
            PrivacyLevel::Private,
        )
        .with_layer(MemoryLayer::Procedural)
        .with_confidence(atom.confidence.get())
        .with_importance(0.7)
        .with_source_kind(SourceKind::Manual)
        .with_source_ref(SOURCE_REF_EXPERIENCE_INGEST)
        .with_provenance(MemoryProvenance::source_reference(
            MemorySource::System,
            SOURCE_EXPERIENCE_ATOM_INGESTION,
        ));

        mem.append_event(input).await?;
        Ok(())
    })
}
