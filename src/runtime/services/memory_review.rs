use anyhow::{Result, bail};

use crate::contracts::ids::{EntityId, SlotKey};
use crate::core::memory::{
    ForgetMode, Memory, MemoryEventInput, MemoryEventType, MemoryProvenance, MemorySource,
    SourceKind,
};
use crate::runtime::diagnostics::control_plane_read_models::{
    MemoryConsolidationStatusReadModel, MemoryCorrectionReadModel, MemoryEntitySummaryReadModel,
    MemoryExposureStatusReadModel, MemorySlotProvenanceReadModel, MemorySlotSummaryReadModel,
    build_memory_consolidation_status_read_model, build_memory_correction_read_model,
    build_memory_entity_list_read_model, build_memory_exposure_status_read_model,
    build_memory_slot_list_read_model,
};
use crate::utils::text::truncate_ellipsis;

/// Build the admin memory-entity inventory from the active memory backend.
///
/// # Errors
/// Returns an error when the backend cannot enumerate entities or slots.
pub async fn list_admin_memory_entities(
    memory: &dyn Memory,
) -> Result<crate::runtime::diagnostics::control_plane_read_models::MemoryEntityListReadModel> {
    let entities = memory.list_entities().await?;
    let mut items = Vec::with_capacity(entities.len());
    for entity_id in entities {
        let slot_count = memory.list_slots(&entity_id).await?.len();
        items.push(MemoryEntitySummaryReadModel {
            entity_id: EntityId::new(entity_id),
            slot_count,
        });
    }
    Ok(build_memory_entity_list_read_model(
        memory.name().to_string(),
        items,
    ))
}

/// Load the latest known background memory-consolidation worker statuses.
#[must_use]
pub fn load_admin_memory_consolidation_statuses() -> MemoryConsolidationStatusReadModel {
    build_memory_consolidation_status_read_model(
        crate::core::memory::consolidation_worker_statuses(),
    )
}

/// Load process-local grounding exposure diagnostics.
#[must_use]
pub fn load_admin_memory_exposure_status() -> MemoryExposureStatusReadModel {
    build_memory_exposure_status_read_model(
        &crate::core::memory::influence::grounding_exposure_monitor_snapshot(),
    )
}

/// Load slot summaries and provenance for a specific entity.
///
/// # Errors
/// Returns an error when the backend cannot enumerate slots or provenance.
pub async fn load_admin_memory_slots(
    memory: &dyn Memory,
    entity_id: &str,
) -> Result<crate::runtime::diagnostics::control_plane_read_models::MemorySlotListReadModel> {
    let slots = memory.list_slots(entity_id).await?;
    let event_count = memory.count_events(Some(entity_id)).await?;
    let mut items = Vec::with_capacity(slots.len());
    for slot in slots {
        let provenance = memory
            .slot_provenance(entity_id, slot.slot_key.as_str())
            .await?
            .map(|provenance| MemorySlotProvenanceReadModel {
                source_class: format!("{:?}", provenance.source_class).to_ascii_lowercase(),
                reference: provenance.reference,
                evidence_uri: provenance.evidence_uri,
            });
        items.push(MemorySlotSummaryReadModel {
            slot_key: slot.slot_key,
            value: slot.value,
            source: format!("{:?}", slot.source).to_ascii_lowercase(),
            confidence: slot.confidence.get(),
            importance: slot.importance.get(),
            privacy_level: format!("{:?}", slot.privacy_level).to_ascii_lowercase(),
            updated_at: slot.updated_at,
            provenance,
        });
    }
    Ok(build_memory_slot_list_read_model(
        EntityId::new(entity_id),
        event_count,
        items,
    ))
}

/// Apply an operator memory correction after confirming the current slot value.
///
/// # Errors
/// Returns an error when the slot is missing, the old value no longer matches,
/// or the corrected event cannot be persisted.
pub async fn correct_admin_memory_slot(
    memory: &dyn Memory,
    principal: &str,
    entity_id: &str,
    slot_key: &str,
    old_value: &str,
    new_value: &str,
    reason: &str,
) -> Result<MemoryCorrectionReadModel> {
    let current_slot = memory
        .resolve_slot(entity_id, slot_key)
        .await?
        .ok_or_else(|| anyhow::anyhow!("slot not found"))?;
    if !normalized_equals(&current_slot.value, old_value) {
        bail!("current slot value no longer matches old_value");
    }

    let prior_provenance = memory.slot_provenance(entity_id, slot_key).await?;
    let mut provenance = MemoryProvenance::source_reference(
        MemorySource::System,
        truncate_ellipsis(
            &match &prior_provenance {
                Some(previous) => format!(
                    "admin.memory.correct:{reason} | prior={} | prior_source={:?}",
                    previous.reference, previous.source_class
                ),
                None => format!("admin.memory.correct:{reason}"),
            },
            256,
        ),
    );
    if let Some(previous) = &prior_provenance
        && let Some(uri) = &previous.evidence_uri
    {
        provenance = provenance.with_evidence_uri(uri.clone());
    }

    let input = MemoryEventInput::new(
        entity_id,
        slot_key,
        MemoryEventType::FactUpdated,
        new_value,
        MemorySource::System,
        current_slot.privacy_level,
    )
    .with_source_kind(SourceKind::Manual)
    .with_source_ref(format!("admin.memory.correct:{principal}"))
    .with_provenance(provenance);

    let event = memory.append_event(input).await?;
    Ok(build_memory_correction_read_model(
        EntityId::new(entity_id),
        SlotKey::new(slot_key),
        event.event_id,
    ))
}

