//! Write-path implementation for the `PostgreSQL` memory backend.
//!
//! The core entry point is `append_event_impl`, which runs inside a
//! single `PostgreSQL` transaction:
//!
//! 1. Normalize and validate the input (trim whitespace, infer emotion).
//! 2. Compute the embedding for the value via the provider.
//! 3. Decide whether the incoming event supersedes the current belief
//!    slot (source priority → confidence → timestamp ordering).
//! 4. Compute the SHA-256 integrity hash chain entry.
//! 5. Insert the `memory_events` row.
//! 6. Upsert the graph projection (`graph_entities` / `graph_edges`).
//! 7. Update the contradiction penalty on the retrieval unit.
//! 8. If replacing: upsert `belief_slots` + `retrieval_units` and
//!    conditionally promote the unit from `raw` → `candidate`.
//! 9. Commit and invalidate the `GraphActivationCache` for the entity.

use chrono::Local;
use pgvector::HalfVector;
use sqlx_core::query::query;
use sqlx_core::row::Row;
use uuid::Uuid;

use super::PostgresMemory;
use super::error::{PostgresMemoryError, PostgresMemoryResult, PostgresMemoryResultExt};
use crate::contracts::ids::EventId;
use crate::core::memory::codec;
use crate::core::memory::embeddings::EmbeddingRole;
use crate::core::memory::emotional_context::infer_emotion_from_text;
use crate::core::memory::traits::{MemoryEvent, MemoryEventInput, SignalTier};

type PgTx<'a> = sqlx_core::transaction::Transaction<'a, sqlx_postgres::Postgres>;

struct AppendEventWriteContext<'a> {
    layer_str: &'static str,
    retention_tier: &'static str,
    retention_expires_at: Option<String>,
    signal_tier_str: &'static str,
    source_str: &'static str,
    privacy_str: &'static str,
    event_type_str: String,
    provenance_source_class: Option<&'static str>,
    provenance_reference: Option<&'a str>,
    provenance_evidence_uri: Option<&'a str>,
    source_kind_str: Option<&'static str>,
}

