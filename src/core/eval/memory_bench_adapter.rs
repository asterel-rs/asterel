//! Memory quality benchmark adapter: normalizes retrieval outcomes
//! into `precision@k`, `useful_recall_rate`, and `contradiction_rate`
//! metrics for weekly quality reports.

use std::fmt;
use std::io::Write;
use std::path::Path;

use num_traits::ToPrimitive;

/// A single memory retrieval trial used as input to the benchmark.
#[derive(Debug, Clone)]
pub struct MemoryBenchTrial {
    /// Items retrieved by the memory system (ordered by score descending).
    pub retrieved_slot_keys: Vec<String>,
    /// Ground-truth relevant slot keys for this query.
    pub relevant_slot_keys: Vec<String>,
    /// Whether a contradiction was detected during this retrieval.
    pub contradiction_detected: bool,
    /// Whether the retrieved items were actually useful in the response.
    pub items_used_in_response: usize,
}

/// Aggregated benchmark report for a set of memory retrieval trials.
#[derive(Debug, Clone)]
pub struct MemoryBenchReport {
    /// Name of the benchmark suite.
    pub suite_name: String,
    /// Number of trials evaluated.
    pub trial_count: usize,
    /// Precision@k: fraction of top-k retrieved items that are relevant.
    /// Expressed in basis points (0-10000).
    pub precision_at_k_bps: u32,
    /// Useful recall rate: fraction of trials where at least one
    /// retrieved item was used in the response. Basis points.
    pub useful_recall_rate_bps: u32,
    /// Contradiction rate: fraction of trials with a contradiction
    /// detected. Basis points.
    pub contradiction_rate_bps: u32,
    /// The k value used for precision@k.
    pub k: usize,
}

/// Default k for precision@k computation.
const DEFAULT_K: usize = 5;

/// Evaluate a set of memory bench trials into an aggregated report.
#[must_use]
pub fn evaluate_memory_bench(
    suite_name: &str,
    trials: &[MemoryBenchTrial],
    k: Option<usize>,
) -> MemoryBenchReport {
    let k = k.unwrap_or(DEFAULT_K);
    let trial_count = trials.len();

    if trial_count == 0 {
        return MemoryBenchReport {
            suite_name: suite_name.to_string(),
            trial_count: 0,
            precision_at_k_bps: 0,
            useful_recall_rate_bps: 0,
            contradiction_rate_bps: 0,
            k,
        };
    }

    let mut total_precision = 0.0_f64;
    let mut useful_count = 0_usize;
    let mut contradiction_count = 0_usize;

    for trial in trials {
        // Precision@k: of the top-k retrieved items, how many are relevant?
        let top_k: Vec<&String> = trial.retrieved_slot_keys.iter().take(k).collect();
        let relevant_in_top_k = top_k
            .iter()
            .filter(|key| trial.relevant_slot_keys.contains(key))
            .count();
        let precision = if top_k.is_empty() {
            0.0
        } else {
            let relevant_in_top_k_u32 = u32::try_from(relevant_in_top_k).unwrap_or(u32::MAX);
            let top_k_len_u32 = u32::try_from(top_k.len()).unwrap_or(u32::MAX).max(1);
            f64::from(relevant_in_top_k_u32) / f64::from(top_k_len_u32)
        };
        total_precision += precision;

        // Useful recall: was at least one retrieved item used?
        if trial.items_used_in_response > 0 {
            useful_count += 1;
        }

        // Contradiction rate
        if trial.contradiction_detected {
            contradiction_count += 1;
        }
    }

    let trial_count_f64 = bounded_count_to_f64(trial_count);
    let avg_precision = total_precision / trial_count_f64;
    let useful_rate = bounded_count_to_f64(useful_count) / trial_count_f64;
    let contradiction_rate = bounded_count_to_f64(contradiction_count) / trial_count_f64;

    MemoryBenchReport {
        suite_name: suite_name.to_string(),
        trial_count,
        precision_at_k_bps: to_bps(avg_precision),
        useful_recall_rate_bps: to_bps(useful_rate),
        contradiction_rate_bps: to_bps(contradiction_rate),
        k,
    }
}

fn to_bps(ratio: f64) -> u32 {
    let clamped = if ratio.is_finite() {
        ratio.clamp(0.0, 1.0)
    } else {
        0.0
    };
    (clamped * 10_000.0).round().to_u32().unwrap_or(0)
}

fn from_bps(bps: u32) -> f64 {
    f64::from(bps) / 10_000.0
}

