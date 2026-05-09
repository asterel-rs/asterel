//! Post-retrieval reranking for memory recall results.
//!
//! Two independent stages that can be composed:
//!
//! 1. **MMR (Maximal Marginal Relevance)** -- reduces redundancy by
//!    balancing relevance against diversity.  Uses embedding cosine
//!    similarity when available; falls back to Jaccard token overlap.
//!
//! 2. **LLM reranking** -- re-scores the top-N candidates via an LLM
//!    prompt, blending the LLM relevance score with the original score.
//!
//! References: [RRF] Cormack et al., 2009 — Reciprocal Rank Fusion. See the
//! public research reference index in the docs site.

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;

use super::graphrag::{GraphSnapshot, PprResult};
use super::memory_types::MemoryRecallEntry as MemoryRecallItem;
use super::vector::cosine_similarity;
use crate::contracts::ids::EntityId;

// ── MMR ──────────────────────────────────────────────────────────

/// Configuration knobs for MMR reranking.
#[derive(Debug, Clone, Copy)]
pub struct MmrConfig {
    /// Trade-off between relevance (`1.0`) and diversity (`0.0`).
    /// Default: `0.7`.
    pub lambda: f64,
}

impl Default for MmrConfig {
    fn default() -> Self {
        Self { lambda: 0.7 }
    }
}

/// Re-rank `items` using Maximal Marginal Relevance.
///
/// When `embeddings` are provided (one per item, same order), cosine
/// similarity drives the diversity penalty.  When `embeddings` is
/// `None` (or length mismatch), the function falls back to Jaccard
/// token overlap on the textual `value` field.
///
/// Returns items in the new MMR order.  The `score` field on each
/// item is **not** mutated -- call-site ordering is the reranking
/// signal.
///
/// # Panics
///
/// Never panics in correct use. The internal `.expect()` guards an
/// algorithm invariant (each candidate index is selected exactly once).
#[must_use]
pub fn mmr_rerank(
    items: Vec<MemoryRecallItem>,
    embeddings: Option<&[Vec<f32>]>,
    config: &MmrConfig,
) -> Vec<MemoryRecallItem> {
    if items.len() <= 1 {
        return items;
    }

    let n = items.len();
    let lambda = config.lambda.clamp(0.0, 1.0);

    // Pre-compute a pairwise similarity function.
    let use_embeddings =
        embeddings.is_some_and(|e| e.len() == n && e.iter().all(|v| !v.is_empty()));

    // Run MMR in a contained scope so `tokens` (which borrows `&str` slices
    // from `items[*].value`) drops before we consume `items` into the output.
    let selected: Vec<usize> = {
        // Tokenize once for Jaccard fallback.
        let tokens: Vec<HashSet<&str>> = if use_embeddings {
            Vec::new()
        } else {
            items
                .iter()
                .map(|item| item.value.split_whitespace().collect::<HashSet<&str>>())
                .collect()
        };

        let similarity = |i: usize, j: usize| -> f64 {
            if use_embeddings && let Some(embs) = embeddings {
                return f64::from(cosine_similarity(&embs[i], &embs[j]));
            }
            // Jaccard fallback
            if tokens.is_empty() {
                return 0.0;
            }
            let inter = tokens[i].intersection(&tokens[j]).count();
            let union = tokens[i].union(&tokens[j]).count();
            if union == 0 {
                0.0
            } else {
                #[allow(clippy::cast_precision_loss)]
                let sim = inter as f64 / union as f64;
                sim
            }
        };

        let mut selected: Vec<usize> = Vec::with_capacity(n);
        let mut remaining: Vec<usize> = (0..n).collect();

        for _ in 0..n {
            let mut best_idx_in_remaining = 0;
            let mut best_mmr = f64::NEG_INFINITY;

            for (pos, &candidate) in remaining.iter().enumerate() {
                let relevance = items[candidate].score;

                let max_sim_to_selected = if selected.is_empty() {
                    0.0
                } else {
                    selected
                        .iter()
                        .map(|&s| similarity(candidate, s))
                        .fold(0.0_f64, f64::max)
                };

                let mmr_score = lambda * relevance - (1.0 - lambda) * max_sim_to_selected;

                if mmr_score > best_mmr {
                    best_mmr = mmr_score;
                    best_idx_in_remaining = pos;
                }
            }

            selected.push(remaining.swap_remove(best_idx_in_remaining));
        }

        selected
        // `tokens` and `similarity` drop here, releasing borrows into `items`
    };

    // Wrap items in Option so we can move them out by index without cloning.
    // Algorithm invariant: each index appears in `selected` exactly once.
    let mut slots: Vec<Option<MemoryRecallItem>> = items.into_iter().map(Some).collect();
    selected
        .into_iter()
        .map(|i| slots[i].take().expect("MMR index selected exactly once"))
        .collect()
}

