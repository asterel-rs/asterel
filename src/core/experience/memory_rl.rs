//! Principle retrieval ranking over persisted experience memory.
//!
//! This module consumes already-persisted `q_value` fields as one signal in a
//! composite retrieval score. It does not update Q-values itself; any future
//! learning loop must own that write path explicitly.
#![allow(clippy::cast_precision_loss)]

use anyhow::Result;

use super::distill_types::Principle;
use crate::contracts::strings::data_model::PREFIX_PRINCIPLE_SLOT;
use crate::core::memory::Memory;

/// `MemRL` retrieval-ranking configuration.
pub(crate) struct MemoryRL {
    /// Weight of Q-value vs similarity in composite scoring.
    /// 0.0 = pure similarity, 1.0 = pure Q-value.
    lambda: f64,
}

/// A principle scored by the `MemRL` composite ranking.
#[derive(Debug)]
pub(crate) struct ScoredPrinciple {
    /// The underlying principle.
    pub principle: Principle,
    /// Normalised keyword similarity score in `[0.0, 1.0]`.
    pub similarity_score: f64,
    /// Normalised Q-value score in `[0.0, 1.0]`.
    pub q_score: f64,
    /// Weighted combination of similarity and Q-value.
    pub composite_score: f64,
}

impl Default for MemoryRL {
    fn default() -> Self {
        Self { lambda: 0.4 }
    }
}

impl MemoryRL {
    /// Create a `MemoryRL` instance with custom retrieval-rank weight.
    #[must_use]
    pub(crate) fn new(lambda: f64) -> Self {
        Self {
            lambda: lambda.clamp(0.0, 1.0),
        }
    }

    /// Rank principles using a composite score of similarity and Q-value.
    ///
    /// Phase 1: Score by keyword similarity (existing heuristic).
    /// Phase 2: Boost by persisted Q-value.
    ///
    /// Returns principles sorted by composite score, truncated to `limit`.
    pub(crate) fn rank_with_q_values(
        &self,
        mut principles: Vec<(f64, Principle)>,
        limit: usize,
    ) -> Vec<ScoredPrinciple> {
        if principles.is_empty() {
            return Vec::new();
        }

        // Normalize similarity scores to [0, 1].
        let (sim_min, sim_max) = min_max_scores(&principles);
        let sim_range = (sim_max - sim_min).max(f64::EPSILON);

        // Normalize Q-values to [0, 1].
        let (q_min, q_max) = principles
            .iter()
            .map(|(_, p)| p.q_value)
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(min, max), q_value| {
                (min.min(q_value), max.max(q_value))
            });
        let q_range = (q_max - q_min).max(f64::EPSILON);

        let mut scored: Vec<ScoredPrinciple> = principles
            .drain(..)
            .map(|(sim, p)| {
                let sim_norm = (sim - sim_min) / sim_range;
                let q_norm = (p.q_value - q_min) / q_range;
                let composite = (1.0 - self.lambda) * sim_norm + self.lambda * q_norm;
                ScoredPrinciple {
                    similarity_score: sim_norm,
                    q_score: q_norm,
                    composite_score: composite,
                    principle: p,
                }
            })
            .collect();

