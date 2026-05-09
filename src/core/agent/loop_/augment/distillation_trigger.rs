//! Distillation trigger: checks whether a distillation round is
//! due and, if so, runs the experience-to-principle pipeline.

use anyhow::Result;

use crate::config::PersonaConfig;
use crate::contracts::memory::MemoryLayer;
use crate::contracts::strings::data_model::ENTITY_PREFIX_PERSON;
use crate::core::experience::distill::run_distillation;
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryProvenance, MemorySource, PrivacyLevel,
    SourceKind,
};

const DISTILLATION_TURN_COUNT_SLOT: &str = "persona.distillation_turn_count";

/// Increment the turn counter and trigger distillation when the
/// configured interval is reached.
pub(super) async fn maybe_run_distillation(
    mem: &dyn Memory,
    entity_id: &str,
    persona_config: &PersonaConfig,
) -> Result<()> {
    if !persona_config.enable_experience_distillation {
        return Ok(());
    }

    let count = load_turn_count(mem, entity_id).await;
    let next = count + 1;

    if next < persona_config.distillation_interval_turns {
        persist_turn_count(mem, entity_id, next).await;
        return Ok(());
    }

    // Reset counter and run distillation.
    persist_turn_count(mem, entity_id, 0).await;

    let experiences =
        crate::core::experience::retrieve_relevant_experiences(mem, entity_id, "distillation", 100)
            .await
            .unwrap_or_default();

    if experiences.is_empty() {
        return Ok(());
    }

    match run_distillation(mem, entity_id, &experiences).await {
        Ok(principles) => {
            tracing::info!(
                count = principles.len(),
                "distillation produced new principles"
            );

            // Rebuild narrative after distillation if enabled.
            if persona_config.enable_narrative_self {
                rebuild_narrative(mem, entity_id, &experiences, &principles).await;
            }
        }
        Err(error) => {
            tracing::warn!(%error, "experience distillation failed");
        }
    }

    Ok(())
}

async fn rebuild_narrative(
    mem: &dyn Memory,
    entity_id: &str,
    experiences: &[crate::core::experience::ExperienceAtom],
    principles: &[crate::core::experience::distill_types::Principle],
) {
    use crate::core::persona::milestone::{
        detect_milestones, load_milestone_state, persist_milestone_state,
    };
    use crate::core::persona::narrative::{NarrativeBuilder, persist_narrative};

    let person_id = entity_id
        .strip_prefix(ENTITY_PREFIX_PERSON)
        .unwrap_or(entity_id);
    let relationship = crate::core::persona::load_relationship(mem, person_id)
        .await
        .ok()
        .flatten();

    let mut milestone_state = load_milestone_state(mem, person_id)
        .await
        .unwrap_or_default();
    let new_milestones = detect_milestones(
        &mut milestone_state,
        experiences,
        principles,
        relationship.as_ref(),
    );
    if let Err(error) = persist_milestone_state(mem, person_id, &milestone_state).await {
        tracing::warn!(%error, "milestone state persistence after distillation failed");
    } else if !new_milestones.is_empty() {
        tracing::info!(
            count = new_milestones.len(),
            "milestones detected during narrative rebuild"
        );
    }

    let narrative = NarrativeBuilder::build_with_milestones(
        experiences,
        principles,
        relationship.as_ref(),
        Some(&milestone_state),
    );
    if let Err(error) = persist_narrative(mem, person_id, &narrative).await {
        tracing::warn!(%error, "narrative persistence after distillation failed");
    } else {
        tracing::info!("narrative rebuilt after distillation");
    }
}

async fn load_turn_count(mem: &dyn Memory, entity_id: &str) -> usize {
    mem.resolve_slot(entity_id, DISTILLATION_TURN_COUNT_SLOT)
        .await
        .ok()
        .flatten()
        .map_or(0, |slot| slot.value.parse::<usize>().unwrap_or(0))
}

async fn persist_turn_count(mem: &dyn Memory, entity_id: &str, count: usize) {
    let input = MemoryEventInput::new(
        entity_id,
        DISTILLATION_TURN_COUNT_SLOT,
        MemoryEventType::FactUpdated,
        count.to_string(),
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_layer(MemoryLayer::Working)
    .with_confidence(1.0)
    .with_importance(0.1)
    .with_source_kind(SourceKind::Conversation)
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::System,
        "distillation.turn_counter",
    ));

    if let Err(error) = mem.append_event(input).await {
        tracing::warn!(%error, "failed to persist distillation turn count");
    }
}
