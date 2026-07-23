//! `PostgreSQL` forget operations: soft-delete, hard-delete, and tombstone modes.
//!
//! Implements the three [`ForgetMode`] variants for the `PostgreSQL` backend:
//!
//! - **`Soft`** — marks `belief_slots.status = 'soft_deleted'` and appends a
//!   `deletion_ledger` row; normal recall suppresses the slot while audit data
//!   remains in the database.
//! - **`Hard`** — physically removes the event log, projections, retrieval
//!   units, and embedding cache entries for the slot, then appends a signed
//!   `deletion_ledger` entry and rebuilds the retained event hash chain.
//! - **`Tombstone`** — writes a `deletion_ledger` row without touching the slot
//!   itself; used when the forget request must be recorded but the slot has
//!   already been removed or never existed.
//!
//! All three modes update the SHA-256 `deletion_ledger` chain via
//! `integrity::next_deletion_ledger_chain` to maintain tamper-evidence.

use sqlx_core::query::query;
use sqlx_core::row::Row;
use uuid::Uuid;

use super::PostgresMemory;
use super::error::{PostgresMemoryResult, PostgresMemoryResultExt};
use crate::contracts::scores::{Confidence, Importance};
use crate::core::memory::codec;
use crate::core::memory::embeddings::EmbeddingRole;
use crate::core::memory::traits::{
    BeliefSlot, ForgetArtifact, ForgetArtifactCheck, ForgetMode, ForgetObservation, ForgetOutcome,
};

struct ForgetContext<'a> {
    entity_id: &'a str,
    slot_key: &'a str,
    unit_id: String,
    phase: &'a str,
    reason: &'a str,
    now: &'a str,
}

struct ForgetApplication {
    applied: bool,
    cache_hashes: Vec<String>,
    projection_ids: Vec<String>,
    owner_ids: Vec<String>,
}

impl PostgresMemory {
    /// Resolve a belief slot by entity and key, falling back to graph
    /// entities and checking the deletion ledger.
    pub(super) async fn resolve_slot_impl(
        &self,
        entity_id: &str,
        slot_key: &str,
    ) -> PostgresMemoryResult<Option<BeliefSlot>> {
        // Acquire a single connection for all read queries to ensure a
        // consistent snapshot and avoid multiple pool checkouts.
        let mut conn = self
            .pool
            .acquire()
            .await
            .pg_query("acquire connection for resolve_slot")?;

        let row = query(
            "SELECT value, source, confidence, importance, privacy_level, \
                    to_char(updated_at, 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at_str \
             FROM belief_slots \
             WHERE entity_id = $1 AND slot_key = $2 AND status = 'active'",
        )
        .bind(entity_id)
        .bind(slot_key)
        .fetch_optional(&mut *conn)
        .await
        .pg_query("resolve belief slot")?;

        if let Some(row) = row {
            return Ok(Some(BeliefSlot {
                entity_id: crate::contracts::ids::EntityId::new(entity_id),
                slot_key: crate::contracts::ids::SlotKey::new(slot_key),
                value: row.get("value"),
                source: codec::str_to_source(&row.get::<String, _>("source")),
                confidence: Confidence::new(row.get::<f64, _>("confidence")),
                importance: Importance::new(row.get::<f64, _>("importance")),
                privacy_level: codec::str_to_privacy(&row.get::<String, _>("privacy_level")),
                updated_at: row.get("updated_at_str"),
            }));
        }

        // Check if a non-active slot row exists
        let slot_exists: bool = query(
            "SELECT EXISTS(SELECT 1 FROM belief_slots WHERE entity_id = $1 AND slot_key = $2)",
        )
        .bind(entity_id)
        .bind(slot_key)
        .fetch_one(&mut *conn)
        .await
        .map(|row| row.get::<bool, _>(0))
        .pg_query("check inactive belief slot existence")?;

        if slot_exists {
            return Ok(None);
        }

        let deleted: bool = query(
            "SELECT EXISTS( \
                SELECT 1 FROM deletion_ledger \
                WHERE entity_id = $1 AND target_slot_key = $2 \
                  AND phase IN ('soft', 'hard', 'tombstone') \
             )",
        )
        .bind(entity_id)
        .bind(slot_key)
        .fetch_one(&mut *conn)
        .await
        .map(|row| row.get::<bool, _>(0))
        .pg_query("check deletion ledger for slot")?;

        if deleted {
            return Ok(None);
        }

        // Fallback to graph entities
        Self::resolve_slot_from_graph(&mut conn, entity_id, slot_key).await
    }

