//! Vector operations: cosine similarity, serialization, and hybrid fusion.
//!
//! Three merge strategies are provided:
//!
//! - [`cosine_similarity`] — raw dot-product similarity for in-process reranking.
//! - [`hybrid_merge`] — weighted linear combination of normalized vector and keyword scores.
//! - [`rrf_merge`] — Reciprocal Rank Fusion (RRF, k=60); avoids scale-mismatch between
//!   BM25 and cosine scores by fusing rank positions rather than raw values.

/// Cosine similarity between two vectors. Returns 0.0–1.0.
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;

    for (x, y) in a.iter().zip(b.iter()) {
        let x = f64::from(*x);
        let y = f64::from(*y);
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if !denom.is_finite() || denom < f64::EPSILON {
        return 0.0;
    }

    let raw = dot / denom;
    if !raw.is_finite() {
        return 0.0;
    }

    // Clamp to [0, 1] — embeddings are typically positive
    // Cast safety: similarity is clamped to [0.0, 1.0] before f32 conversion.
    #[allow(clippy::cast_possible_truncation)]
    let sim = raw.clamp(0.0, 1.0) as f32;
    sim
}

/// Serialize f32 vector to bytes (little-endian)
#[must_use]
pub fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(v.len() * 4);
    for &f in v {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

/// Deserialize bytes to f32 vector (little-endian).
///
/// Trailing bytes that don't fill a complete `f32` are dropped with a
/// warning.
#[must_use]
pub fn bytes_to_vec(bytes: &[u8]) -> Vec<f32> {
    let remainder = bytes.len() % 4;
    if remainder != 0 {
        tracing::warn!(
            byte_count = bytes.len(),
            remainder,
            "bytes_to_vec: input not aligned to 4 bytes; \
             trailing bytes dropped"
        );
    }
    bytes
        .chunks_exact(4)
        .map(|chunk| {
            let arr: [u8; 4] = chunk.try_into().unwrap_or([0; 4]);
            f32::from_le_bytes(arr)
        })
        .collect()
}

/// A scored result for hybrid merging
#[derive(Debug, Clone)]
pub struct ScoredResult {
    /// Unique identifier of the result.
    pub id: String,
    /// Cosine similarity score, if present in vector results.
    pub vector_score: Option<f32>,
    /// Normalized keyword (BM25) score, if present.
    pub keyword_score: Option<f32>,
    /// Weighted fusion of vector and keyword scores.
    pub final_score: f32,
}

/// Hybrid merge: combine vector and keyword results with weighted fusion.
///
/// Normalizes each score set to [0, 1], normalizes non-zero weights to sum to
/// one, then computes:
///   `final_score` = `vector_weight` * `vector_score` + `keyword_weight` * `keyword_score`
///
/// Deduplicates by id, keeping the best score from each source.
pub fn hybrid_merge(
    vector_results: &[(String, f32)],  // (id, cosine_similarity)
    keyword_results: &[(String, f32)], // (id, bm25_score)
    vector_weight: f32,
    keyword_weight: f32,
    limit: usize,
) -> Vec<ScoredResult> {
    use std::collections::HashMap;

    let (vector_weight, keyword_weight) = normalized_hybrid_weights(vector_weight, keyword_weight);

    let mut map: HashMap<String, (Option<f32>, Option<f32>)> = HashMap::new();

    // Normalize vector scores (already 0–1 from cosine similarity)
    for (id, score) in vector_results {
        map.entry(id.clone())
            .and_modify(|(vector_score, _)| *vector_score = Some(*score))
            .or_insert((Some(*score), None));
    }

    // Normalize keyword scores (BM25 can be any positive number)
    let max_kw = keyword_results
        .iter()
        .map(|(_, s)| *s)
        .fold(0.0_f32, f32::max);
    let max_kw = if max_kw < f32::EPSILON { 1.0 } else { max_kw };

    for (id, score) in keyword_results {
        let normalized = score / max_kw;
        map.entry(id.clone())
            .and_modify(|(_, keyword_score)| *keyword_score = Some(normalized))
            .or_insert((None, Some(normalized)));
    }

    let mut results: Vec<ScoredResult> = map
        .into_iter()
        .map(|(id, (vector_score, keyword_score))| {
            let vs = vector_score.unwrap_or(0.0);
            let ks = keyword_score.unwrap_or(0.0);
            ScoredResult {
                id,
                vector_score,
                keyword_score,
                final_score: vector_weight * vs + keyword_weight * ks,
            }
        })
        .collect();

    results.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    results
}

fn normalized_hybrid_weights(vector_weight: f32, keyword_weight: f32) -> (f32, f32) {
    let vector_weight = finite_nonnegative_weight(vector_weight);
    let keyword_weight = finite_nonnegative_weight(keyword_weight);
    let total = vector_weight + keyword_weight;
    if total <= f32::EPSILON {
        (0.0, 0.0)
    } else {
        (vector_weight / total, keyword_weight / total)
    }
}

fn finite_nonnegative_weight(weight: f32) -> f32 {
    if weight.is_finite() {
        weight.max(0.0)
    } else {
        0.0
    }
}

/// Reciprocal Rank Fusion: combine ranked lists using position-based scoring.
///
/// RRF avoids raw score mixing issues (BM25 vs cosine scale differences).
/// Formula: score(d) = Σ 1 / (k + rank(d)) for each ranked list.
/// Default k=60 per the original RRF paper.
#[must_use]
pub fn rrf_merge(
    vector_results: &[(String, f32)],
    keyword_results: &[(String, f32)],
    limit: usize,
) -> Vec<ScoredResult> {
    use std::collections::HashMap;

    const K: f32 = 60.0;

    let mut scores: HashMap<String, (f32, Option<f32>, Option<f32>)> = HashMap::new();

    for (rank, (id, score)) in vector_results.iter().enumerate() {
        let rank_1based = u16::try_from(rank.saturating_add(1)).unwrap_or(u16::MAX);
        let rrf_score = 1.0 / (K + f32::from(rank_1based));
        scores
            .entry(id.clone())
            .and_modify(|(total, vector_score, _)| {
                *total += rrf_score;
                *vector_score = Some(*score);
            })
            .or_insert((rrf_score, Some(*score), None));
    }

    for (rank, (id, score)) in keyword_results.iter().enumerate() {
        let rank_1based = u16::try_from(rank.saturating_add(1)).unwrap_or(u16::MAX);
        let rrf_score = 1.0 / (K + f32::from(rank_1based));
        scores
            .entry(id.clone())
            .and_modify(|(total, _, keyword_score)| {
                *total += rrf_score;
                *keyword_score = Some(*score);
            })
            .or_insert((rrf_score, None, Some(*score)));
    }

    let mut results: Vec<ScoredResult> = scores
        .into_iter()
        .map(|(id, (total, vector_score, keyword_score))| ScoredResult {
            id,
            vector_score,
            keyword_score,
            final_score: total,
        })
        .collect();

    results.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    results
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    clippy::approx_constant,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_vectors() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 0.001);
    }

    #[test]
    fn cosine_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 0.001);
    }

    #[test]
    fn cosine_similar_vectors() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.1, 2.1, 3.1];
        let sim = cosine_similarity(&a, &b);
        assert!(sim > 0.99);
    }

    #[test]
    fn cosine_empty_returns_zero() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn cosine_mismatched_lengths() {
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
    }

    #[test]
    fn cosine_zero_vector() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 2.0, 3.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn vec_bytes_roundtrip() {
        let original = vec![1.0_f32, -2.5, 3.14, 0.0, f32::MAX];
        let bytes = vec_to_bytes(&original);
        let restored = bytes_to_vec(&bytes);
        assert_eq!(original, restored);
    }

    #[test]
    fn vec_bytes_empty() {
        let bytes = vec_to_bytes(&[]);
        assert!(bytes.is_empty());
        let restored = bytes_to_vec(&bytes);
        assert!(restored.is_empty());
    }

    #[test]
    fn hybrid_merge_vector_only() {
        let vec_results = vec![("a".into(), 0.9), ("b".into(), 0.5)];
        let merged = hybrid_merge(&vec_results, &[], 0.7, 0.3, 10);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id, "a");
        assert!(merged[0].final_score > merged[1].final_score);
    }

    #[test]
    fn hybrid_merge_keyword_only() {
        let kw_results = vec![("x".into(), 10.0), ("y".into(), 5.0)];
        let merged = hybrid_merge(&[], &kw_results, 0.7, 0.3, 10);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id, "x");
    }

    #[test]
    fn hybrid_merge_deduplicates() {
        let vec_results = vec![("a".into(), 0.9)];
        let kw_results = vec![("a".into(), 10.0)];
        let merged = hybrid_merge(&vec_results, &kw_results, 0.7, 0.3, 10);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "a");
        // Should have both scores
        assert!(merged[0].vector_score.is_some());
        assert!(merged[0].keyword_score.is_some());
        // Final score should be higher than either alone
        assert!(merged[0].final_score > 0.7 * 0.9);
    }

    #[test]
    fn hybrid_merge_respects_limit() {
        let vec_results: Vec<(String, f32)> = (0..20)
            .map(|i| (format!("item_{i}"), 1.0 - i as f32 * 0.05))
            .collect();
        let merged = hybrid_merge(&vec_results, &[], 1.0, 0.0, 5);
        assert_eq!(merged.len(), 5);
    }

    #[test]
    fn hybrid_merge_empty_inputs() {
        let merged = hybrid_merge(&[], &[], 0.7, 0.3, 10);
        assert!(merged.is_empty());
    }

    // ── Edge cases: cosine similarity ────────────────────────────

    #[test]
    fn cosine_nan_returns_zero() {
        let a = vec![f32::NAN, 1.0, 2.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        // NaN propagates through arithmetic — result should be 0.0 (clamped or denom check)
        assert!(sim.is_finite(), "Expected finite, got {sim}");
    }

    #[test]
    fn cosine_infinity_returns_zero_or_finite() {
        let a = vec![f32::INFINITY, 1.0];
        let b = vec![1.0, 2.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.is_finite(), "Expected finite, got {sim}");
    }

    #[test]
    fn cosine_negative_values() {
        let a = vec![-1.0, -2.0, -3.0];
        let b = vec![-1.0, -2.0, -3.0];
        // Identical negative vectors → cosine = 1.0, but clamped to [0,1]
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 0.001);
    }

    #[test]
    fn cosine_opposite_vectors_clamped() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        // Cosine = -1.0, clamped to 0.0
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < f32::EPSILON);
    }

    #[test]
    fn cosine_high_dimensional() {
        let a: Vec<f32> = (0..1536).map(|i| (f64::from(i) * 0.001) as f32).collect();
        let b: Vec<f32> = (0..1536)
            .map(|i| (f64::from(i) * 0.001 + 0.0001) as f32)
            .collect();
        let sim = cosine_similarity(&a, &b);
        assert!(
            sim > 0.99,
            "High-dim similar vectors should be close: {sim}"
        );
    }

    #[test]
    fn cosine_single_element() {
        assert!((cosine_similarity(&[5.0], &[5.0]) - 1.0).abs() < 0.001);
        assert!(cosine_similarity(&[5.0], &[-5.0]).abs() < f32::EPSILON);
    }

    #[test]
    fn cosine_both_zero_vectors() {
        let a = vec![0.0, 0.0];
        let b = vec![0.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < f32::EPSILON);
    }

    // ── Edge cases: vec↔bytes serialization ──────────────────────

    #[test]
    fn bytes_to_vec_non_aligned_truncates() {
        // 5 bytes → only first 4 used (1 float), last byte dropped
        let bytes = vec![0u8, 0, 0, 0, 0xFF];
        let result = bytes_to_vec(&bytes);
        assert_eq!(result.len(), 1);
        assert!(result[0].abs() < f32::EPSILON);
    }

    #[test]
    fn bytes_to_vec_three_bytes_returns_empty() {
        let bytes = vec![1u8, 2, 3];
        let result = bytes_to_vec(&bytes);
        assert!(result.is_empty());
    }

    #[test]
    fn vec_bytes_roundtrip_special_values() {
        let special = vec![f32::MIN, f32::MAX, f32::EPSILON, -0.0, 0.0];
        let bytes = vec_to_bytes(&special);
        let restored = bytes_to_vec(&bytes);
        assert_eq!(special.len(), restored.len());
        for (a, b) in special.iter().zip(restored.iter()) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }

    #[test]
    fn vec_bytes_roundtrip_nan_preserves_bits() {
        let nan_vec = vec![f32::NAN];
        let bytes = vec_to_bytes(&nan_vec);
        let restored = bytes_to_vec(&bytes);
        assert!(restored[0].is_nan());
    }

    // ── Edge cases: hybrid merge ─────────────────────────────────

    #[test]
    fn hybrid_merge_limit_zero() {
        let vec_results = vec![("a".into(), 0.9)];
        let merged = hybrid_merge(&vec_results, &[], 0.7, 0.3, 0);
        assert!(merged.is_empty());
    }

    #[test]
    fn hybrid_merge_zero_weights() {
        let vec_results = vec![("a".into(), 0.9)];
        let kw_results = vec![("b".into(), 10.0)];
        let merged = hybrid_merge(&vec_results, &kw_results, 0.0, 0.0, 10);
        // All final scores should be 0.0
        for r in &merged {
            assert!(r.final_score.abs() < f32::EPSILON);
        }
    }

    #[test]
    fn hybrid_merge_normalizes_non_unit_weights() {
        let vec_results = vec![("vector".into(), 1.0)];
        let kw_results = vec![("keyword".into(), 10.0)];

        let merged = hybrid_merge(&vec_results, &kw_results, 0.7, 0.7, 10);

        let vector = merged.iter().find(|result| result.id == "vector").unwrap();
        let keyword = merged.iter().find(|result| result.id == "keyword").unwrap();
        assert!((vector.final_score - 0.5).abs() < f32::EPSILON);
        assert!((keyword.final_score - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn hybrid_merge_non_finite_weights_do_not_create_non_finite_scores() {
        let vec_results = vec![("vector".into(), 1.0)];
        let kw_results = vec![("keyword".into(), 10.0)];

        let merged = hybrid_merge(&vec_results, &kw_results, f32::NAN, f32::INFINITY, 10);

        assert!(merged.iter().all(|result| result.final_score.is_finite()));
        assert!(
            merged
                .iter()
                .all(|result| result.final_score.abs() < f32::EPSILON)
        );
    }

    #[test]
    fn hybrid_merge_negative_keyword_scores() {
        // BM25 scores are negated in our code, but raw negatives shouldn't crash
        let kw_results = vec![("a".into(), -5.0), ("b".into(), -1.0)];
        let merged = hybrid_merge(&[], &kw_results, 0.7, 0.3, 10);
        assert_eq!(merged.len(), 2);
        // Should still produce finite scores
        for r in &merged {
            assert!(r.final_score.is_finite());
        }
    }

    #[test]
    fn hybrid_merge_duplicate_ids_in_same_source() {
        let vec_results = vec![("a".into(), 0.9), ("a".into(), 0.5)];
        let merged = hybrid_merge(&vec_results, &[], 1.0, 0.0, 10);
        // Should deduplicate — only 1 entry for "a"
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn hybrid_merge_large_bm25_normalization() {
        let kw_results = vec![("a".into(), 1000.0), ("b".into(), 500.0), ("c".into(), 1.0)];
        let merged = hybrid_merge(&[], &kw_results, 0.0, 1.0, 10);
        // "a" should have normalized score of 1.0
        assert!((merged[0].keyword_score.unwrap() - 1.0).abs() < 0.001);
        // "b" should have 0.5
        assert!((merged[1].keyword_score.unwrap() - 0.5).abs() < 0.001);
    }

    #[test]
    fn hybrid_merge_single_item() {
        let merged = hybrid_merge(&[("only".into(), 0.8)], &[], 0.7, 0.3, 10);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "only");
    }

    #[test]
    fn rrf_merge_basic_fusion() {
        let vector_results = vec![("a".into(), 0.95), ("b".into(), 0.90), ("c".into(), 0.80)];
        let keyword_results = vec![("b".into(), 3.0), ("a".into(), 2.5), ("d".into(), 2.0)];

        let merged = rrf_merge(&vector_results, &keyword_results, 10);
        let top_two = [merged[0].id.as_str(), merged[1].id.as_str()];
        assert!(top_two.contains(&"a"));
        assert!(top_two.contains(&"b"));
        assert!(merged.iter().any(|entry| entry.id == "d"));
    }

    #[test]
    fn rrf_merge_vector_only() {
        let vector_results = vec![("a".into(), 0.9), ("b".into(), 0.8)];
        let merged = rrf_merge(&vector_results, &[], 10);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id, "a");
        assert!(merged.iter().all(|entry| entry.keyword_score.is_none()));
    }

    #[test]
    fn rrf_merge_keyword_only() {
        let keyword_results = vec![("x".into(), 4.0), ("y".into(), 3.0)];
        let merged = rrf_merge(&[], &keyword_results, 10);

        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id, "x");
        assert!(merged.iter().all(|entry| entry.vector_score.is_none()));
    }

    #[test]
    fn rrf_merge_empty_inputs() {
        let merged = rrf_merge(&[], &[], 10);
        assert!(merged.is_empty());
    }

    #[test]
    fn rrf_merge_deduplicates() {
        let vector_results = vec![("shared".into(), 0.9)];
        let keyword_results = vec![("shared".into(), 5.0)];
        let merged = rrf_merge(&vector_results, &keyword_results, 10);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "shared");
        assert!(merged[0].vector_score.is_some());
        assert!(merged[0].keyword_score.is_some());
    }
}
