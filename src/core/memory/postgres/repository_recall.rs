//! Read-path recall for the `PostgreSQL` memory backend.
//!
//! Implements `recall_scoped` as a multi-phase pipeline:
//!
//! 1. **Search & merge** — run FTS (tsvector/trgm) and vector (HNSW) queries
//!    in parallel, then combine with weighted hybrid merge. Tries `note`-tier
//!    nodes first; falls back to `episode`-tier when notes return nothing.
//! 2. **Metadata load** — bulk-fetch `retrieval_units` + `belief_slots` metadata
//!    for all candidates in a single parameterised query.
//! 3. **Graph scores** — optional `graph_edges` traversal when
//!    `graph_retrieval_fusion_enabled` is set.
//! 4. **Scoring** — six-phase composite score per candidate:
//!    filter → recency decay → contradiction penalty → trend boost →
//!    base score blend → graph fusion.
//! 5. **Graph activation reranking** — PPR spreading from top-5 seeds via
//!    `GraphActivationCache`, blended with [`crate::core::memory::reranking`].
//! 6. **Side-effects** — bump access counters and apply recall-hit
//!    reinforcement (recency + reliability nudge) for surfaced items.

use std::collections::{HashMap, HashSet};

use sqlx_core::query::query;
use sqlx_core::row::Row;

use super::PostgresMemory;
use super::error::{PostgresMemoryError, PostgresMemoryResult, PostgresMemoryResultExt};
use crate::contracts::ids::{EntityId, SlotKey};
use crate::core::memory::embeddings::EmbeddingRole;
use crate::core::memory::graphrag::{PprQuery, activation_cache};
use crate::core::memory::reranking::{blend_with_ppr, node_distance_rerank};
use crate::core::memory::traits::{MemoryRecallEntry, RecallQuery, SignalTier};
use crate::core::memory::{MemoryLayer, codec, vector};

/// Metadata loaded for each retrieval unit candidate.
struct RecallMetadata {
    unit_id: String,
    entity_id: EntityId,
    slot_key: SlotKey,
    content: String,
    _content_type: String,
    signal_tier: String,
    recency_score: f64,
    confidence: f64,
    importance: f64,
    reliability: f64,
    contradiction_penalty: f64,
    visibility: String,
    layer: String,
    source: Option<String>,
    slot_status: Option<String>,
    denylisted: bool,
    updated_at: String,
}

#[derive(Debug, Clone, Copy)]
struct RecallUtilityVector {
    relevance: f64,
    correction_freshness: f64,
    continuity: f64,
    exposure_safety: f64,
    graph_fit: f64,
}

impl RecallUtilityVector {
    fn final_score(self, graph_weight: f64) -> f64 {
        let graph_weight = graph_weight.clamp(0.0, 1.0);
        let non_graph = 0.55 * self.relevance.clamp(0.0, 1.0)
            + 0.15 * self.correction_freshness.clamp(0.0, 1.0)
            + 0.20 * self.continuity.clamp(0.0, 1.0)
            + 0.10 * self.exposure_safety.clamp(0.0, 1.0);

        ((1.0 - graph_weight) * non_graph + graph_weight * self.graph_fit.clamp(0.0, 1.0))
            .clamp(0.0, 1.0)
    }
}

