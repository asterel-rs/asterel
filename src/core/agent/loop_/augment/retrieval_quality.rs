//! Retrieval quality assessment: measures how many recalled memory
//! items were actually used in the assistant's response and powers
//! the Self-RAG quality gate.

use crate::core::memory::{Memory, MemoryRecallEntry};

/// Quality signal computed from a single turn's retrieval vs usage.
#[derive(Debug, Clone)]
pub(crate) struct RetrievalQualitySignal {
    /// Total number of items retrieved.
    pub items_retrieved_count: usize,
    /// Fraction of retrieved items that were used (0.0–1.0).
    pub utilization_ratio: f64,
    /// Combined quality score (0.0–1.0).
    pub quality_score: f64,
}

/// Assess retrieval quality by checking which recalled items were
/// referenced in the assistant's answer.
///
/// Uses keyword extraction from each item's value and checks for
/// presence in the answer text.
#[must_use]
pub(crate) fn assess_retrieval_quality(
    recalled_items: &[MemoryRecallEntry],
    assistant_answer: &str,
    _user_message: &str,
) -> RetrievalQualitySignal {
    if recalled_items.is_empty() {
        return RetrievalQualitySignal {
            items_retrieved_count: 0,
            utilization_ratio: 1.0, // no items = nothing wasted
            quality_score: 0.5,     // neutral
        };
    }

    let answer_lower = assistant_answer.to_lowercase();
    let mut used_count = 0usize;
    let mut used_confidence_sum = 0.0_f64;
    // Iterate raw words and lowercase each one only if it is a content word
    // candidate. This replaces the old `extract_keywords` pattern which
    // allocated a fresh `Vec<String>` of up to 20 owned words per recall item
    // (previously the largest allocation hotspot on the post-answer quality
    // pipeline: 22 Strings × 15 items per turn).
    for item in recalled_items {
        let mut item_used = false;
        for raw_word in item.value.split_whitespace().take(20) {
            let trimmed = raw_word.trim_matches(|c: char| !c.is_alphanumeric());
            if trimmed.len() <= 3 {
                continue;
            }
            // Allocate only the lowercased candidate (at most one per hit).
            let lower = trimmed.to_lowercase();
            if answer_lower.contains(lower.as_str()) {
                item_used = true;
                break;
            }
        }
        if item_used {
            used_count += 1;
            used_confidence_sum += item.confidence.get();
        }
    }

    let total = recalled_items.len();
    // Cast safety: retrieved and used item counts are bounded by in-memory recall batch size.
    #[allow(clippy::cast_precision_loss)]
    let utilization_ratio = if total > 0 {
        used_count as f64 / total as f64
    } else {
        1.0
    };

    // Cast safety: used item count is bounded by recalled item count and fits f64 precision.
    #[allow(clippy::cast_precision_loss)]
    let used_avg_confidence = if used_count > 0 {
        used_confidence_sum / used_count as f64
    } else {
        0.0
    };

    // Combined quality: 60% utilization + 40% average confidence of used items.
    // Intuition: using more retrieved items matters most, but highly confident
    // items being used provides a secondary quality signal.
    let quality_score = 0.6 * utilization_ratio + 0.4 * used_avg_confidence;

    RetrievalQualitySignal {
        items_retrieved_count: total,
        utilization_ratio,
        quality_score,
    }
}

/// Convert a quality score [0,1] to a reward value [-1,1].
#[cfg(test)]
#[must_use]
pub(crate) fn quality_to_reward(signal: &RetrievalQualitySignal) -> f64 {
    signal.quality_score * 2.0 - 1.0
}

// ---------------------------------------------------------------------------
// Self-RAG: pre-answer quality gate + query expansion + citation verification
// ---------------------------------------------------------------------------

/// Pre-answer quality assessment result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecallQualityVerdict {
    /// Recall quality is sufficient; proceed with current results.
    Pass,
    /// Recall quality is poor; trigger query expansion + re-query.
    Fail,
    /// No items retrieved at all; re-querying is pointless.
    Empty,
}

/// Configuration for the self-RAG quality gate.
#[derive(Debug, Clone)]
pub(crate) struct SelfRagConfig {
    pub enabled: bool,
    pub min_score_threshold: f64,
    pub min_fact_count: usize,
    pub min_hint_count: usize,
}

impl Default for SelfRagConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_score_threshold: 0.6,
            min_fact_count: 1,
            min_hint_count: 2,
        }
    }
}