    async fn resolve_slot_from_graph(
        conn: &mut sqlx_core::pool::PoolConnection<sqlx_postgres::Postgres>,
        entity_id: &str,
        slot_key: &str,
    ) -> PostgresMemoryResult<Option<BeliefSlot>> {
        let slot_graph_entity_id = format!("slot::{entity_id}::{slot_key}");

        let row = query(
            "SELECT value, source, confidence, importance, privacy_level, \
                    to_char(updated_at, 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at_str \
             FROM graph_entities \
             WHERE graph_entity_id = $1 AND entity_type = 'slot'",
        )
        .bind(&slot_graph_entity_id)
        .fetch_optional(&mut **conn)
        .await
        .pg_query("resolve slot from graph")?;

        Ok(row.map(|row| BeliefSlot {
            entity_id: crate::contracts::ids::EntityId::new(entity_id),
            slot_key: crate::contracts::ids::SlotKey::new(slot_key),
            value: row.get("value"),
            source: codec::str_to_source(&row.get::<String, _>("source")),
            confidence: Confidence::new(row.get::<f64, _>("confidence")),
            importance: Importance::new(row.get::<f64, _>("importance")),
            privacy_level: codec::str_to_privacy(&row.get::<String, _>("privacy_level")),
            updated_at: row.get("updated_at_str"),
        }))
    }

    /// Execute a forget operation within a transaction, recording a
    /// ledger entry and verifying artifact outcomes.
    pub(super) async fn forget_slot_impl(
        &self,
        entity_id: &str,
        slot_key: &str,
        mode: ForgetMode,
        reason: &str,
    ) -> PostgresMemoryResult<ForgetOutcome> {
        let phase = match mode {
            ForgetMode::Soft => "soft",
            ForgetMode::Hard => "hard",
            ForgetMode::Tombstone => "tombstone",
        };

        let mut tx = self
            .pool
            .begin()
            .await
            .pg_write("begin forget_slot transaction")?;
        let now = super::integrity::canonical_db_timestamp(&mut tx).await?;

        if matches!(mode, ForgetMode::Hard | ForgetMode::Tombstone) {
            super::integrity::lock_memory_event_chain(&mut tx).await?;
        }

        // Insert deletion ledger entry with integrity chain
        Self::insert_deletion_ledger_entry(&mut tx, entity_id, slot_key, phase, reason, &now)
            .await?;

        let unit_id = format!("{entity_id}::{slot_key}");
        let application =
            Self::apply_forget_mode(&mut tx, entity_id, slot_key, &unit_id, mode, &now).await?;

        let forget_ctx = ForgetContext {
            entity_id,
            slot_key,
            unit_id,
            phase,
            reason,
            now: &now,
        };
        let artifact_checks = Self::collect_artifact_checks(
            &mut tx,
            &forget_ctx,
            mode,
            &application.cache_hashes,
            &application.projection_ids,
        )
        .await?;

        tx.commit()
            .await
            .pg_write("commit forget_slot transaction")?;

        for owner_id in &application.owner_ids {
            crate::core::memory::graphrag::activation_cache()
                .invalidate(&crate::contracts::ids::EntityId::new(owner_id))
                .await;
        }

        Ok(ForgetOutcome::from_checks(
            entity_id,
            slot_key,
            mode,
            application.applied,
            false,
            artifact_checks,
        ))
    }

