//! `PostgreSQL`-level memory pruning: TTL expiry, quality demotion, and
//! layer-specific cleanup.
//!
//! ## Pruning passes (in order within a single transaction)
//!
//! 1. **TTL expiry** — soft-deletes belief slots whose linked retrieval
//!    unit has an expired `retention_expires_at`, then hard-deletes them
//!    after a [`TTL_SOFT_DELETE_GRACE_DAYS`]-day grace period.
//! 2. **Low-confidence demotion** — demotes `raw`-tier units with
//!    reliability < [`LOW_CONFIDENCE_RELIABILITY_THRESHOLD`] to `demoted`.
//! 3. **Contradiction auto-demotion** — demotes `promoted`/`candidate`
//!    units whose `contradiction_penalty` exceeds
//!    [`HIGH_CONTRADICTION_PENALTY_THRESHOLD`].
//! 4. **Recency refresh** — recomputes all `recency_score` values from
//!    the stored `updated_at` timestamp (90-day linear window, 0.20 floor).
//! 5. **Stale trend demotion** — demotes trend-prefixed slots that have not
//!    been updated within [`STALE_TREND_DAYS`] days.
//! 6. **Layer cleanup** — per-layer hard-delete of soft-deleted /
//!    tombstoned slots and secret retrieval units past their retention date.
//!
//! The `deletion_ledger` is intentionally preserved as an append-only
//! hash-chain audit trail and is never pruned by this module.

#[cfg(feature = "postgres")]
use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{MemoryBackend, MemoryConfig};
#[cfg(feature = "postgres")]
use crate::contracts::ids::EntityId;

const LOW_CONFIDENCE_RELIABILITY_THRESHOLD: f64 = 0.30;
const HIGH_CONTRADICTION_PENALTY_THRESHOLD: f64 = 0.50;
const STALE_TREND_DAYS: i32 = 30;
const TTL_SOFT_DELETE_GRACE_DAYS: i64 = 7;
#[cfg(feature = "postgres")]
const RECENCY_REFRESH_BATCH_SIZE: i64 = 5_000;

/// Counters from a single lifecycle pruning pass.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub(super) struct LifecyclePruneReport {
    /// Slots hard-deleted after TTL grace period elapsed.
    pub(super) ttl_slot_hard_deleted: u64,
    /// Retrieval units purged due to TTL expiry.
    pub(super) ttl_unit_purged: u64,
    /// Low-confidence raw units demoted.
    pub(super) low_confidence_demoted: u64,
    /// Units auto-demoted due to high contradiction penalty.
    pub(super) contradiction_auto_demoted: u64,
    /// Stale trend slots demoted from promoted to candidate.
    pub(super) stale_trend_demoted: u64,
    /// Retrieval units with refreshed recency scores.
    pub(super) recency_refreshed: u64,
    /// Layer-specific cleanup operations executed.
    pub(super) layer_cleanup_actions: u64,
    /// Deletion ledger entries purged (always zero).
    pub(super) ledger_purged: u64,
}

impl LifecyclePruneReport {
    /// Sum of all actions taken in this report.
    pub(super) fn total_actions(self) -> u64 {
        self.ttl_slot_hard_deleted
            + self.ttl_unit_purged
            + self.low_confidence_demoted
            + self.contradiction_auto_demoted
            + self.stale_trend_demoted
            + self.recency_refreshed
            + self.layer_cleanup_actions
            + self.ledger_purged
    }
}

fn lifecycle_pruning_disabled(config: &MemoryConfig) -> bool {
    config.layer_retention_days("working") == 0
        && config.layer_retention_days("episodic") == 0
        && config.layer_retention_days("semantic") == 0
        && config.layer_retention_days("procedural") == 0
        && config.layer_retention_days("identity") == 0
        && config.ledger_retention_or_default() == 0
}

