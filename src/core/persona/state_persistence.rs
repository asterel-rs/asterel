//! Persona state persistence: saves and loads `StateHeader`
//! snapshots to memory and filesystem, recording transition
//! provenance for rollback and audit.

mod mirror;
mod transition_records;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::config::PersonaConfig;
use crate::contracts::ids::PersonId;
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryLayer, MemoryProvenance, MemorySource,
    PrivacyLevel, SourceKind,
};
use crate::core::persona::identity_contract::IdentityContractV1;
use crate::core::persona::identity_events::{
    build_commitment_added_event, build_commitment_completed_event, build_objective_changed_event,
};
use crate::core::persona::person_identity::{
    canonical_state_header_slot_key, person_entity_id, sanitize_person_id,
};
use crate::core::persona::state_header::StateHeader;
use crate::security::writeback_guard::enforce_persona_long_term_write_policy;

/// Memory slot key suffix for the canonical state header.
pub const CANONICAL_STATE_HEADER_KEY: &str = "persona/state_header/v1";
/// Record of a single state header transition for audit/rollback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaTransition {
    /// Schema version for forward compatibility.
    pub schema_version: u32,
    /// Person ID this transition belongs to.
    pub person_id: PersonId,
    /// RFC 3339 timestamp when this record was created.
    pub recorded_at: String,
    /// Timestamp of the previous state header.
    pub from_last_updated_at: String,
    /// Timestamp of the new state header.
    pub to_last_updated_at: String,
    /// State header before the transition.
    pub previous: StateHeader,
    /// State header after the transition.
    pub next: StateHeader,
    /// Human-readable reasons describing why this transition exists.
    #[serde(default)]
    pub why: Vec<PersonaTransitionReason>,
}

/// A single field-level reason for a persona transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaTransitionReason {
    /// State header field that changed.
    pub field: String,
    /// Compact explanation of the observed change.
    pub summary: String,
}

/// Manages canonical state header persistence, mirror sync, and rollback.
pub struct BackendHeaderPersist {
    memory: Arc<dyn Memory>,
    workspace_dir: PathBuf,
    persona: PersonaConfig,
    person_id: PersonId,
}

impl BackendHeaderPersist {
    /// Create a new persistence handler for the given person.
    pub fn new(
        memory: Arc<dyn Memory>,
        workspace_dir: PathBuf,
        persona: PersonaConfig,
        person_id: impl Into<String>,
    ) -> Self {
        Self {
            memory,
            workspace_dir,
            persona,
            person_id: PersonId::new(sanitize_person_id(&person_id.into())),
        }
    }

    fn person_entity_id(&self) -> String {
        person_entity_id(self.person_or_default())
    }

    fn person_slot_namespace(&self) -> String {
        format!(
            "persona/{}/state_header",
            sanitize_person_id(self.person_or_default())
        )
    }

    fn person_canonical_key(&self) -> String {
        canonical_state_header_slot_key(self.person_or_default())
    }

    fn person_provenance_slot_key(&self, to_last_updated_at: &str) -> String {
        format!(
            "{}/provenance/{}",
            self.person_slot_namespace(),
            sanitize_person_id(to_last_updated_at)
        )
    }

    fn person_rollback_slot_key(&self, to_last_updated_at: &str) -> String {
        format!(
            "{}/rollback/{}",
            self.person_slot_namespace(),
            sanitize_person_id(to_last_updated_at)
        )
    }

    fn person_latest_slot_key(&self) -> String {
        format!("{}/rollback/latest", self.person_slot_namespace())
    }

    fn state_transition_records_enabled(&self) -> bool {
        self.persona.enable_state_transition_records
    }

    fn person_or_default(&self) -> &str {
        if self.person_id.as_str().is_empty() {
            "local-default"
        } else {
            self.person_id.as_str()
        }
    }

    /// # Errors
    /// Returns an error if canonical state lookup, parsing, or validation fails.
    pub async fn load_backend_state(&self) -> Result<Option<StateHeader>> {
        let person_entity_id = self.person_entity_id();
        let person_slot_key = self.person_canonical_key();

        let Some(entry) = self
            .memory
            .resolve_slot(&person_entity_id, &person_slot_key)
            .await?
        else {
            return Ok(None);
        };

        let parsed: StateHeader = serde_json::from_str(&entry.value).with_context(|| {
            format!("failed to parse backend canonical state header at slot: {person_slot_key}")
        })?;
        parsed.validate(&self.persona)?;

        Ok(Some(parsed))
    }

