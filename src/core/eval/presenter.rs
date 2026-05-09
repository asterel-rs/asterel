//! Eval result rendering for baseline, replay, and persona-consistency reports.

use std::fmt::Write as _;

use super::persona_consistency::{PERSONA_CONSISTENCY_EVALUATOR_SCOPE, PersonaConsistencyReport};
use super::replay_types::ReplayEvalReport;
use super::types::{
    BASELINE_REPORT_HEADER, EvalReport, format_currency_cents, format_rate, format_retries,
};

#[must_use]
pub fn render_baseline_csv(report: &EvalReport) -> String {
    let mut csv = String::from(BASELINE_REPORT_HEADER);
    csv.push('\n');
    for suite in &report.suites {
        let _ = writeln!(
            csv,
            "{},{},{},{}ms,{}",
            suite.suite,
            format_rate(suite.success_rate_bps),
            format_currency_cents(suite.avg_cost_cents),
            suite.avg_latency_ms,
            format_retries(suite.avg_retries_milli)
        );
    }
    csv
}

#[must_use]
pub fn render_baseline_text_summary(report: &EvalReport, warning: Option<&str>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "seed={}", report.seed);
    let _ = writeln!(out, "summary_fingerprint={}", report.summary_fingerprint);
    out.push_str("columns=success-rate,cost,latency,retries\n");
    for suite in &report.suites {
        let _ = writeln!(
            out,
            "suite={} success-rate={}bps cost={}c latency={}ms retries={}milli",
            suite.suite,
            suite.success_rate_bps,
            suite.avg_cost_cents,
            suite.avg_latency_ms,
            suite.avg_retries_milli
        );
    }
    if let Some(message) = warning {
        let _ = writeln!(out, "warning={message}");
    }
    out
}

#[must_use]
pub fn render_replay_csv(report: &ReplayEvalReport) -> String {
    let mut csv = String::from(
        "suite,record_count,success_rate,contradiction_ratio,calibration_error,verifier_event_ratio\n",
    );
    for suite in &report.suites {
        let _ = writeln!(
            csv,
            "{},{},{:.2}%,{:.2}%,{:.2}%,{:.2}%",
            suite.suite,
            suite.record_count,
            f64::from(suite.success_rate_bps) / 100.0,
            f64::from(suite.contradiction_ratio_bps) / 100.0,
            f64::from(suite.calibration_error_bps) / 100.0,
            f64::from(suite.verifier_event_ratio_bps) / 100.0,
        );
    }
    csv
}

#[must_use]
pub fn render_replay_text_summary(report: &ReplayEvalReport) -> String {
    let mut out = format!("source={}\n", report.source);
    for suite in &report.suites {
        let _ = writeln!(
            out,
            "suite={} records={} success_rate={}bps contradiction_ratio={}bps calibration_error={}bps verifier_event_ratio={}bps verifier_reasons={} fingerprint={}",
            suite.suite,
            suite.record_count,
            suite.success_rate_bps,
            suite.contradiction_ratio_bps,
            suite.calibration_error_bps,
            suite.verifier_event_ratio_bps,
            format_verifier_reason_counts(suite),
            suite.fingerprint,
        );
    }
    out
}

fn format_verifier_reason_counts(suite: &super::replay_types::ReplaySuiteReport) -> String {
    if suite.verifier_reason_counts.is_empty() {
        return "none".to_string();
    }

    suite
        .verifier_reason_counts
        .iter()
        .map(|(reason, count)| format!("{reason}:{count}"))
        .collect::<Vec<_>>()
        .join("|")
}

#[must_use]
pub fn render_persona_consistency_text_summary(report: &PersonaConsistencyReport) -> String {
    format!(
        "scope={}\nprompt_to_line={:.4}\nline_to_line={:.4}\nqa_consistency={:.4}\ncomposite={:.4}\n",
        PERSONA_CONSISTENCY_EVALUATOR_SCOPE,
        report.prompt_to_line,
        report.line_to_line,
        report.qa_consistency,
        report.composite
    )
}

#[must_use]
pub fn render_persona_consistency_csv(report: &PersonaConsistencyReport) -> String {
    format!(
        "scope,prompt_to_line,line_to_line,qa_consistency,composite\n{},{:.4},{:.4},{:.4},{:.4}\n",
        PERSONA_CONSISTENCY_EVALUATOR_SCOPE,
        report.prompt_to_line,
        report.line_to_line,
        report.qa_consistency,
        report.composite,
    )
}