/// Delete inferred conversation slots older than `retention_days`.
///
/// # Errors
///
/// Returns an error when `PostgreSQL` access fails.
pub(super) fn prune_conversation_rows(
    workspace_dir: &Path,
    config: &MemoryConfig,
    retention_days: u32,
) -> Result<u64> {
    if retention_days == 0 || config.backend != MemoryBackend::Postgres {
        return Ok(0);
    }

    #[cfg(feature = "postgres")]
    {
        use sqlx_core::query::query;
        use sqlx_core::row::Row;

        let pool = open_pool(workspace_dir, config)?;
        let cutoff = Utc::now() - Duration::days(i64::from(retention_days));

        block_on_pg_result(async {
            let mut tx = pool
                .begin()
                .await
                .context("begin conversation prune transaction")?;

            let stale_slots = query(
                "SELECT entity_id, slot_key
                 FROM belief_slots
                 WHERE source = $1 AND updated_at < $2",
            )
            .bind("inferred")
            .bind(cutoff)
            .fetch_all(&mut *tx)
            .await
            .context("collect stale inferred conversation slots")?;

            query(
                "DELETE FROM retrieval_units
                 WHERE EXISTS (
                     SELECT 1
                     FROM belief_slots bs
                     WHERE bs.entity_id = retrieval_units.entity_id
                       AND bs.slot_key = retrieval_units.slot_key
                       AND bs.source = $1
                       AND bs.updated_at < $2
                 )",
            )
            .bind("inferred")
            .bind(cutoff)
            .execute(&mut *tx)
            .await
            .context("prune stale retrieval units during conversation row prune")?;

            let mut graph_owners_to_invalidate = HashSet::new();
            for row in stale_slots {
                let entity_id: String = row.get("entity_id");
                let slot_key: String = row.get("slot_key");
                let entity_id = EntityId::new(entity_id);
                let slot_key = crate::contracts::ids::SlotKey::new(slot_key);
                graph_owners_to_invalidate.insert(entity_id.clone());
                let slot_graph_entity_id = format!("slot::{entity_id}::{slot_key}");

                query(
                    "DELETE FROM graph_edges
                     WHERE owner_entity_id = $1
                       AND (from_entity_id = $2 OR to_entity_id = $2)",
                )
                .bind(entity_id.as_str())
                .bind(&slot_graph_entity_id)
                .execute(&mut *tx)
                .await
                .context("prune graph edges for stale conversation slot")?;

                query(
                    "DELETE FROM graph_entities
                     WHERE graph_entity_id = $1",
                )
                .bind(&slot_graph_entity_id)
                .execute(&mut *tx)
                .await
                .context("prune graph entity for stale conversation slot")?;
            }

            let affected = query(
                "DELETE FROM belief_slots
                 WHERE source = $1 AND updated_at < $2",
            )
            .bind("inferred")
            .bind(cutoff)
            .execute(&mut *tx)
            .await
            .context("delete stale conversation rows")?
            .rows_affected();

            tx.commit()
                .await
                .context("commit conversation prune transaction")?;
            for owner_id in graph_owners_to_invalidate {
                crate::core::memory::graphrag::activation_cache()
                    .invalidate(&owner_id)
                    .await;
            }
            Ok(affected)
        })
    }

    #[cfg(not(feature = "postgres"))]
    {
        let _ = workspace_dir;
        let _ = config;
        Ok(0)
    }
}

/// Run all lifecycle pruning passes (TTL, confidence, contradiction,
/// trend staleness, recency refresh, and layer cleanup).
///
/// # Errors
///
/// Returns an error when `PostgreSQL` access fails.
pub(super) fn prune_lifecycle_rows(
    workspace_dir: &Path,
    config: &MemoryConfig,
) -> Result<LifecyclePruneReport> {
    if config.backend != MemoryBackend::Postgres {
        return Ok(LifecyclePruneReport::default());
    }

    if lifecycle_pruning_disabled(config) {
        return Ok(LifecyclePruneReport::default());
    }

    #[cfg(feature = "postgres")]
    {
        let pool = open_pool(workspace_dir, config)?;
        let now = Utc::now();
        let ttl_grace_cutoff = now - Duration::days(TTL_SOFT_DELETE_GRACE_DAYS);

        block_on_pg_result(async {
            let mut tx = pool
                .begin()
                .await
                .context("begin lifecycle prune transaction")?;
            let mut report = apply_core_lifecycle_pruning(&mut tx, now, ttl_grace_cutoff).await?;

            prune_layers_async(&mut tx, config, &mut report).await?;

            // deletion_ledger is intentionally append-only to preserve hash-chain integrity.
            report.ledger_purged = 0;

            tx.commit()
                .await
                .context("commit lifecycle prune transaction")?;
            Ok(report)
        })
    }

    #[cfg(not(feature = "postgres"))]
    {
        let _ = workspace_dir;
        let _ = config;
        Ok(LifecyclePruneReport::default())
    }
}

#[cfg(feature = "postgres")]
async fn apply_core_lifecycle_pruning(
    tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
    now: chrono::DateTime<Utc>,
    ttl_grace_cutoff: chrono::DateTime<Utc>,
) -> Result<LifecyclePruneReport> {
    let mut report = LifecyclePruneReport::default();
    apply_ttl_pruning(tx, now, ttl_grace_cutoff, &mut report).await?;
    apply_quality_pruning(tx, now, &mut report).await?;
    Ok(report)
}