// ── LLM reranking ────────────────────────────────────────────────

/// Trait abstracting the LLM call needed for reranking.
///
/// Implementations can wrap any provider (OpenAI, Anthropic, etc.).
#[allow(clippy::doc_markdown)]
pub trait RerankLlm: Send + Sync {
    /// Score passages for relevance to `query`.
    ///
    /// Returns a JSON array of f64 scores in `[0.0, 1.0]`, one per
    /// passage, in the same order as `passages`.
    fn score_passages<'a>(
        &'a self,
        query: &'a str,
        passages: &'a [&'a str],
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<f64>>> + Send + 'a>>;
}

/// Configuration for LLM-based reranking.
#[derive(Debug, Clone, Copy)]
pub struct LlmRerankConfig {
    /// Max candidates to send to the LLM. Default: 20.
    pub top_n: usize,
    /// LLM score weight in the blend (0.0–1.0). Default: 0.7.
    /// `final = weight * llm_score + (1 - weight) * original_score`
    pub llm_weight: f64,
}

impl Default for LlmRerankConfig {
    fn default() -> Self {
        Self {
            top_n: 20,
            llm_weight: 0.7,
        }
    }
}

/// Re-rank recall items by blending an LLM relevance score with the
/// original retrieval score.
///
/// Only the first `config.top_n` items are sent to the LLM; the rest
/// are appended in their original order.  On LLM failure the original
/// list is returned unchanged.
pub async fn llm_rerank(
    items: Vec<MemoryRecallItem>,
    query: &str,
    llm: &dyn RerankLlm,
    config: &LlmRerankConfig,
) -> Vec<MemoryRecallItem> {
    if items.is_empty() {
        return items;
    }

    let split_at = items.len().min(config.top_n);
    let (candidates, tail) = items.split_at(split_at);

    let passages: Vec<&str> = candidates.iter().map(|item| item.value.as_str()).collect();

    let scores = match llm.score_passages(query, &passages).await {
        Ok(s) if s.len() == candidates.len() => s,
        Ok(s) => {
            tracing::warn!(
                expected = candidates.len(),
                got = s.len(),
                "LLM reranker returned wrong number of scores; using original order"
            );
            return items;
        }
        Err(e) => {
            tracing::warn!(error = %e, "LLM reranking failed; using original order");
            return items;
        }
    };

    let weight = config.llm_weight.clamp(0.0, 1.0);

    let mut rescored_items: Vec<(f64, MemoryRecallItem)> = candidates
        .iter()
        .cloned()
        .zip(scores)
        .map(|(mut item, llm_score)| {
            let llm_score = llm_score.clamp(0.0, 1.0);
            item.score = weight * llm_score + (1.0 - weight) * item.score;
            (item.score, item)
        })
        .collect();

    rescored_items.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut result: Vec<MemoryRecallItem> =
        rescored_items.into_iter().map(|(_, item)| item).collect();
    result.extend_from_slice(tail);
    result
}

