//! `PostgreSQL` I/O for sleep consolidation.
//!
//! Provides the database operations used by `sleep::run_sleep_consolidation`:
//!
//! - Loading candidate retrieval units updated within the scan window, grouped
//!   by `(entity_id, topic_prefix)`.
//! - Refreshing `temporal_decay_score` on `graph_entities` rows using the
//!   `compute_decay_score` formula (importance × recency × access bonus).
//! - Promoting dense episode clusters to `Note`-tier graph entities when
//!   average cosine similarity exceeds [`PROMOTION_SIMILARITY_THRESHOLD`].
//! - Inserting `semantic.sleep.*` snapshot retrieval units and events with
//!   `ON CONFLICT (unit_id) DO NOTHING` to make runs idempotent.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use pgvector::HalfVector;
use sqlx_core::pool::{Pool, PoolOptions};
use sqlx_core::query::query;
use sqlx_core::row::Row;
use sqlx_postgres::Postgres;
use uuid::Uuid;

use super::aggregation::topic_prefix_from_slot_key;
use super::{
    ConsolidationCandidate, GroupAggregate, GroupKey, MIN_PROMOTION_GROUP_SIZE,
    PROMOTION_SIMILARITY_THRESHOLD, compute_decay_score, promote_episode_group_to_note,
};
use crate::config::MemoryConfig;
use crate::contracts::ids::EntityId;
use crate::core::memory::codec;
use crate::core::memory::vector::cosine_similarity;
use crate::core::memory::{GraphEntity, GraphEntityType, NodeTier};

type PgTx<'a> = sqlx_core::transaction::Transaction<'a, Postgres>;

#[derive(Debug)]
struct PromotionCandidate {
    graph_entity: GraphEntity,
    embedding: Option<Vec<f32>>,
}

pub(super) async fn refresh_graph_entity_decay_scores(
    tx: &mut PgTx<'_>,
    owner_entity_id: &str,
    now: DateTime<Utc>,
) -> Result<u64> {
    let rows = query(
        "SELECT graph_entity_id, importance, access_count, pinned, \
                last_accessed_at, updated_at \
         FROM graph_entities \
         WHERE owner_entity_id = $1",
    )
    .bind(owner_entity_id)
    .fetch_all(&mut **tx)
    .await
    .context("load graph entities for decay refresh")?;

    let mut updated = 0_u64;

    for row in rows {
        let graph_entity_id: String = row.get("graph_entity_id");
        let importance: f64 = row.get("importance");
        let access_count_i64: i64 = row.get("access_count");
        let access_count = u64::try_from(access_count_i64).unwrap_or_default();
        let pinned: bool = row.get("pinned");
        let last_accessed_at: Option<DateTime<Utc>> = row.try_get("last_accessed_at").ok();
        let updated_at: DateTime<Utc> = row.get("updated_at");
        let reference_time = last_accessed_at.unwrap_or(updated_at);
        let std_delta = now
            .signed_duration_since(reference_time)
            .to_std()
            .unwrap_or_default();
        let hours_since_access = std_delta.as_secs_f64() / 3600.0;
        let temporal_decay_score = if pinned {
            1.0
        } else {
            compute_decay_score(importance, hours_since_access, access_count)
        };

        updated += query(
            "UPDATE graph_entities \
             SET temporal_decay_score = $1, \
                 last_accessed_at = COALESCE(last_accessed_at, updated_at) \
             WHERE owner_entity_id = $2 \
               AND graph_entity_id = $3",
        )
        .bind(temporal_decay_score)
        .bind(owner_entity_id)
        .bind(&graph_entity_id)
        .execute(&mut **tx)
        .await
        .context("update graph entity decay score")?
        .rows_affected();

        if let Some(unit_id) = graph_entity_id.strip_prefix("slot::") {
            query(
                "UPDATE retrieval_units \
                 SET access_count = $1, \
                     accessed_at = $2, \
                     pinned = $3, \
                     recency_score = $4 \
                 WHERE unit_id = $5",
            )
            .bind(i64::try_from(access_count).unwrap_or(i64::MAX))
            .bind(last_accessed_at)
            .bind(pinned)
            .bind(temporal_decay_score)
            .bind(unit_id)
            .execute(&mut **tx)
            .await
            .context("update retrieval unit decay projection")?;
        }
    }

    Ok(updated)
}