impl PostgresMemory {
    /// Append a memory event: compute embedding, upsert belief slot
    /// and retrieval unit, and maintain the integrity hash chain.
    pub(super) async fn append_event_impl(
        &self,
        input: MemoryEventInput,
    ) -> PostgresMemoryResult<MemoryEvent> {
        let input = Self::normalize_input(input)?;
        let event_id = EventId::new(Uuid::new_v4().to_string());
        let now = Local::now().to_rfc3339();

        let embedding = self
            .get_or_compute_embedding(EmbeddingRole::Document, &input.value)
            .await?;

        let mut tx = self
            .pool
            .begin()
            .await
            .pg_write("begin append_event transaction")?;

        let (should_replace, supersedes_event_id) =
            Self::decide_replacement(&mut tx, &input).await?;

        let write_context = AppendEventWriteContext::from_input(&input, &now);

        let fields = super::integrity::MemoryEventHashFields {
            event_id: event_id.as_str(),
            entity_id: input.entity_id.as_str(),
            slot_key: input.slot_key.as_str(),
            layer: write_context.layer_str,
            event_type: &write_context.event_type_str,
            value: &input.value,
            source: write_context.source_str,
            confidence: input.confidence.get(),
            importance: input.importance.get(),
            provenance_source_class: write_context.provenance_source_class,
            provenance_reference: write_context.provenance_reference,
            provenance_evidence_uri: write_context.provenance_evidence_uri,
            retention_tier: write_context.retention_tier,
            retention_expires_at: write_context.retention_expires_at.as_deref(),
            signal_tier: write_context.signal_tier_str,
            source_kind: write_context.source_kind_str,
            privacy_level: write_context.privacy_str,
            occurred_at: &now,
            ingested_at: &now,
            supersedes_event_id: supersedes_event_id.as_deref(),
        };

        let (integrity_prev_hash, integrity_hash) =
            super::integrity::next_memory_event_chain(&mut tx, &fields).await?;

        Self::insert_memory_event_row(
            &mut tx,
            &input,
            event_id.as_str(),
            &now,
            &write_context,
            supersedes_event_id.as_deref(),
            &integrity_prev_hash,
            &integrity_hash,
        )
        .await?;

        super::projection::upsert_graph_projection(
            &mut tx,
            &input,
            event_id.as_str(),
            &now,
            should_replace,
            supersedes_event_id.as_deref(),
        )
        .await?;

        Self::update_contradiction_penalty(&mut tx, &input, supersedes_event_id.as_deref()).await?;

        if should_replace {
            Self::upsert_belief_slot(&mut tx, &input, event_id.as_str(), &now, &write_context)
                .await?;
            self.upsert_retrieval_unit(&mut tx, &input, &now, &write_context, embedding.as_deref())
                .await?;
            Self::maybe_promote_to_candidate(
                &mut tx,
                input.entity_id.as_str(),
                input.slot_key.as_str(),
            )
            .await?;
        }

        tx.commit()
            .await
            .pg_write("commit append_event transaction")?;

        #[cfg(feature = "postgres")]
        crate::core::memory::graphrag::activation_cache()
            .invalidate(&input.entity_id)
            .await;

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
            occurred_at: now.clone(),
            ingested_at: now,
        })
    }

    async fn insert_memory_event_row(
        tx: &mut PgTx<'_>,
        input: &MemoryEventInput,
        event_id: &str,
        now: &str,
        write_context: &AppendEventWriteContext<'_>,
        supersedes_event_id: Option<&str>,
        integrity_prev_hash: &str,
        integrity_hash: &str,
    ) -> PostgresMemoryResult<()> {
        query(
            "INSERT INTO memory_events ( \
                event_id, entity_id, slot_key, layer, event_type, value, source, \
                confidence, importance, \
                provenance_source_class, provenance_reference, provenance_evidence_uri, \
                retention_tier, retention_expires_at, signal_tier, source_kind, \
                emotion_label, emotion_valence, emotion_arousal, emotion_confidence, \
                privacy_level, occurred_at, ingested_at, supersedes_event_id, \
                integrity_prev_hash, integrity_hash \
             ) VALUES ( \
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, \
                $13, $14::timestamptz, $15, $16, $17, $18, $19, $20, \
                $21, $22::timestamptz, $23::timestamptz, $24, $25, $26 \
             )",
        )
        .bind(event_id)
        .bind(input.entity_id.as_str())
        .bind(input.slot_key.as_str())
        .bind(write_context.layer_str)
        .bind(&write_context.event_type_str)
        .bind(&input.value)
        .bind(write_context.source_str)
        .bind(input.confidence.get())
        .bind(input.importance.get())
        .bind(write_context.provenance_source_class)
        .bind(write_context.provenance_reference)
        .bind(write_context.provenance_evidence_uri)
        .bind(write_context.retention_tier)
        .bind(write_context.retention_expires_at.as_deref())
        .bind(write_context.signal_tier_str)
        .bind(write_context.source_kind_str)
        .bind(input.emotion_label.as_deref())
        .bind(input.emotion_valence)
        .bind(input.emotion_arousal)
        .bind(input.emotion_confidence)
        .bind(write_context.privacy_str)
        .bind(now)
        .bind(now)
        .bind(supersedes_event_id)
        .bind(integrity_prev_hash)
        .bind(integrity_hash)
        .execute(&mut **tx)
        .await
        .pg_write("insert memory_events row")?;

        Ok(())
    }

    async fn update_contradiction_penalty(
        tx: &mut PgTx<'_>,
        input: &MemoryEventInput,
        supersedes_event_id: Option<&str>,
    ) -> PostgresMemoryResult<()> {
        if supersedes_event_id.is_none() {
            return Ok(());
        }

        let unit_id = format!("{}::{}", input.entity_id, input.slot_key);
        let penalty = Self::contradiction_penalty(input.confidence.get(), input.importance.get());
        query(
            "UPDATE retrieval_units SET contradiction_penalty = $1 \
             WHERE unit_id = $2 AND contradiction_penalty < $1",
        )
        .bind(penalty)
        .bind(&unit_id)
        .execute(&mut **tx)
        .await
        .pg_write("update contradiction penalty")?;

        Ok(())
    }

    async fn upsert_belief_slot(
        tx: &mut PgTx<'_>,
        input: &MemoryEventInput,
        event_id: &str,
        now: &str,
        write_context: &AppendEventWriteContext<'_>,
    ) -> PostgresMemoryResult<()> {
        query(
            "INSERT INTO belief_slots ( \
                entity_id, slot_key, value, status, winner_event_id, source, \
                confidence, importance, privacy_level, updated_at \
             ) VALUES ($1, $2, $3, 'active', $4, $5, $6, $7, $8, $9::timestamptz) \
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
        .bind(input.entity_id.as_str())
        .bind(input.slot_key.as_str())
        .bind(&input.value)
        .bind(event_id)
        .bind(write_context.source_str)
        .bind(input.confidence.get())
        .bind(input.importance.get())
        .bind(write_context.privacy_str)
        .bind(now)
        .execute(&mut **tx)
        .await
        .pg_write("upsert belief_slots")?;

        Ok(())
    }

    async fn upsert_retrieval_unit(
        &self,
        tx: &mut PgTx<'_>,
        input: &MemoryEventInput,
        now: &str,
        write_context: &AppendEventWriteContext<'_>,
        embedding: Option<&[f32]>,
    ) -> PostgresMemoryResult<()> {
        let unit_id = format!("{}::{}", input.entity_id, input.slot_key);
        let embedding_vec = embedding.map(HalfVector::from_f32_slice);
        let embedding_dim = embedding.map(Self::embedding_dim);
        let embedding_model = embedding.map(|_| self.embedder.name().to_string());

        query(
            "INSERT INTO retrieval_units ( \
                unit_id, entity_id, slot_key, content, content_type, signal_tier, \
                promotion_status, source_kind, \
                recency_score, importance, reliability, visibility, \
                embedding, embedding_model, embedding_dim, layer, \
                provenance_source_class, provenance_reference, provenance_evidence_uri, \
                retention_tier, retention_expires_at, \
                created_at, updated_at \
             ) VALUES ( \
                $1, $2, $3, $4, 'belief', $5, 'promoted', $6, \
                1.0, $7, 0.8, $8, \
                $9, $10, $11, $12, \
                $13, $14, $15, \
                $16, $17::timestamptz, \
                $18::timestamptz, $19::timestamptz \
             ) ON CONFLICT (unit_id) DO UPDATE SET \
                content = EXCLUDED.content, \
                signal_tier = EXCLUDED.signal_tier, \
                source_kind = EXCLUDED.source_kind, \
                importance = EXCLUDED.importance, \
                visibility = EXCLUDED.visibility, \
                embedding = EXCLUDED.embedding, \
                embedding_model = EXCLUDED.embedding_model, \
                embedding_dim = EXCLUDED.embedding_dim, \
                layer = EXCLUDED.layer, \
                provenance_source_class = EXCLUDED.provenance_source_class, \
                provenance_reference = EXCLUDED.provenance_reference, \
                provenance_evidence_uri = EXCLUDED.provenance_evidence_uri, \
                retention_tier = EXCLUDED.retention_tier, \
                retention_expires_at = EXCLUDED.retention_expires_at, \
                recency_score = GREATEST(0.0, 1.0 - EXTRACT(EPOCH FROM (now() - retrieval_units.created_at)) / (90.0 * 86400.0)), \
                updated_at = EXCLUDED.updated_at",
        )
        .bind(&unit_id)
        .bind(input.entity_id.as_str())
        .bind(input.slot_key.as_str())
        .bind(&input.value)
        .bind(write_context.signal_tier_str)
        .bind(write_context.source_kind_str)
        .bind(input.importance.get())
        .bind(write_context.privacy_str)
        .bind(embedding_vec)
        .bind(embedding_model)
        .bind(embedding_dim)
        .bind(write_context.layer_str)
        .bind(write_context.provenance_source_class)
        .bind(write_context.provenance_reference)
        .bind(write_context.provenance_evidence_uri)
        .bind(write_context.retention_tier)
        .bind(write_context.retention_expires_at.as_deref())
        .bind(now)
        .bind(now)
        .execute(&mut **tx)
        .await
        .pg_write("upsert retrieval_units")?;

        Ok(())
    }

    fn embedding_dim(embedding: &[f32]) -> i32 {
        // Cast safety: embedding dimensions are small model constants and fit i32.
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let dim = embedding.len() as i32;
        dim
    }

    async fn decide_replacement(
        tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
        input: &MemoryEventInput,
    ) -> PostgresMemoryResult<(bool, Option<String>)> {
        let row = query(
            "SELECT winner_event_id, source, confidence, \
                    to_char(updated_at, 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at_str \
             FROM belief_slots \
             WHERE entity_id = $1 AND slot_key = $2 AND status = 'active'",
        )
        .bind(input.entity_id.as_str())
        .bind(input.slot_key.as_str())
        .fetch_optional(&mut **tx)
        .await
        .pg_query("query belief_slots for replacement decision")?;

        let Some(row) = row else {
            return Ok((true, None));
        };

        let incumbent_event_id: String = row.get("winner_event_id");
        let incumbent_source_str: String = row.get("source");
        let incumbent_confidence: f64 = row.get("confidence");
        let incumbent_updated_at: String = row.get("updated_at_str");

        let incumbent_source = codec::str_to_source(&incumbent_source_str);
        let incoming_priority = Self::source_priority(input.source);
        let incumbent_priority = Self::source_priority(incumbent_source);

        let should_replace = match incoming_priority.cmp(&incumbent_priority) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => {
                // Practical tolerance for confidence (0.0–1.0 range)
                const CONFIDENCE_EPSILON: f64 = 0.001;
                if (input.confidence.get() - incumbent_confidence).abs() > CONFIDENCE_EPSILON {
                    input.confidence.get() > incumbent_confidence
                } else {
                    Self::compare_normalized_timestamps(&input.occurred_at, &incumbent_updated_at)
                        != std::cmp::Ordering::Less
                }
            }
        };

        let supersedes = if should_replace {
            Some(incumbent_event_id)
        } else {
            None
        };

        Ok((should_replace, supersedes))
    }

    /// Promote a retrieval unit from `raw` to `candidate` once the same slot
    /// has been confirmed by at least two independent sources. Cross-source
    /// corroboration is the minimum bar for elevating a raw signal into the
    /// candidate tier used by recall scoring.
    async fn maybe_promote_to_candidate(
        tx: &mut sqlx_core::transaction::Transaction<'_, sqlx_postgres::Postgres>,
        entity_id: &str,
        slot_key: &str,
    ) -> PostgresMemoryResult<()> {
        let distinct_sources: i64 = query(
            "SELECT COUNT(DISTINCT source) FROM memory_events \
             WHERE entity_id = $1 AND slot_key = $2",
        )
        .bind(entity_id)
        .bind(slot_key)
        .fetch_one(&mut **tx)
        .await
        .map(|row| row.get::<i64, _>(0))
        .pg_query("count distinct memory event sources for promotion")?;

        if distinct_sources >= 2 {
            let unit_id = format!("{entity_id}::{slot_key}");
            query(
                "UPDATE retrieval_units SET promotion_status = 'candidate' \
                 WHERE unit_id = $1 AND promotion_status = 'raw'",
            )
            .bind(&unit_id)
            .execute(&mut **tx)
            .await
            .pg_write("promote raw to candidate")?;
        }

        Ok(())
    }

    fn normalize_input(mut input: MemoryEventInput) -> PostgresMemoryResult<MemoryEventInput> {
        input.entity_id = crate::contracts::ids::EntityId::new(input.entity_id.as_str().trim());
        input.slot_key = crate::contracts::ids::SlotKey::new(input.slot_key.as_str().trim());
        input.value = input.value.trim().to_string();
        input = input
            .normalize_for_ingress()
            .map_err(PostgresMemoryError::validation)?;
        if input.emotion_label.is_none()
            && let Some(ctx) = infer_emotion_from_text(&input.value)
        {
            input.emotion_label = Some(ctx.label);
            input.emotion_valence = Some(ctx.valence);
            input.emotion_arousal = Some(ctx.arousal);
            input.emotion_confidence = Some(ctx.confidence);
        }
        Ok(input)
    }
}

impl<'a> AppendEventWriteContext<'a> {
    fn from_input(input: &'a MemoryEventInput, now: &str) -> Self {
        let signal_tier = input.signal_tier.unwrap_or(SignalTier::Raw);
        let provenance = input.provenance.as_ref();

        Self {
            layer_str: codec::layer_to_str(input.layer),
            retention_tier: codec::retention_tier_for_layer(input.layer),
            retention_expires_at: codec::retention_expiry_for_layer(input.layer, now),
            signal_tier_str: codec::signal_tier_to_str(signal_tier),
            source_str: codec::source_to_str(input.source),
            privacy_str: codec::privacy_to_str(&input.privacy_level),
            event_type_str: input.event_type.to_string(),
            provenance_source_class: provenance.map(|p| codec::source_to_str(p.source_class)),
            provenance_reference: provenance.map(|p| p.reference.as_str()),
            provenance_evidence_uri: provenance.and_then(|p| p.evidence_uri.as_deref()),
            source_kind_str: input.source_kind.map(codec::source_kind_to_str),
        }
    }
}