impl PostgresMemory {
    /// Execute a scoped recall query combining FTS, vector search,
    /// and multi-phase scoring.
    pub(super) async fn recall_scoped_impl(
        &self,
        q: RecallQuery,
    ) -> PostgresMemoryResult<Vec<MemoryRecallEntry>> {
        q.enforce_policy().map_err(PostgresMemoryError::policy)?;

        if q.query.trim().is_empty() || q.limit == 0 {
            return Ok(Vec::new());
        }

        let search_limit = buffered_search_limit(q.limit)?;

        let merged = self.search_and_merge(&q, search_limit).await?;

        if merged.is_empty() {
            return Ok(Vec::new());
        }

        let mut candidate_ids = Vec::with_capacity(merged.len());
        candidate_ids.extend(merged.iter().map(|result| result.id.clone()));

        // Load metadata for all candidates
        let metadata_map = self
            .load_recall_metadata(q.entity_id.as_str(), &candidate_ids)
            .await?;

        let graph_scores = if self.graph_retrieval_fusion_enabled {
            self.graph_scores_for_candidates(q.entity_id.as_str(), &candidate_ids)
                .await?
        } else {
            HashMap::new()
        };

        let mut scored = self.score_candidates(
            merged,
            &metadata_map,
            &graph_scores,
            q.layer_filter.as_ref(),
        );

        #[cfg(feature = "postgres")]
        self.apply_graph_activation_reranking(q.entity_id.as_str(), &mut scored)
            .await?;

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(q.limit);
        let scored: Vec<MemoryRecallEntry> = scored.into_iter().map(|(item, _)| item).collect();

        if let Err(error) = self.bump_recall_access_counters(&scored).await {
            tracing::warn!(%error, "recall access tracking skipped");
        }

        if let Err(error) = self.reinforce_recall_hits(&scored).await {
            tracing::warn!(%error, "recall-hit reinforcement skipped");
        }

        Ok(scored)
    }

    /// Run hybrid search for the `note` tier first; if no results are found,
    /// retry against the `episode` tier. This preference reflects the
    /// knowledge hierarchy: notes are synthesized and higher-signal than
    /// raw episode records.
    async fn search_and_merge(
        &self,
        q: &RecallQuery,
        limit: usize,
    ) -> PostgresMemoryResult<Vec<vector::ScoredResult>> {
        let merged = self
            .search_and_merge_for_tier(q, "note", limit, q.layer_filter.as_ref())
            .await?;
        if merged.is_empty() {
            return self
                .search_and_merge_for_tier(q, "episode", limit, q.layer_filter.as_ref())
                .await;
        }

        Ok(merged)
    }

    async fn search_and_merge_for_tier(
        &self,
        q: &RecallQuery,
        node_tier: &str,
        limit: usize,
        layer_filter: Option<&MemoryLayer>,
    ) -> PostgresMemoryResult<Vec<vector::ScoredResult>> {
        let layer_filter = layer_filter.map(|layer| codec::layer_to_str(*layer));
        let fts = self
            .fts_search_scoped(
                q.entity_id.as_str(),
                &q.query,
                node_tier,
                limit,
                layer_filter,
            )
            .await?;

        let query_embedding = self
            .get_or_compute_embedding(EmbeddingRole::Query, &q.query)
            .await?;
        let vector = if let Some(ref emb) = query_embedding {
            self.vector_search_scoped(q.entity_id.as_str(), emb, node_tier, limit, layer_filter)
                .await?
        } else {
            Vec::new()
        };

        let merged = if vector.is_empty() {
            fts.into_iter()
                .map(|(id, score)| vector::ScoredResult {
                    id,
                    vector_score: None,
                    keyword_score: Some(score),
                    final_score: score,
                })
                .collect()
        } else if fts.is_empty() {
            vector
                .into_iter()
                .map(|(id, score)| vector::ScoredResult {
                    id,
                    vector_score: Some(score),
                    keyword_score: None,
                    final_score: score,
                })
                .collect()
        } else {
            #[allow(clippy::cast_possible_truncation)]
            vector::hybrid_merge(
                &vector,
                &fts,
                self.vector_weight as f32,
                self.keyword_weight as f32,
                limit,
            )
        };

        Ok(merged)
    }

