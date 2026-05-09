//! Turn outcome records: structured correlation of situation
//! features, policy decisions, and observed outcomes persisted to
//! memory as training data for Phase 2B learning systems.

use anyhow::Result;

pub(crate) use crate::contracts::policy::TurnOutcomeRecord;
use crate::contracts::strings::data_model::{
    PREFIX_OUTCOME_RECORD_SLOT, SOURCE_REF_AUGMENT_OUTCOME_RECORD,
};
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryLayer, MemoryProvenance, MemorySource,
    PrivacyLevel, SourceKind,
};

/// Persist a turn outcome record to memory.
pub(crate) async fn persist_outcome_record(
    mem: &dyn Memory,
    entity_id: &str,
    record: &TurnOutcomeRecord,
) -> Result<()> {
    let slot_key = format!("{PREFIX_OUTCOME_RECORD_SLOT}{}", record.id);
    let payload = serde_json::to_string(record)?;

    let input = MemoryEventInput::new(
        entity_id,
        slot_key,
        MemoryEventType::FactAdded,
        payload,
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_confidence(0.8)
    .with_importance(0.5)
    .with_layer(MemoryLayer::Episodic)
    .with_source_kind(SourceKind::Manual)
    .with_source_ref(SOURCE_REF_AUGMENT_OUTCOME_RECORD)
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::System,
        SOURCE_REF_AUGMENT_OUTCOME_RECORD,
    ));

    mem.append_event(input).await?;
    Ok(())
}

/// Retrieve recent turn outcome records for learning.
pub(crate) async fn retrieve_recent_outcomes(
    mem: &dyn Memory,
    entity_id: &str,
    limit: usize,
) -> Result<Vec<TurnOutcomeRecord>> {
    let mut records: Vec<TurnOutcomeRecord> = crate::core::memory::recall_helpers::recall_typed(
        mem,
        entity_id,
        PREFIX_OUTCOME_RECORD_SLOT,
        limit,
    )
    .await?;

    // Most recent first.
    records.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at));
    records.truncate(limit);
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agent::loop_::augment::policy::{
        OutcomeScore, PolicyDecision, SituationFeatures, TurnOutcome,
    };

    fn test_outcome() -> TurnOutcome {
        TurnOutcome {
            success: OutcomeScore::new(0.8),
            user_effort: OutcomeScore::new(0.2),
            response_length: 150,
            had_tool_calls: false,
        }
    }

    #[test]
    fn outcome_record_construction() {
        let record = TurnOutcomeRecord::new(
            SituationFeatures::default(),
            PolicyDecision::default(),
            test_outcome(),
        );
        assert!(!record.id.is_empty());
        assert!(!record.occurred_at.is_empty());
    }

    #[test]
    fn serde_round_trip() {
        let record = TurnOutcomeRecord::new(
            SituationFeatures::default(),
            PolicyDecision::default(),
            test_outcome(),
        );
        let json = serde_json::to_string(&record).unwrap();
        let back: TurnOutcomeRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, record.id);
    }

    #[tokio::test]
    async fn persist_and_retrieve_outcome_record() {
        let temp = tempfile::TempDir::new().unwrap();
        let mem = crate::core::memory::MarkdownMemory::new(temp.path());

        let record = TurnOutcomeRecord::new(
            SituationFeatures::default(),
            PolicyDecision::default(),
            test_outcome(),
        );
        persist_outcome_record(&mem, "test-entity", &record)
            .await
            .unwrap();

        let retrieved = retrieve_recent_outcomes(&mem, "test-entity", 10)
            .await
            .unwrap();
        assert!(!retrieved.is_empty());
        assert_eq!(retrieved[0].id, record.id);
    }
}
