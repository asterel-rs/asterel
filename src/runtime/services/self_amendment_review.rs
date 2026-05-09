use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};

use crate::contracts::ids::{EntityId, SlotKey};
use crate::contracts::strings::data_model::PREFIX_SELF_AMENDMENT_SLOT;
use crate::contracts::tenant::TenantPolicyContext;
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryLayer, MemoryProvenance, MemorySource,
    PrivacyLevel, SourceKind,
};
use crate::core::persona::person_identity::person_entity_id;
use crate::core::persona::soul_core::{SelfAmendmentCandidate, SelfAmendmentCandidateSink};
use crate::runtime::diagnostics::control_plane_read_models::{
    SelfAmendmentApprovalReadModel, SelfAmendmentReviewReadModel,
    build_self_amendment_approval_read_model, build_self_amendment_review_read_model,
};
use crate::utils::text::{sanitize_prompt_line, truncate_ellipsis};

const DEFAULT_CAPACITY: usize = 100;
const MAX_REVIEW_SCOPE_MULTIPLIER: usize = 64;

#[derive(Clone)]
pub struct SelfAmendmentCandidateReviewStore {
    inner: Arc<Mutex<VecDeque<SelfAmendmentCandidate>>>,
    capacity: usize,
}

impl Default for SelfAmendmentCandidateReviewStore {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl SelfAmendmentCandidateReviewStore {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    #[must_use]
    pub fn for_workspace(workspace_dir: &Path, capacity: usize) -> Self {
        static STORES: OnceLock<
            Mutex<HashMap<std::path::PathBuf, SelfAmendmentCandidateReviewStore>>,
        > = OnceLock::new();

        let stores = STORES.get_or_init(|| Mutex::new(HashMap::new()));
        let Ok(mut guard) = stores.lock() else {
            tracing::warn!("self-amendment review store registry lock poisoned; using local store");
            return Self::new(capacity);
        };

        guard
            .entry(workspace_dir.to_path_buf())
            .or_insert_with(|| Self::new(capacity))
            .clone()
    }

    #[must_use]
    pub(crate) fn snapshot(&self) -> Vec<SelfAmendmentCandidate> {
        let Ok(guard) = self.inner.lock() else {
            tracing::warn!("self-amendment review store lock poisoned; returning empty snapshot");
            return Vec::new();
        };
        guard.iter().cloned().collect()
    }

    pub(crate) fn take_latest_candidate_for_tenant(
        &self,
        candidate_id: &str,
        tenant_id: Option<&str>,
    ) -> Option<SelfAmendmentCandidate> {
        let Ok(mut guard) = self.inner.lock() else {
            tracing::warn!("self-amendment review store lock poisoned; returning no candidate");
            return None;
        };

        let mut selected = None;
        let mut retained = VecDeque::with_capacity(guard.len());
        while let Some(candidate) = guard.pop_front() {
            if candidate_matches_scope(&candidate, candidate_id, tenant_id) {
                selected = Some(candidate);
            } else {
                retained.push_back(candidate);
            }
        }
        *guard = retained;
        selected
    }