pub(super) async fn load_sleep_consolidation_groups(
    tx: &mut PgTx<'_>,
    scan_cutoff: DateTime<Utc>,
) -> Result<HashMap<GroupKey, Vec<ConsolidationCandidate>>> {
    let rows = query(
        "SELECT entity_id, slot_key, content, signal_tier, importance, reliability, visibility \
         FROM retrieval_units \
         WHERE layer = 'episodic' \
           AND promotion_status = 'promoted' \
           AND created_at > $1",
    )
    .bind(scan_cutoff)
    .fetch_all(&mut **tx)
    .await
    .context("load sleep consolidation candidates")?;

    let mut grouped: HashMap<GroupKey, Vec<ConsolidationCandidate>> = HashMap::new();
    for row in rows {
        let entity_id: String = row.get("entity_id");
        let slot_key: String = row.get("slot_key");
        let topic_prefix = topic_prefix_from_slot_key(&slot_key);

        grouped
            .entry(GroupKey {
                entity_id: EntityId::new(entity_id.clone()),
                topic_prefix,
            })
            .or_default()
            .push(ConsolidationCandidate {
                slot_key,
                content: row.get("content"),
                signal_tier: row.get("signal_tier"),
                importance: row.get("importance"),
                reliability: row.get("reliability"),
                visibility: row.get("visibility"),
            });
    }

    Ok(grouped)
}

pub(super) async fn promote_sleep_episode_groups(
    tx: &mut PgTx<'_>,
    owner_entity_id: &EntityId,
    now_rfc3339: &str,
) -> Result<u64> {
    let grouped = load_episode_promotion_candidates(tx, owner_entity_id.as_str()).await?;
    let mut promoted_groups = 0_u64;

    for candidates in grouped.into_values() {
        let promotion_group = select_promotion_group(candidates);
        if promotion_group.len() < MIN_PROMOTION_GROUP_SIZE {
            continue;
        }

        persist_promoted_note(tx, &promotion_group, now_rfc3339).await?;
        promoted_groups += 1;
    }

    Ok(promoted_groups)
}

async fn load_episode_promotion_candidates(
    tx: &mut PgTx<'_>,
    owner_entity_id: &str,
) -> Result<HashMap<String, Vec<PromotionCandidate>>> {
    let rows = query(
        "SELECT ge.graph_entity_id, ge.label, ge.value, ge.source, ge.confidence, ge.importance, \
                ge.access_count, ge.pinned, ge.temporal_decay_score, ge.privacy_level, \
                to_char(ge.updated_at, 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at_str, \
                ru.embedding \
         FROM graph_entities ge \
         LEFT JOIN retrieval_units ru ON ge.graph_entity_id = ('slot::' || ru.unit_id) \
         WHERE ge.owner_entity_id = $1 \
           AND ge.entity_type = 'slot' \
           AND ge.node_tier = 'episode' \
           AND ge.parent_graph_entity_id IS NULL",
    )
    .bind(owner_entity_id)
    .fetch_all(&mut **tx)
    .await
    .context("load episode promotion candidates")?;

    let mut grouped: HashMap<String, Vec<PromotionCandidate>> = HashMap::new();
    for row in rows {
        let label: String = row.get("label");
        let embedding = row
            .try_get::<Option<HalfVector>, _>("embedding")
            .ok()
            .flatten()
            .map(|vector| vector.to_vec().into_iter().map(f32::from).collect());

        grouped
            .entry(label.clone())
            .or_default()
            .push(PromotionCandidate {
                graph_entity: GraphEntity {
                    graph_entity_id: EntityId::new(row.get::<String, _>("graph_entity_id")),
                    owner_entity_id: EntityId::new(owner_entity_id),
                    entity_type: GraphEntityType::Slot,
                    label,
                    value: row.get("value"),
                    source: codec::str_to_source(&row.get::<String, _>("source")),
                    confidence: row.get::<f64, _>("confidence").into(),
                    importance: row.get::<f64, _>("importance").into(),
                    access_count: u64::try_from(row.get::<i64, _>("access_count")).unwrap_or(0),
                    last_accessed_at: None,
                    is_pinned: row.get("pinned"),
                    temporal_decay_score: row.get("temporal_decay_score"),
                    node_tier: NodeTier::Episode,
                    parent_graph_entity_id: None,
                    promoted_at: None,
                    privacy_level: codec::str_to_privacy(&row.get::<String, _>("privacy_level")),
                    updated_at: row.get("updated_at_str"),
                },
                embedding,
            });
    }

    Ok(grouped)
}