fn bounded_count_to_f64(value: usize) -> f64 {
    match u32::try_from(value) {
        Ok(value) => f64::from(value),
        Err(_) => f64::from(u32::MAX),
    }
}

impl fmt::Display for MemoryBenchReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Memory Bench: {}", self.suite_name)?;
        writeln!(f, "  trials:              {}", self.trial_count)?;
        writeln!(f, "  k:                   {}", self.k)?;
        writeln!(
            f,
            "  precision@{:<10} {:.2}%",
            self.k,
            from_bps(self.precision_at_k_bps) * 100.0
        )?;
        writeln!(
            f,
            "  useful_recall_rate:  {:.2}%",
            from_bps(self.useful_recall_rate_bps) * 100.0
        )?;
        write!(
            f,
            "  contradiction_rate:  {:.2}%",
            from_bps(self.contradiction_rate_bps) * 100.0
        )
    }
}

/// Write memory bench evidence files to an output directory.
///
/// Creates `{slug}.memory_bench.txt` and `{slug}.memory_bench.csv`.
///
/// # Errors
///
/// Returns an error if file creation or writing fails.
pub fn write_memory_bench_evidence(
    output_dir: &Path,
    slug: &str,
    report: &MemoryBenchReport,
) -> std::io::Result<()> {
    std::fs::create_dir_all(output_dir)?;

    // Text report
    let safe_slug = evidence_slug(slug);
    let txt_path = output_dir.join(format!("{safe_slug}.memory_bench.txt"));
    let mut txt = std::fs::File::create(txt_path)?;
    write!(txt, "{report}")?;

    // CSV report
    let csv_path = output_dir.join(format!("{safe_slug}.memory_bench.csv"));
    let mut csv = std::fs::File::create(csv_path)?;
    writeln!(
        csv,
        "suite,trials,k,precision_at_k_bps,useful_recall_rate_bps,contradiction_rate_bps"
    )?;
    writeln!(
        csv,
        "{},{},{},{},{},{}",
        csv_field(&report.suite_name),
        report.trial_count,
        report.k,
        report.precision_at_k_bps,
        report.useful_recall_rate_bps,
        report.contradiction_rate_bps
    )?;

    Ok(())
}