/// Assess whether initial recall results meet the quality gate.
#[must_use]
pub(crate) fn assess_pre_answer_quality(
    items: &[MemoryRecallEntry],
    config: &SelfRagConfig,
) -> RecallQualityVerdict {
    if items.is_empty() {
        return RecallQualityVerdict::Empty;
    }

    let fact_count = items.iter().filter(|i| i.confidence.get() >= 0.8).count();
    if fact_count >= config.min_fact_count {
        return RecallQualityVerdict::Pass;
    }

    let hint_count = items.iter().filter(|i| i.confidence.get() >= 0.4).count();
    if hint_count >= config.min_hint_count {
        return RecallQualityVerdict::Pass;
    }

    let max_score = items.iter().map(|i| i.score).fold(0.0_f64, f64::max);
    if max_score >= config.min_score_threshold {
        return RecallQualityVerdict::Pass;
    }

    RecallQualityVerdict::Fail
}

/// Expand a user query for re-retrieval when initial recall quality is poor.
///
/// Strategy: extract content words, add slot-key hints for short queries,
/// add bilingual bridges. No LLM call.
#[must_use]
pub(crate) fn expand_query(user_message: &str, _entity_id: &str) -> String {
    let content_words = extract_content_words(user_message);

    let mut expanded = content_words.clone();

    if content_words.len() < 5 {
        let slot_hints = [
            "profile",
            "preference",
            "hobby",
            "interest",
            "name",
            "memory",
            "fact",
        ];
        for word in &content_words {
            let lower = word.to_lowercase();
            for hint in &slot_hints {
                if lower.contains(hint) || hint.contains(lower.as_str()) {
                    expanded.push((*hint).to_string());
                }
            }
        }
    }

    add_bilingual_bridges(&content_words, &mut expanded);

    expanded.sort_unstable();
    expanded.dedup();
    expanded.join(" ")
}

fn extract_content_words(text: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "the",
        "a",
        "an",
        "is",
        "are",
        "was",
        "were",
        "be",
        "been",
        "have",
        "has",
        "had",
        "do",
        "does",
        "did",
        "will",
        "would",
        "shall",
        "should",
        "may",
        "might",
        "must",
        "can",
        "could",
        "to",
        "of",
        "in",
        "for",
        "on",
        "with",
        "at",
        "by",
        "from",
        "as",
        "and",
        "but",
        "or",
        "not",
        "no",
        "it",
        "its",
        "this",
        "that",
        "i",
        "me",
        "my",
        "you",
        "your",
        "we",
        "they",
        "them",
        "what",
        "which",
        "who",
        "when",
        "where",
        "how",
        "about",
        // Japanese particles
        "の",
        "は",
        "が",
        "を",
        "に",
        "で",
        "と",
        "も",
        "か",
        "な",
        "だ",
        "です",
        "ます",
        "って",
        "だっけ",
        "かな",
        "ね",
        "よ",
        "て",
        "し",
        "けど",
        "から",
        "まで",
        "より",
    ];

    text.split(|c: char| c.is_whitespace() || c == '?' || c == '\u{FF1F}' || c == '!')
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric() && !is_cjk(c)))
        .filter(|w| !w.is_empty() && w.len() > 1)
        .filter(|w| !STOP_WORDS.contains(&w.to_lowercase().as_str()))
        .map(str::to_string)
        .collect()
}

fn is_cjk(c: char) -> bool {
    matches!(
        c,
        '\u{3040}'..='\u{309F}'
            | '\u{30A0}'..='\u{30FF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{3400}'..='\u{4DBF}'
    )
}

fn add_bilingual_bridges(content_words: &[String], expanded: &mut Vec<String>) {
    const BRIDGES: &[(&str, &[&str])] = &[
        ("趣味", &["hobby", "interest", "preference"]),
        ("名前", &["name", "profile"]),
        ("好き", &["like", "favorite", "preference"]),
        ("誕生日", &["birthday"]),
        ("仕事", &["work", "job"]),
        ("hobby", &["趣味"]),
        ("name", &["名前"]),
        ("birthday", &["誕生日"]),
        ("favorite", &["好き"]),
    ];

    for word in content_words {
        let lower = word.to_lowercase();
        for &(key, synonyms) in BRIDGES {
            if lower.contains(key) || key.contains(lower.as_str()) {
                for syn in synonyms {
                    expanded.push((*syn).to_string());
                }
            }
        }
    }
}

/// Merge two recall result sets, deduplicating by `slot_key` (higher score wins).
#[must_use]
pub(crate) fn merge_recall_results(
    original: Vec<MemoryRecallEntry>,
    expanded: Vec<MemoryRecallEntry>,
) -> Vec<MemoryRecallEntry> {
    let mut best: std::collections::HashMap<String, MemoryRecallEntry> =
        std::collections::HashMap::new();

    for item in original.into_iter().chain(expanded) {
        let key = item.slot_key.as_str().to_string();
        match best.entry(key) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(item);
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if item.score > entry.get().score {
                    entry.insert(item);
                }
            }
        }
    }

    let mut results: Vec<MemoryRecallEntry> = best.into_values().collect();
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results
}

