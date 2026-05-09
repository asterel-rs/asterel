//! Evaluation types: scenario specs, suite specs, summaries, and
//! reports used by the deterministic eval harness.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Specification for a single evaluation scenario within a suite.
#[derive(Debug, Clone)]
pub struct EvalScenarioSpec {
    /// Unique scenario identifier within the suite.
    pub id: &'static str,
    /// Target success rate as a percentage (0--100).
    pub success_target_percent: u8,
    /// Minimum simulated cost in cents.
    pub min_cost_cents: u32,
    /// Maximum simulated cost in cents.
    pub max_cost_cents: u32,
    /// Minimum simulated latency in milliseconds.
    pub min_latency_ms: u32,
    /// Maximum simulated latency in milliseconds.
    pub max_latency_ms: u32,
    /// Upper bound on retry attempts per scenario run.
    pub retry_cap: u32,
}

/// Specification for an evaluation suite containing multiple scenarios.
#[derive(Debug, Clone)]
pub struct EvalSuiteSpec {
    /// Human-readable suite name used as a key in reports.
    pub name: &'static str,
    /// Ordered list of scenario specifications in this suite.
    pub scenarios: Vec<EvalScenarioSpec>,
}

/// Aggregated summary statistics for a single evaluated suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalSuiteSummary {
    /// Name of the evaluated suite.
    pub suite: String,
    /// Number of scenario cases in the suite.
    pub case_count: u32,
    /// Success rate in basis points (10 000 = 100%).
    pub success_rate_bps: u32,
    /// Average cost per case in cents.
    pub avg_cost_cents: u32,
    /// Average latency per case in milliseconds.
    pub avg_latency_ms: u32,
    /// Average retries per case in milli-retries (1000 = 1.0).
    pub avg_retries_milli: u32,
}

/// Complete evaluation report produced by the harness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalReport {
    /// RNG seed used for deterministic reproduction.
    pub seed: u64,
    /// Per-suite summary statistics.
    pub suites: Vec<EvalSuiteSummary>,
    /// Hash fingerprint of the combined suite summaries.
    pub summary_fingerprint: u64,
}

const BASELINE_REPORT_REQUIRED_COLUMNS: [&str; 5] =
    ["suite", "success-rate", "cost", "latency", "retries"];
pub(crate) const BASELINE_REPORT_HEADER: &str = "suite,success-rate,cost,latency,retries";

fn expected_baseline_columns() -> Vec<String> {
    BASELINE_REPORT_REQUIRED_COLUMNS
        .iter()
        .map(ToString::to_string)
        .collect()
}

fn parse_csv_columns(line: &str) -> Vec<String> {
    line.split(',')
        .map(|entry| entry.trim().to_ascii_lowercase())
        .collect()
}

impl EvalReport {
    /// Return the required column names for baseline CSV reports.
    #[must_use]
    pub fn required_csv_columns() -> &'static [&'static str; 5] {
        &BASELINE_REPORT_REQUIRED_COLUMNS
    }

    #[must_use]
    pub fn render_csv(&self) -> String {
        super::presenter::render_baseline_csv(self)
    }

    #[must_use]
    pub fn render_text_summary(&self, warning: Option<&str>) -> String {
        super::presenter::render_baseline_text_summary(self, warning)
    }
}

/// Validate that a baseline CSV report has the expected column header.
///
/// # Errors
///
/// Returns an error when required columns are missing, duplicated,
/// or out of order.
pub fn validate_baseline_report_columns(csv: &str) -> Result<()> {
    let header = csv
        .lines()
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing eval report csv header"))?;
    let columns = parse_csv_columns(header);
    let expected = expected_baseline_columns();

    if columns.is_empty() {
        bail!("missing eval report csv header");
    }

    let mut seen = HashSet::new();
    let mut duplicates = Vec::new();
    for column in &columns {
        if !seen.insert(column.clone()) {
            duplicates.push(column.clone());
        }
    }

    if !duplicates.is_empty() {
        bail!("duplicate columns: {}", duplicates.join(", "));
    }

    if columns == expected {
        return Ok(());
    }

    let mut missing: Vec<String> = Vec::new();
    for required in &expected {
        if !columns.iter().any(|entry| entry == required) {
            missing.push(required.clone());
        }
    }

    let mut unexpected: Vec<String> = Vec::new();
    for column in &columns {
        if !expected.iter().any(|expected| expected == column) {
            unexpected.push(column.clone());
        }
    }

    if !missing.is_empty() {
        bail!("missing required columns: {}", missing.join(", "));
    }

    if !unexpected.is_empty() {
        bail!("unexpected report columns: {}", unexpected.join(", "));
    }

    bail!(
        "unexpected column order: expected {} got {}",
        expected.join(", "),
        columns.join(", ")
    )
}

pub(crate) fn format_rate(success_rate_bps: u32) -> String {
    format!("{:.2}%", f64::from(success_rate_bps) / 100.0)
}

pub(crate) fn format_currency_cents(avg_cost_cents: u32) -> String {
    format!("${:.2}", f64::from(avg_cost_cents) / 100.0)
}

pub(crate) fn format_retries(avg_retries_milli: u32) -> String {
    format!("{:.3}", f64::from(avg_retries_milli) / 1_000.0)
}
