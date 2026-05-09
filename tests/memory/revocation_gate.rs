use std::future::Future;
use std::pin::Pin;

use asterel::core::agent::loop_::{ContextBudget, build_context_for_integration};
use asterel::core::memory::{
    BeliefSlot, ForgetMode, ForgetOutcome, MemoryError, MemoryEvent, MemoryEventInput,
    MemoryGovernance, MemoryReader, MemoryRecallEntry, MemoryResult, MemorySource, MemoryWriter,
    PrivacyLevel, RecallQuery,
};
use asterel::security::policy::TenantPolicyContext;
use chrono::Utc;

struct ReplayBypassMemory;

impl MemoryWriter for ReplayBypassMemory {
    fn append_event(
        &self,
        _input: MemoryEventInput,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<MemoryEvent>> + Send + '_>> {
        Box::pin(async move { Err(MemoryError::unsupported("append_event not used")) })
    }
}

impl MemoryReader for ReplayBypassMemory {
    fn recall_scoped(
        &self,
        _query: RecallQuery,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<Vec<MemoryRecallEntry>>> + Send + '_>> {
        Box::pin(async move {
            Ok(vec![MemoryRecallEntry {
                entity_id: "default".into(),
                slot_key: "profile.cached_secret".into(),
                value: "should-not-replay".to_string(),
                source: MemorySource::System,
                confidence: 0.9.into(),
                importance: 0.9.into(),
                privacy_level: PrivacyLevel::Private,
                score: 0.95,
                occurred_at: Utc::now().to_rfc3339(),
            }])
        })
    }

    fn resolve_slot<'a>(
        &'a self,
        _entity_id: &'a str,
        _slot_key: &'a str,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<Option<BeliefSlot>>> + Send + 'a>> {
        Box::pin(async move { Ok(None) })
    }
}

impl MemoryGovernance for ReplayBypassMemory {
    fn name(&self) -> &str {
        "mock-replay-bypass"
    }

    fn health_check(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async move { true })
    }

    fn forget_slot<'a>(
        &'a self,
        _entity_id: &'a str,
        _slot_key: &'a str,
        _mode: ForgetMode,
        _reason: &'a str,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<ForgetOutcome>> + Send + 'a>> {
        Box::pin(async move { Err(MemoryError::unsupported("forget_slot not used")) })
    }

    fn count_events<'a>(
        &'a self,
        _entity_id: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = MemoryResult<usize>> + Send + 'a>> {
        Box::pin(async move { Ok(0) })
    }
}

#[tokio::test]
async fn memory_revocation_gate_applies_in_context_builder() {
    let mem = ReplayBypassMemory;
    let context = build_context_for_integration(
        &mem,
        "default",
        "cached_secret",
        TenantPolicyContext::disabled(),
        ContextBudget::default(),
    )
    .await
    .unwrap();

    assert!(
        context.is_empty(),
        "context builder must apply replay gate even when recall path is stale"
    );
}
