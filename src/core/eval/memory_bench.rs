//! Memory bench: evaluation harness for memory subsystem quality.
//!
//! Measures Fact Recall Rate (FRR), Memory Consolidation Fidelity (MCF),
//! and provides the framework for Shared-State Destruction Rate (SSDR).

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::contracts::ids::{EntityId, SlotKey};
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryLayer, MemorySource, PrivacyLevel, RecallQuery,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlantedFact {
    pub entity_id: EntityId,
    pub slot_key: SlotKey,
    pub value: String,
    pub planted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallProbe {
    pub entity_id: EntityId,
    pub query: String,
    pub expected_slot_key: SlotKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_value: Option<String>,
    #[serde(default)]
    pub expect_absent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBenchConfig {
    pub planted_facts: Vec<PlantedFact>,
    pub recall_probes: Vec<RecallProbe>,
    pub max_acceptable_latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallProbeResult {
    pub probe: RecallProbe,
    pub recalled: bool,
    pub rank: usize,
    pub value_matched: bool,
    pub false_positive: bool,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBenchReport {
    pub fact_recall_rate: f64,
    pub avg_recall_rank: f64,
    pub avg_recall_latency_ms: u64,
    pub false_positive_rate: f64,
    pub probe_results: Vec<RecallProbeResult>,
    pub facts_planted: usize,
    pub probes_executed: usize,
}

#[must_use]
pub fn sanitize_memory_bench_evidence_slug(raw: &str) -> String {
    crate::utils::text::sanitize_slug(raw, "memory-bench")
}

/// # Errors
///
/// Returns an error if fact planting or scoped recall fails.
pub async fn run_memory_bench(
    memory: &dyn Memory,
    config: &MemoryBenchConfig,
) -> anyhow::Result<MemoryBenchReport> {
    for fact in &config.planted_facts {
        let input = MemoryEventInput::new(
            fact.entity_id.as_str(),
            fact.slot_key.as_str(),
            MemoryEventType::FactAdded,
            &fact.value,
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        )
        .with_layer(MemoryLayer::Working)
        .with_occurred_at(&fact.planted_at);
        memory.append_event(input).await?;
    }

    let mut probe_results = Vec::with_capacity(config.recall_probes.len());
    let mut total_latency_ms = 0_u64;

    for probe in &config.recall_probes {
        let request = RecallQuery::new(probe.entity_id.as_str(), &probe.query, 10);
        let started_at = Instant::now();
        let entries = memory.recall_scoped(request).await?;
        let latency_ms = u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        total_latency_ms = total_latency_ms.saturating_add(latency_ms);

        let matching_index = entries
            .iter()
            .position(|entry| entry.slot_key == probe.expected_slot_key)
            .map(|index| index + 1);
        let rank = matching_index.unwrap_or(0);
        let value_matched = matching_index.is_some_and(|rank| {
            let entry = &entries[rank - 1];
            probe
                .expected_value
                .as_ref()
                .is_none_or(|expected| entry.value == *expected)
        });
        let false_positive = probe.expect_absent && rank > 0;
        let recalled = !probe.expect_absent && rank > 0 && value_matched;

        probe_results.push(RecallProbeResult {
            probe: probe.clone(),
            recalled,
            rank,
            value_matched,
            false_positive,
            latency_ms,
        });
    }

    let probes_executed = probe_results.len();
    let avg_recall_latency_ms = average_latency(total_latency_ms, probes_executed);

    Ok(MemoryBenchReport {
        fact_recall_rate: compute_frr(&probe_results),
        avg_recall_rank: compute_avg_rank(&probe_results),
        avg_recall_latency_ms,
        false_positive_rate: compute_false_positive_rate(&probe_results),
        probe_results,
        facts_planted: config.planted_facts.len(),
        probes_executed,
    })
}

#[must_use]
pub fn compute_false_positive_rate(results: &[RecallProbeResult]) -> f64 {
    let negative_count = results
        .iter()
        .filter(|result| result.probe.expect_absent)
        .count();
    if negative_count == 0 {
        return 0.0;
    }

    let false_positive_count = results
        .iter()
        .filter(|result| result.false_positive)
        .count();
    usize_ratio(false_positive_count, negative_count)
}

#[must_use]
pub fn compute_frr(results: &[RecallProbeResult]) -> f64 {
    if results.is_empty() {
        return 0.0;
    }

    let recalled_count = results.iter().filter(|result| result.recalled).count();
    usize_ratio(recalled_count, results.len())
}

#[must_use]
pub fn compute_avg_rank(results: &[RecallProbeResult]) -> f64 {
    let recalled_ranks: Vec<usize> = results
        .iter()
        .filter(|result| result.recalled && result.rank > 0)
        .map(|result| result.rank)
        .collect();

    if recalled_ranks.is_empty() {
        return 0.0;
    }

    let total_rank: usize = recalled_ranks.iter().sum();
    usize_ratio(total_rank, recalled_ranks.len())
}

#[allow(clippy::cast_precision_loss)]
fn usize_ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    numerator as f64 / denominator as f64
}

fn average_latency(total_latency_ms: u64, probes_executed: usize) -> u64 {
    let Ok(divisor) = u64::try_from(probes_executed) else {
        return 0;
    };
    if divisor == 0 {
        return 0;
    }

    total_latency_ms / divisor
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;

    fn probe_result(recalled: bool, rank: usize, latency_ms: u64) -> RecallProbeResult {
        RecallProbeResult {
            probe: RecallProbe {
                entity_id: EntityId::new("default"),
                query: "probe".to_string(),
                expected_slot_key: SlotKey::new("slot"),
                expected_value: None,
                expect_absent: false,
            },
            recalled,
            rank,
            value_matched: recalled,
            false_positive: false,
            latency_ms,
        }
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn compute_frr_all_recalled() {
        let results = vec![probe_result(true, 1, 10), probe_result(true, 2, 11)];
        assert_eq!(compute_frr(&results), 1.0);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn compute_frr_none_recalled() {
        let results = vec![probe_result(false, 0, 10), probe_result(false, 0, 11)];
        assert_eq!(compute_frr(&results), 0.0);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn compute_frr_partial() {
        let results = vec![
            probe_result(true, 1, 10),
            probe_result(false, 0, 12),
            probe_result(true, 3, 14),
        ];
        assert_eq!(compute_frr(&results), 2.0 / 3.0);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn compute_avg_rank_excludes_not_found() {
        let results = vec![
            probe_result(true, 1, 10),
            probe_result(false, 0, 12),
            probe_result(true, 3, 14),
        ];
        assert_eq!(compute_avg_rank(&results), 2.0);
    }

    #[test]
    fn memory_bench_config_serde_roundtrip() {
        let config = MemoryBenchConfig {
            planted_facts: vec![PlantedFact {
                entity_id: EntityId::new("default"),
                slot_key: SlotKey::new("favorite.language"),
                value: "Rust".to_string(),
                planted_at: Utc::now().to_rfc3339(),
            }],
            recall_probes: vec![RecallProbe {
                entity_id: EntityId::new("default"),
                query: "Rust".to_string(),
                expected_slot_key: SlotKey::new("favorite.language"),
                expected_value: Some("Rust".to_string()),
                expect_absent: false,
            }],
            max_acceptable_latency_ms: 250,
        };

        let serialized = serde_json::to_value(&config).unwrap();
        let deserialized: MemoryBenchConfig = serde_json::from_value(serialized).unwrap();
        assert_eq!(deserialized.planted_facts.len(), 1);
        assert_eq!(deserialized.recall_probes.len(), 1);
        assert_eq!(deserialized.max_acceptable_latency_ms, 250);
    }

    #[test]
    fn memory_bench_report_serde_roundtrip() {
        let report = MemoryBenchReport {
            fact_recall_rate: 1.0,
            avg_recall_rank: 1.5,
            avg_recall_latency_ms: 12,
            false_positive_rate: 0.0,
            probe_results: vec![probe_result(true, 1, 12)],
            facts_planted: 1,
            probes_executed: 1,
        };

        let serialized = serde_json::to_value(&report).unwrap();
        let deserialized: MemoryBenchReport = serde_json::from_value(serialized).unwrap();
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(deserialized.fact_recall_rate, 1.0);
            assert_eq!(deserialized.avg_recall_rank, 1.5);
        }
        assert_eq!(deserialized.avg_recall_latency_ms, 12);
        assert_eq!(deserialized.probe_results.len(), 1);
    }

    #[tokio::test]
    async fn run_memory_bench_plants_and_recalls_facts() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let memory = crate::core::memory::MarkdownMemory::new(temp_dir.path());
        let config = MemoryBenchConfig {
            planted_facts: vec![PlantedFact {
                entity_id: EntityId::new("default"),
                slot_key: SlotKey::new("favorite.language"),
                value: "Rust".to_string(),
                planted_at: Utc::now().to_rfc3339(),
            }],
            recall_probes: vec![RecallProbe {
                entity_id: EntityId::new("default"),
                query: "Rust".to_string(),
                expected_slot_key: SlotKey::new("favorite.language"),
                expected_value: Some("Rust".to_string()),
                expect_absent: false,
            }],
            max_acceptable_latency_ms: 500,
        };

        let report = run_memory_bench(&memory, &config).await.unwrap();
        assert_eq!(report.facts_planted, 1);
        assert_eq!(report.probes_executed, 1);
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(report.fact_recall_rate, 1.0);
            assert_eq!(report.avg_recall_rank, 1.0);
            assert_eq!(report.false_positive_rate, 0.0);
        }
    }

    #[tokio::test]
    async fn memory_bench_requires_expected_value_match() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let memory = crate::core::memory::MarkdownMemory::new(temp_dir.path());
        let config = MemoryBenchConfig {
            planted_facts: vec![PlantedFact {
                entity_id: EntityId::new("default"),
                slot_key: SlotKey::new("favorite.language"),
                value: "Rust".to_string(),
                planted_at: Utc::now().to_rfc3339(),
            }],
            recall_probes: vec![RecallProbe {
                entity_id: EntityId::new("default"),
                query: "Rust".to_string(),
                expected_slot_key: SlotKey::new("favorite.language"),
                expected_value: Some("Go".to_string()),
                expect_absent: false,
            }],
            max_acceptable_latency_ms: 500,
        };

        let report = run_memory_bench(&memory, &config).await.unwrap();
        assert!(!report.probe_results[0].recalled);
        assert_eq!(report.probe_results[0].rank, 1);
        assert!(!report.probe_results[0].value_matched);
        assert!((report.fact_recall_rate - 0.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn memory_bench_counts_negative_probe_false_positives() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let memory = crate::core::memory::MarkdownMemory::new(temp_dir.path());
        let config = MemoryBenchConfig {
            planted_facts: vec![PlantedFact {
                entity_id: EntityId::new("default"),
                slot_key: SlotKey::new("favorite.language"),
                value: "Rust".to_string(),
                planted_at: Utc::now().to_rfc3339(),
            }],
            recall_probes: vec![RecallProbe {
                entity_id: EntityId::new("default"),
                query: "Rust".to_string(),
                expected_slot_key: SlotKey::new("favorite.language"),
                expected_value: None,
                expect_absent: true,
            }],
            max_acceptable_latency_ms: 500,
        };

        let report = run_memory_bench(&memory, &config).await.unwrap();
        assert!(report.probe_results[0].false_positive);
        assert!((report.false_positive_rate - 1.0).abs() < f64::EPSILON);
    }
}