/// Boost retrieval scores based on graph distance from a set of center entities.
///
/// Runs BFS from `center_entity_ids` up to `max_hops` hops. Items whose
/// owning entity is reachable receive a proximity boost of up to +0.20
/// (capped), inversely proportional to hop count: `boost = min(0.40 / (1 + hops), 0.20)`.
/// Items not reachable from any center entity are left unchanged.
pub fn node_distance_rerank(
    results: &mut [MemoryRecallItem],
    center_entity_ids: &[EntityId],
    snapshot: &GraphSnapshot,
    max_hops: usize,
) {
    if results.is_empty() || center_entity_ids.is_empty() {
        return;
    }

    let mut distances: HashMap<u32, usize> = HashMap::new();
    let mut queue = VecDeque::new();

    for entity_id in center_entity_ids {
        let Some(index) = snapshot.node_idx(entity_id) else {
            continue;
        };
        if distances.insert(index, 0).is_none() {
            queue.push_back(index);
        }
    }

    if queue.is_empty() {
        return;
    }

    while let Some(current) = queue.pop_front() {
        let hop_count = distances[&current];
        if hop_count >= max_hops {
            continue;
        }

        for &neighbor in snapshot.neighbor_indices(current) {
            if distances.contains_key(&neighbor) {
                continue;
            }
            distances.insert(neighbor, hop_count + 1);
            queue.push_back(neighbor);
        }
    }

    for item in results {
        let Some(index) = snapshot.node_idx(&item.entity_id) else {
            continue;
        };
        let Some(hop_count) = distances.get(&index).copied() else {
            continue;
        };
        let bounded_hop_count = u32::try_from(hop_count).unwrap_or(u32::MAX);
        let boost = (0.40 / (1.0 + f64::from(bounded_hop_count))).min(0.20);
        item.score += boost;
    }
}

/// Blend PPR activation scores into retrieval item scores.
///
/// For each item, looks up its entity in `ppr_results` and applies:
/// `score = (1 − ppr_weight) * score + ppr_weight * ppr_activation`.
/// Items absent from `ppr_results` are treated as having activation 0.0,
/// so their score is down-weighted proportional to `ppr_weight`.
pub fn blend_with_ppr(items: &mut [MemoryRecallItem], ppr_results: &[PprResult], ppr_weight: f64) {
    if items.is_empty() {
        return;
    }

    let ppr_weight = ppr_weight.clamp(0.0, 1.0);
    let ppr_scores: HashMap<&EntityId, f64> = ppr_results
        .iter()
        .map(|result| (&result.entity_id, f64::from(result.activation_score)))
        .collect();

    for item in items {
        let ppr_score = ppr_scores.get(&item.entity_id).copied().unwrap_or(0.0);
        item.score = (1.0 - ppr_weight) * item.score + ppr_weight * ppr_score;
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::cast_precision_loss)]
mod tests {
    use super::*;
    use crate::contracts::scores::{Confidence, Importance};
    use crate::core::memory::memory_types::{MemorySource, PrivacyLevel};

    fn make_item(entity_id: &str, slot: &str, value: &str, score: f64) -> MemoryRecallItem {
        MemoryRecallItem {
            entity_id: entity_id.into(),
            slot_key: slot.into(),
            value: value.into(),
            source: MemorySource::ExplicitUser,
            confidence: Confidence::new(1.0),
            importance: Importance::new(0.5),
            privacy_level: PrivacyLevel::Private,
            score,
            occurred_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    // ── MMR tests ────────────────────────────────────────────────

    #[test]
    fn mmr_empty_input() {
        let result = mmr_rerank(vec![], None, &MmrConfig::default());
        assert!(result.is_empty());
    }

    #[test]
    fn mmr_single_item() {
        let items = vec![make_item("test", "s1", "hello world", 0.9)];
        let result = mmr_rerank(items.clone(), None, &MmrConfig::default());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].slot_key, "s1".into());
    }

    #[test]
    fn mmr_pure_relevance_lambda_1() {
        let items = vec![
            make_item("test", "s1", "alpha beta gamma", 0.9),
            make_item("test", "s2", "alpha beta gamma", 0.7),
            make_item("test", "s3", "delta epsilon zeta", 0.5),
        ];
        let config = MmrConfig { lambda: 1.0 };
        let result = mmr_rerank(items, None, &config);
        // Pure relevance: original order by score
        assert_eq!(result[0].slot_key, "s1".into());
        assert_eq!(result[1].slot_key, "s2".into());
        assert_eq!(result[2].slot_key, "s3".into());
    }

    #[test]
    fn mmr_diversity_demotes_duplicates() {
        // s1 and s2 have identical text (high Jaccard similarity).
        // With lambda=0.7, s3 (diverse) should rank above s2 (duplicate).
        let items = vec![
            make_item("test", "s1", "router vlan iot configuration", 0.92),
            make_item("test", "s2", "router vlan iot configuration", 0.89),
            make_item("test", "s3", "adguard dns setup 192.168.10.2", 0.85),
        ];
        let config = MmrConfig { lambda: 0.7 };
        let result = mmr_rerank(items, None, &config);
        assert_eq!(result[0].slot_key, "s1".into());
        // s3 should be promoted above s2 due to diversity
        assert_eq!(result[1].slot_key, "s3".into());
        assert_eq!(result[2].slot_key, "s2".into());
    }

