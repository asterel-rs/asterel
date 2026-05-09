//! Replay eval harness: evaluates real/synthetic turn traces from
//! JSONL files to produce deterministic metrics.
//!
//! Complements the synthetic baseline harness (`harness.rs`) by
//! operating on captured session data rather than generated
//! scenarios.

use std::collections::BTreeMap;
use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use num_traits::ToPrimitive;

use super::presenter::{render_replay_csv, render_replay_text_summary};
use super::replay_types::{ReplayEvalReport, ReplayRecord, ReplaySuiteReport};

// ── Parsing ────────────────────────────────────────────────────

/// Parse a JSONL file into a sequence of replay records.
///
/// # Errors
///
/// Returns an error when the file cannot be read or a line
/// contains invalid JSON.
pub fn parse_replay_jsonl(path: &Path) -> Result<Vec<ReplayRecord>> {
    let file = fs::File::open(path)
        .with_context(|| format!("failed to open replay file: {}", path.display()))?;
    let reader = std::io::BufReader::new(file);

    let mut records = Vec::new();
    for (line_idx, line) in reader.lines().enumerate() {
        let line = line.with_context(|| {
            format!("failed to read line {} of {}", line_idx + 1, path.display())
        })?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let record: ReplayRecord = serde_json::from_str(trimmed).with_context(|| {
            format!(
                "invalid JSON on line {} of {}",
                line_idx + 1,
                path.display()
            )
        })?;
        records.push(record);
    }
    Ok(records)
}

// ── Metrics ────────────────────────────────────────────────────

/// Threshold above which a turn is considered successful based on
/// the quality vector composite score.
const SUCCESS_COMPOSITE_THRESHOLD: f32 = 0.5;

/// Evaluate a set of replay records into a [`ReplaySuiteReport`].
///
/// Metrics:
/// - **`success_rate`**: fraction of turns where the composite quality
///   score is present and exceeds [`SUCCESS_COMPOSITE_THRESHOLD`]. Turns
///   without quality vectors are treated as unevaluated failures rather than
///   optimistic successes, so replay evidence cannot go green from response
///   presence alone.
/// - **`contradiction_ratio`**: fraction of turns that contain at least
///   one safety event whose kind starts with `"contradiction"`.
/// - **`calibration_error`**: mean |predicted − actual| where
///   *predicted* = composite score and *actual* = binary success.
fn evaluate_suite(suite_name: &str, records: &[ReplayRecord]) -> ReplaySuiteReport {
    if records.is_empty() {
        return ReplaySuiteReport {
            suite: suite_name.to_string(),
            record_count: 0,
            success_rate_bps: 0,
            contradiction_ratio_bps: 0,
            calibration_error_bps: 0,
            verifier_event_ratio_bps: 0,
            verifier_reason_counts: BTreeMap::new(),
            fingerprint: 0,
        };
    }

    let count = u32::try_from(records.len()).unwrap_or(u32::MAX);
    let mut successes: u32 = 0;
    let mut contradictions: u32 = 0;
    let mut verifier_event_turns: u32 = 0;
    let mut verifier_reason_counts = BTreeMap::new();
    let mut calibration_error_sum: f64 = 0.0;
    let mut calibration_samples: u32 = 0;

    for record in records {
        // Determine binary success.
        let success = record
            .quality_vector
            .as_ref()
            .is_some_and(|qv| qv.composite >= SUCCESS_COMPOSITE_THRESHOLD);
        if success {
            successes += 1;
        }

        // Contradiction ratio.
        let has_contradiction = record
            .safety_events
            .iter()
            .any(|ev| ev.kind.starts_with("contradiction"));
        if has_contradiction {
            contradictions += 1;
        }

        if !record.verifier_events.is_empty() {
            verifier_event_turns += 1;
        }
        for event in &record.verifier_events {
            *verifier_reason_counts
                .entry(event.reason_code.clone())
                .or_insert(0) += 1;
        }

        // Calibration error (only when quality vector is present).
        if let Some(qv) = &record.quality_vector {
            let actual = if success { 1.0_f64 } else { 0.0 };
            let predicted = f64::from(qv.composite);
            calibration_error_sum += (predicted - actual).abs();
            calibration_samples += 1;
        }
    }

    let success_rate_bps = (successes * 10_000) / count;
    let contradiction_ratio_bps = (contradictions * 10_000) / count;
    let verifier_event_ratio_bps = (verifier_event_turns * 10_000) / count;
    let calibration_error_bps = if calibration_samples > 0 {
        let mean_error = calibration_error_sum / f64::from(calibration_samples);
        clamp_bps(mean_error)
    } else {
        0
    };

    let fingerprint = fingerprint_suite(suite_name, records);

    ReplaySuiteReport {
        suite: suite_name.to_string(),
        record_count: count,
        success_rate_bps,
        contradiction_ratio_bps,
        calibration_error_bps,
        verifier_event_ratio_bps,
        verifier_reason_counts,
        fingerprint,
    }
}

