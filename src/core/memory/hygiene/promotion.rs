//! Working-memory promotion: lifts expiring high-value working slots
//! into the episodic layer before their TTL expires.
//!
//! A working slot is eligible for promotion when it meets at least one
//! of the following quality signals:
//!
//! - `importance >= HIGH_IMPORTANCE_THRESHOLD` (unconditional), or
//! - `importance >= IMPORTANCE_THRESHOLD` **and** `reliability >= RELIABILITY_THRESHOLD`
//!   **and** `access_count >= ACCESS_COUNT_THRESHOLD` (composite gate).
//!
//! Promotion writes a new `episodic.promoted.*` retrieval unit and a
//! corresponding `memory_events` row with a 30-day episodic retention
//! window and a provenance reference of the form
//! `memory.promotion.from:<original_slot_key>`. The `ON CONFLICT DO NOTHING`
//! guard prevents duplicate promotion across successive hygiene runs.

use std::path::Path;

use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{MemoryBackend, MemoryConfig};
use crate::contracts::ids::{EntityId, SlotKey};

const PROMOTION_WINDOW_HOURS: i64 = 6;
const HIGH_IMPORTANCE_THRESHOLD: f64 = 0.75;
const IMPORTANCE_THRESHOLD: f64 = 0.55;
const RELIABILITY_THRESHOLD: f64 = 0.70;
const ACCESS_COUNT_THRESHOLD: i64 = 2;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub(super) struct PromotionReport {
    pub(super) promoted_count: u64,
}

#[cfg(feature = "postgres")]
#[derive(Debug)]
struct PromotionCandidate {
    entity_id: EntityId,
    original_slot_key: SlotKey,
    content: String,
    signal_tier: String,
    importance: f64,
    reliability: f64,
    visibility: String,
}

pub(super) fn promote_expiring_working_memories(
    workspace_dir: &Path,
    config: &MemoryConfig,
) -> Result<PromotionReport> {
    if config.backend != MemoryBackend::Postgres {
        return Ok(PromotionReport::default());
    }

    #[cfg(feature = "postgres")]
    {
        let pool = open_pool(workspace_dir, config)?;
        let now = Utc::now();
        let promotion_cutoff = now + Duration::hours(PROMOTION_WINDOW_HOURS);

        block_on_pg_result(async {
            let mut tx = pool
                .begin()
                .await
                .context("begin working-memory promotion transaction")?;
            crate::core::memory::postgres::integrity::lock_memory_event_chain(&mut tx).await?;
            let now_rfc3339 =
                crate::core::memory::postgres::integrity::canonical_db_timestamp(&mut tx).await?;
            let episodic_expiry_rfc3339 = crate::core::memory::codec::retention_expiry_for_layer(
                crate::core::memory::MemoryLayer::Episodic,
                &now_rfc3339,
            )
            .context("compute episodic promotion retention expiry")?;

            let candidates = load_promotion_candidates(&mut tx, now, promotion_cutoff).await?;

            let mut promoted_count = 0_u64;
            for candidate in candidates {
                let promoted_slot_key =
                    format!("episodic.promoted.{}", candidate.original_slot_key);
                let promoted_unit_id = format!("{}::{promoted_slot_key}", candidate.entity_id);

                let retrieval_inserted = insert_promoted_retrieval_unit(
                    &mut tx,
                    &candidate,
                    &promoted_unit_id,
                    &promoted_slot_key,
                    &episodic_expiry_rfc3339,
                    &now_rfc3339,
                )
                .await?;

                if retrieval_inserted == 0 {
                    continue;
                }

                insert_promoted_memory_event(
                    &mut tx,
                    &candidate,
                    &promoted_slot_key,
                    &episodic_expiry_rfc3339,
                    &now_rfc3339,
                )
                .await?;

                promoted_count += 1;
            }

            tx.commit()
                .await
                .context("commit working-memory promotion transaction")?;

            Ok(PromotionReport { promoted_count })
        })
    }

    #[cfg(not(feature = "postgres"))]
    {
        let _ = workspace_dir;
        let _ = config;
        Ok(PromotionReport::default())
    }
}