fn evidence_slug(slug: &str) -> String {
    let mut out = String::with_capacity(slug.len().max(1));
    for ch in slug.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "memory_bench".to_string()
    } else {
        trimmed.to_string()
    }
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_trial(
        retrieved: &[&str],
        relevant: &[&str],
        used: usize,
        contradiction: bool,
    ) -> MemoryBenchTrial {
        MemoryBenchTrial {
            retrieved_slot_keys: retrieved
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
            relevant_slot_keys: relevant
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
            contradiction_detected: contradiction,
            items_used_in_response: used,
        }
    }

    #[test]
    fn empty_trials_produce_zero_report() {
        let report = evaluate_memory_bench("empty", &[], None);
        assert_eq!(report.trial_count, 0);
        assert_eq!(report.precision_at_k_bps, 0);
        assert_eq!(report.useful_recall_rate_bps, 0);
        assert_eq!(report.contradiction_rate_bps, 0);
    }

    #[test]
    fn perfect_precision_all_relevant() {
        let trials = vec![make_trial(&["a", "b", "c"], &["a", "b", "c"], 3, false)];
        let report = evaluate_memory_bench("perfect", &trials, Some(3));
        assert_eq!(report.precision_at_k_bps, 10_000);
        assert_eq!(report.useful_recall_rate_bps, 10_000);
        assert_eq!(report.contradiction_rate_bps, 0);
    }

    #[test]
    fn zero_precision_none_relevant() {
        let trials = vec![make_trial(&["a", "b"], &["x", "y"], 0, false)];
        let report = evaluate_memory_bench("none", &trials, Some(5));
        assert_eq!(report.precision_at_k_bps, 0);
        assert_eq!(report.useful_recall_rate_bps, 0);
    }

    #[test]
    fn partial_precision() {
        let trials = vec![make_trial(&["a", "b", "c", "d"], &["a", "c"], 1, false)];
        // k=4: 2 relevant out of 4 = 0.5 = 5000 bps
        let report = evaluate_memory_bench("partial", &trials, Some(4));
        assert_eq!(report.precision_at_k_bps, 5000);
        assert_eq!(report.useful_recall_rate_bps, 10_000);
    }

    #[test]
    fn contradiction_rate_computed() {
        let trials = vec![
            make_trial(&["a"], &["a"], 1, true),
            make_trial(&["b"], &["b"], 1, false),
            make_trial(&["c"], &["c"], 1, true),
            make_trial(&["d"], &["d"], 1, false),
        ];
        let report = evaluate_memory_bench("contra", &trials, None);
        assert_eq!(report.contradiction_rate_bps, 5000); // 2/4
    }

    #[test]
    fn useful_recall_rate_partial() {
        let trials = vec![
            make_trial(&["a"], &["a"], 1, false),
            make_trial(&["b"], &["b"], 0, false),
            make_trial(&["c"], &["c"], 0, false),
        ];
        let report = evaluate_memory_bench("useful", &trials, None);
        // 1/3 used = 3333 bps
        assert_eq!(report.useful_recall_rate_bps, 3333);
    }

    #[test]
    fn display_format_includes_all_metrics() {
        let report = MemoryBenchReport {
            suite_name: "test_suite".to_string(),
            trial_count: 10,
            precision_at_k_bps: 7500,
            useful_recall_rate_bps: 8000,
            contradiction_rate_bps: 1000,
            k: 5,
        };
        let output = format!("{report}");
        assert!(output.contains("test_suite"));
        assert!(output.contains("75.00%"));
        assert!(output.contains("80.00%"));
        assert!(output.contains("10.00%"));
    }

    #[test]
    fn to_bps_boundaries() {
        assert_eq!(to_bps(0.0), 0);
        assert_eq!(to_bps(1.0), 10_000);
        assert_eq!(to_bps(0.5), 5000);
        assert_eq!(to_bps(-0.1), 0);
        assert_eq!(to_bps(1.5), 10_000);
    }

    #[test]
    fn from_bps_round_trip() {
        assert!((from_bps(5000) - 0.5).abs() < f64::EPSILON);
        assert!((from_bps(10_000) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn write_evidence_creates_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let report = MemoryBenchReport {
            suite_name: "write_test".to_string(),
            trial_count: 5,
            precision_at_k_bps: 6000,
            useful_recall_rate_bps: 7000,
            contradiction_rate_bps: 500,
            k: 5,
        };
        write_memory_bench_evidence(dir.path(), "test_slug", &report).unwrap();
        assert!(dir.path().join("test_slug.memory_bench.txt").exists());
        assert!(dir.path().join("test_slug.memory_bench.csv").exists());

        let csv = std::fs::read_to_string(dir.path().join("test_slug.memory_bench.csv")).unwrap();
        assert!(csv.contains("write_test"));
        assert!(csv.contains("6000"));
    }

    #[test]
    fn write_evidence_sanitizes_slug_and_escapes_csv() {
        let dir = tempfile::TempDir::new().unwrap();
        let report = MemoryBenchReport {
            suite_name: "suite,\n\"quoted\"".to_string(),
            trial_count: 1,
            precision_at_k_bps: 6000,
            useful_recall_rate_bps: 7000,
            contradiction_rate_bps: 500,
            k: 5,
        };
        write_memory_bench_evidence(dir.path(), "../bad/slug", &report).unwrap();

        assert!(dir.path().join("bad_slug.memory_bench.txt").exists());
        assert!(dir.path().join("bad_slug.memory_bench.csv").exists());
        assert!(!dir.path().join("..").join("bad").exists());

        let csv = std::fs::read_to_string(dir.path().join("bad_slug.memory_bench.csv")).unwrap();
        assert!(csv.contains("\"suite,\n\"\"quoted\"\"\""));
    }

    #[test]
    fn k_truncates_retrieved_list() {
        // Retrieved 10 items but k=3, only first 3 considered
        let trials = vec![make_trial(
            &["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"],
            &["a", "b", "c"],
            3,
            false,
        )];
        let report = evaluate_memory_bench("k_trunc", &trials, Some(3));
        assert_eq!(report.precision_at_k_bps, 10_000); // 3/3 = perfect
    }

    #[test]
    fn multiple_trials_averaged() {
        let trials = vec![
            make_trial(&["a", "b"], &["a", "b"], 2, false), // precision=1.0
            make_trial(&["c", "d"], &["x", "y"], 0, false), // precision=0.0
        ];
        let report = evaluate_memory_bench("avg", &trials, Some(2));
        assert_eq!(report.precision_at_k_bps, 5000); // average = 0.5
    }

    #[test]
    fn empty_retrieved_gives_zero_precision() {
        let trials = vec![make_trial(&[], &["a", "b"], 0, false)];
        let report = evaluate_memory_bench("empty_ret", &trials, Some(5));
        assert_eq!(report.precision_at_k_bps, 0);
    }
}
