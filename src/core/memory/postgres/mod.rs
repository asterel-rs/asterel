//! `PostgreSQL` memory backend: full-stack persistent store for the companion.
//!
//! Implements [`Memory`] using a `sqlx` async connection pool backed by
//! `PostgreSQL`. The backend layers three complementary retrieval strategies:
//!
//! 1. **Keyword search** — `tsvector` + `pg_trgm` full-text and fuzzy match.
//! 2. **Vector search** — `pgvector` HNSW index with cosine similarity.
//! 3. **Graph fusion** — `graph_edges` traversal scores blended into results.
//!
//! These are merged via weighted RRF fusion, then post-processed by
//! [`crate::core::memory::reranking`] (MMR, PPR blending, node-distance boost).
//!
//! ## Internal structure
//!
//! | Sub-module            | Responsibility                                      |
//! |-----------------------|-----------------------------------------------------|
//! | `schema`              | Migration runner (v1–v15, idempotent)               |
//! | `repository_write`    | `append_event`: embed → upsert belief slot + unit   |
//! | `repository_recall`   | `recall_scoped`: FTS + vector search + scoring      |
//! | `projection`          | Graph projection: upsert entities/edges on write    |
//! | `events`              | `append_inference_events` batch path                |
//! | `integrity`           | SHA-256 hash-chain for tamper-evident event log     |
//! | `search`              | Low-level FTS and vector query helpers              |

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use super::embeddings::{EmbeddingProvider, EmbeddingRole};
use super::traits::{
    BeliefSlot, ForgetMode, ForgetOutcome, MemoryEvent, MemoryEventInput, MemoryGovernance,
    MemoryIntegrityReport, MemoryReader, MemoryRecallEntry, MemorySource, MemoryWriter,
    RecallQuery,
};
use crate::contracts::memory::MemoryProvenance;
use crate::contracts::memory_error::MemoryResult;

use self::error::{PostgresMemoryError, PostgresMemoryResult, PostgresMemoryResultExt};

mod error;
mod events;
pub(crate) mod integrity;
mod projection;
mod repository_recall;
mod repository_write;
mod schema;
mod search;

/// Connection and tuning options for [`PostgresMemory`].
pub struct PostgresConnectOptions {
    /// Maximum entries in the embedding cache (0 disables caching).
    pub cache_max: usize,
    /// Enable graph-based retrieval fusion.
    pub graph_retrieval_fusion_enabled: bool,
    /// Weight of graph scores in fused retrieval (0.0-1.0).
    pub graph_retrieval_weight: f64,
    /// Maximum connections in the pool.
    pub max_connections: u32,
    /// Minimum idle connections kept alive.
    pub min_connections: u32,
    /// Timeout for acquiring a connection.
    pub connect_timeout: std::time::Duration,
    /// Timeout before idle connections are closed.
    pub idle_timeout: std::time::Duration,
    /// Weight of vector scores in hybrid merge (0.0-1.0).
    pub vector_weight: f64,
    /// Weight of keyword scores in hybrid merge (0.0-1.0).
    pub keyword_weight: f64,
    /// Maximum connection lifetime before replacement.
    pub max_lifetime: std::time::Duration,
    /// HNSW `ef_search` set via `after_connect` hook (0 = skip).
    pub hnsw_ef_search: u32,
}

/// `PostgreSQL`-backed persistent memory.
///
/// Owns the `sqlx` pool and the embedding provider. All write and read
/// operations are delegated to the sub-modules (`repository_write`,
/// `repository_recall`). The struct also manages the embedding cache
/// (LRU eviction via `evict_embedding_cache`) and retention pruning
/// (TTL-expired rows purged on each `health_check`).
pub struct PostgresMemory {
    pool: sqlx_core::pool::Pool<sqlx_postgres::Postgres>,
    embedder: Arc<dyn EmbeddingProvider>,
    cache_max: usize,
    graph_retrieval_fusion_enabled: bool,
    graph_retrieval_weight: f64,
    vector_weight: f64,
    keyword_weight: f64,
}

impl PostgresMemory {
    const TREND_TTL_DAYS: f64 = 30.0;
    const TREND_DECAY_WINDOW_DAYS: f64 = 45.0;

