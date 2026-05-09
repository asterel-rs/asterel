//! Synthetic baseline eval harness: runs deterministic scenario
//! simulations for trend tracking without live model calls.

use std::cmp::max;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::behavioral::{
    AssertionDirection, BehavioralAssertion, BehavioralEvalSpec, run_behavioral_eval,
};
use super::presenter::{render_baseline_csv, render_baseline_text_summary};
use super::rng::{
    DeterministicRng, bounded_inclusive, fingerprint_summary, mix_seed, u32_saturating_from_u64,
};
use super::types::{EvalReport, EvalScenarioSpec, EvalSuiteSpec, EvalSuiteSummary};

/// Deterministic synthetic eval harness for baseline trend tracking.
#[derive(Debug, Clone)]
pub struct EvalHarness {
    seed: u64,
}

impl EvalHarness {
    /// Synthetic baseline harness for deterministic trend tracking.
    ///
    /// This does not directly execute live model/provider behavior; release gates
    /// should pair it with behavioral regression checks.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self { seed }
    }

    /// Run all suites deterministically and produce an `EvalReport`.
    #[must_use]
    pub fn run(&self, suites: &[EvalSuiteSpec]) -> EvalReport {
        run_behavioral_smoke_eval(self.seed);

        let mut ordered_suites = suites.to_vec();
        ordered_suites.sort_by(|a, b| a.name.cmp(b.name));

        let mut summaries = Vec::with_capacity(ordered_suites.len());
        for suite in &ordered_suites {
            let mut scenarios = suite.scenarios.clone();
            scenarios.sort_by(|a, b| a.id.cmp(b.id));

            let mut success_count = 0_u32;
            let mut total_cost = 0_u64;
            let mut total_latency = 0_u64;
            let mut total_retries = 0_u64;

            for scenario in &scenarios {
                let local_seed = mix_seed(self.seed, suite.name, scenario.id);
                let mut rng = DeterministicRng::new(local_seed);
                let roll = rng.next_bounded(100);
                let success = roll < u64::from(scenario.success_target_percent);
                if success {
                    success_count += 1;
                }

                let cost = bounded_inclusive(
                    scenario.min_cost_cents,
                    scenario.max_cost_cents,
                    rng.next_u64(),
                );
                let latency = bounded_inclusive(
                    scenario.min_latency_ms,
                    scenario.max_latency_ms,
                    rng.next_u64(),
                );

                let retries_sample = rng.next_bounded(u64::from(scenario.retry_cap) + 1);
                let retries = if success {
                    u32_saturating_from_u64(retries_sample)
                } else {
                    max(1, u32_saturating_from_u64(retries_sample))
                };

                total_cost += u64::from(cost);
                total_latency += u64::from(latency);
                total_retries += u64::from(retries);
            }

            let case_count = u32::try_from(scenarios.len()).unwrap_or(u32::MAX);
            let summary = if case_count == 0 {
                EvalSuiteSummary {
                    suite: suite.name.to_string(),
                    case_count: 0,
                    success_rate_bps: 0,
                    avg_cost_cents: 0,
                    avg_latency_ms: 0,
                    avg_retries_milli: 0,
                }
            } else {
                EvalSuiteSummary {
                    suite: suite.name.to_string(),
                    case_count,
                    success_rate_bps: (success_count * 10_000) / case_count,
                    avg_cost_cents: u32_saturating_from_u64(total_cost / u64::from(case_count)),
                    avg_latency_ms: u32_saturating_from_u64(total_latency / u64::from(case_count)),
                    avg_retries_milli: u32_saturating_from_u64(
                        (total_retries * 1_000) / u64::from(case_count),
                    ),
                }
            };
            summaries.push(summary);
        }

        let summary_fingerprint = fingerprint_summary(self.seed, &summaries);
        EvalReport {
            seed: self.seed,
            suites: summaries,
            summary_fingerprint,
        }
    }
}