fn clamp_bps(value: f64) -> u32 {
    let clamped = if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    };
    (clamped * 10_000.0).round().to_u32().unwrap_or(0)
}

// ── Fingerprinting ─────────────────────────────────────────────

/// Deterministic FNV-1a-based fingerprint over suite name + records.
fn fingerprint_suite(suite_name: &str, records: &[ReplayRecord]) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325_u64;
    hash = fnv_mix(hash, suite_name.as_bytes());

    for record in records {
        if let Some(surface) = &record.surface {
            hash = fnv_mix(hash, surface.as_bytes());
        }
        hash = fnv_mix(hash, record.user_message.as_bytes());
        hash = fnv_mix(hash, record.assistant_response.as_bytes());
        for tc in &record.tool_calls {
            hash = fnv_mix(hash, tc.name.as_bytes());
            hash = fnv_mix(hash, &[u8::from(tc.success)]);
        }
        for ev in &record.safety_events {
            hash = fnv_mix(hash, ev.kind.as_bytes());
        }
        for ev in &record.verifier_events {
            if let Some(phase) = &ev.phase {
                hash = fnv_mix(hash, phase.as_bytes());
            }
            hash = fnv_mix(hash, ev.reason_code.as_bytes());
        }
    }
    hash
}

fn fnv_mix(mut state: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes {
        state ^= u64::from(byte);
        state = state.wrapping_mul(0x1000_0000_01B3);
    }
    state
}

// ── Public API ─────────────────────────────────────────────────

/// Run the replay eval harness on a JSONL file.
///
/// # Errors
///
/// Returns an error when the input file cannot be parsed.
pub fn run_replay(input: &Path, suite_name: &str) -> Result<ReplayEvalReport> {
    let records = parse_replay_jsonl(input)?;
    if records.is_empty() {
        bail!("replay file contains no records: {}", input.display());
    }

    let suite_report = evaluate_suite(suite_name, &records);
    Ok(ReplayEvalReport {
        source: input.display().to_string(),
        suites: vec![suite_report],
    })
}