    async fn load_recall_metadata(
        &self,
        entity_id: &str,
        candidate_ids: &[String],
    ) -> PostgresMemoryResult<HashMap<String, RecallMetadata>> {
        if candidate_ids.is_empty() {
            return Ok(HashMap::new());
        }

        // Build placeholder list: $2, $3, $4, ...
        let placeholders: Vec<String> = (2..=candidate_ids.len() + 1)
            .map(|i| format!("${i}"))
            .collect();
        let placeholder_list = placeholders.join(", ");

        let sql = format!(
            "SELECT ru.unit_id, ru.entity_id, ru.slot_key, ru.content, ru.content_type, \
                    ru.signal_tier, ru.recency_score, ru.importance, ru.reliability, \
                    ru.contradiction_penalty, ru.visibility, ru.layer, \
                    to_char(ru.updated_at, 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at_str, \
                    bs.source AS belief_source, \
                    bs.confidence AS belief_confidence, \
                    bs.status AS slot_status, \
                    EXISTS( \
                        SELECT 1 FROM deletion_ledger dl \
                        WHERE dl.entity_id = ru.entity_id AND dl.target_slot_key = ru.slot_key \
                          AND dl.phase IN ('soft', 'hard', 'tombstone') \
                    ) AS denylisted \
             FROM retrieval_units ru \
             LEFT JOIN belief_slots bs ON bs.entity_id = ru.entity_id AND bs.slot_key = ru.slot_key \
             WHERE ru.entity_id = $1 AND ru.unit_id IN ({placeholder_list})"
        );

        let mut q = query(&sql).bind(entity_id);
        for id in candidate_ids {
            q = q.bind(id);
        }

        let rows = q
            .fetch_all(&self.pool)
            .await
            .pg_query("load recall metadata")?;

        let mut map = HashMap::with_capacity(rows.len());
        for row in rows {
            let meta = RecallMetadata {
                unit_id: row.get("unit_id"),
                entity_id: EntityId::new(row.get::<String, _>("entity_id")),
                slot_key: SlotKey::new(row.get::<String, _>("slot_key")),
                content: row.get("content"),
                _content_type: row.get("content_type"),
                signal_tier: row.get("signal_tier"),
                recency_score: row.get("recency_score"),
                confidence: row
                    .try_get::<Option<f64>, _>("belief_confidence")
                    .pg_query("decode belief confidence")?
                    .unwrap_or(0.5),
                importance: row.get::<f64, _>("importance"),
                reliability: row.get("reliability"),
                contradiction_penalty: row.get("contradiction_penalty"),
                visibility: row.get("visibility"),
                layer: row.get("layer"),
                source: row
                    .try_get::<Option<String>, _>("belief_source")
                    .pg_query("decode belief source")?,
                slot_status: row
                    .try_get::<Option<String>, _>("slot_status")
                    .pg_query("decode slot status")?,
                denylisted: row.get("denylisted"),
                updated_at: row.get("updated_at_str"),
            };
            map.insert(meta.unit_id.clone(), meta);
        }

        Ok(map)
    }

    async fn graph_scores_for_candidates(
        &self,
        entity_id: &str,
        candidate_ids: &[String],
    ) -> PostgresMemoryResult<HashMap<String, f64>> {
        if candidate_ids.is_empty() {
            return Ok(HashMap::new());
        }

        // Map unit_id to slot graph entity ids.
        // unit_id format is "entity_id::slot_key" — convert to graph entity "slot::entity_id::slot_key"
        let slot_ids: Vec<String> = candidate_ids
            .iter()
            .filter(|uid| uid.contains("::"))
            .map(|uid| format!("slot::{uid}"))
            .collect();

        if slot_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let placeholders: Vec<String> = (2..=slot_ids.len() + 1).map(|i| format!("${i}")).collect();
        let placeholder_list = placeholders.join(", ");

        let sql = format!(
            "SELECT to_entity_id, relation_type, weight \
             FROM graph_edges \
             WHERE owner_entity_id = $1 AND to_entity_id IN ({placeholder_list})"
        );

        let mut q = query(&sql).bind(entity_id);
        for id in &slot_ids {
            q = q.bind(id);
        }

        let rows = q
            .fetch_all(&self.pool)
            .await
            .pg_query("load graph scores")?;

        let mut raw_scores: HashMap<String, f64> = HashMap::with_capacity(rows.len());
        for row in rows {
            let to_id: String = row.get("to_entity_id");
            let relation: String = row.get("relation_type");
            let weight: f64 = row.get("weight");

            let contribution = graph_relation_contribution(&relation, weight);

            // Convert slot graph id back to unit_id ("slot::entity_id::slot_key" → "entity_id::slot_key")
            let unit_id = to_id.strip_prefix("slot::").map(String::from);

            if let Some(uid) = unit_id {
                *raw_scores.entry(uid).or_insert(0.0) += contribution;
            }
        }

        // Normalize scores to [0, 1]
        let normalized: HashMap<String, f64> = raw_scores
            .into_iter()
            .map(|(uid, score)| {
                let clamped = score.clamp(-1.0, 1.0);
                let normalized = f64::midpoint(clamped, 1.0).clamp(0.0, 1.0);
                (uid, normalized)
            })
            .collect();

        Ok(normalized)
    }