#[cfg(feature = "postgres")]
async fn load_promotion_candidates(
    tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
    now: chrono::DateTime<Utc>,
    promotion_cutoff: chrono::DateTime<Utc>,
) -> Result<Vec<PromotionCandidate>> {
    use sqlx_core::query::query;
    use sqlx_core::row::Row;

    let rows = query(
        "SELECT ru.entity_id, ru.slot_key, ru.content, ru.signal_tier, \
                ru.importance, ru.reliability, ru.visibility \
         FROM retrieval_units ru \
         WHERE ru.layer = 'working' \
           AND ru.retention_expires_at IS NOT NULL \
           AND ru.retention_expires_at > $1 \
           AND ru.retention_expires_at <= $2 \
           AND ( \
               ru.importance >= $3 \
               OR ( \
                   ru.importance >= $4 \
                   AND ru.reliability >= $5 \
                   AND ru.access_count >= $6 \
               ) \
           ) \
           AND NOT EXISTS ( \
               SELECT 1 \
               FROM memory_events me \
               WHERE me.entity_id = ru.entity_id \
                 AND me.layer = 'episodic' \
                 AND me.provenance_reference = ('memory.promotion.from:' || ru.slot_key) \
           )",
    )
    .bind(now)
    .bind(promotion_cutoff)
    .bind(HIGH_IMPORTANCE_THRESHOLD)
    .bind(IMPORTANCE_THRESHOLD)
    .bind(RELIABILITY_THRESHOLD)
    .bind(ACCESS_COUNT_THRESHOLD)
    .fetch_all(&mut **tx)
    .await
    .context("load expiring working memory promotion candidates")?;

    Ok(rows
        .into_iter()
        .map(|row| PromotionCandidate {
            entity_id: EntityId::new(row.get::<String, _>("entity_id")),
            original_slot_key: SlotKey::new(row.get::<String, _>("slot_key")),
            content: row.get("content"),
            signal_tier: row.get("signal_tier"),
            importance: row.get("importance"),
            reliability: row.get("reliability"),
            visibility: row.get("visibility"),
        })
        .collect())
}

#[cfg(feature = "postgres")]
async fn insert_promoted_retrieval_unit(
    tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
    candidate: &PromotionCandidate,
    promoted_unit_id: &str,
    promoted_slot_key: &str,
    episodic_expiry_rfc3339: &str,
    now_rfc3339: &str,
) -> Result<u64> {
    use sqlx_core::query::query;

    query(
        "INSERT INTO retrieval_units ( \
            unit_id, entity_id, slot_key, content, content_type, signal_tier, \
            promotion_status, recency_score, importance, reliability, visibility, \
            layer, retention_tier, retention_expires_at, created_at, updated_at \
         ) VALUES ( \
            $1, $2, $3, $4, 'belief', $5, \
            'promoted', 1.0, $6, $7, $8, \
            'episodic', 'episodic', $9::timestamptz, $10::timestamptz, $11::timestamptz \
         ) ON CONFLICT (unit_id) DO NOTHING",
    )
    .bind(promoted_unit_id)
    .bind(candidate.entity_id.as_str())
    .bind(promoted_slot_key)
    .bind(&candidate.content)
    .bind(&candidate.signal_tier)
    .bind(candidate.importance)
    .bind(candidate.reliability)
    .bind(&candidate.visibility)
    .bind(episodic_expiry_rfc3339)
    .bind(now_rfc3339)
    .bind(now_rfc3339)
    .execute(&mut **tx)
    .await
    .context("insert episodic retrieval unit from working promotion")
    .map(|result| result.rows_affected())
}

