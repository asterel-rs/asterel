//! Replay trace types for the eval-replay harness.
//!
//! A replay trace is a JSONL file where each line is a serialised
//! [`ReplayRecord`].  Records capture the minimal information
//! needed to re-derive evaluation metrics (success rate,
//! contradiction ratio, calibration error) without live model
//! calls.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ── Trace record ───────────────────────────────────────────────

/// Single turn captured from a real (or synthetic-seed) session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayRecord {
    /// Product surface where the turn was observed, such as `discord_public`,
    /// `discord_dm`, or `gateway`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,

    /// The user's input message for this turn.
    pub user_message: String,

    /// The assistant's textual response.
    pub assistant_response: String,

    /// Tool invocations that occurred during the turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ReplayToolCall>,

    /// Optional quality vector captured at turn time (WS-S2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_vector: Option<ReplayQualitySnapshot>,

    /// Safety / policy events that fired during the turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub safety_events: Vec<SafetyEvent>,

    /// Verifier events emitted during response finalization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verifier_events: Vec<VerifierEvent>,
}

/// A tool call recorded in a replay trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayToolCall {
    /// Tool name (e.g. `"web_search"`, `"code_exec"`).
    pub name: String,

    /// Whether the tool invocation succeeded.
    #[serde(default)]
    pub success: bool,
}

/// Lightweight quality vector snapshot embedded in a trace record.
///
/// Mirrors the dimensions of `TurnQualityVector` but uses plain
/// `f32` fields so that replay traces remain self-contained.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayQualitySnapshot {
    pub task_completion: f32,
    pub tool_effectiveness: f32,
    pub retrieval_utilization: f32,
    pub contradiction_safety: f32,
    pub user_friction: f32,
    pub explanation_quality: f32,
    pub composite: f32,
}

/// A safety event recorded during a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyEvent {
    /// Machine-readable event kind (e.g. `"prompt_injection_blocked"`).
    pub kind: String,

    /// Human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Verifier event recorded during response finalization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifierEvent {
    /// Pipeline phase that emitted the event, such as `"output"` or `"exposure"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,

    /// Machine-readable reason code, such as `"anti_template"` or `"over_explain"`.
    pub reason_code: String,
}

// ── Replay report ──────────────────────────────────────────────

/// Aggregated metrics produced by running a replay suite.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplaySuiteReport {
    /// Suite name (from CLI `--suite`).
    pub suite: String,

    /// Number of records evaluated.
    pub record_count: u32,

    /// Success rate in basis points (10 000 = 100%).
    pub success_rate_bps: u32,

    /// Fraction of turns that contained a contradiction safety
    /// event, expressed in basis points.
    pub contradiction_ratio_bps: u32,

    /// Mean absolute calibration error of the quality vector
    /// composite vs. observed binary outcome, in basis points.
    pub calibration_error_bps: u32,

    /// Fraction of turns with at least one verifier event, in basis points.
    pub verifier_event_ratio_bps: u32,

    /// Verifier event counts grouped by reason code.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub verifier_reason_counts: BTreeMap<String, u32>,

    /// Deterministic fingerprint over the input + metrics.
    pub fingerprint: u64,
}

/// Complete replay evaluation report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayEvalReport {
    /// Path to the source JSONL file.
    pub source: String,

    /// Per-suite reports (currently one suite per run).
    pub suites: Vec<ReplaySuiteReport>,
}

impl ReplayEvalReport {
    #[must_use]
    pub fn render_csv(&self) -> String {
        crate::core::eval::presenter::render_replay_csv(self)
    }

    #[must_use]
    pub fn render_text_summary(&self) -> String {
        crate::core::eval::presenter::render_replay_text_summary(self)
    }
}