    #[test]
    fn mmr_with_embeddings() {
        let items = vec![
            make_item("test", "s1", "a", 0.9),
            make_item("test", "s2", "b", 0.85),
            make_item("test", "s3", "c", 0.8),
        ];
        // s1 and s2 have very similar embeddings; s3 is different
        let embeddings = vec![
            vec![1.0, 0.0, 0.0],
            vec![0.99, 0.1, 0.0],
            vec![0.0, 0.0, 1.0],
        ];
        let config = MmrConfig { lambda: 0.5 };
        let result = mmr_rerank(items, Some(&embeddings), &config);
        assert_eq!(result[0].slot_key, "s1".into());
        // s3 should beat s2 because s2 is too similar to s1
        assert_eq!(result[1].slot_key, "s3".into());
    }

    #[test]
    fn mmr_embedding_length_mismatch_falls_back() {
        let items = vec![
            make_item("test", "s1", "same words here", 0.9),
            make_item("test", "s2", "same words here", 0.8),
        ];
        // Wrong number of embeddings -- should fall back to Jaccard
        let embeddings = vec![vec![1.0]];
        let config = MmrConfig::default();
        let result = mmr_rerank(items, Some(&embeddings), &config);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn mmr_preserves_all_items() {
        let items: Vec<MemoryRecallItem> = (0..10)
            .map(|i| {
                make_item(
                    "test",
                    &format!("s{i}"),
                    &format!("text {i}"),
                    1.0 - f64::from(i) * 0.05,
                )
            })
            .collect();
        let result = mmr_rerank(items.clone(), None, &MmrConfig::default());
        assert_eq!(result.len(), items.len());
    }

    // ── LLM reranking tests ─────────────────────────────────────

    struct MockRerankLlm {
        scores: Vec<f64>,
    }

    impl RerankLlm for MockRerankLlm {
        fn score_passages<'a>(
            &'a self,
            _query: &'a str,
            _passages: &'a [&'a str],
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<f64>>> + Send + 'a>> {
            let scores = self.scores.clone();
            Box::pin(async move { Ok(scores) })
        }
    }

    struct FailingRerankLlm;

    impl RerankLlm for FailingRerankLlm {
        fn score_passages<'a>(
            &'a self,
            _query: &'a str,
            _passages: &'a [&'a str],
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<f64>>> + Send + 'a>> {
            Box::pin(async { anyhow::bail!("LLM unavailable") })
        }
    }

    #[tokio::test]
    async fn llm_rerank_empty() {
        let llm = MockRerankLlm { scores: vec![] };
        let result = llm_rerank(vec![], "query", &llm, &LlmRerankConfig::default()).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn llm_rerank_reorders_by_blended_score() {
        let items = vec![
            make_item("test", "s1", "first", 0.9),
            make_item("test", "s2", "second", 0.7),
            make_item("test", "s3", "third", 0.5),
        ];
        // LLM thinks s2 is most relevant, s3 next, s1 least
        let llm = MockRerankLlm {
            scores: vec![0.2, 0.95, 0.6],
        };
        let config = LlmRerankConfig {
            top_n: 20,
            llm_weight: 0.7,
        };
        let result = llm_rerank(items, "test query", &llm, &config).await;

        // s2: 0.7*0.95 + 0.3*0.7 = 0.665 + 0.21 = 0.875
        // s3: 0.7*0.6  + 0.3*0.5 = 0.42  + 0.15 = 0.57
        // s1: 0.7*0.2  + 0.3*0.9 = 0.14  + 0.27 = 0.41
        assert_eq!(result[0].slot_key, "s2".into());
        assert_eq!(result[1].slot_key, "s3".into());
        assert_eq!(result[2].slot_key, "s1".into());
    }

    #[tokio::test]
    async fn llm_rerank_respects_top_n() {
        let items = vec![
            make_item("test", "s1", "first", 0.9),
            make_item("test", "s2", "second", 0.7),
            make_item("test", "s3", "third", 0.5),
        ];
        // Only rerank top 2
        let llm = MockRerankLlm {
            scores: vec![0.3, 0.9],
        };
        let config = LlmRerankConfig {
            top_n: 2,
            llm_weight: 0.7,
        };
        let result = llm_rerank(items, "query", &llm, &config).await;
        assert_eq!(result.len(), 3);
        // s3 should be last (untouched tail)
        assert_eq!(result[2].slot_key, "s3".into());
    }

    #[tokio::test]
    async fn llm_rerank_failure_returns_original() {
        let items = vec![
            make_item("test", "s1", "first", 0.9),
            make_item("test", "s2", "second", 0.7),
        ];
        let llm = FailingRerankLlm;
        let config = LlmRerankConfig::default();
        let result = llm_rerank(items.clone(), "query", &llm, &config).await;
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].slot_key, "s1".into());
        assert_eq!(result[1].slot_key, "s2".into());
    }

    #[tokio::test]
    async fn llm_rerank_wrong_count_returns_original() {
        let items = vec![
            make_item("test", "s1", "first", 0.9),
            make_item("test", "s2", "second", 0.7),
        ];
        // Wrong number of scores
        let llm = MockRerankLlm { scores: vec![0.5] };
        let config = LlmRerankConfig::default();
        let result = llm_rerank(items.clone(), "query", &llm, &config).await;
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].slot_key, "s1".into());
    }

    #[tokio::test]
    async fn llm_rerank_clamps_scores() {
        let items = vec![make_item("test", "s1", "only", 0.5)];
        let llm = MockRerankLlm {
            scores: vec![1.5], // out of range
        };
        let config = LlmRerankConfig {
            top_n: 10,
            llm_weight: 0.7,
        };
        let result = llm_rerank(items, "query", &llm, &config).await;
        // 0.7 * 1.0 (clamped) + 0.3 * 0.5 = 0.85
        assert!((result[0].score - 0.85).abs() < 1e-9);
    }

    fn make_snapshot() -> GraphSnapshot {
        GraphSnapshot::from_edge_list(
            vec![
                EntityId::new("center"),
                EntityId::new("adjacent"),
                EntityId::new("far"),
                EntityId::new("isolated"),
            ],
            &[
                (EntityId::new("center"), EntityId::new("adjacent"), 1.0),
                (EntityId::new("adjacent"), EntityId::new("far"), 1.0),
            ],
        )
    }

    #[test]
    fn node_distance_rerank_boosts_adjacent_nodes() {
        let snapshot = make_snapshot();
        let mut items = vec![
            make_item("adjacent", "s1", "adjacent", 0.50),
            make_item("far", "s2", "far", 0.49),
        ];

        node_distance_rerank(&mut items, &[EntityId::new("center")], &snapshot, 4);

        assert!(items[0].score > items[1].score);
        assert!((items[0].score - 0.70).abs() < 1e-9);
    }

    #[test]
    fn node_distance_rerank_leaves_disconnected_scores_unchanged() {
        let snapshot = make_snapshot();
        let mut items = vec![make_item("isolated", "s1", "isolated", 0.50)];

        node_distance_rerank(&mut items, &[EntityId::new("center")], &snapshot, 4);

        assert!((items[0].score - 0.50).abs() < 1e-9);
    }

    #[test]
    fn node_distance_rerank_respects_max_hops() {
        let snapshot = make_snapshot();
        let mut items = vec![make_item("far", "s1", "far", 0.50)];

        node_distance_rerank(&mut items, &[EntityId::new("center")], &snapshot, 1);

        assert!((items[0].score - 0.50).abs() < 1e-9);
    }

    #[test]
    fn blend_with_ppr_updates_scores() {
        let mut items = vec![
            make_item("adjacent", "s1", "adjacent", 0.80),
            make_item("isolated", "s2", "isolated", 0.40),
        ];
        let ppr_results = vec![PprResult {
            entity_id: EntityId::new("adjacent"),
            activation_score: 0.20,
        }];

        blend_with_ppr(&mut items, &ppr_results, 0.25);

        assert!((items[0].score - 0.65).abs() < 1e-9);
        assert!((items[1].score - 0.30).abs() < 1e-9);
    }
}