    /// Connect to `PostgreSQL` using the provided URL.
    ///
    /// # Errors
    /// Returns an error if pool creation or schema migration fails.
    pub async fn connect(
        database_url: &str,
        embedder: Arc<dyn EmbeddingProvider>,
        cache_max: usize,
        graph_retrieval_fusion_enabled: bool,
        graph_retrieval_weight: f64,
    ) -> MemoryResult<Self> {
        use sqlx_core::pool::PoolOptions;
        let pool = PoolOptions::<sqlx_postgres::Postgres>::new()
            .max_connections(10)
            .max_lifetime(std::time::Duration::from_secs(1800))
            .connect(database_url)
            .await
            .map_err(PostgresMemoryError::connect)?;

        schema::run_migrations(&pool).await?;

        Ok(Self {
            pool,
            embedder,
            cache_max,
            graph_retrieval_fusion_enabled,
            graph_retrieval_weight: graph_retrieval_weight.clamp(0.0, 1.0),
            vector_weight: 0.7,
            keyword_weight: 0.3,
        })
    }

    /// Connect with detailed pool configuration.
    ///
    /// # Errors
    /// Returns an error if pool creation or schema migration fails.
    pub async fn connect_with_options(
        database_url: &str,
        embedder: Arc<dyn EmbeddingProvider>,
        opts: PostgresConnectOptions,
    ) -> MemoryResult<Self> {
        use sqlx_core::pool::PoolOptions;
        let ef_search = opts.hnsw_ef_search;
        let mut pool_opts = PoolOptions::<sqlx_postgres::Postgres>::new()
            .max_connections(opts.max_connections)
            .min_connections(opts.min_connections)
            .acquire_timeout(opts.connect_timeout)
            .idle_timeout(opts.idle_timeout)
            .max_lifetime(opts.max_lifetime);

        if ef_search > 0 {
            pool_opts = pool_opts.after_connect(move |conn, _meta| {
                Box::pin(async move {
                    use sqlx_core::executor::Executor;
                    // PostgreSQL SET statements do not accept $1 placeholders, so
                    // string formatting is the only option. `ef_search` is `u32`,
                    // guaranteeing a valid non-negative integer with no injection surface.
                    let sql = format!("SET hnsw.ef_search = {ef_search}");
                    conn.execute(sql.as_str()).await?;
                    Ok(())
                })
            });
        }

        let pool = pool_opts
            .connect(database_url)
            .await
            .map_err(PostgresMemoryError::connect)?;

        schema::run_migrations(&pool).await?;

        Ok(Self {
            pool,
            embedder,
            cache_max: opts.cache_max,
            graph_retrieval_fusion_enabled: opts.graph_retrieval_fusion_enabled,
            graph_retrieval_weight: opts.graph_retrieval_weight.clamp(0.0, 1.0),
            vector_weight: opts.vector_weight.clamp(0.0, 1.0),
            keyword_weight: opts.keyword_weight.clamp(0.0, 1.0),
        })
    }

    /// Map a memory source to an ordinal priority used in belief-slot
    /// replacement decisions. Higher beats lower; equal sources fall back
    /// to confidence, then timestamp ordering.
    fn source_priority(source: MemorySource) -> u8 {
        match source {
            MemorySource::ExplicitUser => 5,
            MemorySource::ToolVerified => 4,
            MemorySource::ExternalPrimary => 3,
            MemorySource::System => 2,
            MemorySource::ExternalSecondary => 1,
            MemorySource::Inferred => 0,
        }
    }

    fn compare_normalized_timestamps(incoming: &str, incumbent: &str) -> std::cmp::Ordering {
        let incoming_normalized = chrono::DateTime::parse_from_rfc3339(incoming)
            .ok()
            .and_then(|parsed| parsed.timestamp_nanos_opt());
        let incumbent_normalized = chrono::DateTime::parse_from_rfc3339(incumbent)
            .ok()
            .and_then(|parsed| parsed.timestamp_nanos_opt());

        match (incoming_normalized, incumbent_normalized) {
            (Some(incoming), Some(incumbent)) => incoming.cmp(&incumbent),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => std::cmp::Ordering::Equal,
        }
    }

    /// Compute the contradiction penalty applied when a belief slot is
    /// superseded. Higher confidence/importance of the incoming event implies
    /// a stronger contradiction signal against the incumbent.
    fn contradiction_penalty(confidence: f64, importance: f64) -> f64 {
        let confidence = confidence.clamp(0.0, 1.0);
        let importance = importance.clamp(0.0, 1.0);
        (0.12 + 0.10 * confidence + 0.08 * importance).clamp(0.0, 1.0)
    }