    pub(super) async fn insert_deletion_ledger_entry(
        tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
        entity_id: &str,
        slot_key: &str,
        phase: &str,
        reason: &str,
        now: &str,
    ) -> PostgresMemoryResult<()> {
        let ledger_id = Uuid::new_v4().to_string();
        let requested_by = "memory_forget";

        let (integrity_prev_hash, integrity_hash) = super::integrity::next_deletion_ledger_chain(
            tx,
            &super::integrity::DeletionLedgerHashFields {
                ledger_id: &ledger_id,
                entity_id,
                target_slot_key: slot_key,
                phase,
                reason,
                requested_by,
                executed_at: now,
            },
        )
        .await
        .pg_integrity("compute deletion ledger integrity chain")?;

        query(
            "INSERT INTO deletion_ledger ( \
                ledger_id, entity_id, target_slot_key, phase, reason, requested_by, \
                executed_at, integrity_prev_hash, integrity_hash \
             ) VALUES ($1, $2, $3, $4, $5, $6, $7::timestamptz, $8, $9)",
        )
        .bind(&ledger_id)
        .bind(entity_id)
        .bind(slot_key)
        .bind(phase)
        .bind(reason)
        .bind(requested_by)
        .bind(now)
        .bind(&integrity_prev_hash)
        .bind(&integrity_hash)
        .execute(&mut **tx)
        .await
        .pg_write("insert deletion ledger entry")?;

        Ok(())
    }

    async fn apply_forget_mode(
        tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
        entity_id: &str,
        slot_key: &str,
        unit_id: &str,
        mode: ForgetMode,
        now: &str,
    ) -> PostgresMemoryResult<ForgetApplication> {
        match mode {
            ForgetMode::Soft => {
                let applied =
                    Self::apply_soft_delete(tx, entity_id, slot_key, unit_id, now).await?;
                Ok(ForgetApplication {
                    applied,
                    cache_hashes: Vec::new(),
                    projection_ids: Vec::new(),
                    owner_ids: Vec::new(),
                })
            }
            ForgetMode::Hard => Self::apply_hard_delete(tx, entity_id, slot_key, unit_id).await,
            ForgetMode::Tombstone => {
                let mut application =
                    Self::apply_hard_delete(tx, entity_id, slot_key, unit_id).await?;
                application.applied =
                    Self::apply_tombstone(tx, entity_id, slot_key, unit_id, now).await?;
                Ok(application)
            }
        }
    }

    async fn apply_soft_delete(
        tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
        entity_id: &str,
        slot_key: &str,
        unit_id: &str,
        now: &str,
    ) -> PostgresMemoryResult<bool> {
        let result = query(
            "UPDATE belief_slots SET status = 'soft_deleted', updated_at = $3::timestamptz \
             WHERE entity_id = $1 AND slot_key = $2",
        )
        .bind(entity_id)
        .bind(slot_key)
        .bind(now)
        .execute(&mut **tx)
        .await
        .pg_write("soft delete belief slot")?;

        query(
            "UPDATE retrieval_units SET visibility = 'secret', updated_at = $2::timestamptz \
             WHERE unit_id = $1",
        )
        .bind(unit_id)
        .bind(now)
        .execute(&mut **tx)
        .await
        .pg_write("soft-delete retrieval unit")?;

        Ok(result.rows_affected() > 0)
    }