fn select_promotion_group(candidates: Vec<PromotionCandidate>) -> Vec<PromotionCandidate> {
    let mut selected = Vec::new();

    for candidate in candidates {
        let is_similar = selected.iter().all(|existing: &PromotionCandidate| {
            episode_similarity(
                candidate.embedding.as_deref(),
                existing.embedding.as_deref(),
            ) >= PROMOTION_SIMILARITY_THRESHOLD
        });

        if is_similar {
            selected.push(candidate);
        }
    }

    selected
}

fn episode_similarity(lhs: Option<&[f32]>, rhs: Option<&[f32]>) -> f32 {
    match (lhs, rhs) {
        (Some(lhs), Some(rhs)) => cosine_similarity(lhs, rhs),
        // When embeddings are missing, return a neutral similarity rather than
        // maximum (1.0) to avoid merging unrelated episodes.
        _ => 0.5,
    }
}

async fn persist_promoted_note(
    tx: &mut PgTx<'_>,
    promotion_group: &[PromotionCandidate],
    now_rfc3339: &str,
) -> Result<()> {
    let episodes: Vec<GraphEntity> = promotion_group
        .iter()
        .map(|candidate| candidate.graph_entity.clone())
        .collect();
    let mut synthesized_text = String::new();
    for candidate in promotion_group {
        if !synthesized_text.is_empty() {
            synthesized_text.push('\n');
        }
        synthesized_text.push_str(&candidate.graph_entity.value);
    }
    let note = promote_episode_group_to_note(&episodes, &synthesized_text);

    let existing_note_id = query(
        "SELECT graph_entity_id \
         FROM graph_entities \
         WHERE owner_entity_id = $1 \
           AND label = $2 \
           AND node_tier = 'note' \
         LIMIT 1",
    )
    .bind(note.owner_entity_id.as_str())
    .bind(&note.label)
    .fetch_optional(&mut **tx)
    .await
    .context("lookup existing promoted note")?
    .map(|row| EntityId::new(row.get::<String, _>("graph_entity_id")));

    let note_id = existing_note_id.unwrap_or_else(|| note.graph_entity_id.clone());

    query(
        "INSERT INTO graph_entities ( \
            graph_entity_id, owner_entity_id, entity_type, label, value, \
            source, confidence, importance, access_count, pinned, temporal_decay_score, \
            node_tier, parent_graph_entity_id, promoted_at, privacy_level, updated_at \
         ) VALUES ( \
            $1, $2, 'slot', $3, $4, $5, $6, $7, 0, false, 1.0, \
            'note', NULL, NULL, $8, $9::timestamptz \
         ) ON CONFLICT (graph_entity_id) DO UPDATE SET \
             value = EXCLUDED.value, \
             source = EXCLUDED.source, \
             confidence = EXCLUDED.confidence, \
             importance = EXCLUDED.importance, \
             node_tier = EXCLUDED.node_tier, \
             privacy_level = EXCLUDED.privacy_level, \
             updated_at = EXCLUDED.updated_at",
    )
    .bind(note_id.as_str())
    .bind(note.owner_entity_id.as_str())
    .bind(&note.label)
    .bind(&note.value)
    .bind(codec::source_to_str(note.source))
    .bind(note.confidence.get())
    .bind(note.importance.get())
    .bind(codec::privacy_to_str(&note.privacy_level))
    .bind(now_rfc3339)
    .execute(&mut **tx)
    .await
    .context("upsert promoted note graph entity")?;

    let episode_ids: Vec<String> = promotion_group
        .iter()
        .map(|candidate| candidate.graph_entity.graph_entity_id.to_string())
        .collect();

    query(
        "UPDATE graph_entities \
         SET parent_graph_entity_id = $1, \
             promoted_at = $2::timestamptz \
         WHERE owner_entity_id = $3 \
           AND graph_entity_id = ANY($4::text[])",
    )
    .bind(note_id.as_str())
    .bind(now_rfc3339)
    .bind(note.owner_entity_id.as_str())
    .bind(&episode_ids)
    .execute(&mut **tx)
    .await
    .context("mark promoted episodes with note lineage")?;

    Ok(())
}