/// Orchestrate the Self-RAG pipeline: assess quality, optionally expand + re-query.
///
/// Returns the (potentially improved) recall items and whether a re-query was performed.
pub(crate) async fn self_rag_recall(
    mem: &dyn Memory,
    entity_id: &str,
    user_message: &str,
    initial_items: Vec<MemoryRecallEntry>,
    config: &SelfRagConfig,
    top_k: usize,
) -> (Vec<MemoryRecallEntry>, bool) {
    if !config.enabled {
        return (initial_items, false);
    }

    let verdict = assess_pre_answer_quality(&initial_items, config);

    match verdict {
        RecallQualityVerdict::Pass | RecallQualityVerdict::Empty => {
            tracing::debug!(
                ?verdict,
                items = initial_items.len(),
                "self-RAG: no re-query needed"
            );
            (initial_items, false)
        }
        RecallQualityVerdict::Fail => {
            let expanded_query = expand_query(user_message, entity_id);
            tracing::debug!(
                original = user_message,
                expanded = %expanded_query,
                initial = initial_items.len(),
                "self-RAG: triggering re-query"
            );

            let query = crate::core::memory::RecallQuery::new(entity_id, &expanded_query, top_k);
            let expanded_items = mem.recall_scoped(query).await.unwrap_or_else(|error| {
                tracing::warn!(%error, "self-RAG expanded recall failed");
                Vec::new()
            });

            let merged = merge_recall_results(initial_items, expanded_items);
            tracing::debug!(merged = merged.len(), "self-RAG: merge completed");
            (merged, true)
        }
    }
}

/// Post-answer citation verification signal.
#[derive(Debug, Clone)]
pub(crate) struct CitationVerificationSignal {
    pub citations_found: usize,
    pub items_available: usize,
    pub any_citation_used: bool,
    pub reward_modifier: f64,
}

/// Verify whether the assistant's response cited any grounding items.
#[must_use]
pub(crate) fn verify_citations(
    assistant_answer: &str,
    items_available: usize,
) -> CitationVerificationSignal {
    if items_available == 0 {
        return CitationVerificationSignal {
            citations_found: 0,
            items_available: 0,
            any_citation_used: false,
            reward_modifier: 0.0,
        };
    }

    let citations_found = count_citation_markers(assistant_answer);
    let any_citation_used = citations_found > 0;
    let reward_modifier = if any_citation_used { 0.1 } else { -0.05 };

    CitationVerificationSignal {
        citations_found,
        items_available,
        any_citation_used,
        reward_modifier,
    }
}