    /// Deterministic content hash for embedding cache (SHA-256, 32 hex chars / 128-bit).
    fn content_hash(role: EmbeddingRole, text: &str) -> String {
        use sha2::{Digest, Sha256};
        let prefix = match role {
            EmbeddingRole::Document => "document:",
            EmbeddingRole::Query => "query:",
        };
        let hash = Sha256::digest(format!("{prefix}{text}").as_bytes());
        // Use first 16 bytes (128-bit) — birthday-attack safe to ~2^64 entries.
        hash[..16]
            .iter()
            .fold(String::with_capacity(32), |mut acc, byte| {
                use std::fmt::Write;
                write!(acc, "{byte:02x}").ok();
                acc
            })
    }

    /// Get embedding from cache, or compute + cache it.
    ///
    /// When `cache_max` is 0, caching is bypassed entirely — embeddings are
    /// computed on every call but never stored.
    async fn get_or_compute_embedding(
        &self,
        role: EmbeddingRole,
        text: &str,
    ) -> PostgresMemoryResult<Option<Vec<f32>>> {
        use sqlx_core::query::query;
        use sqlx_core::row::Row;

        if self.embedder.dimensions() == 0 {
            return Ok(None);
        }

        // cache_max == 0 means "no caching" — skip cache lookup and storage.
        if self.cache_max == 0 {
            return self.embed_one_with_degraded_fallback(role, text).await;
        }

        let hash = Self::content_hash(role, text);

        let cached: Option<Vec<u8>> =
            query("SELECT embedding FROM embedding_cache WHERE content_hash = $1")
                .bind(&hash)
                .fetch_optional(&self.pool)
                .await
                .pg_query("fetch embedding cache entry")?
                .map(|row| row.get("embedding"));

        if let Some(bytes) = cached {
            // Update accessed_at for LRU
            query("UPDATE embedding_cache SET accessed_at = now() WHERE content_hash = $1")
                .bind(&hash)
                .execute(&self.pool)
                .await
                .pg_write("touch embedding cache entry")?;
            return Ok(Some(crate::core::memory::vector::bytes_to_vec(&bytes)));
        }

        let Some(embedding) = self.embed_one_with_degraded_fallback(role, text).await? else {
            return Ok(None);
        };
        let bytes = crate::core::memory::vector::vec_to_bytes(&embedding);

        query(
            "INSERT INTO embedding_cache (content_hash, embedding) \
             VALUES ($1, $2) \
             ON CONFLICT (content_hash) DO UPDATE SET embedding = $2, accessed_at = now()",
        )
        .bind(&hash)
        .bind(&bytes)
        .execute(&self.pool)
        .await
        .pg_write("upsert embedding cache entry")?;

        Ok(Some(embedding))
    }

    async fn embed_one_with_degraded_fallback(
        &self,
        role: EmbeddingRole,
        text: &str,
    ) -> PostgresMemoryResult<Option<Vec<f32>>> {
        let outcome = match role {
            EmbeddingRole::Document => self.embedder.embed_one_document(text).await,
            EmbeddingRole::Query => self.embedder.embed_one_query(text).await,
        };

        match outcome {
            Ok(embedding) => Ok(Some(embedding)),
            Err(error) => {
                tracing::warn!(
                    %error,
                    embedder = self.embedder.name(),
                    "embedding unavailable; continuing without vector embedding"
                );
                Ok(None)
            }
        }
    }