pub(super) async fn insert_sleep_snapshot_retrieval_unit(
    tx: &mut PgTx<'_>,
    group_key: &GroupKey,
    snapshot_slot_key: &str,
    unit_id: &str,
    aggregate: &GroupAggregate,
    now_rfc3339: &str,
) -> Result<u64> {
    query(
        "INSERT INTO retrieval_units ( \
            unit_id, entity_id, slot_key, content, content_type, signal_tier, \
            promotion_status, recency_score, importance, reliability, visibility, \
            layer, retention_tier, retention_expires_at, created_at, updated_at \
         ) VALUES ( \
            $1, $2, $3, $4, 'belief', $5, \
            'promoted', 1.0, $6, $7, $8, \
            'semantic', 'semantic', NULL, $9::timestamptz, $10::timestamptz \
         ) ON CONFLICT (unit_id) DO NOTHING",
    )
    .bind(unit_id)
    .bind(group_key.entity_id.as_str())
    .bind(snapshot_slot_key)
    .bind(&aggregate.combined_content)
    .bind(&aggregate.signal_tier)
    .bind(aggregate.importance)
    .bind(aggregate.reliability_avg)
    .bind(&aggregate.visibility)
    .bind(now_rfc3339)
    .bind(now_rfc3339)
    .execute(&mut **tx)
    .await
    .context("insert semantic sleep snapshot retrieval unit")
    .map(|result| result.rows_affected())
}

pub(super) async fn insert_sleep_snapshot_event(
    tx: &mut PgTx<'_>,
    group_key: &GroupKey,
    snapshot_slot_key: &str,
    aggregate: &GroupAggregate,
    candidates: &[ConsolidationCandidate],
    now_rfc3339: &str,
) -> Result<()> {
    let event_id = Uuid::new_v4().to_string();
    let provenance_reference = format!("memory.sleep.consolidation:{}", group_key.topic_prefix);

    let hash_fields = crate::core::memory::postgres::integrity::MemoryEventHashFields {
        event_id: &event_id,
        entity_id: group_key.entity_id.as_str(),
        slot_key: snapshot_slot_key,
        layer: "semantic",
        event_type: "summary_compacted",
        value: &aggregate.combined_content,
        source: "system",
        confidence: aggregate.reliability_avg,
        importance: aggregate.importance,
        provenance_source_class: None,
        provenance_reference: Some(&provenance_reference),
        provenance_evidence_uri: None,
        retention_tier: "semantic",
        retention_expires_at: None,
        signal_tier: &aggregate.signal_tier,
        source_kind: None,
        privacy_level: &aggregate.visibility,
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
            $1, $2, $3, 'semantic', 'summary_compacted', $4, 'system', \
            $5, $6, $7, 'semantic', NULL, $8, $9, \
            $10::timestamptz, $11::timestamptz, \
            $12, $13 \
         )",
    )
    .bind(&event_id)
    .bind(group_key.entity_id.as_str())
    .bind(snapshot_slot_key)
    .bind(&aggregate.combined_content)
    .bind(aggregate.reliability_avg)
    .bind(aggregate.importance)
    .bind(&provenance_reference)
    .bind(&aggregate.signal_tier)
    .bind(&aggregate.visibility)
    .bind(now_rfc3339)
    .bind(now_rfc3339)
    .bind(&integrity_prev_hash)
    .bind(&integrity_hash)
    .execute(&mut **tx)
    .await
    .context("insert semantic sleep snapshot event")?;

    for candidate in candidates {
        query(
            "INSERT INTO memory_derivations ( \
                derived_entity_id, derived_slot_key, source_entity_id, source_slot_key \
             ) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
        )
        .bind(group_key.entity_id.as_str())
        .bind(snapshot_slot_key)
        .bind(group_key.entity_id.as_str())
        .bind(&candidate.slot_key)
        .execute(&mut **tx)
        .await
        .context("record sleep snapshot lineage")?;
    }

    Ok(())
}

pub(super) fn open_pool(workspace_dir: &Path, config: &MemoryConfig) -> Result<Pool<Postgres>> {
    let database_url = crate::utils::postgres::require_postgres_url(
        config.postgres_url.as_deref(),
        Some(workspace_dir),
        "memory hygiene sleep consolidation",
    )?;

    block_on_pg_result(async {
        PoolOptions::<Postgres>::new()
            .max_connections(config.pg_max_connections.max(1))
            .connect(&database_url)
            .await
            .context("connect postgres for memory hygiene sleep consolidation")
    })
}

pub(super) fn block_on_pg_result<T, F>(future: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread {
            tokio::task::block_in_place(|| handle.block_on(future))
        } else {
            anyhow::bail!(
                "memory hygiene postgres sleep consolidation requires multi-thread tokio runtime; skipping in current-thread runtime"
            );
        }
    } else {
        let runtime = tokio::runtime::Runtime::new().context("create memory hygiene runtime")?;
        runtime.block_on(future)
    }
}