    fn score_candidates(
        &self,
        candidates: Vec<vector::ScoredResult>,
        metadata: &HashMap<String, RecallMetadata>,
        graph_scores: &HashMap<String, f64>,
        layer_filter: Option<&MemoryLayer>,
    ) -> Vec<(MemoryRecallEntry, f64)> {
        candidates
            .into_iter()
            .filter_map(|candidate| {
                let meta = metadata.get(&candidate.id)?;
                if !Self::matches_layer_filter(meta, layer_filter) {
                    return None;
                }
                let (item, score) = self.score_candidate(&candidate, meta, graph_scores)?;
                Some((item, score))
            })
            .collect()
    }

    fn matches_layer_filter(meta: &RecallMetadata, layer_filter: Option<&MemoryLayer>) -> bool {
        layer_filter.is_none_or(|layer| meta.layer == codec::layer_to_str(*layer))
    }

    #[cfg(feature = "postgres")]
    async fn apply_graph_activation_reranking(
        &self,
        owner_entity_id: &str,
        scored: &mut [(MemoryRecallEntry, f64)],
    ) -> PostgresMemoryResult<()> {
        if scored.is_empty() {
            return Ok(());
        }

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let owner_id = EntityId::new(owner_entity_id);
        let snapshot = activation_cache()
            .get_or_load(&owner_id, &self.pool)
            .await
            .pg_query("load graph activation snapshot")?;

        let seed_count = scored.len().min(5);
        #[allow(clippy::cast_possible_truncation)]
        let seeds: Vec<(EntityId, f32)> = scored
            .iter()
            .take(seed_count)
            .map(|(item, score)| {
                let graph_entity_id = EntityId::new(format!(
                    "slot::{}::{}",
                    owner_entity_id,
                    item.slot_key.as_str()
                ));
                (graph_entity_id, (*score).clamp(0.0, 1.0) as f32)
            })
            .collect();

        if seeds.is_empty() {
            return Ok(());
        }

        let ppr_results = snapshot.run_ppr(&PprQuery {
            seeds: seeds.clone(),
            top_k: scored.len(),
            ..PprQuery::default()
        });

        let mut graph_items: Vec<MemoryRecallEntry> = scored
            .iter()
            .map(|(item, _)| {
                let mut graph_item = item.clone();
                graph_item.entity_id = EntityId::new(format!(
                    "slot::{}::{}",
                    owner_entity_id,
                    item.slot_key.as_str()
                ));
                graph_item
            })
            .collect();

        blend_with_ppr(&mut graph_items, &ppr_results, 0.3);

        let center_entity_ids: Vec<EntityId> =
            seeds.into_iter().map(|(entity_id, _)| entity_id).collect();
        node_distance_rerank(&mut graph_items, &center_entity_ids, &snapshot, 2);

        for ((item, score), graph_item) in scored.iter_mut().zip(graph_items) {
            item.score = graph_item.score;
            *score = graph_item.score;
        }

        Ok(())
    }