/// Apply an operator forget action for a slot.
///
/// # Errors
/// Returns an error when the forget operation fails in the memory backend.
pub async fn forget_admin_memory_slot(
    memory: &dyn Memory,
    principal: &str,
    entity_id: &str,
    slot_key: &str,
    mode: ForgetMode,
    reason: &str,
) -> Result<crate::core::memory::ForgetOutcome> {
    let full_reason = format!("{reason} (operator={principal})");
    memory
        .forget_slot(entity_id, slot_key, mode, &full_reason)
        .await
        .map_err(Into::into)
}

fn normalized_equals(lhs: &str, rhs: &str) -> bool {
    lhs == rhs
}

#[cfg(test)]
mod tests {
    use super::{correct_admin_memory_slot, forget_admin_memory_slot, normalized_equals};
    use crate::contracts::ids::EventId;
    use crate::core::memory::{
        BeliefSlot, ForgetArtifact, ForgetArtifactCheck, ForgetMode, ForgetObservation,
        ForgetOutcome, ForgetRequirement, ForgetStatus, MarkdownMemory, MemoryEvent,
        MemoryEventInput, MemoryEventType, MemoryGovernance, MemoryReader, MemoryRecallEntry,
        MemorySource, MemoryWriter, PrivacyLevel, RecallQuery,
    };
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;

    #[test]
    fn normalized_equals_requires_exact_match_after_trim() {
        assert!(normalized_equals("Haru", "Haru"));
        assert!(!normalized_equals("Haru likes tea", "haru"));
        assert!(!normalized_equals("  Haru  ", "Haru"));
    }

    async fn seed_slot(memory: &MarkdownMemory, entity_id: &str, slot_key: &str, value: &str) {
        memory
            .append_event(
                MemoryEventInput::new(
                    entity_id,
                    slot_key,
                    MemoryEventType::FactAdded,
                    value,
                    MemorySource::ExplicitUser,
                    PrivacyLevel::Private,
                )
                .with_confidence(0.9)
                .with_importance(0.6),
            )
            .await
            .expect("seed memory slot");
    }

    #[tokio::test]
    async fn admin_memory_correction_is_visible_to_next_read() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let memory = MarkdownMemory::new(temp.path());
        seed_slot(&memory, "user-1", "profile.nickname", "old nick").await;

        let correction = correct_admin_memory_slot(
            &memory,
            "operator",
            "user-1",
            "profile.nickname",
            "old nick",
            "new nick",
            "user corrected nickname",
        )
        .await
        .expect("correction should persist");

        assert_eq!(correction.entity_id.as_str(), "user-1");
        assert_eq!(correction.slot_key.as_str(), "profile.nickname");

        let slot = memory
            .resolve_slot("user-1", "profile.nickname")
            .await
            .expect("resolve corrected slot")
            .expect("corrected slot should exist");
        assert_eq!(slot.value, "new nick");