fn count_citation_markers(text: &str) -> usize {
    let mut count = 0;
    for prefix in ["[F", "[H", "[C"] {
        let mut search_from = 0;
        while let Some(pos) = text[search_from..].find(prefix) {
            let abs_pos = search_from + pos + prefix.len();
            if abs_pos < text.len() && text[abs_pos..].starts_with(|c: char| c.is_ascii_digit()) {
                count += 1;
            }
            search_from = abs_pos;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::memory::{MemorySource, PrivacyLevel};

    fn make_item(value: &str, confidence: f64) -> MemoryRecallEntry {
        MemoryRecallEntry {
            entity_id: "test".into(),
            slot_key: "test.slot".into(),
            value: value.to_string(),
            source: MemorySource::ExplicitUser,
            confidence: confidence.into(),
            importance: 0.5.into(),
            privacy_level: PrivacyLevel::Private,
            score: 0.8,
            occurred_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn assess_empty_items_returns_neutral() {
        let signal = assess_retrieval_quality(&[], "some answer", "some question");
        assert_eq!(signal.items_retrieved_count, 0);
        assert!((signal.quality_score - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn assess_all_items_used() {
        let items = vec![
            make_item("The user likes Rust programming", 0.9),
            make_item("They prefer functional style", 0.8),
        ];
        let answer = "Based on your interest in Rust programming and functional style, ...";
        let signal = assess_retrieval_quality(&items, answer, "tell me about coding");
        assert!((signal.utilization_ratio - 1.0).abs() < f64::EPSILON);
        assert!(signal.quality_score > 0.7);
    }

    #[test]
    fn assess_no_items_used() {
        let items = vec![
            make_item("The user likes painting", 0.9),
            make_item("They enjoy classical music", 0.8),
        ];
        let answer = "Here is some code for you.";
        let signal = assess_retrieval_quality(&items, answer, "write code");
        assert!((signal.utilization_ratio).abs() < f64::EPSILON);
        assert!(signal.quality_score < 0.1);
    }

    // --- Self-RAG tests ---

    #[test]
    fn pre_answer_quality_empty_is_empty() {
        let config = SelfRagConfig::default();
        assert_eq!(
            assess_pre_answer_quality(&[], &config),
            RecallQualityVerdict::Empty
        );
    }

    #[test]
    fn pre_answer_quality_high_confidence_passes() {
        let config = SelfRagConfig::default();
        let items = vec![make_item("fact", 0.9)];
        assert_eq!(
            assess_pre_answer_quality(&items, &config),
            RecallQualityVerdict::Pass
        );
    }

    #[test]
    fn pre_answer_quality_two_hints_passes() {
        let config = SelfRagConfig::default();
        let items = vec![make_item("hint1", 0.5), make_item("hint2", 0.6)];
        assert_eq!(
            assess_pre_answer_quality(&items, &config),
            RecallQualityVerdict::Pass
        );
    }

    #[test]
    fn pre_answer_quality_high_score_passes() {
        let config = SelfRagConfig::default();
        let items = vec![MemoryRecallEntry {
            score: 0.8,
            confidence: 0.3.into(),
            ..make_item("low conf but high score", 0.3)
        }];
        assert_eq!(
            assess_pre_answer_quality(&items, &config),
            RecallQualityVerdict::Pass
        );
    }

    #[test]
    fn pre_answer_quality_all_low_fails() {
        let config = SelfRagConfig::default();
        let items = vec![MemoryRecallEntry {
            score: 0.2,
            confidence: 0.3.into(),
            ..make_item("poor", 0.3)
        }];
        assert_eq!(
            assess_pre_answer_quality(&items, &config),
            RecallQualityVerdict::Fail
        );
    }

    #[test]
    fn expand_query_adds_bridges() {
        let expanded = expand_query("私の趣味って何？", "user-1");
        assert!(expanded.contains("hobby") || expanded.contains("interest"));
    }

    #[test]
    fn expand_query_handles_english() {
        let expanded = expand_query("What is my favorite hobby?", "user-1");
        assert!(expanded.contains("hobby"));
    }

    #[test]
    fn merge_deduplicates_by_slot_key() {
        let a = MemoryRecallEntry {
            slot_key: "profile.name".into(),
            score: 0.5,
            ..make_item("Haru", 0.9)
        };
        let b = MemoryRecallEntry {
            slot_key: "profile.name".into(),
            score: 0.9,
            ..make_item("Haru v2", 0.9)
        };
        let merged = merge_recall_results(vec![a], vec![b]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].value, "Haru v2"); // higher score wins
    }

    #[test]
    fn merge_keeps_disjoint_items() {
        let a = MemoryRecallEntry {
            slot_key: "profile.name".into(),
            ..make_item("Haru", 0.9)
        };
        let b = MemoryRecallEntry {
            slot_key: "profile.hobby".into(),
            ..make_item("Rust", 0.8)
        };
        let merged = merge_recall_results(vec![a], vec![b]);
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn citation_count_finds_markers() {
        assert_eq!(count_citation_markers("Based on [F1] and [H2]"), 2);
        assert_eq!(count_citation_markers("No citations here"), 0);
        assert_eq!(count_citation_markers("[F1] [F2] [H1] [C1]"), 4);
    }

    #[test]
    fn verify_citations_positive_when_used() {
        let signal = verify_citations("The answer is [F1] based.", 3);
        assert!(signal.any_citation_used);
        assert!(signal.reward_modifier > 0.0);
    }

    #[test]
    fn verify_citations_negative_when_ignored() {
        let signal = verify_citations("The answer is just a guess.", 3);
        assert!(!signal.any_citation_used);
        assert!(signal.reward_modifier < 0.0);
    }

    #[test]
    fn verify_citations_neutral_when_no_items() {
        let signal = verify_citations("Whatever", 0);
        assert!((signal.reward_modifier).abs() < f64::EPSILON);
    }

    #[test]
    fn quality_to_reward_boundaries() {
        let low = RetrievalQualitySignal {
            items_retrieved_count: 10,
            utilization_ratio: 0.0,
            quality_score: 0.0,
        };
        assert!((quality_to_reward(&low) - (-1.0)).abs() < f64::EPSILON);

        let high = RetrievalQualitySignal {
            items_retrieved_count: 10,
            utilization_ratio: 1.0,
            quality_score: 1.0,
        };
        assert!((quality_to_reward(&high) - 1.0).abs() < f64::EPSILON);
    }
}