    async fn reinforce_recall_hits(&self, items: &[MemoryRecallEntry]) -> PostgresMemoryResult<()> {
        if items.is_empty() {
            return Ok(());
        }

        let mut visited = HashSet::new();
        for item in items {
            let dedup_key = format!("{}::{}", item.entity_id, item.slot_key);
            if !visited.insert(dedup_key) {
                continue;
            }

            let (recency_delta, reliability_delta) = Self::recall_reinforcement_delta(item);
            if recency_delta <= 0.0 && reliability_delta <= 0.0 {
                continue;
            }

            query(
                "UPDATE retrieval_units
                 SET recency_score = LEAST(1.0, recency_score + $1),
                     reliability = LEAST(1.0, reliability + $2)
                 WHERE entity_id = $3
                   AND slot_key = $4
                   AND promotion_status IN ('promoted', 'candidate')
                   AND visibility != 'secret'",
            )
            .bind(recency_delta)
            .bind(reliability_delta)
            .bind(item.entity_id.as_str())
            .bind(item.slot_key.as_str())
            .execute(&self.pool)
            .await
            .pg_write("reinforce recall hit")?;
        }

        Ok(())
    }

    async fn bump_recall_access_counters(
        &self,
        items: &[MemoryRecallEntry],
    ) -> PostgresMemoryResult<()> {
        if items.is_empty() {
            return Ok(());
        }

        let unit_ids: Vec<String> = items
            .iter()
            .map(|item| format!("{}::{}", item.entity_id, item.slot_key))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        query(
            "UPDATE retrieval_units
             SET access_count = access_count + 1,
                 accessed_at = now()
             WHERE unit_id = ANY($1::text[])",
        )
        .bind(&unit_ids)
        .execute(&self.pool)
        .await
        .pg_write("update retrieval_units access counters after recall")?;

        Ok(())
    }

    fn recall_reinforcement_delta(item: &MemoryRecallEntry) -> (f64, f64) {
        let score = item.score.clamp(0.0, 1.0);
        let confidence = item.confidence.get();
        let importance = item.importance.get();

        let recency_delta = (0.008 + 0.014 * score + 0.008 * importance).clamp(0.0, 0.04);
        let reliability_delta = (0.001 + 0.006 * confidence + 0.003 * importance).clamp(0.0, 0.015);
        (recency_delta, reliability_delta)
    }

    fn score_candidate(
        &self,
        candidate: &vector::ScoredResult,
        meta: &RecallMetadata,
        graph_scores: &HashMap<String, f64>,
    ) -> Option<(MemoryRecallEntry, f64)> {
        // Phase 1: Filtering
        if !Self::allowed_for_replay(meta) {
            return None;
        }

        let signal_tier = codec::str_to_signal_tier(&meta.signal_tier);

        // Phase 2: Recency decay
        let is_trend = meta.slot_key.as_str().starts_with("trend.")
            || meta.slot_key.as_str().starts_with("trend/")
            || meta.slot_key.as_str().contains("trend.");
        let recency = if is_trend {
            Self::trend_recency_decay(meta.recency_score)
        } else {
            Self::standard_recency_decay(meta.recency_score)
        };

        // Phase 3: Contradiction
        let penalty = meta.contradiction_penalty.clamp(0.0, 1.0);

        // Phase 4: Trend boost
        let trend_boost = if is_trend
            && signal_tier == SignalTier::Raw
            && meta.recency_score > (1.0 - Self::TREND_TTL_DAYS / Self::TREND_DECAY_WINDOW_DAYS)
        {
            0.05
        } else {
            0.0
        };

        // Phase 5: Guardrailed utility scoring
        let search_score = f64::from(candidate.final_score);
        let relevance =
            ((search_score + trend_boost).clamp(0.0, 1.0) * (1.0 - penalty)).clamp(0.0, 1.0);
        let continuity =
            ((0.5 * meta.importance + 0.5 * meta.reliability) * (1.0 - penalty)).clamp(0.0, 1.0);

        // Phase 6: Graph fusion
        let graph_score = graph_scores.get(&candidate.id).copied().unwrap_or(0.5);
        let utility = RecallUtilityVector {
            relevance,
            correction_freshness: recency,
            continuity,
            exposure_safety: Self::exposure_safety_score(&meta.visibility),
            graph_fit: graph_score,
        };
        let final_score = utility.final_score(self.graph_retrieval_weight);

        let item = MemoryRecallEntry {
            entity_id: meta.entity_id.clone(),
            slot_key: meta.slot_key.clone(),
            value: meta.content.clone(),
            source: codec::str_to_source(meta.source.as_deref().unwrap_or("system")),
            confidence: meta.confidence.into(),
            importance: meta.importance.into(),
            privacy_level: codec::str_to_privacy(&meta.visibility),
            score: final_score,
            occurred_at: meta.updated_at.clone(),
        };

        Some((item, final_score))
    }