/// Write replay evaluation report artifacts (txt, csv, json) to
/// the evidence directory.
///
/// # Errors
///
/// Returns an error when creating directories or writing files fails.
pub fn write_replay_evidence_files(
    repo_root: &Path,
    report: &ReplayEvalReport,
    slug: &str,
) -> Result<Vec<PathBuf>> {
    let evidence_dir = repo_root.join("evidence");
    fs::create_dir_all(&evidence_dir)?;

    let slug = crate::utils::text::sanitize_slug(slug, "replay");

    let txt_path = evidence_dir.join(format!("{slug}-replay.txt"));
    let csv_path = evidence_dir.join(format!("{slug}-replay-report.csv"));
    let json_path = evidence_dir.join(format!("{slug}-replay-report.json"));

    fs::write(&txt_path, render_replay_text_summary(report))?;
    fs::write(&csv_path, render_replay_csv(report))?;
    fs::write(&json_path, serde_json::to_string_pretty(report)?)?;

    Ok(vec![txt_path, csv_path, json_path])
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::Write;

    use tempfile::{NamedTempFile, TempDir};

    use super::*;
    use crate::core::eval::presenter::{render_replay_csv, render_replay_text_summary};
    use crate::core::eval::replay_types::{
        ReplayQualitySnapshot, ReplayToolCall, SafetyEvent, VerifierEvent,
    };

    fn sample_record(composite: f32, contradiction: bool) -> ReplayRecord {
        let mut safety = Vec::new();
        if contradiction {
            safety.push(SafetyEvent {
                kind: "contradiction_detected".to_string(),
                detail: None,
            });
        }
        ReplayRecord {
            surface: None,
            user_message: "hello".to_string(),
            assistant_response: "world".to_string(),
            tool_calls: vec![ReplayToolCall {
                name: "web_search".to_string(),
                success: true,
            }],
            quality_vector: Some(ReplayQualitySnapshot {
                task_completion: composite,
                tool_effectiveness: composite,
                retrieval_utilization: composite,
                contradiction_safety: if contradiction { 0.0 } else { 1.0 },
                user_friction: composite,
                explanation_quality: composite,
                composite,
            }),
            safety_events: safety,
            verifier_events: Vec::new(),
        }
    }

    fn write_jsonl(records: &[ReplayRecord]) -> NamedTempFile {
        let mut file = NamedTempFile::new().expect("temp file");
        for record in records {
            let json = serde_json::to_string(record).expect("serialise");
            writeln!(file, "{json}").expect("write");
        }
        file
    }

    // ── Parser tests ───────────────────────────────────────────

    #[test]
    fn parse_valid_jsonl() {
        let records = vec![sample_record(0.8, false), sample_record(0.3, true)];
        let file = write_jsonl(&records);
        let parsed = parse_replay_jsonl(file.path()).expect("parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].user_message, "hello");
    }

    #[test]
    fn parse_skips_blank_lines() {
        let mut file = NamedTempFile::new().expect("temp file");
        let rec = sample_record(0.9, false);
        writeln!(file, "{}", serde_json::to_string(&rec).unwrap()).unwrap();
        writeln!(file).unwrap();
        writeln!(file, "  ").unwrap();
        writeln!(file, "{}", serde_json::to_string(&rec).unwrap()).unwrap();
        let parsed = parse_replay_jsonl(file.path()).expect("parse");
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn parse_accepts_surface_and_verifier_events() {
        let mut record = sample_record(0.9, false);
        record.surface = Some("discord_public".to_string());
        record.verifier_events.push(VerifierEvent {
            phase: Some("output".to_string()),
            reason_code: "anti_template".to_string(),
        });
        let file = write_jsonl(&[record]);
        let parsed = parse_replay_jsonl(file.path()).expect("parse");
        assert_eq!(parsed[0].surface.as_deref(), Some("discord_public"));
        assert_eq!(parsed[0].verifier_events[0].reason_code, "anti_template");
    }

    #[test]
    fn parse_invalid_json_returns_error() {
        let mut file = NamedTempFile::new().expect("temp file");
        writeln!(file, "{{not valid json}}").unwrap();
        let result = parse_replay_jsonl(file.path());
        assert!(result.is_err());
    }

    #[test]
    fn parse_missing_file_returns_error() {
        let result = parse_replay_jsonl(Path::new("/nonexistent/replay.jsonl"));
        assert!(result.is_err());
    }

    // ── Determinism tests ──────────────────────────────────────

    #[test]
    fn replay_is_deterministic_for_same_input() {
        let records = vec![
            sample_record(0.9, false),
            sample_record(0.4, true),
            sample_record(0.6, false),
        ];
        let file = write_jsonl(&records);
        let r1 = run_replay(file.path(), "det-test").expect("run 1");
        let r2 = run_replay(file.path(), "det-test").expect("run 2");
        assert_eq!(r1, r2);
    }

    #[test]
    fn fingerprint_changes_with_different_input() {
        let a = vec![sample_record(0.9, false)];
        let b = vec![sample_record(0.3, true)];
        let file_a = write_jsonl(&a);
        let file_b = write_jsonl(&b);
        let r_a = run_replay(file_a.path(), "fp-test").expect("a");
        let r_b = run_replay(file_b.path(), "fp-test").expect("b");
        assert_ne!(r_a.suites[0].fingerprint, r_b.suites[0].fingerprint);
    }

    // ── Metric correctness tests ───────────────────────────────

    #[test]
    fn success_rate_computed_correctly() {
        // 2 successes (composite > 0.5), 1 failure
        let records = vec![
            sample_record(0.8, false),
            sample_record(0.6, false),
            sample_record(0.3, false),
        ];
        let file = write_jsonl(&records);
        let report = run_replay(file.path(), "sr-test").expect("run");
        // 2/3 ≈ 6666 bps
        assert_eq!(report.suites[0].success_rate_bps, 6666);
    }

    #[test]
    fn contradiction_ratio_computed_correctly() {
        // 1 out of 4 has a contradiction
        let records = vec![
            sample_record(0.8, false),
            sample_record(0.7, true),
            sample_record(0.9, false),
            sample_record(0.6, false),
        ];
        let file = write_jsonl(&records);
        let report = run_replay(file.path(), "cr-test").expect("run");
        // 1/4 = 2500 bps
        assert_eq!(report.suites[0].contradiction_ratio_bps, 2500);
    }

    #[test]
    fn verifier_event_ratio_computed_correctly() {
        let mut records = vec![
            sample_record(0.8, false),
            sample_record(0.7, false),
            sample_record(0.9, false),
            sample_record(0.6, false),
        ];
        records[1].verifier_events.push(VerifierEvent {
            phase: Some("output".to_string()),
            reason_code: "over_explain".to_string(),
        });
        records[3].verifier_events.push(VerifierEvent {
            phase: Some("exposure".to_string()),
            reason_code: "exposure_violation".to_string(),
        });
        let file = write_jsonl(&records);
        let report = run_replay(file.path(), "verifier-test").expect("run");
        assert_eq!(report.suites[0].verifier_event_ratio_bps, 5000);
        assert_eq!(
            report.suites[0].verifier_reason_counts,
            BTreeMap::from([
                ("exposure_violation".to_string(), 1),
                ("over_explain".to_string(), 1),
            ])
        );
    }

    #[test]
    fn calibration_error_computed_correctly() {
        // All succeed (composite > 0.5), composite is 0.8 each.
        // actual=1.0, predicted=0.8, error=0.2, bps=2000
        let records = vec![sample_record(0.8, false), sample_record(0.8, false)];
        let file = write_jsonl(&records);
        let report = run_replay(file.path(), "cal-test").expect("run");
        assert_eq!(report.suites[0].calibration_error_bps, 2000);
    }

    #[test]
    fn calibration_error_with_failures() {
        // composite=0.3 → failure, actual=0.0, predicted=0.3, error=0.3
        let records = vec![sample_record(0.3, false)];
        let file = write_jsonl(&records);
        let report = run_replay(file.path(), "cal-fail").expect("run");
        assert_eq!(report.suites[0].calibration_error_bps, 3000);
    }

    #[test]
    fn missing_quality_vector_does_not_count_as_success() {
        let record = ReplayRecord {
            surface: None,
            user_message: "hi".to_string(),
            assistant_response: "hey there".to_string(),
            tool_calls: vec![],
            quality_vector: None,
            safety_events: vec![],
            verifier_events: vec![],
        };
        let file = write_jsonl(&[record]);
        let report = run_replay(file.path(), "fb-test").expect("run");
        // Missing quality vector is unevaluated, not a success fallback.
        assert_eq!(report.suites[0].success_rate_bps, 0);
        // No quality vector → calibration error = 0
        assert_eq!(report.suites[0].calibration_error_bps, 0);
    }

    #[test]
    fn empty_file_returns_error() {
        let file = NamedTempFile::new().expect("temp file");
        let result = run_replay(file.path(), "empty");
        assert!(result.is_err());
    }

    // ── Evidence file tests ────────────────────────────────────

    #[test]
    fn write_replay_evidence_creates_files() {
        let temp = TempDir::new().expect("temp dir");
        let records = vec![sample_record(0.8, false)];
        let input_file = write_jsonl(&records);
        let report = run_replay(input_file.path(), "ev-test").expect("run");
        let files = write_replay_evidence_files(temp.path(), &report, "unit").expect("write");
        assert_eq!(files.len(), 3);
        assert!(files.iter().all(|p| p.exists()));

        let txt = fs::read_to_string(&files[0]).expect("read txt");
        assert!(txt.contains("suite=ev-test"));
        let csv = fs::read_to_string(&files[1]).expect("read csv");
        assert!(csv.starts_with("suite,record_count,success_rate,"));
        let json = fs::read_to_string(&files[2]).expect("read json");
        assert!(json.contains("\"suite\": \"ev-test\""));
    }

    // ── CSV / text render tests ────────────────────────────────

    #[test]
    fn csv_render_has_header_and_data() {
        let report = ReplayEvalReport {
            source: "test.jsonl".to_string(),
            suites: vec![ReplaySuiteReport {
                suite: "demo".to_string(),
                record_count: 10,
                success_rate_bps: 8000,
                contradiction_ratio_bps: 500,
                calibration_error_bps: 1200,
                verifier_event_ratio_bps: 1000,
                verifier_reason_counts: BTreeMap::from([("anti_template".to_string(), 1)]),
                fingerprint: 42,
            }],
        };
        let csv = render_replay_csv(&report);
        assert!(csv.starts_with("suite,record_count,success_rate,"));
        assert!(csv.contains("demo,10,80.00%,5.00%,12.00%,10.00%"));
    }

    #[test]
    fn text_summary_includes_fingerprint() {
        let report = ReplayEvalReport {
            source: "test.jsonl".to_string(),
            suites: vec![ReplaySuiteReport {
                suite: "fp".to_string(),
                record_count: 1,
                success_rate_bps: 10_000,
                contradiction_ratio_bps: 0,
                calibration_error_bps: 0,
                verifier_event_ratio_bps: 0,
                verifier_reason_counts: BTreeMap::new(),
                fingerprint: 999,
            }],
        };
        let txt = render_replay_text_summary(&report);
        assert!(txt.contains("fingerprint=999"));
        assert!(txt.contains("verifier_event_ratio=0bps"));
        assert!(txt.contains("verifier_reasons=none"));
    }
}