fn run_behavioral_smoke_eval(seed: u64) {
    let spec = BehavioralEvalSpec {
        name: "baseline-behavioral-smoke".to_string(),
        description: "smoke behavioral assertions paired with synthetic baseline eval".to_string(),
        assertions: vec![
            BehavioralAssertion::PersonalityStability {
                trait_name: "openness".to_string(),
                max_drift: 0.15,
                adversarial_turns: 4,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::PreferenceCoherence {
                domain: "communication_style".to_string(),
                min_consistency: 0.8,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::CounterfactualQuality {
                min_distinct_factors: 2,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::IdentityContinuity {
                contract_layer: "stable".to_string(),
                max_violation_rate: 0.1,
                direction: AssertionDirection::Regression,
            },
        ],
        scenario_count: 3,
    };

    match run_behavioral_eval(&spec) {
        Ok(report) => tracing::debug!(
            seed,
            pass_rate = report.pass_rate,
            assertion_count = report.results.len(),
            "behavioral smoke eval completed"
        ),
        Err(error) => tracing::warn!(%error, seed, "behavioral smoke eval failed"),
    }
}

fn autonomy_regression_suite() -> EvalSuiteSpec {
    EvalSuiteSpec {
        name: "autonomy-regression",
        scenarios: vec![
            EvalScenarioSpec {
                id: "bounded-repair-success",
                success_target_percent: 93,
                min_cost_cents: 8,
                max_cost_cents: 23,
                min_latency_ms: 80,
                max_latency_ms: 190,
                retry_cap: 2,
            },
            EvalScenarioSpec {
                id: "policy-limit-enforced",
                success_target_percent: 88,
                min_cost_cents: 6,
                max_cost_cents: 19,
                min_latency_ms: 70,
                max_latency_ms: 170,
                retry_cap: 2,
            },
            EvalScenarioSpec {
                id: "scheduler-agent-split",
                success_target_percent: 90,
                min_cost_cents: 7,
                max_cost_cents: 21,
                min_latency_ms: 90,
                max_latency_ms: 210,
                retry_cap: 3,
            },
            EvalScenarioSpec {
                id: "temperature-clamp",
                success_target_percent: 91,
                min_cost_cents: 5,
                max_cost_cents: 17,
                min_latency_ms: 65,
                max_latency_ms: 155,
                retry_cap: 2,
            },
        ],
    }
}

fn injection_defense_suite() -> EvalSuiteSpec {
    EvalSuiteSpec {
        name: "injection-defense-regression",
        scenarios: vec![
            EvalScenarioSpec {
                id: "raw-payload-replay-blocked",
                success_target_percent: 95,
                min_cost_cents: 3,
                max_cost_cents: 12,
                min_latency_ms: 45,
                max_latency_ms: 125,
                retry_cap: 1,
            },
            EvalScenarioSpec {
                id: "prompt-injection-writeback-denied",
                success_target_percent: 92,
                min_cost_cents: 4,
                max_cost_cents: 15,
                min_latency_ms: 50,
                max_latency_ms: 130,
                retry_cap: 1,
            },
            EvalScenarioSpec {
                id: "sanitization-allows-low-risk",
                success_target_percent: 94,
                min_cost_cents: 4,
                max_cost_cents: 13,
                min_latency_ms: 40,
                max_latency_ms: 120,
                retry_cap: 1,
            },
            EvalScenarioSpec {
                id: "marker-collision-detection",
                success_target_percent: 90,
                min_cost_cents: 5,
                max_cost_cents: 14,
                min_latency_ms: 55,
                max_latency_ms: 140,
                retry_cap: 2,
            },
        ],
    }
}

fn companion_memory_ingestion_suite() -> EvalSuiteSpec {
    EvalSuiteSpec {
        name: "companion-memory-ingestion",
        scenarios: vec![
            EvalScenarioSpec {
                id: "tool-loop-success-rate",
                success_target_percent: 92,
                min_cost_cents: 6,
                max_cost_cents: 19,
                min_latency_ms: 75,
                max_latency_ms: 185,
                retry_cap: 2,
            },
            EvalScenarioSpec {
                id: "memory-recall-precision",
                success_target_percent: 90,
                min_cost_cents: 5,
                max_cost_cents: 16,
                min_latency_ms: 60,
                max_latency_ms: 160,
                retry_cap: 2,
            },
            EvalScenarioSpec {
                id: "ingestion-throughput",
                success_target_percent: 91,
                min_cost_cents: 4,
                max_cost_cents: 14,
                min_latency_ms: 50,
                max_latency_ms: 145,
                retry_cap: 1,
            },
        ],
    }
}

fn relational_quality_suite() -> EvalSuiteSpec {
    EvalSuiteSpec {
        name: "relational-quality-suite",
        scenarios: vec![
            EvalScenarioSpec {
                id: "coherence-turn-to-turn",
                success_target_percent: 91,
                min_cost_cents: 6,
                max_cost_cents: 20,
                min_latency_ms: 75,
                max_latency_ms: 195,
                retry_cap: 2,
            },
            EvalScenarioSpec {
                id: "trust-repair-after-mistake",
                success_target_percent: 88,
                min_cost_cents: 7,
                max_cost_cents: 22,
                min_latency_ms: 85,
                max_latency_ms: 210,
                retry_cap: 3,
            },
            EvalScenarioSpec {
                id: "emotional-appropriateness",
                success_target_percent: 90,
                min_cost_cents: 5,
                max_cost_cents: 17,
                min_latency_ms: 70,
                max_latency_ms: 180,
                retry_cap: 2,
            },
            EvalScenarioSpec {
                id: "boundary-respect-under-pressure",
                success_target_percent: 93,
                min_cost_cents: 4,
                max_cost_cents: 14,
                min_latency_ms: 60,
                max_latency_ms: 155,
                retry_cap: 2,
            },
        ],
    }
}

fn taste_benchmark_corpus_suite() -> EvalSuiteSpec {
    EvalSuiteSpec {
        name: "taste-benchmark-corpus",
        scenarios: vec![
            EvalScenarioSpec {
                id: "text-coherence-baseline",
                success_target_percent: 89,
                min_cost_cents: 5,
                max_cost_cents: 16,
                min_latency_ms: 65,
                max_latency_ms: 175,
                retry_cap: 2,
            },
            EvalScenarioSpec {
                id: "visual-hierarchy-baseline",
                success_target_percent: 90,
                min_cost_cents: 6,
                max_cost_cents: 18,
                min_latency_ms: 70,
                max_latency_ms: 185,
                retry_cap: 2,
            },
            EvalScenarioSpec {
                id: "intentionality-baseline",
                success_target_percent: 88,
                min_cost_cents: 6,
                max_cost_cents: 19,
                min_latency_ms: 75,
                max_latency_ms: 190,
                retry_cap: 2,
            },
            EvalScenarioSpec {
                id: "style-consistency-across-turns",
                success_target_percent: 87,
                min_cost_cents: 7,
                max_cost_cents: 21,
                min_latency_ms: 85,
                max_latency_ms: 205,
                retry_cap: 3,
            },
            EvalScenarioSpec {
                id: "anti-generic-response-quality",
                success_target_percent: 90,
                min_cost_cents: 5,
                max_cost_cents: 17,
                min_latency_ms: 70,
                max_latency_ms: 180,
                retry_cap: 2,
            },
        ],
    }
}

/// Return the built-in set of baseline evaluation suites.
#[must_use]
pub fn default_baseline_suites() -> Vec<EvalSuiteSpec> {
    vec![
        autonomy_regression_suite(),
        injection_defense_suite(),
        companion_memory_ingestion_suite(),
        relational_quality_suite(),
        taste_benchmark_corpus_suite(),
    ]
}

/// Return a warning message if the seed or fingerprint changed between
/// two eval reports.
#[must_use]
pub fn detect_seed_change_warning(previous: &EvalReport, current: &EvalReport) -> Option<String> {
    if previous.seed == current.seed {
        return None;
    }

    if previous.summary_fingerprint != current.summary_fingerprint {
        return Some(format!(
            "seed changed ({} -> {}) and summary fingerprint changed ({} -> {})",
            previous.seed, current.seed, previous.summary_fingerprint, current.summary_fingerprint
        ));
    }

    Some(format!(
        "seed changed ({} -> {}), summary fingerprint unchanged",
        previous.seed, current.seed
    ))
}

/// Write evaluation report artifacts (txt, csv, json) to the evidence
/// directory.
///
/// # Errors
///
/// Returns an error when creating directories, serializing reports,
/// or writing files fails.
pub fn write_evidence_files(
    repo_root: &Path,
    report: &EvalReport,
    slug: &str,
    warning: Option<&str>,
) -> Result<Vec<PathBuf>> {
    let evidence_dir = repo_root.join("evidence");
    fs::create_dir_all(&evidence_dir)?;

    let slug = crate::utils::text::sanitize_slug(slug, "eval");

    let txt_path = evidence_dir.join(format!("{slug}.txt"));
    let csv_path = evidence_dir.join(format!("{slug}-baseline-report.csv"));
    let json_path = evidence_dir.join(format!("{slug}-baseline-report.json"));

    fs::write(&txt_path, render_baseline_text_summary(report, warning))?;
    fs::write(&csv_path, render_baseline_csv(report))?;
    fs::write(&json_path, serde_json::to_string_pretty(report)?)?;

    Ok(vec![txt_path, csv_path, json_path])
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn run_is_deterministic_for_same_seed_and_inputs() {
        let suites = default_baseline_suites();
        let harness = EvalHarness::new(42);

        let first = harness.run(&suites);
        let second = harness.run(&suites);

        assert_eq!(first, second);
        assert_eq!(first.seed, 42);
        assert_eq!(first.suites.len(), suites.len());
    }

    #[test]
    fn baseline_suites_cover_companion_memory_and_ingestion_metrics() {
        let suites = default_baseline_suites();
        let memory_suite = suites
            .iter()
            .find(|suite| suite.name == "companion-memory-ingestion")
            .expect("companion-memory-ingestion suite should exist");

        let ids = memory_suite
            .scenarios
            .iter()
            .map(|scenario| scenario.id)
            .collect::<Vec<_>>();
        assert!(ids.contains(&"tool-loop-success-rate"));
        assert!(ids.contains(&"memory-recall-precision"));
        assert!(ids.contains(&"ingestion-throughput"));
    }

    #[test]
    fn baseline_suites_cover_relational_quality_signals() {
        let suites = default_baseline_suites();
        let relational_suite = suites
            .iter()
            .find(|suite| suite.name == "relational-quality-suite")
            .expect("relational-quality-suite should exist");

        let ids = relational_suite
            .scenarios
            .iter()
            .map(|scenario| scenario.id)
            .collect::<Vec<_>>();
        assert!(ids.contains(&"coherence-turn-to-turn"));
        assert!(ids.contains(&"trust-repair-after-mistake"));
        assert!(ids.contains(&"emotional-appropriateness"));
        assert!(ids.contains(&"boundary-respect-under-pressure"));
    }

    #[test]
    fn baseline_suites_cover_taste_benchmark_corpus() {
        let suites = default_baseline_suites();
        let taste_suite = suites
            .iter()
            .find(|suite| suite.name == "taste-benchmark-corpus")
            .expect("taste-benchmark-corpus should exist");

        let ids = taste_suite
            .scenarios
            .iter()
            .map(|scenario| scenario.id)
            .collect::<Vec<_>>();
        assert!(ids.contains(&"text-coherence-baseline"));
        assert!(ids.contains(&"visual-hierarchy-baseline"));
        assert!(ids.contains(&"intentionality-baseline"));
        assert!(ids.contains(&"style-consistency-across-turns"));
        assert!(ids.contains(&"anti-generic-response-quality"));
    }

    #[test]
    fn detect_seed_change_warning_reports_fingerprint_change() {
        let suites = default_baseline_suites();
        let previous = EvalHarness::new(100).run(&suites);
        let current = EvalHarness::new(200).run(&suites);

        let warning = detect_seed_change_warning(&previous, &current)
            .expect("different seeds should produce a warning");

        assert!(warning.contains("seed changed (100 -> 200)"));
        assert!(warning.contains("summary fingerprint changed"));
    }

    #[test]
    fn write_evidence_files_creates_files() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let report = EvalHarness::new(7).run(&default_baseline_suites());

        let files = write_evidence_files(temp_dir.path(), &report, "unit", Some("warn"))
            .expect("writing evidence files should succeed");

        assert_eq!(files.len(), 3);
        assert!(files.iter().all(|path| path.exists()));

        let txt = std::fs::read_to_string(&files[0]).expect("txt file should be readable");
        let csv = std::fs::read_to_string(&files[1]).expect("csv file should be readable");
        let json = std::fs::read_to_string(&files[2]).expect("json file should be readable");

        assert!(txt.contains("warning=warn"));
        assert!(csv.starts_with("suite,success-rate,cost,latency,retries"));
        assert!(json.contains("\"seed\": 7"));
    }

    #[test]
    fn write_evidence_files_sanitize_slug() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let report = EvalHarness::new(7).run(&default_baseline_suites());

        let files = write_evidence_files(temp_dir.path(), &report, " ../A/B C?* ", None)
            .expect("writing evidence files should succeed");

        assert_eq!(files.len(), 3);
        for path in files {
            let name = path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or_default();
            assert!(name.contains("a-b-c"), "unexpected file name: {name}");
            assert!(!name.contains(".."), "path traversal leaked: {name}");
        }
    }

    #[test]
    fn write_evidence_files_default_slug() {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let report = EvalHarness::new(7).run(&default_baseline_suites());

        let files = write_evidence_files(temp_dir.path(), &report, "   ", None)
            .expect("writing evidence files should succeed");

        assert_eq!(files.len(), 3);
        for path in files {
            let name = path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or_default();
            assert!(name.contains("eval"), "default slug should be used: {name}");
        }
    }
}