    fn allowed_for_replay(meta: &RecallMetadata) -> bool {
        if meta.denylisted {
            return false;
        }
        match meta.slot_status.as_deref() {
            Some("soft_deleted" | "tombstoned") => false,
            _ => meta.visibility != "secret",
        }
    }

    fn exposure_safety_score(visibility: &str) -> f64 {
        match visibility {
            "public" => 1.0,
            "private" => 0.85,
            "internal" => 0.70,
            "secret" => 0.0,
            _ => 0.75,
        }
    }

    /// Apply a linear recency weighting with a 0.20 floor so even very old
    /// memories remain discoverable when they are the best keyword match.
    fn standard_recency_decay(recency_score: f64) -> f64 {
        (recency_score * 0.80 + 0.20).clamp(0.0, 1.0)
    }

    /// Apply TTL-gated recency for trend slots: full score while within
    /// `TREND_TTL_DAYS`, linearly scaled to zero over the remaining
    /// `TREND_DECAY_WINDOW_DAYS` window.
    fn trend_recency_decay(recency_score: f64) -> f64 {
        let threshold = 1.0 - Self::TREND_TTL_DAYS / Self::TREND_DECAY_WINDOW_DAYS;
        if recency_score >= threshold {
            1.0
        } else {
            (recency_score / threshold).clamp(0.0, 1.0)
        }
    }
}

fn graph_relation_contribution(relation: &str, weight: f64) -> f64 {
    match relation {
        "has_slot" => 0.05 * weight,
        "recorded_event" => 0.20 * weight,
        "supersedes" => 0.15 * weight,
        "contradicted_by" => -0.30 * weight,
        "participates_in" => 0.26 * weight,
        "prefers" => 0.34 * weight,
        "discusses" => 0.30 * weight,
        "reflects" => 0.28 * weight,
        "continues_from" => 0.38 * weight,
        "occurs_in" => 0.24 * weight,
        "mentions" => 0.18 * weight,
        _ => 0.0,
    }
}

fn buffered_search_limit(limit: usize) -> PostgresMemoryResult<usize> {
    limit
        .checked_mul(3)
        .ok_or_else(|| PostgresMemoryError::conversion("recall search limit overflow"))
}

#[cfg(test)]
mod tests {
    use super::{
        PostgresMemory, RecallUtilityVector, buffered_search_limit, graph_relation_contribution,
    };
    use crate::core::memory::{MemoryLayer, MemoryRecallEntry, MemorySource, PrivacyLevel};