    /// # Errors
    /// Returns an error if the rollback record cannot be resolved, parsed, validated, or persisted.
    pub async fn rollback_to_transition(&self, to_last_updated_at: &str) -> Result<StateHeader> {
        let person_entity_id = self.person_entity_id();
        let rollback_slot_key = self.person_rollback_slot_key(to_last_updated_at);

        let Some(entry) = self
            .memory
            .resolve_slot(&person_entity_id, &rollback_slot_key)
            .await?
        else {
            anyhow::bail!("rollback record not found for state transition: {to_last_updated_at}");
        };

        let record: PersonaTransition =
            serde_json::from_str(&entry.value).context("parse rollback transition record")?;
        if record.person_id.as_str() != self.person_or_default() {
            anyhow::bail!("rollback record person_id mismatch");
        }

        record.previous.validate(&self.persona)?;
        self.persist_backend_state(&record.previous, false).await?;
        Ok(record.previous)
    }

    /// # Errors
    /// Returns an error if canonical state load, bootstrap persistence, or mirror sync fails.
    pub async fn reconcile_mirror_from_backend_on_startup(&self) -> Result<Option<StateHeader>> {
        let canonical = if let Some(existing) = self.load_backend_state().await? {
            existing
        } else {
            let seeded = Self::seed_minimal_backend_canonical();
            self.persist_backend_sync(&seeded).await?;
            seeded
        };

        self.sync_mirror_from_backend_canonical(&canonical)?;
        Ok(Some(canonical))
    }

    /// # Errors
    /// Returns an error if state validation or canonical persistence fails.
    ///
    /// Runtime contract: persona state header writes are single-writer per
    /// workspace/person. This runtime does not support multi-replica persona
    /// writers. The read-then-write between `load_backend_state` and
    /// `append_event` is therefore intentionally not a compare-and-swap API;
    /// if multi-replica writing is introduced, this boundary must move to a
    /// backend transaction/CAS operation before the deployment mode is enabled.
    pub async fn persist_backend_sync(&self, state: &StateHeader) -> Result<()> {
        self.persist_backend_state(state, true).await
    }

    async fn persist_backend_state(
        &self,
        state: &StateHeader,
        enforce_forward_transition: bool,
    ) -> Result<()> {
        state.validate(&self.persona)?;

        let previous_state = self.load_backend_state().await?;
        if enforce_forward_transition
            && let Some(previous) = previous_state.as_ref()
            && previous != state
        {
            StateHeader::validate_writeback_candidate(previous, state, &self.persona)
                .context("validate persona state transition")?;
            let previous_contract = IdentityContractV1::from_state_header(previous);
            let candidate_contract = IdentityContractV1::from_state_header(state);
            IdentityContractV1::validate_mutation(
                &previous_contract,
                &candidate_contract,
                &self.persona,
            )
            .context("validate persona identity contract transition")?;
        }

        let person_entity_id = self.person_entity_id();
        let person_slot_key = self.person_canonical_key();

        let serialized = serde_json::to_string(state)?;
        let input = MemoryEventInput::new(
            person_entity_id,
            person_slot_key,
            MemoryEventType::FactUpdated,
            serialized,
            MemorySource::System,
            PrivacyLevel::Private,
        )
        .with_confidence(0.95)
        .with_importance(1.0)
        .with_layer(MemoryLayer::Identity)
        .with_source_kind(SourceKind::Manual)
        .with_source_ref(format!("persona-state-writeback:{}", state.last_updated_at))
        .with_provenance(MemoryProvenance::source_reference(
            MemorySource::System,
            "persona.state_header.writeback",
        ));
        enforce_persona_long_term_write_policy(&input, self.person_or_default())
            .context("enforce persona canonical write policy")?;
        self.memory.append_event(input).await?;

        if self.state_transition_records_enabled()
            && let Some(previous) = previous_state.as_ref()
            && *previous != *state
        {
            transition_records::persist_transition_records(self, previous, state)
                .await
                .context("persist persona transition provenance/rollback records")?;
        }

        if let Some(previous) = &previous_state
            && previous != state
        {
            transition_records::emit_identity_transition_events(self, previous, state).await;
        }

        if let Err(error) = self.sync_mirror_from_backend_canonical(state) {
            tracing::warn!(
                %error,
                "failed to sync persona state mirror after canonical backend write"
            );
        }

        Ok(())
    }