    #[allow(clippy::too_many_lines)]
    async fn apply_hard_delete(
        tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
        entity_id: &str,
        slot_key: &str,
        _unit_id: &str,
    ) -> PostgresMemoryResult<ForgetApplication> {
        let target_rows = query(
            "WITH RECURSIVE targets(entity_id, slot_key) AS ( \
                SELECT $1::text, $2::text \
                UNION \
                SELECT md.derived_entity_id, md.derived_slot_key \
                FROM memory_derivations md \
                JOIN targets source ON source.entity_id = md.source_entity_id \
                    AND source.slot_key = md.source_slot_key \
             ) SELECT entity_id, slot_key FROM targets",
        )
        .bind(entity_id)
        .bind(slot_key)
        .fetch_all(&mut **tx)
        .await
        .pg_query("resolve hard-delete memory lineage")?;
        let targets = target_rows
            .iter()
            .map(|row| {
                (
                    row.get::<String, _>("entity_id"),
                    row.get::<String, _>("slot_key"),
                )
            })
            .collect::<Vec<_>>();

        let mut event_rows = Vec::new();
        for (target_entity_id, target_slot_key) in &targets {
            event_rows.extend(
                query(
                    "SELECT event_id, value FROM memory_events \
                     WHERE entity_id = $1 AND slot_key = $2",
                )
                .bind(target_entity_id)
                .bind(target_slot_key)
                .fetch_all(&mut **tx)
                .await
                .pg_query("fetch hard-delete memory events")?,
            );
        }
        let event_ids = event_rows
            .iter()
            .map(|row| row.get::<String, _>("event_id"))
            .collect::<Vec<_>>();
        let event_graph_ids = event_ids
            .iter()
            .map(|event_id| format!("event::{event_id}"))
            .collect::<Vec<_>>();
        let cache_hashes = event_rows
            .iter()
            .flat_map(|row| {
                let value = row.get::<String, _>("value");
                [
                    PostgresMemory::content_hash(EmbeddingRole::Document, &value),
                    PostgresMemory::content_hash(EmbeddingRole::Query, &value),
                ]
            })
            .collect::<Vec<_>>();
        let slot_graph_ids = targets
            .iter()
            .map(|(target_entity_id, target_slot_key)| {
                format!("slot::{target_entity_id}::{target_slot_key}")
            })
            .collect::<Vec<_>>();
        let mut graph_artifact_ids = event_graph_ids.clone();
        graph_artifact_ids.extend(slot_graph_ids.iter().cloned());
        let parent_rows = query(
            "WITH RECURSIVE parents(graph_entity_id) AS ( \
                SELECT parent_graph_entity_id FROM graph_entities \
                WHERE graph_entity_id = ANY($1) AND parent_graph_entity_id IS NOT NULL \
                UNION \
                SELECT ge.parent_graph_entity_id FROM graph_entities ge \
                JOIN parents child ON child.graph_entity_id = ge.graph_entity_id \
                WHERE ge.parent_graph_entity_id IS NOT NULL \
             ) SELECT graph_entity_id FROM parents",
        )
        .bind(&graph_artifact_ids)
        .fetch_all(&mut **tx)
        .await
        .pg_query("resolve hard-delete graph lineage")?;
        let parent_ids = parent_rows
            .iter()
            .map(|row| row.get::<String, _>("graph_entity_id"))
            .collect::<Vec<_>>();
        graph_artifact_ids.extend(parent_ids.iter().cloned());
        graph_artifact_ids.sort();
        graph_artifact_ids.dedup();

        query(
            "UPDATE graph_entities SET parent_graph_entity_id = NULL, promoted_at = NULL \
             WHERE parent_graph_entity_id = ANY($1) AND NOT (graph_entity_id = ANY($2))",
        )
        .bind(&parent_ids)
        .bind(&graph_artifact_ids)
        .execute(&mut **tx)
        .await
        .pg_write("detach retained graph episodes from deleted notes")?;

        query(
            "DELETE FROM graph_edges \
             WHERE from_entity_id = ANY($1) OR to_entity_id = ANY($1) \
                OR event_id = ANY($2)",
        )
        .bind(&graph_artifact_ids)
        .bind(&event_ids)
        .execute(&mut **tx)
        .await
        .pg_write("hard-delete graph edges")?;

        query(
            "DELETE FROM graph_entity_aliases \
             WHERE canonical_graph_entity_id = ANY($1)",
        )
        .bind(&graph_artifact_ids)
        .execute(&mut **tx)
        .await
        .pg_write("hard-delete graph entity aliases")?;

        query(
            "DELETE FROM graph_entities \
             WHERE graph_entity_id = ANY($1)",
        )
        .bind(&graph_artifact_ids)
        .execute(&mut **tx)
        .await
        .pg_write("hard-delete graph entities")?;

        for cache_hash in &cache_hashes {
            query("DELETE FROM embedding_cache WHERE content_hash = $1")
                .bind(cache_hash)
                .execute(&mut **tx)
                .await
                .pg_write("hard-delete embedding cache entry")?;
        }

        let mut deleted_projection_rows = 0_u64;
        for (target_entity_id, target_slot_key) in &targets {
            query("DELETE FROM memory_events WHERE entity_id = $1 AND slot_key = $2")
                .bind(target_entity_id)
                .bind(target_slot_key)
                .execute(&mut **tx)
                .await
                .pg_write("hard-delete memory events")?;

            deleted_projection_rows +=
                query("DELETE FROM belief_slots WHERE entity_id = $1 AND slot_key = $2")
                    .bind(target_entity_id)
                    .bind(target_slot_key)
                    .execute(&mut **tx)
                    .await
                    .pg_write("hard-delete belief slot")?
                    .rows_affected();

            deleted_projection_rows +=
                query("DELETE FROM retrieval_units WHERE entity_id = $1 AND slot_key = $2")
                    .bind(target_entity_id)
                    .bind(target_slot_key)
                    .execute(&mut **tx)
                    .await
                    .pg_write("hard-delete retrieval units")?
                    .rows_affected();

            query(
                "DELETE FROM memory_derivations \
                 WHERE (derived_entity_id = $1 AND derived_slot_key = $2) \
                    OR (source_entity_id = $1 AND source_slot_key = $2)",
            )
            .bind(target_entity_id)
            .bind(target_slot_key)
            .execute(&mut **tx)
            .await
            .pg_write("hard-delete memory lineage")?;
        }
        super::integrity::rebuild_memory_event_chain(tx).await?;

        let projection_ids = graph_artifact_ids;
        let mut owner_ids = targets
            .iter()
            .map(|(target_entity_id, _)| target_entity_id.clone())
            .collect::<Vec<_>>();
        owner_ids.sort();
        owner_ids.dedup();

        Ok(ForgetApplication {
            applied: deleted_projection_rows > 0 || !event_ids.is_empty(),
            cache_hashes,
            projection_ids,
            owner_ids,
        })
    }