    fn item(score: f64, confidence: f64, importance: f64) -> MemoryRecallEntry {
        MemoryRecallEntry {
            entity_id: "tenant-alpha:user-1".into(),
            slot_key: "profile.preference.language".into(),
            value: "Rust".to_string(),
            source: MemorySource::ExplicitUser,
            confidence: confidence.into(),
            importance: importance.into(),
            privacy_level: PrivacyLevel::Private,
            score,
            occurred_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn metadata_with_layer(layer: &str) -> super::RecallMetadata {
        super::RecallMetadata {
            unit_id: "person:test::identity.objective".to_string(),
            entity_id: "person:test".into(),
            slot_key: "identity.objective".into(),
            content: "objective".to_string(),
            _content_type: "belief".to_string(),
            signal_tier: "belief".to_string(),
            recency_score: 1.0,
            confidence: 0.9,
            importance: 0.8,
            reliability: 0.8,
            contradiction_penalty: 0.0,
            visibility: "private".to_string(),
            layer: layer.to_string(),
            source: Some("system".to_string()),
            slot_status: None,
            denylisted: false,
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn reinforcement_delta_is_bounded() {
        let (recency_delta, reliability_delta) =
            PostgresMemory::recall_reinforcement_delta(&item(1.0, 1.0, 1.0));
        assert!(recency_delta > 0.0 && recency_delta <= 0.04);
        assert!(reliability_delta > 0.0 && reliability_delta <= 0.015);
    }

    #[test]
    fn reinforcement_delta_scales_with_higher_quality_hits() {
        let low = PostgresMemory::recall_reinforcement_delta(&item(0.2, 0.2, 0.2));
        let high = PostgresMemory::recall_reinforcement_delta(&item(0.9, 0.9, 0.9));
        assert!(high.0 > low.0);
        assert!(high.1 > low.1);
    }

    #[test]
    fn companion_graph_relations_score_above_generic_has_slot() {
        let generic = graph_relation_contribution("has_slot", 1.0);
        let continuity = graph_relation_contribution("continues_from", 1.0);
        let preference = graph_relation_contribution("prefers", 1.0);

        assert!(continuity > generic);
        assert!(preference > generic);
    }

    #[test]
    fn buffered_search_limit_rejects_overflow() {
        let err = buffered_search_limit(usize::MAX)
            .expect_err("overflowing recall over-fetch should be rejected")
            .to_string();

        assert!(err.contains("recall search limit overflow"));
    }

    #[test]
    fn buffered_search_limit_applies_three_x_buffer() {
        assert_eq!(buffered_search_limit(20).unwrap(), 60);
    }

    #[test]
    fn contradicted_relations_penalize_candidates() {
        let penalty = graph_relation_contribution("contradicted_by", 1.0);
        assert!(penalty < 0.0);
    }

    #[test]
    fn recall_utility_keeps_relevance_above_freshness_alone() {
        let old_relevant = RecallUtilityVector {
            relevance: 0.95,
            correction_freshness: 0.20,
            continuity: 0.75,
            exposure_safety: 0.85,
            graph_fit: 0.5,
        };
        let fresh_weak = RecallUtilityVector {
            relevance: 0.20,
            correction_freshness: 1.0,
            continuity: 0.75,
            exposure_safety: 0.85,
            graph_fit: 0.5,
        };

        assert!(old_relevant.final_score(0.0) > fresh_weak.final_score(0.0));
    }

    #[test]
    fn recall_utility_keeps_graph_weight_bounded() {
        let low_graph_high_direct = RecallUtilityVector {
            relevance: 0.90,
            correction_freshness: 0.70,
            continuity: 0.80,
            exposure_safety: 0.85,
            graph_fit: 0.10,
        };
        let high_graph_low_direct = RecallUtilityVector {
            relevance: 0.20,
            correction_freshness: 0.20,
            continuity: 0.20,
            exposure_safety: 0.85,
            graph_fit: 1.0,
        };

        assert!(low_graph_high_direct.final_score(0.3) > high_graph_low_direct.final_score(0.3));
    }

    #[test]
    fn exposure_safety_scores_secret_as_unusable() {
        assert_eq!(PostgresMemory::exposure_safety_score("secret"), 0.0);
        assert!(PostgresMemory::exposure_safety_score("public") > 0.9);
        assert!(PostgresMemory::exposure_safety_score("private") >= 0.8);
    }

    #[test]
    fn recall_layer_filter_matches_identity_metadata() {
        let identity = metadata_with_layer("identity");
        let semantic = metadata_with_layer("semantic");

        assert!(PostgresMemory::matches_layer_filter(
            &identity,
            Some(&MemoryLayer::Identity)
        ));
        assert!(!PostgresMemory::matches_layer_filter(
            &semantic,
            Some(&MemoryLayer::Identity)
        ));
        assert!(PostgresMemory::matches_layer_filter(&semantic, None));
    }
}