#[cfg(feature = "postgres")]
async fn apply_ttl_pruning(
    tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
    now: chrono::DateTime<Utc>,
    ttl_grace_cutoff: chrono::DateTime<Utc>,
    report: &mut LifecyclePruneReport,
) -> Result<()> {
    use sqlx_core::query::query;

    query(
        "UPDATE belief_slots
         SET status = 'soft_deleted', updated_at = $1
         WHERE status NOT IN ('soft_deleted', 'hard_deleted', 'tombstoned')
           AND EXISTS (
               SELECT 1 FROM retrieval_units
               WHERE retrieval_units.entity_id = belief_slots.entity_id
                 AND retrieval_units.slot_key = belief_slots.slot_key
                 AND retrieval_units.retention_expires_at IS NOT NULL
                 AND retrieval_units.retention_expires_at <= $1
           )",
    )
    .bind(now)
    .execute(&mut **tx)
    .await
    .context("soft-delete ttl-expired belief slots")?;

    report.ttl_slot_hard_deleted = query(
        "UPDATE belief_slots
         SET status = 'hard_deleted', updated_at = $1
         WHERE status = 'soft_deleted'
           AND updated_at <= $2
           AND EXISTS (
               SELECT 1 FROM retrieval_units
               WHERE retrieval_units.entity_id = belief_slots.entity_id
                 AND retrieval_units.slot_key = belief_slots.slot_key
                 AND retrieval_units.retention_expires_at IS NOT NULL
                 AND retrieval_units.retention_expires_at <= $2
           )",
    )
    .bind(now)
    .bind(ttl_grace_cutoff)
    .execute(&mut **tx)
    .await
    .context("hard-delete ttl-expired belief slots")?
    .rows_affected();

    report.ttl_unit_purged = query(
        "DELETE FROM retrieval_units
         WHERE retention_expires_at IS NOT NULL
           AND retention_expires_at <= $1",
    )
    .bind(ttl_grace_cutoff)
    .execute(&mut **tx)
    .await
    .context("purge ttl-expired retrieval units")?
    .rows_affected();

    Ok(())
}

#[cfg(feature = "postgres")]
async fn apply_quality_pruning(
    tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
    now: chrono::DateTime<Utc>,
    report: &mut LifecyclePruneReport,
) -> Result<()> {
    use sqlx_core::query::query;

    report.low_confidence_demoted = query(
        "UPDATE retrieval_units
         SET promotion_status = 'demoted', updated_at = $1
         WHERE promotion_status = 'raw'
           AND signal_tier = 'raw'
           AND reliability < $2",
    )
    .bind(now)
    .bind(LOW_CONFIDENCE_RELIABILITY_THRESHOLD)
    .execute(&mut **tx)
    .await
    .context("demote low-confidence retrieval units")?
    .rows_affected();

    report.contradiction_auto_demoted = query(
        "UPDATE retrieval_units
         SET promotion_status = 'demoted', updated_at = $1
         WHERE promotion_status IN ('promoted', 'candidate')
           AND signal_tier != 'governance'
           AND contradiction_penalty > $2",
    )
    .bind(now)
    .bind(HIGH_CONTRADICTION_PENALTY_THRESHOLD)
    .execute(&mut **tx)
    .await
    .context("demote contradiction-heavy retrieval units")?
    .rows_affected();

    report.recency_refreshed = refresh_retrieval_recency_scores_batched(tx, now).await?;

    report.stale_trend_demoted = query(
        "UPDATE retrieval_units
         SET promotion_status = 'candidate', updated_at = $1
         WHERE promotion_status = 'promoted'
           AND signal_tier != 'governance'
           AND (
             slot_key LIKE 'trend.%'
             OR slot_key LIKE 'trend/%'
             OR slot_key LIKE '%.trend.%'
             OR slot_key LIKE '%/trend/%'
           )
           AND updated_at <= ($1 - make_interval(days => $2))",
    )
    .bind(now)
    .bind(STALE_TREND_DAYS)
    .execute(&mut **tx)
    .await
    .context("demote stale trend retrieval units")?
    .rows_affected();

    Ok(())
}

