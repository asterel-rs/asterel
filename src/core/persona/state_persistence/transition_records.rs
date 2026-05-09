//! Persona transition record persistence and identity event emission.
//!
//! Every successful `persist_backend_sync` call writes three memory slots:
//! - **Provenance** (`person_provenance_slot_key`): the full `PersonaTransition`
//!   keyed by `next.last_updated_at` for point-in-time audit.
//! - **Rollback record** (`person_rollback_slot_key`): same transition, keyed
//!   separately so rollback tooling can locate it independently.
//! - **Latest rollback pointer** (`person_latest_slot_key`): overwritten on
//!   each transition; always points to the most recent `PersonaTransition`.
//!
//! All three slots are written with confidence 0.95 and importance 1.0 to
//! signal they are authoritative identity records.
//!
//! `emit_identity_transition_events` fires best-effort memory events for
//! objective changes and commitment additions/completions; failures are
//! logged at debug level rather than propagated.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use chrono::Utc;

use super::{
    BackendHeaderPersist, PersonaTransition, PersonaTransitionReason, StateHeader,
    build_commitment_added_event, build_commitment_completed_event, build_objective_changed_event,
};
use crate::core::memory::{
    MemoryEventInput, MemoryEventType, MemoryLayer, MemoryProvenance, MemorySource, PrivacyLevel,
    SourceKind,
};
use crate::security::writeback_guard::enforce_persona_long_term_write_policy;

async fn persist_transition_record(
    service: &BackendHeaderPersist,
    slot_key: crate::contracts::ids::SlotKey,
    source_ref: String,
    provenance_ref: &'static str,
    occurred_at: &str,
    record: &PersonaTransition,
) -> Result<()> {
    let person_entity_id = service.person_entity_id();
    let serialized = serde_json::to_string(record)?;
    let input = MemoryEventInput::new(
        person_entity_id,
        slot_key,
        MemoryEventType::FactUpdated,
        serialized,
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_confidence(0.95)
    .with_importance(1.0)
    .with_layer(MemoryLayer::Identity)
    .with_source_kind(SourceKind::Manual)
    .with_source_ref(source_ref)
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::System,
        provenance_ref,
    ))
    .with_occurred_at(occurred_at.to_string());
    enforce_persona_long_term_write_policy(&input, service.person_or_default())
        .context("enforce persona transition record policy")?;
    service.memory.append_event(input).await?;
    Ok(())
}

pub(super) async fn persist_transition_records(
    service: &BackendHeaderPersist,
    previous: &StateHeader,
    next: &StateHeader,
) -> Result<()> {
    let record = PersonaTransition {
        schema_version: 1,
        person_id: crate::contracts::ids::PersonId::new(service.person_or_default()),
        recorded_at: Utc::now().to_rfc3339(),
        from_last_updated_at: previous.last_updated_at.clone(),
        to_last_updated_at: next.last_updated_at.clone(),
        previous: previous.clone(),
        next: next.clone(),
        why: build_transition_why(previous, next),
    };

    persist_transition_record(
        service,
        crate::contracts::ids::SlotKey::new(
            service.person_provenance_slot_key(&next.last_updated_at),
        ),
        format!("persona-state-provenance:{}", next.last_updated_at),
        "persona.state_header.provenance",
        &record.to_last_updated_at,
        &record,
    )
    .await?;
    persist_transition_record(
        service,
        crate::contracts::ids::SlotKey::new(
            service.person_rollback_slot_key(&next.last_updated_at),
        ),
        format!("persona-state-rollback-record:{}", next.last_updated_at),
        "persona.state_header.rollback_record",
        &record.to_last_updated_at,
        &record,
    )
    .await?;
    persist_transition_record(
        service,
        crate::contracts::ids::SlotKey::new(service.person_latest_slot_key()),
        format!("persona-state-rollback-latest:{}", next.last_updated_at),
        "persona.state_header.rollback_latest",
        &record.recorded_at,
        &record,
    )
    .await
}