    /// Batch LRU eviction using `pg_class.reltuples` for O(1) row estimate.
    ///
    /// Evicts to 90% of `cache_max` to provide headroom and avoid
    /// re-triggering eviction on the very next insert.
    async fn evict_embedding_cache(&self) -> PostgresMemoryResult<()> {
        use sqlx_core::query::query;
        use sqlx_core::row::Row;

        if self.cache_max == 0 {
            return Ok(());
        }

        let approx_count: f32 =
            query("SELECT reltuples FROM pg_class WHERE relname = 'embedding_cache'")
                .fetch_optional(&self.pool)
                .await
                .pg_query("estimate embedding cache size")?
                .map_or(0.0, |row| row.get::<f32, _>(0));

        // Cast safety: cache_max is a configured capacity far below f32 precision limits.
        #[allow(clippy::cast_precision_loss)]
        let max_f = self.cache_max as f32;
        if approx_count <= max_f {
            return Ok(());
        }

        // Evict down to 90% of max to provide headroom.
        // Cast safety: eviction count is clamped non-negative and derived from bounded cache sizes.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let evict_count = (approx_count - max_f * 0.9).max(0.0) as i64;
        if evict_count <= 0 {
            return Ok(());
        }

        let deleted = query(
            "DELETE FROM embedding_cache WHERE content_hash IN ( \
                SELECT content_hash FROM embedding_cache \
                ORDER BY accessed_at ASC \
                LIMIT $1 \
             )",
        )
        .bind(evict_count)
        .execute(&self.pool)
        .await
        .map_or(0, |r| r.rows_affected());

        if deleted > 0 {
            tracing::info!(deleted, "embedding cache LRU eviction completed");
        }

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn restore_retained_slot_projection(
        tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
        entity_id: String,
        slot_key: String,
    ) -> PostgresMemoryResult<bool> {
        use sqlx_core::query::query;
        use sqlx_core::row::Row;

        let rows = query(
            "SELECT event_id, value, source, confidence, importance, privacy_level, \
                    signal_tier, layer, provenance_source_class, provenance_reference, \
                    provenance_evidence_uri, retention_tier, retention_expires_at, \
                    to_char(occurred_at AT TIME ZONE 'UTC', \
                        'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS occurred_at_str \
             FROM memory_events WHERE entity_id = $1 AND slot_key = $2",
        )
        .bind(&entity_id)
        .bind(&slot_key)
        .fetch_all(&mut **tx)
        .await
        .pg_query("select retained belief candidates")?;
        let Some(row) = rows.iter().max_by(|left, right| {
            let source_order = Self::source_priority(super::codec::str_to_source(
                &left.get::<String, _>("source"),
            ))
            .cmp(&Self::source_priority(super::codec::str_to_source(
                &right.get::<String, _>("source"),
            )));
            if !source_order.is_eq() {
                return source_order;
            }

            let left_confidence = left.get::<f64, _>("confidence");
            let right_confidence = right.get::<f64, _>("confidence");
            if (left_confidence - right_confidence).abs() > 0.001 {
                return left_confidence.total_cmp(&right_confidence);
            }

            Self::compare_normalized_timestamps(
                &left.get::<String, _>("occurred_at_str"),
                &right.get::<String, _>("occurred_at_str"),
            )
        }) else {
            return Ok(false);
        };
        let event_id = row.get::<String, _>("event_id");
        let value = row.get::<String, _>("value");
        let source = row.get::<String, _>("source");
        let confidence = row.get::<f64, _>("confidence");
        let importance = row.get::<f64, _>("importance");
        let privacy_level = row.get::<String, _>("privacy_level");
        let signal_tier = row.get::<String, _>("signal_tier");
        let layer = row.get::<String, _>("layer");
        let provenance_source_class = row.try_get::<String, _>("provenance_source_class").ok();
        let provenance_reference = row.try_get::<String, _>("provenance_reference").ok();
        let provenance_evidence_uri = row.try_get::<String, _>("provenance_evidence_uri").ok();
        let retention_tier = row.get::<String, _>("retention_tier");
        let retention_expires_at = row
            .try_get::<chrono::DateTime<chrono::Utc>, _>("retention_expires_at")
            .ok();
        drop(rows);

        query(
            "INSERT INTO belief_slots ( \
                entity_id, slot_key, value, status, winner_event_id, source, confidence, \
                importance, privacy_level, updated_at \
             ) VALUES ($1, $2, $3, 'active', $4, $5, $6, $7, $8, now()) \
             ON CONFLICT (entity_id, slot_key) DO UPDATE SET \
                value = EXCLUDED.value, status = EXCLUDED.status, \
                winner_event_id = EXCLUDED.winner_event_id, source = EXCLUDED.source, \
                confidence = EXCLUDED.confidence, importance = EXCLUDED.importance, \
                privacy_level = EXCLUDED.privacy_level, updated_at = EXCLUDED.updated_at",
        )
        .bind(&entity_id)
        .bind(&slot_key)
        .bind(&value)
        .bind(&event_id)
        .bind(&source)
        .bind(confidence)
        .bind(importance)
        .bind(&privacy_level)
        .execute(&mut **tx)
        .await
        .pg_write("restore retained belief slot")?;

        let unit_id = format!("{entity_id}::{slot_key}");
        query(
            "INSERT INTO retrieval_units ( \
                unit_id, entity_id, slot_key, content, signal_tier, promotion_status, \
                importance, reliability, visibility, layer, provenance_source_class, \
                provenance_reference, provenance_evidence_uri, retention_tier, \
                retention_expires_at, updated_at \
             ) VALUES ($1, $2, $3, $4, $5, 'promoted', $6, $7, $8, $9, $10, $11, $12, \
                $13, $14, now()) \
             ON CONFLICT (unit_id) DO UPDATE SET \
                content = EXCLUDED.content, signal_tier = EXCLUDED.signal_tier, \
                promotion_status = EXCLUDED.promotion_status, importance = EXCLUDED.importance, \
                reliability = EXCLUDED.reliability, visibility = EXCLUDED.visibility, \
                layer = EXCLUDED.layer, provenance_source_class = EXCLUDED.provenance_source_class, \
                provenance_reference = EXCLUDED.provenance_reference, \
                provenance_evidence_uri = EXCLUDED.provenance_evidence_uri, \
                retention_tier = EXCLUDED.retention_tier, \
                retention_expires_at = EXCLUDED.retention_expires_at, updated_at = EXCLUDED.updated_at",
        )
        .bind(unit_id)
        .bind(&entity_id)
        .bind(&slot_key)
        .bind(&value)
        .bind(&signal_tier)
        .bind(importance)
        .bind(confidence)
        .bind(&privacy_level)
        .bind(&layer)
        .bind(provenance_source_class)
        .bind(provenance_reference)
        .bind(provenance_evidence_uri)
        .bind(&retention_tier)
        .bind(retention_expires_at)
        .execute(&mut **tx)
        .await
        .pg_write("restore retained retrieval unit")?;

        query(
            "UPDATE graph_entities SET value = $3, source = $4, confidence = $5, \
                importance = $6, privacy_level = $7, updated_at = now() \
             WHERE graph_entity_id = ('slot::' || $1 || '::' || $2)",
        )
        .bind(&entity_id)
        .bind(&slot_key)
        .bind(&value)
        .bind(&source)
        .bind(confidence)
        .bind(importance)
        .bind(&privacy_level)
        .execute(&mut **tx)
        .await
        .pg_write("restore retained slot graph projection")?;

        Ok(true)
    }

    /// Delete `memory_events` and `retrieval_units` whose retention has expired.
    #[allow(clippy::too_many_lines)]
    async fn prune_expired_retention(&self) -> PostgresMemoryResult<()> {
        use sqlx_core::query::query;
        use sqlx_core::row::Row;

        let mut tx = self
            .pool
            .begin()
            .await
            .pg_write("begin retention pruning transaction")?;
        integrity::lock_memory_event_chain(&mut tx).await?;

        let expired_events = query(
            "DELETE FROM memory_events \
             WHERE retention_expires_at IS NOT NULL AND retention_expires_at < now() \
             RETURNING event_id, entity_id, slot_key, value",
        )
        .fetch_all(&mut *tx)
        .await
        .pg_write("prune expired memory events")?;
        let event_ids = expired_events
            .iter()
            .map(|row| row.get::<String, _>("event_id"))
            .collect::<Vec<_>>();
        let event_graph_ids = event_ids
            .iter()
            .map(|event_id| format!("event::{event_id}"))
            .collect::<Vec<_>>();

        if !event_ids.is_empty() {
            query(
                "DELETE FROM graph_edges WHERE event_id = ANY($1) \
                    OR from_entity_id = ANY($2) OR to_entity_id = ANY($2)",
            )
            .bind(&event_ids)
            .bind(&event_graph_ids)
            .execute(&mut *tx)
            .await
            .pg_write("prune expired graph edges")?;

            query("DELETE FROM graph_entity_aliases WHERE canonical_graph_entity_id = ANY($1)")
                .bind(&event_graph_ids)
                .execute(&mut *tx)
                .await
                .pg_write("prune expired graph aliases")?;

            query("DELETE FROM graph_entities WHERE graph_entity_id = ANY($1)")
                .bind(&event_graph_ids)
                .execute(&mut *tx)
                .await
                .pg_write("prune expired event graph entities")?;

            let expired_winners = query(
                "DELETE FROM belief_slots WHERE winner_event_id = ANY($1) \
                 RETURNING entity_id, slot_key",
            )
            .bind(&event_ids)
            .fetch_all(&mut *tx)
            .await
            .pg_write("prune expired winning belief slots")?;
            for row in expired_winners {
                let entity_id = row.get::<String, _>("entity_id");
                let slot_key = row.get::<String, _>("slot_key");
                if Self::restore_retained_slot_projection(
                    &mut tx,
                    entity_id.clone(),
                    slot_key.clone(),
                )
                .await?
                {
                    continue;
                }
                let slot_graph_id = format!("slot::{entity_id}::{slot_key}");
                query("DELETE FROM retrieval_units WHERE entity_id = $1 AND slot_key = $2")
                    .bind(&entity_id)
                    .bind(&slot_key)
                    .execute(&mut *tx)
                    .await
                    .pg_write("prune expired winning retrieval units")?;
                query("DELETE FROM graph_edges WHERE from_entity_id = $1 OR to_entity_id = $1")
                    .bind(&slot_graph_id)
                    .execute(&mut *tx)
                    .await
                    .pg_write("prune expired winning slot graph edges")?;
                query("DELETE FROM graph_entity_aliases WHERE canonical_graph_entity_id = $1")
                    .bind(&slot_graph_id)
                    .execute(&mut *tx)
                    .await
                    .pg_write("prune expired winning slot graph aliases")?;
                query("DELETE FROM graph_entities WHERE graph_entity_id = $1")
                    .bind(&slot_graph_id)
                    .execute(&mut *tx)
                    .await
                    .pg_write("prune expired winning slot graph entity")?;
            }

            for row in &expired_events {
                let value = row.get::<String, _>("value");
                for role in [EmbeddingRole::Document, EmbeddingRole::Query] {
                    query("DELETE FROM embedding_cache WHERE content_hash = $1")
                        .bind(Self::content_hash(role, &value))
                        .execute(&mut *tx)
                        .await
                        .pg_write("prune expired embedding cache entry")?;
                }
            }

            let now = integrity::canonical_db_timestamp(&mut tx).await?;
            Self::insert_deletion_ledger_entry(
                &mut tx,
                "system:retention",
                "expired",
                "retention",
                &format!("pruned {} expired memory events", event_ids.len()),
                &now,
            )
            .await?;
            integrity::rebuild_memory_event_chain(&mut tx).await?;
        }

        let units_pruned = query(
            "DELETE FROM retrieval_units \
             WHERE retention_expires_at IS NOT NULL AND retention_expires_at < now()",
        )
        .execute(&mut *tx)
        .await
        .map(|result| result.rows_affected())
        .pg_write("prune expired retrieval units")?;

        tx.commit()
            .await
            .pg_write("commit retention pruning transaction")?;

        let mut owner_ids = expired_events
            .iter()
            .map(|row| row.get::<String, _>("entity_id"))
            .collect::<Vec<_>>();
        owner_ids.sort();
        owner_ids.dedup();
        for owner_id in owner_ids {
            crate::core::memory::graphrag::activation_cache()
                .invalidate(&crate::contracts::ids::EntityId::new(owner_id))
                .await;
        }

        if !event_ids.is_empty() || units_pruned > 0 {
            tracing::info!(
                events_pruned = event_ids.len(),
                units_pruned,
                "retention pruning completed"
            );
        }

        Ok(())
    }
}

impl MemoryWriter for PostgresMemory {
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
        Box::pin(async move { self.append_event_impl(input).await.map_err(Into::into) })
    }
}