#[cfg(feature = "postgres")]
async fn refresh_retrieval_recency_scores_batched(
    tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
    now: chrono::DateTime<Utc>,
) -> Result<u64> {
    use sqlx_core::query::query;

    let mut total = 0_u64;
    loop {
        let rows = query(
            "WITH candidates AS (
                 SELECT unit_id,
                        GREATEST(
                            0.20,
                            1.0 - (EXTRACT(EPOCH FROM ($1 - updated_at)) / 86400.0 / 90.0)
                        ) AS next_recency
                 FROM retrieval_units
                 WHERE updated_at IS NOT NULL
                   AND ABS(
                       COALESCE(recency_score, -1.0) - GREATEST(
                           0.20,
                           1.0 - (EXTRACT(EPOCH FROM ($1 - updated_at)) / 86400.0 / 90.0)
                       )
                   ) > 0.0001
                 ORDER BY updated_at ASC, unit_id ASC
                 LIMIT $2
             )
             UPDATE retrieval_units
             SET recency_score = candidates.next_recency
             FROM candidates
             WHERE retrieval_units.unit_id = candidates.unit_id",
        )
        .bind(now)
        .bind(RECENCY_REFRESH_BATCH_SIZE)
        .execute(&mut **tx)
        .await
        .context("refresh retrieval recency scores batch")?
        .rows_affected();

        total = total.saturating_add(rows);
        if rows == 0 {
            break;
        }
    }

    Ok(total)
}

#[cfg(feature = "postgres")]
async fn prune_layers_async(
    tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
    config: &MemoryConfig,
    report: &mut LifecyclePruneReport,
) -> Result<()> {
    use sqlx_core::query::query;

    let layer_purge_ops = [
        ("working", config.layer_retention_days("working")),
        ("episodic", config.layer_retention_days("episodic")),
        ("semantic", config.layer_retention_days("semantic")),
        ("procedural", config.layer_retention_days("procedural")),
        ("identity", config.layer_retention_days("identity")),
    ];

    for (layer, retention_days) in layer_purge_ops {
        if retention_days == 0 {
            continue;
        }

        let cutoff = Utc::now() - Duration::days(i64::from(retention_days));

        report.layer_cleanup_actions += query(
            "UPDATE belief_slots
             SET status = 'hard_deleted', updated_at = $1
             WHERE status = 'soft_deleted'
               AND updated_at < $1
               AND EXISTS (
                   SELECT 1 FROM retrieval_units
                   WHERE retrieval_units.entity_id = belief_slots.entity_id
                     AND retrieval_units.slot_key = belief_slots.slot_key
                     AND retrieval_units.layer = $2
               )",
        )
        .bind(cutoff)
        .bind(layer)
        .execute(&mut **tx)
        .await
        .context("layer cleanup: hard delete soft-deleted slots")?
        .rows_affected();

        report.layer_cleanup_actions += query(
            "DELETE FROM belief_slots
             WHERE status = 'tombstoned'
               AND updated_at < $1
               AND EXISTS (
                   SELECT 1 FROM retrieval_units
                   WHERE retrieval_units.entity_id = belief_slots.entity_id
                     AND retrieval_units.slot_key = belief_slots.slot_key
                     AND retrieval_units.layer = $2
               )",
        )
        .bind(cutoff)
        .bind(layer)
        .execute(&mut **tx)
        .await
        .context("layer cleanup: delete tombstoned slots")?
        .rows_affected();

        report.layer_cleanup_actions += query(
            "DELETE FROM retrieval_units
             WHERE visibility = 'secret'
               AND layer = $2
               AND updated_at < $1",
        )
        .bind(cutoff)
        .bind(layer)
        .execute(&mut **tx)
        .await
        .context("layer cleanup: delete secret retrieval units")?
        .rows_affected();
    }

    Ok(())
}

#[cfg(feature = "postgres")]
fn open_pool(
    workspace_dir: &Path,
    config: &MemoryConfig,
) -> Result<sqlx_core::pool::Pool<sqlx_postgres::Postgres>> {
    use sqlx_core::pool::PoolOptions;

    let database_url = crate::utils::postgres::require_postgres_url(
        config.postgres_url.as_deref(),
        Some(workspace_dir),
        "memory hygiene pruning",
    )?;

    block_on_pg_result(async {
        PoolOptions::<sqlx_postgres::Postgres>::new()
            .max_connections(config.pg_max_connections.max(1))
            .connect(&database_url)
            .await
            .context("connect postgres for memory hygiene")
    })
}

#[cfg(feature = "postgres")]
fn block_on_pg_result<T, F>(future: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread {
            tokio::task::block_in_place(|| handle.block_on(future))
        } else {
            anyhow::bail!(
                "memory hygiene postgres pruning requires multi-thread tokio runtime; skipping in current-thread runtime"
            );
        }
    } else {
        let runtime = tokio::runtime::Runtime::new().context("create memory hygiene runtime")?;
        runtime.block_on(future)
    }
}