fn build_transition_why(
    previous: &StateHeader,
    next: &StateHeader,
) -> Vec<PersonaTransitionReason> {
    let mut reasons = Vec::new();
    push_text_change(
        &mut reasons,
        "identity_principles_hash",
        &previous.identity_principles_hash,
        &next.identity_principles_hash,
    );
    push_text_change(
        &mut reasons,
        "safety_posture",
        &previous.safety_posture,
        &next.safety_posture,
    );
    push_text_change(
        &mut reasons,
        "current_objective",
        &previous.current_objective,
        &next.current_objective,
    );
    push_list_change(
        &mut reasons,
        "open_loops",
        &previous.open_loops,
        &next.open_loops,
    );
    push_list_change(
        &mut reasons,
        "next_actions",
        &previous.next_actions,
        &next.next_actions,
    );
    push_list_change(
        &mut reasons,
        "commitments",
        &previous.commitments,
        &next.commitments,
    );
    push_text_change(
        &mut reasons,
        "recent_context_summary",
        &previous.recent_context_summary,
        &next.recent_context_summary,
    );
    if previous.last_updated_at != next.last_updated_at {
        reasons.push(PersonaTransitionReason {
            field: "last_updated_at".to_string(),
            summary: format!(
                "advanced from {} to {}",
                previous.last_updated_at, next.last_updated_at
            ),
        });
    }
    reasons
}

fn push_text_change(
    reasons: &mut Vec<PersonaTransitionReason>,
    field: &str,
    previous: &str,
    next: &str,
) {
    if previous == next {
        return;
    }
    reasons.push(PersonaTransitionReason {
        field: field.to_string(),
        summary: format!("changed from {previous:?} to {next:?}"),
    });
}

fn push_list_change(
    reasons: &mut Vec<PersonaTransitionReason>,
    field: &str,
    previous: &[String],
    next: &[String],
) {
    if previous == next {
        return;
    }
    let (added, removed) = list_count_delta(previous, next);
    reasons.push(PersonaTransitionReason {
        field: field.to_string(),
        summary: format!(
            "changed from {} item(s) to {} item(s); added={added}, removed={removed}",
            previous.len(),
            next.len()
        ),
    });
}

fn list_count_delta(previous: &[String], next: &[String]) -> (usize, usize) {
    let previous_counts = item_counts(previous);
    let next_counts = item_counts(next);
    let added = next_counts
        .iter()
        .map(|(item, next_count)| {
            next_count.saturating_sub(*previous_counts.get(item).unwrap_or(&0))
        })
        .sum();
    let removed = previous_counts
        .iter()
        .map(|(item, previous_count)| {
            previous_count.saturating_sub(*next_counts.get(item).unwrap_or(&0))
        })
        .sum();
    (added, removed)
}

fn item_counts(items: &[String]) -> BTreeMap<&str, usize> {
    let mut counts = BTreeMap::new();
    for item in items {
        *counts.entry(item.as_str()).or_insert(0) += 1;
    }
    counts
}

pub(super) async fn emit_identity_transition_events(
    service: &BackendHeaderPersist,
    previous: &StateHeader,
    current: &StateHeader,
) {
    let entity_id = service.person_entity_id();

    if previous.current_objective != current.current_objective {
        let event = build_objective_changed_event(
            &entity_id,
            &previous.current_objective,
            &current.current_objective,
        );
        if let Err(error) = service.memory.append_event(event).await {
            tracing::debug!(%error, "identity objective change event failed");
        }
    }

    for commitment in &current.commitments {
        if !previous.commitments.contains(commitment) {
            let event = build_commitment_added_event(&entity_id, commitment);
            if let Err(error) = service.memory.append_event(event).await {
                tracing::debug!(%error, "identity commitment added event failed");
            }
        }
    }

    for commitment in &previous.commitments {
        if !current.commitments.contains(commitment) {
            let event = build_commitment_completed_event(&entity_id, commitment);
            if let Err(error) = service.memory.append_event(event).await {
                tracing::debug!(%error, "identity commitment completed event failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_count_delta_counts_duplicate_additions_and_removals() {
        let previous = vec!["a".to_string(), "a".to_string(), "b".to_string()];
        let next = vec!["a".to_string(), "c".to_string(), "c".to_string()];

        assert_eq!(list_count_delta(&previous, &next), (2, 2));
    }
}