    async fn apply_tombstone(
        tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
        entity_id: &str,
        slot_key: &str,
        unit_id: &str,
        now: &str,
    ) -> PostgresMemoryResult<bool> {
        query(
            "INSERT INTO belief_slots ( \
                entity_id, slot_key, value, status, winner_event_id, source, \
                confidence, importance, privacy_level, updated_at \
             ) VALUES ($1, $2, '', 'tombstoned', $3, 'system', 1.0, 1.0, 'secret', $4::timestamptz) \
             ON CONFLICT (entity_id, slot_key) DO UPDATE SET \
                value = EXCLUDED.value, \
                status = EXCLUDED.status, \
                winner_event_id = EXCLUDED.winner_event_id, \
                source = EXCLUDED.source, \
                confidence = EXCLUDED.confidence, \
                importance = EXCLUDED.importance, \
                privacy_level = EXCLUDED.privacy_level, \
                updated_at = EXCLUDED.updated_at",
        )
        .bind(entity_id)
        .bind(slot_key)
        .bind(Uuid::new_v4().to_string())
        .bind(now)
        .execute(&mut **tx)
        .await
        .pg_write("tombstone belief slot")?;

        query("DELETE FROM retrieval_units WHERE unit_id = $1")
            .bind(unit_id)
            .execute(&mut **tx)
            .await
            .pg_write("tombstone-delete retrieval unit")?;

        Ok(true)
    }