    /// # Errors
    /// Returns an error if mirror state cannot be read, parsed, or validated.
    pub fn read_mirror_state(&self) -> Result<Option<StateHeader>> {
        mirror::read_mirror_state(self)
    }

    fn state_mirror_path(&self) -> PathBuf {
        self.workspace_dir.join(&self.persona.state_mirror_filename)
    }

    fn sync_mirror_from_backend_canonical(&self, state: &StateHeader) -> Result<()> {
        mirror::sync_mirror_from_backend_canonical(self, state)
    }

    fn seed_minimal_backend_canonical() -> StateHeader {
        StateHeader {
            identity_principles_hash: "bootstrap-minimal-v1".to_string(),
            safety_posture: "strict".to_string(),
            current_objective: "Initialize persona state continuity from backend canonical."
                .to_string(),
            open_loops: Vec::new(),
            next_actions: Vec::new(),
            commitments: Vec::new(),
            recent_context_summary:
                "Seeded minimal valid state because canonical backend entry was missing at startup."
                    .to_string(),
            last_updated_at: Utc::now().to_rfc3339(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;
    use crate::core::memory::embeddings::{EmbeddingFuture, EmbeddingProvider};
    use crate::core::memory::postgres::{PostgresConnectOptions, PostgresMemory};
    use crate::core::memory::{MarkdownMemory, Memory};
    use crate::utils::test_env::EnvVarGuard;

    struct NoopEmbedding;

    impl EmbeddingProvider for NoopEmbedding {
        fn name(&self) -> &'static str {
            "noop_test"
        }

        fn dimensions(&self) -> usize {
            0
        }

        fn embed<'a>(&'a self, _texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
            Box::pin(async move { Ok(Vec::new()) })
        }
    }

    fn sample_state_at(objective: &str, summary: &str, last_updated_at: &str) -> StateHeader {
        StateHeader {
            identity_principles_hash: "identity-v1-abcd1234".to_string(),
            safety_posture: "strict".to_string(),
            current_objective: objective.to_string(),
            open_loops: vec!["Ship startup reconciliation".to_string()],
            next_actions: vec!["Sync backend to mirror".to_string()],
            commitments: vec!["Backend is canonical".to_string()],
            recent_context_summary: summary.to_string(),
            last_updated_at: last_updated_at.to_string(),
        }
    }

    fn sample_state(objective: &str, summary: &str) -> StateHeader {
        sample_state_at(objective, summary, &Utc::now().to_rfc3339())
    }

    fn service_with_postgres(tmp: &TempDir, mirror_filename: &str) -> BackendHeaderPersist {
        let memory: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
        let persona = PersonaConfig {
            state_mirror_filename: mirror_filename.to_string(),
            enable_state_transition_records: true,
            ..PersonaConfig::default()
        };

        BackendHeaderPersist::new(memory, tmp.path().to_path_buf(), persona, "person-test")
    }

    fn service_with_postgres_transition_toggle(
        tmp: &TempDir,
        mirror_filename: &str,
        enable_state_transition_records: bool,
    ) -> BackendHeaderPersist {
        let memory: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
        let persona = PersonaConfig {
            state_mirror_filename: mirror_filename.to_string(),
            enable_state_transition_records,
            ..PersonaConfig::default()
        };

        BackendHeaderPersist::new(memory, tmp.path().to_path_buf(), persona, "person-test")
    }

    async fn service_with_actual_postgres(
        tmp: &TempDir,
        mirror_filename: &str,
    ) -> (BackendHeaderPersist, EnvVarGuard) {
        let env_guard = EnvVarGuard::require_postgres_url();
        let database_url =
            std::env::var("ASTEREL_POSTGRES_URL").expect("ASTEREL_POSTGRES_URL must be set");
        let memory = PostgresMemory::connect_with_options(
            &database_url,
            Arc::new(NoopEmbedding),
            PostgresConnectOptions {
                cache_max: 0,
                graph_retrieval_fusion_enabled: false,
                graph_retrieval_weight: 0.0,
                max_connections: 4,
                min_connections: 1,
                connect_timeout: Duration::from_secs(5),
                idle_timeout: Duration::from_secs(30),
                vector_weight: 0.7,
                keyword_weight: 0.3,
                max_lifetime: Duration::from_secs(60),
                hnsw_ef_search: 0,
            },
        )
        .await
        .expect("PostgresMemory::connect_with_options should succeed");

        let persona = PersonaConfig {
            state_mirror_filename: mirror_filename.to_string(),
            enable_state_transition_records: true,
            ..PersonaConfig::default()
        };
        let person_id = format!("person-test-{}", Uuid::new_v4().simple());
        let service = BackendHeaderPersist::new(
            Arc::new(memory),
            tmp.path().to_path_buf(),
            persona,
            person_id,
        );
        (service, env_guard)
    }

    #[tokio::test]
    async fn state_header_person_namespace_persists_under_person_slot() {
        let tmp = TempDir::new().unwrap();
        let memory: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(tmp.path()));
        let persona = PersonaConfig::default();
        let service = BackendHeaderPersist::new(
            Arc::clone(&memory),
            tmp.path().to_path_buf(),
            persona,
            "alice",
        );

        let state = sample_state("Person objective", "Person summary");
        service.persist_backend_sync(&state).await.unwrap();

        let slot = memory
            .resolve_slot("person:alice", "persona/alice/state_header/v1")
            .await
            .unwrap()
            .unwrap();
        let parsed: StateHeader = serde_json::from_str(&slot.value).unwrap();
        assert_eq!(parsed, state);
    }

