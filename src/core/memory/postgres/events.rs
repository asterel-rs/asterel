//! `PostgreSQL` forget operations: soft-delete, hard-delete, and tombstone modes.
//!
//! Implements the three [`ForgetMode`] variants for the `PostgreSQL` backend:
//!
//! - **`Soft`** — marks `belief_slots.status = 'soft_deleted'` and appends a
//!   `deletion_ledger` row; normal recall suppresses the slot while audit data
//!   remains in the database.
//! - **`Hard`** — physically removes the `belief_slots` row and appends a
//!   signed `deletion_ledger` entry; the slot is unrecoverable after commit.
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
        let now = chrono::Local::now().to_rfc3339();
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

        // Insert deletion ledger entry with integrity chain
        Self::insert_deletion_ledger_entry(&mut tx, entity_id, slot_key, phase, reason, &now)
            .await?;

        let unit_id = format!("{entity_id}::{slot_key}");
        let applied =
            Self::apply_forget_mode(&mut tx, entity_id, slot_key, &unit_id, mode, &now).await?;

        let forget_ctx = ForgetContext {
            entity_id,
            slot_key,
            unit_id,
            phase,
            reason,
            now: &now,
        };
        let artifact_checks = Self::collect_artifact_checks(&mut tx, &forget_ctx, mode).await?;

        tx.commit()
            .await
            .pg_write("commit forget_slot transaction")?;

        Ok(ForgetOutcome::from_checks(
            entity_id,
            slot_key,
            mode,
            applied,
            false,
            artifact_checks,
        ))
    }

    async fn insert_deletion_ledger_entry(
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
    ) -> PostgresMemoryResult<bool> {
        match mode {
            ForgetMode::Soft => {
                Self::apply_soft_delete(tx, entity_id, slot_key, unit_id, now).await
            }
            ForgetMode::Hard => Self::apply_hard_delete(tx, entity_id, slot_key, unit_id).await,
            ForgetMode::Tombstone => {
                Self::apply_tombstone(tx, entity_id, slot_key, unit_id, now).await
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

    async fn apply_hard_delete(
        tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
        entity_id: &str,
        slot_key: &str,
        unit_id: &str,
    ) -> PostgresMemoryResult<bool> {
        let result = query("DELETE FROM belief_slots WHERE entity_id = $1 AND slot_key = $2")
            .bind(entity_id)
            .bind(slot_key)
            .execute(&mut **tx)
            .await
            .pg_write("delete belief slot")?;

        query("DELETE FROM retrieval_units WHERE unit_id = $1")
            .bind(unit_id)
            .execute(&mut **tx)
            .await
            .pg_write("hard-delete retrieval unit")?;

        Ok(result.rows_affected() > 0)
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
    ) -> PostgresMemoryResult<Vec<ForgetArtifactCheck>> {
        let slot_observed = Self::observe_slot_artifact(tx, ctx.entity_id, ctx.slot_key).await;
        let retrieval_observed = Self::observe_retrieval_artifact(tx, &ctx.unit_id).await;
        let cache_observed = ForgetObservation::Absent;
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