#[cfg(feature = "postgres")]
async fn insert_promoted_memory_event(
    tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
    candidate: &PromotionCandidate,
    promoted_slot_key: &str,
    episodic_expiry_rfc3339: &str,
    now_rfc3339: &str,
) -> Result<()> {
    use sqlx_core::query::query;
    use uuid::Uuid;

    let event_id = Uuid::new_v4().to_string();
    let source_ref = format!("memory.promotion.from:{}", candidate.original_slot_key);

    let hash_fields = crate::core::memory::postgres::integrity::MemoryEventHashFields {
        event_id: &event_id,
        entity_id: candidate.entity_id.as_str(),
        slot_key: promoted_slot_key,
        layer: "episodic",
        event_type: "upsert",
        value: &candidate.content,
        source: "system",
        confidence: candidate.reliability,
        importance: candidate.importance,
        provenance_source_class: None,
        provenance_reference: Some(&source_ref),
        provenance_evidence_uri: None,
        retention_tier: "episodic",
        retention_expires_at: Some(episodic_expiry_rfc3339),
        signal_tier: &candidate.signal_tier,
        source_kind: None,
        privacy_level: &candidate.visibility,
        occurred_at: now_rfc3339,
        ingested_at: now_rfc3339,
        supersedes_event_id: None,
    };
    let (integrity_prev_hash, integrity_hash) =
        crate::core::memory::postgres::integrity::next_memory_event_chain(tx, &hash_fields).await?;

    query(
        "INSERT INTO memory_events ( \
            event_id, entity_id, slot_key, layer, event_type, value, source, \
            confidence, importance, provenance_reference, retention_tier, \
            retention_expires_at, signal_tier, privacy_level, occurred_at, ingested_at, \
            integrity_prev_hash, integrity_hash \
         ) VALUES ( \
            $1, $2, $3, 'episodic', 'upsert', $4, 'system', \
            $5, $6, $7, 'episodic', $8::timestamptz, $9, $10, \
            $11::timestamptz, $12::timestamptz, \
            $13, $14 \
         )",
    )
    .bind(&event_id)
    .bind(candidate.entity_id.as_str())
    .bind(promoted_slot_key)
    .bind(&candidate.content)
    .bind(candidate.reliability)
    .bind(candidate.importance)
    .bind(&source_ref)
    .bind(episodic_expiry_rfc3339)
    .bind(&candidate.signal_tier)
    .bind(&candidate.visibility)
    .bind(now_rfc3339)
    .bind(now_rfc3339)
    .bind(&integrity_prev_hash)
    .bind(&integrity_hash)
    .execute(&mut **tx)
    .await
    .context("insert episodic memory event from working promotion")?;

    query(
        "INSERT INTO memory_derivations ( \
            derived_entity_id, derived_slot_key, source_entity_id, source_slot_key \
         ) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(candidate.entity_id.as_str())
    .bind(promoted_slot_key)
    .bind(candidate.entity_id.as_str())
    .bind(candidate.original_slot_key.as_str())
    .execute(&mut **tx)
    .await
    .context("record promoted memory lineage")?;

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
        "memory hygiene promotion",
    )?;

    block_on_pg_result(async {
        PoolOptions::<sqlx_postgres::Postgres>::new()
            .max_connections(config.pg_max_connections.max(1))
            .connect(&database_url)
            .await
            .context("connect postgres for memory hygiene promotion")
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
                "memory hygiene postgres promotion requires multi-thread tokio runtime; skipping in current-thread runtime"
            );
        }
    } else {
        let runtime = tokio::runtime::Runtime::new().context("create memory hygiene runtime")?;
        runtime.block_on(future)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ACCESS_COUNT_THRESHOLD, HIGH_IMPORTANCE_THRESHOLD, IMPORTANCE_THRESHOLD,
        RELIABILITY_THRESHOLD,
    };

    fn should_promote_by_signals(importance: f64, reliability: f64, access_count: i64) -> bool {
        importance >= HIGH_IMPORTANCE_THRESHOLD
            || (importance >= IMPORTANCE_THRESHOLD
                && reliability >= RELIABILITY_THRESHOLD
                && access_count >= ACCESS_COUNT_THRESHOLD)
    }

    #[test]
    fn promotes_when_importance_is_high_even_with_low_access_count() {
        assert!(should_promote_by_signals(0.80, 0.10, 0));
    }

    #[test]
    fn promotes_when_composite_signal_thresholds_are_met() {
        assert!(should_promote_by_signals(0.55, 0.70, 2));
        assert!(should_promote_by_signals(0.60, 0.90, 3));
    }

    #[test]
    fn does_not_promote_when_composite_signals_are_incomplete() {
        assert!(!should_promote_by_signals(0.54, 0.95, 8));
        assert!(!should_promote_by_signals(0.70, 0.69, 10));
        assert!(!should_promote_by_signals(0.70, 0.90, 1));
    }
}