    #[tokio::test]
    async fn persona_bootstrap_seeds_minimal_state() {
        let tmp = TempDir::new().unwrap();
        let service = service_with_postgres(&tmp, "STATE.md");

        let seeded = service
            .reconcile_mirror_from_backend_on_startup()
            .await
            .unwrap()
            .unwrap();

        assert!(!seeded.identity_principles_hash.trim().is_empty());
        assert!(!seeded.safety_posture.trim().is_empty());
        assert!(!seeded.current_objective.trim().is_empty());
        assert!(!seeded.recent_context_summary.trim().is_empty());

        let backend = service.load_backend_state().await.unwrap().unwrap();
        let mirror = service.read_mirror_state().unwrap().unwrap();
        assert_eq!(backend, seeded);
        assert_eq!(mirror, seeded);
    }

    #[tokio::test]
    async fn state_header_repairs_divergence() {
        let tmp = TempDir::new().unwrap();
        let service = service_with_postgres(&tmp, "STATE.md");

        let backend_state = sample_state(
            "Ship deterministic persistence",
            "Backend snapshot is the canonical source of truth.",
        );
        service.persist_backend_sync(&backend_state).await.unwrap();

        let divergent_mirror = sample_state(
            "Divergent mirror objective",
            "This should be repaired from backend on startup.",
        );
        let mirror_path = tmp.path().join("STATE.md");
        fs::write(
            &mirror_path,
            super::super::presenter::render_state_header_mirror_markdown(&divergent_mirror)
                .unwrap(),
        )
        .unwrap();

        let reconciled = service
            .reconcile_mirror_from_backend_on_startup()
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reconciled, backend_state);