        scored.sort_by(|a, b| {
            b.composite_score
                .partial_cmp(&a.composite_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        scored
    }
}

/// Retrieve principles with Q-value-aware ranking.
///
/// This is the main entry point that combines keyword/domain matching with
/// persisted Q-values. It reads Q-values only; it does not perform a learning
/// update.
pub(crate) async fn retrieve_principles_with_q(
    mem: &dyn Memory,
    entity_id: &str,
    user_message: &str,
    memory_rl: &MemoryRL,
    limit: usize,
) -> Result<Vec<Principle>> {
    let all: Vec<Principle> = crate::core::memory::recall_helpers::recall_typed(
        mem,
        entity_id,
        PREFIX_PRINCIPLE_SLOT,
        30,
    )
    .await?;
    if all.is_empty() {
        return Ok(Vec::new());
    }

    let target_domain = super::domain_tag::infer_domain(user_message);
    let domain_filtered = super::domain_tag::filter_by_domain_affinity(&all, target_domain, 0.6);
    let candidates: Vec<Principle> = if domain_filtered.is_empty() {
        all
    } else {
        domain_filtered.into_iter().cloned().collect()
    };

    // Pre-lowercase the user message words once to avoid O(principles × words)
    // String allocations inside `keyword_overlap_score`.
    let msg_words_lower = crate::utils::text::lowercase_words_over_len(user_message, 3);
    let msg_words_lower_refs: Vec<&str> = msg_words_lower.iter().map(String::as_str).collect();

    let scored_principles: Vec<(f64, Principle)> = candidates
        .into_iter()
        .map(|p| {
            let overlap = keyword_overlap_score(&msg_words_lower_refs, &p.statement);
            let principle_domain = p
                .domain
                .unwrap_or_else(|| super::domain_tag::infer_domain(&p.statement));
            let affinity = super::domain_tag::domain_similarity(principle_domain, target_domain);
            let similarity = p.confidence.get() * 0.5 + overlap * 0.3 + affinity * 0.2;
            (similarity, p)
        })
        .collect();

    let ranked = memory_rl.rank_with_q_values(scored_principles, limit);
    for scored in &ranked {
        tracing::debug!(
            principle_id = %scored.principle.id,
            similarity = scored.similarity_score,
            q_score = scored.q_score,
            composite = scored.composite_score,
            "principle ranked with MemRL"
        );
    }
    Ok(ranked.into_iter().map(|sp| sp.principle).collect())
}

use crate::utils::text::keyword_overlap_score;

fn min_max_scores(scored: &[(f64, Principle)]) -> (f64, f64) {
    let min = scored.iter().map(|(s, _)| *s).fold(f64::INFINITY, f64::min);
    let max = scored
        .iter()
        .map(|(s, _)| *s)
        .fold(f64::NEG_INFINITY, f64::max);
    (min, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::experience::distill_types::{Principle, PrincipleCategory};

    fn make_principle(statement: &str, confidence: f64, q_value: f64) -> Principle {
        Principle {
            id: uuid::Uuid::new_v4().to_string(),
            category: PrincipleCategory::Heuristic,
            statement: statement.to_string(),
            confidence: confidence.into(),
            source_experience_ids: vec![],
            validation_count: 1,
            created_at: String::new(),
            domain: None,
            q_value,
            times_applied: 0,
        }
    }

    #[test]
    fn rank_with_q_values_boosts_high_q() {
        // Lambda=0.7 gives Q-values dominant weight over similarity.
        let rl = MemoryRL::new(0.7);

        let principles = vec![
            (0.8, make_principle("low q principle", 0.7, -0.5)),
            (0.6, make_principle("high q principle", 0.7, 0.9)),
            (0.7, make_principle("mid q principle", 0.7, 0.3)),
        ];

        let ranked = rl.rank_with_q_values(principles, 3);
        assert_eq!(ranked.len(), 3);
        // High Q-value principle should be ranked higher despite lower similarity.
        assert!(ranked[0].principle.statement.contains("high q"));
    }

    #[test]
    fn rank_with_q_values_respects_limit() {
        let rl = MemoryRL::default();
        let principles = vec![
            (0.9, make_principle("a", 0.8, 0.1)),
            (0.8, make_principle("b", 0.7, 0.2)),
            (0.7, make_principle("c", 0.6, 0.3)),
        ];
        let ranked = rl.rank_with_q_values(principles, 2);
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn rank_empty_returns_empty() {
        let rl = MemoryRL::default();
        let ranked = rl.rank_with_q_values(vec![], 5);
        assert!(ranked.is_empty());
    }

    #[test]
    fn rank_single_principle() {
        let rl = MemoryRL::default();
        let principles = vec![(0.5, make_principle("only one", 0.7, 0.0))];
        let ranked = rl.rank_with_q_values(principles, 5);
        assert_eq!(ranked.len(), 1);
    }

    #[test]
    fn q_value_ema_update_formula() {
        const ALPHA: f64 = 0.2;
        // Simulated EMA: Q_new = Q_old + alpha * (reward - Q_old)
        let q_old = 0.0;
        let reward = 1.0;
        let q_new = q_old + ALPHA * (reward - q_old);
        assert!((q_new - 0.2).abs() < f64::EPSILON);

        // After many positive rewards, Q approaches 1.0
        let mut q = 0.0;
        for _ in 0..50 {
            q += ALPHA * (1.0 - q);
        }
        assert!(q > 0.99);
    }

    #[test]
    fn q_value_ema_negative_reward() {
        const ALPHA: f64 = 0.2;
        let q_old = 0.5;
        let reward = -1.0;
        let q_new = q_old + ALPHA * (reward - q_old);
        // 0.5 + 0.2 * (-1.0 - 0.5) = 0.5 - 0.3 = 0.2
        assert!((q_new - 0.2).abs() < f64::EPSILON);
    }

    #[test]
    fn lambda_zero_ignores_q_values() {
        let rl = MemoryRL::new(0.0);

        let principles = vec![
            (0.9, make_principle("high sim", 0.8, -1.0)),
            (0.3, make_principle("low sim", 0.8, 1.0)),
        ];

        let ranked = rl.rank_with_q_values(principles, 2);
        // With lambda=0, Q-values are ignored — pure similarity ranking
        assert!(ranked[0].principle.statement.contains("high sim"));
    }

    #[test]
    fn lambda_one_ignores_similarity() {
        let rl = MemoryRL::new(1.0);

        let principles = vec![
            (0.9, make_principle("high sim", 0.8, -1.0)),
            (0.3, make_principle("low sim", 0.8, 1.0)),
        ];

        let ranked = rl.rank_with_q_values(principles, 2);
        // With lambda=1, similarity is ignored — pure Q-value ranking
        assert!(ranked[0].principle.statement.contains("low sim"));
    }
}