    async fn collect_artifact_checks(
        tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
        ctx: &ForgetContext<'_>,
        mode: ForgetMode,
        cache_hashes: &[String],
        projection_ids: &[String],
    ) -> PostgresMemoryResult<Vec<ForgetArtifactCheck>> {
        let slot_observed = Self::observe_slot_artifact(tx, ctx.entity_id, ctx.slot_key).await;
        let retrieval_observed = Self::observe_retrieval_artifact(tx, &ctx.unit_id).await;
        let documents_observed =
            Self::observe_document_artifacts(tx, ctx.entity_id, ctx.slot_key, projection_ids)
                .await?;
        let cache_observed = Self::observe_cache_artifacts(tx, cache_hashes).await?;
        let ledger_observed = Self::observe_ledger_artifact(
            tx,
            ctx.entity_id,
            ctx.slot_key,
            ctx.phase,
            ctx.reason,
            ctx.now,
        )
        .await?;

        Ok(vec![
            ForgetArtifactCheck::new(
                ForgetArtifact::Slot,
                mode.artifact_requirement(ForgetArtifact::Slot),
                slot_observed,
            ),
            ForgetArtifactCheck::new(
                ForgetArtifact::RetrievalUnits,
                mode.artifact_requirement(ForgetArtifact::RetrievalUnits),
                retrieval_observed,
            ),
            ForgetArtifactCheck::new(
                ForgetArtifact::RetrievalDocs,
                mode.artifact_requirement(ForgetArtifact::RetrievalDocs),
                documents_observed,
            ),
            ForgetArtifactCheck::new(
                ForgetArtifact::Caches,
                mode.artifact_requirement(ForgetArtifact::Caches),
                cache_observed,
            ),
            ForgetArtifactCheck::new(
                ForgetArtifact::Ledger,
                mode.artifact_requirement(ForgetArtifact::Ledger),
                ledger_observed,
            ),
        ])
    }

    async fn observe_slot_artifact(
        tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
        entity_id: &str,
        slot_key: &str,
    ) -> ForgetObservation {
        let status: Option<String> =
            query("SELECT status FROM belief_slots WHERE entity_id = $1 AND slot_key = $2")
                .bind(entity_id)
                .bind(slot_key)
                .fetch_optional(&mut **tx)
                .await
                .ok()
                .flatten()
                .map(|row| row.get("status"));

        match status.as_deref() {
            None => ForgetObservation::Absent,
            Some("active") => ForgetObservation::PresentRetrievable,
            Some(_) => ForgetObservation::PresentNonRetrievable,
        }
    }

    async fn observe_retrieval_artifact(
        tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
        unit_id: &str,
    ) -> ForgetObservation {
        let visibility: Option<String> =
            query("SELECT visibility FROM retrieval_units WHERE unit_id = $1")
                .bind(unit_id)
                .fetch_optional(&mut **tx)
                .await
                .ok()
                .flatten()
                .map(|row| row.get("visibility"));

        match visibility.as_deref() {
            None => ForgetObservation::Absent,
            Some("secret") => ForgetObservation::PresentNonRetrievable,
            Some(_) => ForgetObservation::PresentRetrievable,
        }
    }

    async fn observe_document_artifacts(
        tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
        entity_id: &str,
        slot_key: &str,
        projection_ids: &[String],
    ) -> PostgresMemoryResult<ForgetObservation> {
        let slot_graph_id = format!("slot::{entity_id}::{slot_key}");
        let exists: bool = query(
            "SELECT EXISTS( \
                SELECT 1 FROM memory_events WHERE entity_id = $1 AND slot_key = $2 \
                UNION ALL \
                SELECT 1 FROM graph_entities WHERE graph_entity_id = $3 \
                    OR graph_entity_id = ANY($4) \
                    OR graph_entity_id IN ( \
                        SELECT 'event::' || event_id FROM memory_events \
                        WHERE entity_id = $1 AND slot_key = $2 \
                    ) \
             )",
        )
        .bind(entity_id)
        .bind(slot_key)
        .bind(&slot_graph_id)
        .bind(projection_ids)
        .fetch_one(&mut **tx)
        .await
        .map(|row| row.get::<bool, _>(0))
        .pg_query("observe forget document artifacts")?;

        Ok(if exists {
            ForgetObservation::PresentNonRetrievable
        } else {
            ForgetObservation::Absent
        })
    }