        let recall = memory
            .recall_scoped(RecallQuery::new("user-1", "new nick", 10))
            .await
            .expect("recall corrected slot");
        assert!(recall.iter().any(|item| {
            item.slot_key.as_str() == "profile.nickname" && item.value == "new nick"
        }));
    }

    #[tokio::test]
    async fn admin_memory_forget_is_visible_to_next_read_and_recall() {
        let memory = InMemoryReviewMemory::default();
        memory
            .append_event(MemoryEventInput::new(
                "user-1",
                "profile.disliked_topic",
                MemoryEventType::FactAdded,
                "old rumor",
                MemorySource::ExplicitUser,
                PrivacyLevel::Private,
            ))
            .await
            .expect("seed memory slot");

        let outcome = forget_admin_memory_slot(
            &memory,
            "operator",
            "user-1",
            "profile.disliked_topic",
            ForgetMode::Soft,
            "user asked to forget",
        )
        .await
        .expect("forget should apply");

        assert!(outcome.was_applied);
        assert!(
            memory
                .resolve_slot("user-1", "profile.disliked_topic")
                .await
                .expect("resolve forgotten slot")
                .is_none()
        );
        let recall = memory
            .recall_scoped(RecallQuery::new("user-1", "old rumor", 10))
            .await
            .expect("recall after forget");
        assert!(recall.is_empty());
    }

    #[derive(Default)]
    struct InMemoryReviewMemory {
        slots: Mutex<BTreeMap<(String, String), MemoryEventInput>>,
    }

    impl MemoryWriter for InMemoryReviewMemory {
        fn append_event(
            &self,
            input: MemoryEventInput,
        ) -> Pin<
            Box<
                dyn Future<Output = crate::contracts::memory_error::MemoryResult<MemoryEvent>>
                    + Send
                    + '_,
            >,
        > {
            Box::pin(async move {
                self.slots
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(
                        (input.entity_id.to_string(), input.slot_key.to_string()),
                        input.clone(),
                    );
                Ok(MemoryEvent {
                    event_id: EventId::new("test-event"),
                    entity_id: input.entity_id,
                    slot_key: input.slot_key,
                    event_type: input.event_type,
                    value: input.value,
                    source: input.source,
                    confidence: input.confidence,
                    importance: input.importance,
                    provenance: input.provenance,
                    privacy_level: input.privacy_level,
                    occurred_at: input.occurred_at,
                    ingested_at: chrono::Utc::now().to_rfc3339(),
                })
            })
        }
    }

    impl MemoryReader for InMemoryReviewMemory {
        fn recall_scoped(
            &self,
            query: RecallQuery,
        ) -> Pin<
            Box<
                dyn Future<
                        Output = crate::contracts::memory_error::MemoryResult<
                            Vec<MemoryRecallEntry>,
                        >,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async move {
                let slots = self
                    .slots
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                Ok(slots
                    .iter()
                    .filter(|((entity_id, _), input)| {
                        entity_id == query.entity_id.as_str()
                            && (query.query.is_empty() || input.value.contains(&query.query))
                    })
                    .take(query.limit)
                    .map(|((entity_id, slot_key), input)| MemoryRecallEntry {
                        entity_id: crate::contracts::ids::EntityId::new(entity_id),
                        slot_key: crate::contracts::ids::SlotKey::new(slot_key),
                        value: input.value.clone(),
                        source: input.source,
                        confidence: input.confidence,
                        importance: input.importance,
                        privacy_level: input.privacy_level.clone(),
                        score: 1.0,
                        occurred_at: input.occurred_at.clone(),
                    })
                    .collect())
            })
        }

        fn resolve_slot<'a>(
            &'a self,
            entity_id: &'a str,
            slot_key: &'a str,
        ) -> Pin<
            Box<
                dyn Future<
                        Output = crate::contracts::memory_error::MemoryResult<Option<BeliefSlot>>,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                let slots = self
                    .slots
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                Ok(slots
                    .get(&(entity_id.to_string(), slot_key.to_string()))
                    .map(|input| BeliefSlot {
                        entity_id: input.entity_id.clone(),
                        slot_key: input.slot_key.clone(),
                        value: input.value.clone(),
                        source: input.source,
                        confidence: input.confidence,
                        importance: input.importance,
                        privacy_level: input.privacy_level.clone(),
                        updated_at: input.occurred_at.clone(),
                    }))
            })
        }
    }

    impl MemoryGovernance for InMemoryReviewMemory {
        fn name(&self) -> &str {
            "in-memory-review-test"
        }

        fn health_check(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
            Box::pin(async { true })
        }

        fn forget_slot<'a>(
            &'a self,
            entity_id: &'a str,
            slot_key: &'a str,
            mode: ForgetMode,
            _reason: &'a str,
        ) -> Pin<
            Box<
                dyn Future<Output = crate::contracts::memory_error::MemoryResult<ForgetOutcome>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                let applied = self
                    .slots
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&(entity_id.to_string(), slot_key.to_string()))
                    .is_some();
                Ok(ForgetOutcome {
                    entity_id: crate::contracts::ids::EntityId::new(entity_id),
                    slot_key: crate::contracts::ids::SlotKey::new(slot_key),
                    mode,
                    was_applied: applied,
                    is_complete: applied,
                    is_degraded: false,
                    status: if applied {
                        ForgetStatus::Complete
                    } else {
                        ForgetStatus::NotApplied
                    },
                    artifact_checks: vec![
                        ForgetArtifactCheck::new(
                            ForgetArtifact::Slot,
                            ForgetRequirement::MustBeNonRetrievable,
                            ForgetObservation::Absent,
                        ),
                        ForgetArtifactCheck::new(
                            ForgetArtifact::Ledger,
                            ForgetRequirement::MustExist,
                            ForgetObservation::PresentNonRetrievable,
                        ),
                    ],
                })
            })
        }

        fn count_events<'a>(
            &'a self,
            entity_id: Option<&'a str>,
        ) -> Pin<
            Box<
                dyn Future<Output = crate::contracts::memory_error::MemoryResult<usize>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                let slots = self
                    .slots
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                Ok(match entity_id {
                    Some(entity_id) => slots
                        .keys()
                        .filter(|(entity, _)| entity == entity_id)
                        .count(),
                    None => slots.len(),
                })
            })
        }
    }
}