        let repaired_mirror = service.read_mirror_state().unwrap().unwrap();
        assert_eq!(repaired_mirror, backend_state);
    }

    #[tokio::test]
    async fn state_header_post_write_syncs_mirror() {
        let tmp = TempDir::new().unwrap();
        let service = service_with_postgres(&tmp, "STATE.md");

        let initial = sample_state_at("Objective A", "Summary A", "2026-02-26T00:00:00Z");
        service.persist_backend_sync(&initial).await.unwrap();

        let updated = sample_state_at("Objective B", "Summary B", "2026-02-26T00:05:00Z");
        service.persist_backend_sync(&updated).await.unwrap();

        let backend = service.load_backend_state().await.unwrap().unwrap();
        let mirror = service.read_mirror_state().unwrap().unwrap();
        assert_eq!(backend, updated);
        assert_eq!(mirror, updated);
    }

    #[tokio::test]
    async fn state_header_writeback_rejects_immutable_field_mutation() {
        let tmp = TempDir::new().unwrap();
        let service = service_with_postgres(&tmp, "STATE.md");

        let initial = sample_state_at("Objective A", "Summary A", "2026-02-26T00:00:00Z");
        service.persist_backend_sync(&initial).await.unwrap();

        let mut changed_stable =
            sample_state_at("Objective B", "Summary B", "2026-02-26T00:05:00Z");
        changed_stable.identity_principles_hash = "changed-stable-layer".to_string();

        let err = service
            .persist_backend_sync(&changed_stable)
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("validate persona state transition"),
            "unexpected error: {err:#}"
        );

        let backend = service.load_backend_state().await.unwrap().unwrap();
        assert_eq!(backend, initial);
    }

    #[tokio::test]
    async fn state_header_writeback_rejects_non_advancing_timestamp() {
        let tmp = TempDir::new().unwrap();
        let service = service_with_postgres(&tmp, "STATE.md");

        let initial = sample_state_at("Objective A", "Summary A", "2026-02-26T00:05:00Z");
        service.persist_backend_sync(&initial).await.unwrap();

        let candidate = sample_state_at("Objective B", "Summary B", "2026-02-26T00:04:59Z");

        let err = service.persist_backend_sync(&candidate).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("validate persona state transition"),
            "unexpected error: {err:#}"
        );

        let backend = service.load_backend_state().await.unwrap().unwrap();
        assert_eq!(backend, initial);
    }

    #[tokio::test]
    async fn state_header_writeback_atomicity() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("STATE.md")).unwrap();

        let service = service_with_postgres(&tmp, "STATE.md");
        let state = sample_state(
            "Preserve canonical write under mirror failure",
            "Mirror sync failure must not corrupt backend canonical state.",
        );

        service.persist_backend_sync(&state).await.unwrap();

        let backend = service.load_backend_state().await.unwrap().unwrap();
        assert_eq!(backend, state);
    }

    #[tokio::test]
    async fn state_header_persists_provenance_and_rollback_records() {
        let tmp = TempDir::new().unwrap();
        let service = service_with_postgres(&tmp, "STATE.md");

        let initial = sample_state_at("Objective A", "Summary A", "2026-02-26T00:00:00Z");
        service.persist_backend_sync(&initial).await.unwrap();

        let updated = sample_state_at("Objective B", "Summary B", "2026-02-26T00:05:00Z");
        service.persist_backend_sync(&updated).await.unwrap();

        let person_entity_id = service.person_entity_id();
        let provenance_slot = service.person_provenance_slot_key(&updated.last_updated_at);
        let rollback_slot = service.person_rollback_slot_key(&updated.last_updated_at);
        let rollback_latest_slot = service.person_latest_slot_key();

        let provenance_entry = service
            .memory
            .resolve_slot(&person_entity_id, &provenance_slot)
            .await
            .unwrap()
            .unwrap();
        let rollback_entry = service
            .memory
            .resolve_slot(&person_entity_id, &rollback_slot)
            .await
            .unwrap()
            .unwrap();
        let rollback_latest_entry = service
            .memory
            .resolve_slot(&person_entity_id, &rollback_latest_slot)
            .await
            .unwrap()
            .unwrap();

        let provenance_record: PersonaTransition =
            serde_json::from_str(&provenance_entry.value).unwrap();
        let rollback_record: PersonaTransition =
            serde_json::from_str(&rollback_entry.value).unwrap();
        let rollback_latest_record: PersonaTransition =
            serde_json::from_str(&rollback_latest_entry.value).unwrap();

        assert_eq!(
            provenance_record.from_last_updated_at,
            initial.last_updated_at
        );
        assert_eq!(
            provenance_record.to_last_updated_at,
            updated.last_updated_at
        );
        assert_eq!(provenance_record.previous, initial);
        assert_eq!(provenance_record.next, updated);
        assert!(provenance_record.why.iter().any(|reason| {
            reason.field == "current_objective" && reason.summary.contains("Objective A")
        }));
        assert!(
            provenance_record
                .why
                .iter()
                .any(|reason| reason.field == "recent_context_summary")
        );
        assert_eq!(rollback_record, provenance_record);
        assert_eq!(rollback_latest_record, provenance_record);
    }

    #[tokio::test]
    async fn rollback_latest_advances_across_multiple_postgres_transitions() {
        let tmp = TempDir::new().unwrap();
        let service = service_with_postgres(&tmp, "STATE.md");

        let state_a = sample_state_at("Objective A", "Summary A", "2026-02-26T00:00:00Z");
        let state_b = sample_state_at("Objective B", "Summary B", "2026-02-26T00:05:00Z");
        let state_c = sample_state_at("Objective C", "Summary C", "2026-02-26T00:10:00Z");

        service.persist_backend_sync(&state_a).await.unwrap();
        service.persist_backend_sync(&state_b).await.unwrap();
        service.persist_backend_sync(&state_c).await.unwrap();

        let rollback_latest_entry = service
            .memory
            .resolve_slot(
                &service.person_entity_id(),
                &service.person_latest_slot_key(),
            )
            .await
            .unwrap()
            .unwrap();
        let rollback_latest_record: PersonaTransition =
            serde_json::from_str(&rollback_latest_entry.value).unwrap();

        assert_eq!(rollback_latest_record.previous, state_b);
        assert_eq!(rollback_latest_record.next, state_c);
        assert_eq!(
            rollback_latest_record.to_last_updated_at,
            state_c.last_updated_at
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn rollback_latest_advances_across_multiple_actual_postgres_transitions() {
        let _db_guard = crate::utils::test_env::acquire_test_db().await;
        let tmp = TempDir::new().unwrap();
        let (service, _env_guard) = service_with_actual_postgres(&tmp, "STATE.md").await;

        let state_a = sample_state_at("Objective A", "Summary A", "2026-02-26T00:00:00Z");
        let state_b = sample_state_at("Objective B", "Summary B", "2026-02-26T00:05:00Z");
        let state_c = sample_state_at("Objective C", "Summary C", "2026-02-26T00:10:00Z");

        service.persist_backend_sync(&state_a).await.unwrap();
        service.persist_backend_sync(&state_b).await.unwrap();
        service.persist_backend_sync(&state_c).await.unwrap();

        let rollback_latest_entry = service
            .memory
            .resolve_slot(
                &service.person_entity_id(),
                &service.person_latest_slot_key(),
            )
            .await
            .unwrap()
            .unwrap();
        let rollback_latest_record: PersonaTransition =
            serde_json::from_str(&rollback_latest_entry.value).unwrap();

        assert_eq!(rollback_latest_record.previous, state_b);
        assert_eq!(rollback_latest_record.next, state_c);
        assert_eq!(
            rollback_latest_record.to_last_updated_at,
            state_c.last_updated_at
        );
    }

    #[tokio::test]
    async fn state_header_rollback_simulation_restores_previous_state() {
        let tmp = TempDir::new().unwrap();
        let service = service_with_postgres(&tmp, "STATE.md");

        let initial = sample_state_at("Objective A", "Summary A", "2026-02-26T00:00:00Z");
        service.persist_backend_sync(&initial).await.unwrap();

        let updated = sample_state_at("Objective B", "Summary B", "2026-02-26T00:05:00Z");
        service.persist_backend_sync(&updated).await.unwrap();

        let rolled_back = service
            .rollback_to_transition(&updated.last_updated_at)
            .await
            .unwrap();

        assert_eq!(rolled_back, initial);
        let backend = service.load_backend_state().await.unwrap().unwrap();
        let mirror = service.read_mirror_state().unwrap().unwrap();
        assert_eq!(backend, initial);
        assert_eq!(mirror, initial);
    }

    #[tokio::test]
    async fn provenance_write_path_can_be_disabled() {
        let tmp = TempDir::new().unwrap();
        let service = service_with_postgres_transition_toggle(&tmp, "STATE.md", false);

        let initial = sample_state_at("Objective A", "Summary A", "2026-02-26T00:00:00Z");
        service.persist_backend_sync(&initial).await.unwrap();

        let updated = sample_state_at("Objective B", "Summary B", "2026-02-26T00:05:00Z");
        service.persist_backend_sync(&updated).await.unwrap();

        let person_entity_id = service.person_entity_id();
        let provenance_slot = service.person_provenance_slot_key(&updated.last_updated_at);
        let rollback_slot = service.person_rollback_slot_key(&updated.last_updated_at);
        let rollback_latest_slot = service.person_latest_slot_key();

        assert!(
            service
                .memory
                .resolve_slot(&person_entity_id, &provenance_slot)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            service
                .memory
                .resolve_slot(&person_entity_id, &rollback_slot)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            service
                .memory
                .resolve_slot(&person_entity_id, &rollback_latest_slot)
                .await
                .unwrap()
                .is_none()
        );
    }
}