    async fn observe_cache_artifacts(
        tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
        cache_hashes: &[String],
    ) -> PostgresMemoryResult<ForgetObservation> {
        if cache_hashes.is_empty() {
            return Ok(ForgetObservation::Absent);
        }

        let exists: bool =
            query("SELECT EXISTS(SELECT 1 FROM embedding_cache WHERE content_hash = ANY($1))")
                .bind(cache_hashes)
                .fetch_one(&mut **tx)
                .await
                .map(|row| row.get::<bool, _>(0))
                .pg_query("observe forget cache artifacts")?;

        Ok(if exists {
            ForgetObservation::PresentRetrievable
        } else {
            ForgetObservation::Absent
        })
    }

    async fn observe_ledger_artifact(
        tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
        entity_id: &str,
        slot_key: &str,
        phase: &str,
        reason: &str,
        executed_at: &str,
    ) -> PostgresMemoryResult<ForgetObservation> {
        let exists: bool = query(
            "SELECT EXISTS( \
                SELECT 1 FROM deletion_ledger \
                WHERE entity_id = $1 AND target_slot_key = $2 \
                  AND phase = $3 AND reason = $4 AND executed_at = $5::timestamptz \
             )",
        )
        .bind(entity_id)
        .bind(slot_key)
        .bind(phase)
        .bind(reason)
        .bind(executed_at)
        .fetch_one(&mut **tx)
        .await
        .map(|row| row.get::<bool, _>(0))
        .pg_query("check deletion ledger entry")?;

        Ok(if exists {
            ForgetObservation::PresentNonRetrievable
        } else {
            ForgetObservation::Absent
        })
    }

    /// Count persisted memory events, optionally filtered by entity.
    pub(super) async fn count_events_impl(
        &self,
        entity_id: Option<&str>,
    ) -> PostgresMemoryResult<usize> {
        let count: i64 = if let Some(entity) = entity_id {
            query("SELECT COUNT(*) FROM memory_events WHERE entity_id = $1")
                .bind(entity)
                .fetch_one(&self.pool)
                .await
                .map(|row| row.get::<i64, _>(0))
                .pg_query("count memory events by entity")?
        } else {
            query("SELECT COUNT(*) FROM memory_events")
                .fetch_one(&self.pool)
                .await
                .map(|row| row.get::<i64, _>(0))
                .pg_query("count all memory events")?
        };

        // Cast safety: SQL COUNT(*) is non-negative and event totals remain within usize.
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        Ok(count as usize)
    }

    /// Compute the fraction of retrieval units with a non-zero
    /// contradiction penalty.
    pub(super) async fn contradiction_ratio_impl(&self) -> PostgresMemoryResult<f64> {
        let row = query(
            "SELECT \
                COUNT(*) FILTER (WHERE contradiction_penalty > 0.0) AS contradicted, \
                COUNT(*) AS total \
             FROM retrieval_units",
        )
        .fetch_one(&self.pool)
        .await
        .pg_query("compute contradiction ratio")?;

        let contradicted: i64 = row.get("contradicted");
        let total: i64 = row.get("total");

        if total == 0 {
            return Ok(0.0);
        }

        // Cast safety: contradicted/total counters are bounded by retrieval-unit cardinality.
        #[allow(clippy::cast_precision_loss)]
        Ok(contradicted as f64 / total as f64)
    }
}