    pub(crate) fn latest_candidate_for_tenant(
        &self,
        candidate_id: &str,
        tenant_id: Option<&str>,
    ) -> Option<SelfAmendmentCandidate> {
        let Ok(guard) = self.inner.lock() else {
            tracing::warn!("self-amendment review store lock poisoned; returning no candidate");
            return None;
        };

        guard
            .iter()
            .rev()
            .find(|candidate| candidate_matches_scope(candidate, candidate_id, tenant_id))
            .cloned()
    }
}

fn candidate_matches_scope(
    candidate: &SelfAmendmentCandidate,
    candidate_id: &str,
    tenant_id: Option<&str>,
) -> bool {
    candidate.candidate_id == candidate_id && candidate.tenant_id.as_deref() == tenant_id
}

impl SelfAmendmentCandidateSink for SelfAmendmentCandidateReviewStore {
    fn record_self_amendment_candidates(&self, candidates: &[SelfAmendmentCandidate]) {
        if candidates.is_empty() || self.capacity == 0 {
            return;
        }

        let Ok(mut guard) = self.inner.lock() else {
            tracing::warn!("self-amendment review store lock poisoned; dropping candidates");
            return;
        };

        for candidate in candidates {
            let workspace_hard_capacity = self.capacity.saturating_mul(MAX_REVIEW_SCOPE_MULTIPLIER);
            while guard.len() >= workspace_hard_capacity.max(self.capacity) {
                guard.pop_front();
            }

            let tenant_id = candidate.tenant_id.as_deref();
            let tenant_count = guard
                .iter()
                .filter(|existing| existing.tenant_id.as_deref() == tenant_id)
                .count();
            if tenant_count >= self.capacity
                && let Some(position) = guard
                    .iter()
                    .position(|existing| existing.tenant_id.as_deref() == tenant_id)
            {
                guard.remove(position);
            }
            guard.push_back(candidate.clone());
        }
    }
}

#[must_use]
pub fn load_self_amendment_candidate_review(
    store: &SelfAmendmentCandidateReviewStore,
) -> SelfAmendmentReviewReadModel {
    let mut model = build_self_amendment_review_read_model(&store.snapshot());
    model.durable_writes_enabled = true;
    model
}

#[must_use]
pub fn load_self_amendment_candidate_review_for_tenant(
    store: &SelfAmendmentCandidateReviewStore,
    tenant_id: Option<&str>,
) -> SelfAmendmentReviewReadModel {
    let mut candidates = store.snapshot();
    candidates.retain(|candidate| candidate.tenant_id.as_deref() == tenant_id);
    let mut model = build_self_amendment_review_read_model(&candidates);
    model.durable_writes_enabled = true;
    model
}

/// Persist an operator-reviewed self-amendment candidate through the memory ledger.
///
/// # Errors
/// Returns an error when the candidate is missing, outside tenant scope, or the
/// memory backend rejects the append.
pub async fn approve_self_amendment_candidate(
    memory: &dyn Memory,
    store: &SelfAmendmentCandidateReviewStore,
    principal: &str,
    policy_context: &TenantPolicyContext,
    candidate_id: &str,
    reason: &str,
) -> Result<SelfAmendmentApprovalReadModel> {
    let candidate_id = candidate_id.trim();
    let reason = sanitize_prompt_line(reason);
    if candidate_id.is_empty() || reason.trim().is_empty() {
        bail!("candidate_id and reason must not be empty");
    }

    let tenant_id = approval_tenant_scope(policy_context)?;
    let candidate = store
        .latest_candidate_for_tenant(candidate_id, tenant_id)
        .ok_or_else(|| anyhow::anyhow!("self-amendment candidate not found"))?;

    let approval =
        persist_approved_candidate(memory, principal, policy_context, &candidate, reason).await?;
    store.take_latest_candidate_for_tenant(candidate_id, tenant_id);
    Ok(approval)
}

async fn persist_approved_candidate(
    memory: &dyn Memory,
    principal: &str,
    policy_context: &TenantPolicyContext,
    candidate: &SelfAmendmentCandidate,
    reason: String,
) -> Result<SelfAmendmentApprovalReadModel> {
    let entity_id = candidate_entity_id(candidate);
    policy_context
        .enforce_recall_scope(&entity_id)
        .map_err(|error| anyhow::anyhow!(error))?;

    let slot_key = candidate_slot_key(&candidate.candidate_id);
    let event_type = if memory.resolve_slot(&entity_id, &slot_key).await?.is_some() {
        MemoryEventType::FactUpdated
    } else {
        MemoryEventType::FactAdded
    };

    let value = reviewed_amendment_value(candidate);
    let provenance = MemoryProvenance::source_reference(
        MemorySource::System,
        truncate_ellipsis(
            &format!(
                "admin.memory.self_amendment.approve:{reason} | candidate_id={} | evidence_ids={}",
                truncate_ellipsis(&sanitize_prompt_line(&candidate.candidate_id), 128),
                reviewed_evidence_ids(candidate)
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            256,
        ),
    );

    let input = MemoryEventInput::new(
        &entity_id,
        &slot_key,
        event_type,
        value,
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_layer(MemoryLayer::Procedural)
    .with_confidence(0.86)
    .with_importance(0.7)
    .with_source_kind(SourceKind::Manual)
    .with_source_ref(format!("admin.memory.self_amendment.approve:{principal}"))
    .with_provenance(provenance);

    let event = memory.append_event(input).await?;

    Ok(build_self_amendment_approval_read_model(
        &candidate.candidate_id,
        EntityId::new(entity_id),
        SlotKey::new(slot_key),
        event.event_id,
    ))
}

fn approval_tenant_scope(policy_context: &TenantPolicyContext) -> Result<Option<&str>> {
    if !policy_context.tenant_mode_enabled {
        return Ok(None);
    }

    let tenant_id = policy_context
        .tenant_id
        .as_deref()
        .filter(|tenant_id| !tenant_id.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("tenant scoped approval requires tenant_id"))?;
    Ok(Some(tenant_id))
}

fn candidate_entity_id(candidate: &SelfAmendmentCandidate) -> String {
    let base = person_entity_id(&candidate.person_id);
    candidate
        .tenant_id
        .as_ref()
        .map_or(base.clone(), |tenant_id| {
            TenantPolicyContext::enabled(tenant_id).scope_entity_id(&base)
        })
}

fn candidate_slot_key(candidate_id: &str) -> String {
    format!(
        "{PREFIX_SELF_AMENDMENT_SLOT}{}",
        sanitize_candidate_id_for_slot(candidate_id)
    )
}

fn sanitize_candidate_id_for_slot(candidate_id: &str) -> String {
    let sanitized = candidate_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .chars()
        .take(96)
        .collect::<String>();
    if sanitized.is_empty() {
        "unknown".to_string()
    } else {
        sanitized
    }
}

fn reviewed_amendment_value(candidate: &SelfAmendmentCandidate) -> String {
    serde_json::json!({
        "kind": "self_amendment",
        "candidate_kind": format!("{:?}", candidate.kind),
        "status": "operator_reviewed",
        "surface": truncate_ellipsis(&sanitize_prompt_line(&candidate.surface), 80),
        "privacy": format!("{:?}", candidate.privacy),
        "reason": truncate_ellipsis(&sanitize_prompt_line(&candidate.reason), 256),
        "proposed_amendment": truncate_ellipsis(&sanitize_prompt_line(&candidate.proposed_amendment), 512),
        "evidence_ids": reviewed_evidence_ids(candidate),
        "raw_payload_redacted": true,
    })
    .to_string()
}

fn reviewed_evidence_ids(candidate: &SelfAmendmentCandidate) -> Vec<String> {
    let mut out = Vec::new();
    for evidence_id in &candidate.evidence_ids {
        let projected = truncate_ellipsis(&sanitize_prompt_line(evidence_id), 120);
        if projected.is_empty() || out.iter().any(|existing| existing == &projected) {
            continue;
        }
        out.push(projected);
        if out.len() >= 12 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::ids::EventId;
    use crate::core::memory::{
        BeliefSlot, ForgetMode, ForgetOutcome, MemoryEvent, MemoryGovernance, MemoryReader,
        MemoryRecallEntry, MemoryWriter, RecallQuery,
    };
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;

    fn candidate(id: &str) -> SelfAmendmentCandidate {
        SelfAmendmentCandidate {
            candidate_id: id.to_string(),
            kind: crate::core::persona::soul_core::SelfAmendmentCandidateKind::RepairPractice,
            status: crate::core::persona::soul_core::SelfAmendmentCandidateStatus::DryRunOnly,
            tenant_id: Some("tenant-alpha".to_string()),
            person_id: "person-test".to_string(),
            surface: "discord".to_string(),
            privacy: crate::core::persona::soul_core::SelfAmendmentPrivacy::PrivateInternal,
            evidence_ids: vec!["post_turn:soul_pressure".to_string()],
            reason: "explicit correction raised repair pressure; candidate is dry-run only"
                .to_string(),
            proposed_amendment: "When corrected, acknowledge briefly, avoid defending, and preserve relationship distance."
                .to_string(),
        }
    }

    #[test]
    fn review_store_keeps_bounded_dry_run_candidates() {
        let store = SelfAmendmentCandidateReviewStore::new(2);
        store.record_self_amendment_candidates(&[candidate("candidate:1")]);
        store.record_self_amendment_candidates(&[
            candidate("candidate:2"),
            candidate("candidate:3"),
        ]);

        let model = load_self_amendment_candidate_review(&store);

        assert_eq!(model.count, 2);
        assert_eq!(model.items[0].candidate_id, "candidate:2");
        assert_eq!(model.items[1].candidate_id, "candidate:3");
        assert!(model.raw_payloads_redacted);
        assert!(model.durable_writes_enabled);
    }

    #[test]
    fn review_store_filters_candidates_by_tenant() {
        let store = SelfAmendmentCandidateReviewStore::new(10);
        store.record_self_amendment_candidates(&[
            candidate("tenant-alpha:candidate"),
            SelfAmendmentCandidate {
                tenant_id: Some("tenant-beta".to_string()),
                ..candidate("tenant-beta:candidate")
            },
        ]);

        let model = load_self_amendment_candidate_review_for_tenant(&store, Some("tenant-alpha"));

        assert_eq!(model.count, 1);
        assert_eq!(model.items[0].tenant_id.as_deref(), Some("tenant-alpha"));
    }

    #[test]
    fn review_store_unscoped_view_excludes_tenant_scoped_candidates() {
        let store = SelfAmendmentCandidateReviewStore::new(10);
        store.record_self_amendment_candidates(&[
            candidate("tenant-alpha:candidate"),
            SelfAmendmentCandidate {
                tenant_id: None,
                ..candidate("global:candidate")
            },
        ]);

        let model = load_self_amendment_candidate_review_for_tenant(&store, None);

        assert_eq!(model.count, 1);
        assert_eq!(model.items[0].candidate_id, "global:candidate");
        assert_eq!(model.items[0].tenant_id, None);
    }

    #[test]
    fn review_store_capacity_is_applied_per_tenant_scope() {
        let store = SelfAmendmentCandidateReviewStore::new(2);
        store.record_self_amendment_candidates(&[
            candidate("alpha:1"),
            candidate("alpha:2"),
            SelfAmendmentCandidate {
                tenant_id: Some("tenant-beta".to_string()),
                ..candidate("beta:1")
            },
            SelfAmendmentCandidate {
                tenant_id: Some("tenant-beta".to_string()),
                ..candidate("beta:2")
            },
            SelfAmendmentCandidate {
                tenant_id: Some("tenant-beta".to_string()),
                ..candidate("beta:3")
            },
        ]);

        let alpha = load_self_amendment_candidate_review_for_tenant(&store, Some("tenant-alpha"));
        let beta = load_self_amendment_candidate_review_for_tenant(&store, Some("tenant-beta"));

        assert_eq!(alpha.count, 2);
        assert_eq!(alpha.items[0].candidate_id, "alpha:1");
        assert_eq!(alpha.items[1].candidate_id, "alpha:2");
        assert_eq!(beta.count, 2);
        assert_eq!(beta.items[0].candidate_id, "beta:2");
        assert_eq!(beta.items[1].candidate_id, "beta:3");
    }

    #[test]
    fn review_store_has_workspace_hard_capacity_across_tenant_scopes() {
        let store = SelfAmendmentCandidateReviewStore::new(1);
        let candidates = (0..(MAX_REVIEW_SCOPE_MULTIPLIER + 2))
            .map(|index| SelfAmendmentCandidate {
                tenant_id: Some(format!("tenant-{index}")),
                ..candidate(&format!("candidate:{index}"))
            })
            .collect::<Vec<_>>();

        store.record_self_amendment_candidates(&candidates);

        let snapshot = store.snapshot();
        assert_eq!(snapshot.len(), MAX_REVIEW_SCOPE_MULTIPLIER);
        assert_eq!(snapshot[0].candidate_id, "candidate:2");
    }

    #[tokio::test]
    async fn approving_candidate_persists_procedural_memory_and_removes_review_item() {
        let memory = InMemorySelfAmendmentMemory::default();
        let store = SelfAmendmentCandidateReviewStore::new(10);
        store.record_self_amendment_candidates(&[candidate("candidate:approve")]);

        let approval = approve_self_amendment_candidate(
            &memory,
            &store,
            "operator",
            &TenantPolicyContext::enabled("tenant-alpha"),
            "candidate:approve",
            "operator reviewed repair pattern",
        )
        .await
        .expect("approval should persist");

        assert_eq!(approval.status, "persisted");
        assert!(approval.persisted);
        assert!(approval.raw_payload_redacted);
        assert_eq!(
            approval.entity_id.as_str(),
            "tenant-alpha:person:person-test"
        );
        assert_eq!(
            approval.slot_key.as_str(),
            "persona.writeback.self_amendment.candidate_approve"
        );
        assert_eq!(store.snapshot().len(), 0);

        let input = memory.last_input().expect("memory append input");
        assert_eq!(input.layer, MemoryLayer::Procedural);
        assert_eq!(input.event_type, MemoryEventType::FactAdded);
        assert_eq!(input.source, MemorySource::System);
        assert_eq!(input.privacy_level, PrivacyLevel::Private);
        assert_eq!(input.source_kind, Some(SourceKind::Manual));
        assert_eq!(
            input.source_ref.as_deref(),
            Some("admin.memory.self_amendment.approve:operator")
        );
        assert!(input.provenance.as_ref().is_some_and(|provenance| {
            provenance
                .reference
                .contains("candidate_id=candidate:approve")
        }));
        assert!(!input.value.contains("user_message"));
        assert!(!input.value.contains("assistant_response"));
        assert!(input.value.contains("raw_payload_redacted"));
    }

    #[tokio::test]
    async fn approving_candidate_is_single_use() {
        let memory = InMemorySelfAmendmentMemory::default();
        let store = SelfAmendmentCandidateReviewStore::new(10);
        store.record_self_amendment_candidates(&[
            candidate("candidate:single-use"),
            candidate("candidate:single-use"),
        ]);

        approve_self_amendment_candidate(
            &memory,
            &store,
            "operator",
            &TenantPolicyContext::enabled("tenant-alpha"),
            "candidate:single-use",
            "operator reviewed repair pattern",
        )
        .await
        .expect("first approval should persist");

        let error = approve_self_amendment_candidate(
            &memory,
            &store,
            "operator",
            &TenantPolicyContext::enabled("tenant-alpha"),
            "candidate:single-use",
            "operator reviewed repair pattern",
        )
        .await
        .expect_err("second approval should not duplicate writes");

        assert!(error.to_string().contains("candidate not found"));
        assert_eq!(memory.input_count(), 1);
        assert_eq!(store.snapshot().len(), 0);
    }

    #[tokio::test]
    async fn failed_approval_append_retains_review_candidate() {
        let memory = InMemorySelfAmendmentMemory {
            fail_appends: true,
            ..InMemorySelfAmendmentMemory::default()
        };
        let store = SelfAmendmentCandidateReviewStore::new(10);
        store.record_self_amendment_candidates(&[candidate("candidate:retry")]);

        let error = approve_self_amendment_candidate(
            &memory,
            &store,
            "operator",
            &TenantPolicyContext::enabled("tenant-alpha"),
            "candidate:retry",
            "operator reviewed repair pattern",
        )
        .await
        .expect_err("memory append failure should reject approval");

        assert!(error.to_string().contains("forced append failure"));
        assert_eq!(memory.input_count(), 0);
        assert_eq!(store.snapshot().len(), 1);
    }

    #[tokio::test]
    async fn tenant_scoped_operator_cannot_approve_other_tenant_candidate() {
        let memory = InMemorySelfAmendmentMemory::default();
        let store = SelfAmendmentCandidateReviewStore::new(10);
        store.record_self_amendment_candidates(&[SelfAmendmentCandidate {
            tenant_id: Some("tenant-beta".to_string()),
            ..candidate("candidate:beta")
        }]);

        let error = approve_self_amendment_candidate(
            &memory,
            &store,
            "operator",
            &TenantPolicyContext::enabled("tenant-alpha"),
            "candidate:beta",
            "operator reviewed repair pattern",
        )
        .await
        .expect_err("cross-tenant candidate should not be approvable");

        assert!(error.to_string().contains("candidate not found"));
        assert_eq!(store.snapshot().len(), 1);
        assert!(memory.last_input().is_none());
    }

    #[tokio::test]
    async fn unscoped_operator_cannot_approve_tenant_scoped_candidate() {
        let memory = InMemorySelfAmendmentMemory::default();
        let store = SelfAmendmentCandidateReviewStore::new(10);
        store.record_self_amendment_candidates(&[candidate("candidate:tenant-only")]);

        let error = approve_self_amendment_candidate(
            &memory,
            &store,
            "operator",
            &TenantPolicyContext::disabled(),
            "candidate:tenant-only",
            "operator reviewed repair pattern",
        )
        .await
        .expect_err("tenant-scoped candidate requires tenant-scoped approval");

        assert!(error.to_string().contains("candidate not found"));
        assert_eq!(store.snapshot().len(), 1);
        assert!(memory.last_input().is_none());
    }

    #[derive(Default)]
    struct InMemorySelfAmendmentMemory {
        slots: Mutex<BTreeMap<(String, String), MemoryEventInput>>,
        inputs: Mutex<Vec<MemoryEventInput>>,
        fail_appends: bool,
    }

    impl InMemorySelfAmendmentMemory {
        fn last_input(&self) -> Option<MemoryEventInput> {
            self.inputs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .last()
                .cloned()
        }

        fn input_count(&self) -> usize {
            self.inputs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
        }
    }

    impl MemoryWriter for InMemorySelfAmendmentMemory {
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
                if self.fail_appends {
                    return Err(crate::contracts::memory_error::MemoryError::write(
                        "forced append failure",
                    ));
                }
                let event_id = EventId::new(format!(
                    "test-event-{}",
                    self.inputs
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .len()
                        + 1
                ));
                self.slots
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(
                        (input.entity_id.to_string(), input.slot_key.to_string()),
                        input.clone(),
                    );
                self.inputs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(input.clone());
                Ok(MemoryEvent {
                    event_id,
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

    impl MemoryReader for InMemorySelfAmendmentMemory {
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
                        entity_id: EntityId::new(entity_id),
                        slot_key: SlotKey::new(slot_key),
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

    impl MemoryGovernance for InMemorySelfAmendmentMemory {
        fn name(&self) -> &str {
            "in-memory-self-amendment-test"
        }

        fn health_check(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
            Box::pin(async { true })
        }

        fn forget_slot<'a>(
            &'a self,
            _entity_id: &'a str,
            _slot_key: &'a str,
            _mode: ForgetMode,
            _reason: &'a str,
        ) -> Pin<
            Box<
                dyn Future<Output = crate::contracts::memory_error::MemoryResult<ForgetOutcome>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async {
                Err(crate::contracts::memory_error::MemoryError::unsupported(
                    "forget not supported in test memory",
                ))
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
                let inputs = self
                    .inputs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                Ok(match entity_id {
                    Some(entity_id) => inputs
                        .iter()
                        .filter(|input| input.entity_id.as_str() == entity_id)
                        .count(),
                    None => inputs.len(),
                })
            })
        }
    }
}