impl MemoryReader for PostgresMemory {
    fn recall_scoped(
        &self,
        query: RecallQuery,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = crate::contracts::memory_error::MemoryResult<Vec<MemoryRecallEntry>>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move { self.recall_scoped_impl(query).await.map_err(Into::into) })
    }

    fn resolve_slot<'a>(
        &'a self,
        entity_id: &'a str,
        slot_key: &'a str,
    ) -> Pin<
        Box<
            dyn Future<Output = crate::contracts::memory_error::MemoryResult<Option<BeliefSlot>>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.resolve_slot_impl(entity_id, slot_key)
                .await
                .map_err(Into::into)
        })
    }
}

impl MemoryGovernance for PostgresMemory {
    fn name(&self) -> &'static str {
        "postgres"
    }

    fn health_check(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async move {
            use sqlx_core::query::query;
            use sqlx_core::row::Row;
            let ok = query("SELECT 1 AS ok")
                .fetch_one(&self.pool)
                .await
                .is_ok_and(|row| row.get::<i32, _>("ok") == 1);

            if ok {
                // Prune rows with expired retention on each health check
                if let Err(e) = self.prune_expired_retention().await {
                    tracing::warn!("retention pruning failed during health_check: {e}");
                }
                // Batch LRU eviction for embedding cache
                if let Err(e) = self.evict_embedding_cache().await {
                    tracing::warn!("embedding cache eviction failed during health_check: {e}");
                }
            }

            ok
        })
    }

    fn forget_slot<'a>(
        &'a self,
        entity_id: &'a str,
        slot_key: &'a str,
        mode: ForgetMode,
        reason: &'a str,
    ) -> Pin<
        Box<
            dyn Future<Output = crate::contracts::memory_error::MemoryResult<ForgetOutcome>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            self.forget_slot_impl(entity_id, slot_key, mode, reason)
                .await
                .map_err(Into::into)
        })
    }

    fn count_events<'a>(
        &'a self,
        entity_id: Option<&'a str>,
    ) -> Pin<
        Box<dyn Future<Output = crate::contracts::memory_error::MemoryResult<usize>> + Send + 'a>,
    > {
        Box::pin(async move { self.count_events_impl(entity_id).await.map_err(Into::into) })
    }

    #[allow(dead_code)]
    fn list_entities(
        &self,
    ) -> Pin<
        Box<
            dyn Future<Output = crate::contracts::memory_error::MemoryResult<Vec<String>>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            use sqlx_core::query::query;
            use sqlx_core::row::Row;

            let rows = query("SELECT DISTINCT entity_id FROM belief_slots ORDER BY entity_id ASC")
                .fetch_all(&self.pool)
                .await
                .map_err(|error| {
                    crate::contracts::memory_error::MemoryError::from(PostgresMemoryError::query(
                        error,
                    ))
                })?;

            Ok(rows
                .into_iter()
                .map(|row| row.get::<String, _>("entity_id"))
                .collect())
        })
    }

    #[allow(dead_code)]
    fn list_slots<'a>(
        &'a self,
        entity_id: &'a str,
    ) -> Pin<
        Box<
            dyn Future<Output = crate::contracts::memory_error::MemoryResult<Vec<BeliefSlot>>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            use sqlx_core::query::query;
            use sqlx_core::row::Row;

            let rows = query(
                "SELECT slot_key, value, source, confidence, importance, privacy_level, \
                        to_char(updated_at, 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at_str \
                 FROM belief_slots \
                 WHERE entity_id = $1 AND status = 'active' \
                 ORDER BY updated_at DESC, slot_key ASC",
            )
            .bind(entity_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|error| {
                crate::contracts::memory_error::MemoryError::from(PostgresMemoryError::query(error))
            })?;

            Ok(rows
                .into_iter()
                .map(|row| BeliefSlot {
                    entity_id: crate::contracts::ids::EntityId::new(entity_id),
                    slot_key: crate::contracts::ids::SlotKey::new(row.get::<String, _>("slot_key")),
                    value: row.get("value"),
                    source: super::codec::str_to_source(&row.get::<String, _>("source")),
                    confidence: row.get::<f64, _>("confidence").into(),
                    importance: row.get::<f64, _>("importance").into(),
                    privacy_level: super::codec::str_to_privacy(
                        &row.get::<String, _>("privacy_level"),
                    ),
                    updated_at: row.get("updated_at_str"),
                })
                .collect())
        })
    }

    #[allow(dead_code)]
    fn slot_provenance<'a>(
        &'a self,
        entity_id: &'a str,
        slot_key: &'a str,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = crate::contracts::memory_error::MemoryResult<Option<MemoryProvenance>>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            use sqlx_core::query::query;
            use sqlx_core::row::Row;

            let row = query(
                "SELECT provenance_source_class, provenance_reference, provenance_evidence_uri \
                 FROM memory_events \
                 WHERE entity_id = $1 AND slot_key = $2 \
                 ORDER BY occurred_at DESC, ingested_at DESC \
                 LIMIT 1",
            )
            .bind(entity_id)
            .bind(slot_key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| {
                crate::contracts::memory_error::MemoryError::from(PostgresMemoryError::query(error))
            })?;

            Ok(row.and_then(|row| {
                let source = row
                    .try_get::<String, _>("provenance_source_class")
                    .ok()
                    .filter(|value| !value.is_empty())?;
                let reference = row
                    .try_get::<String, _>("provenance_reference")
                    .ok()
                    .filter(|value| !value.is_empty())?;
                Some(MemoryProvenance {
                    source_class: super::codec::str_to_source(&source),
                    reference,
                    evidence_uri: row.try_get::<String, _>("provenance_evidence_uri").ok(),
                })
            }))
        })
    }

    fn verify_integrity(
        &self,
    ) -> Pin<
        Box<
            dyn Future<Output = crate::contracts::memory_error::MemoryResult<MemoryIntegrityReport>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async move { self.verify_integrity_impl().await.map_err(Into::into) })
    }

    fn contradiction_ratio(
        &self,
    ) -> Pin<Box<dyn Future<Output = crate::contracts::memory_error::MemoryResult<f64>> + Send + '_>>
    {
        Box::pin(async move { self.contradiction_ratio_impl().await.map_err(Into::into) })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use sqlx_core::query::query;
    use sqlx_core::row::Row;
    use uuid::Uuid;

    use super::{PostgresConnectOptions, PostgresMemory};
    use crate::core::memory::embeddings::{EmbeddingFuture, EmbeddingProvider};
    use crate::core::memory::{
        MemoryEventInput, MemoryEventType, MemoryReader, MemorySource, MemoryWriter, PrivacyLevel,
        RecallQuery,
    };
    use crate::utils::test_env::EnvVarGuard;

    struct FailingEmbedding;

    impl EmbeddingProvider for FailingEmbedding {
        fn name(&self) -> &'static str {
            "failing_test"
        }

        fn dimensions(&self) -> usize {
            3
        }

        fn embed<'a>(&'a self, _texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
            Box::pin(async move { anyhow::bail!("synthetic embedding failure") })
        }
    }

    async fn memory_with_failing_embedder() -> (PostgresMemory, EnvVarGuard) {
        let env_guard = EnvVarGuard::require_postgres_url();
        let database_url =
            std::env::var("ASTEREL_POSTGRES_URL").expect("ASTEREL_POSTGRES_URL must be set");
        let memory = PostgresMemory::connect_with_options(
            &database_url,
            Arc::new(FailingEmbedding),
            PostgresConnectOptions {
                cache_max: 16,
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

        (memory, env_guard)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn embedding_failures_degrade_append_and_recall_instead_of_failing() {
        let _db_guard = crate::utils::test_env::acquire_test_db().await;
        let (memory, _env_guard) = memory_with_failing_embedder().await;

        let entity_id = format!("person:embedding-degraded-{}", Uuid::new_v4().simple());
        let slot_key = "persona/test/state_header/v1";
        let value = "bootstrap canonical persona state for degraded embedding coverage";

        memory
            .append_event(
                MemoryEventInput::new(
                    &entity_id,
                    slot_key,
                    MemoryEventType::FactUpdated,
                    value,
                    MemorySource::System,
                    PrivacyLevel::Private,
                )
                .with_confidence(0.95)
                .with_importance(0.8),
            )
            .await
            .expect("append should succeed even when embeddings fail");

        let resolved = memory
            .resolve_slot(&entity_id, slot_key)
            .await
            .expect("resolve_slot should succeed")
            .expect("slot should exist");
        assert_eq!(resolved.value, value);

        let unit_id = format!("{entity_id}::{slot_key}");
        let embedding_is_null = query(
            "SELECT embedding IS NULL AS embedding_is_null \
             FROM retrieval_units WHERE unit_id = $1",
        )
        .bind(&unit_id)
        .fetch_one(&memory.pool)
        .await
        .expect("retrieval unit query should succeed")
        .get::<bool, _>("embedding_is_null");
        assert!(embedding_is_null);

        let recalled = memory
            .recall_scoped(RecallQuery::new(
                &entity_id,
                "bootstrap canonical persona",
                5,
            ))
            .await
            .expect("recall should degrade to keyword-only instead of failing");
        assert!(
            recalled
                .iter()
                .any(|item| item.slot_key.as_str() == slot_key),
            "keyword-only recall should still find the inserted retrieval unit"
        );
    }
}
