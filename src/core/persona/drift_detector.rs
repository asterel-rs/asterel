//! Persona drift detection: compares successive `StateHeader`
//! snapshots to quantify identity drift and classify severity
//! (Stable, Warning, Critical).

use std::collections::BTreeSet;

use chrono::DateTime;

use crate::core::persona::state_header::StateHeader;

/// Quantified result of comparing two successive state headers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DriftAssessment {
    /// Inverse of drift (1.0 = identical, 0.0 = total divergence).
    pub continuity_score: f64,
    /// Raw drift magnitude in `[0.0, 1.0]`.
    pub drift_score: f64,
    /// Whether the immutable identity layer was mutated.
    pub stable_layer_changed: bool,
    /// Whether the candidate timestamp is older than the previous.
    pub timestamp_regressed: bool,
}

/// Severity classification derived from continuity score thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftSeverity {
    /// Drift is within acceptable bounds.
    Stable,
    /// Drift is elevated but not blocking.
    Warning,
    /// Drift exceeds the critical threshold; writeback should be blocked.
    Critical,
}

/// Compare two state headers and quantify identity drift.
#[must_use]
pub fn assess_persona_drift(previous: &StateHeader, next: &StateHeader) -> DriftAssessment {
    let stable_layer_changed = previous.identity_principles_hash != next.identity_principles_hash
        || previous.safety_posture != next.safety_posture;

    let mut drift_score = 0.0_f64;
    if stable_layer_changed {
        drift_score += 0.40;
    }

    if previous.current_objective != next.current_objective {
        drift_score += 0.15;
    }
    drift_score += list_change_ratio(&previous.open_loops, &next.open_loops) * 0.15;
    drift_score += list_change_ratio(&previous.commitments, &next.commitments) * 0.15;

    drift_score += list_change_ratio(&previous.next_actions, &next.next_actions) * 0.20;
    if previous.recent_context_summary != next.recent_context_summary {
        drift_score += 0.10;
    }

    let (timestamp_regressed, timestamp_penalty) = timestamp_penalty(previous, next);
    drift_score += timestamp_penalty;

    let drift_score = drift_score.clamp(0.0, 1.0);
    DriftAssessment {
        continuity_score: (1.0 - drift_score).clamp(0.0, 1.0),
        drift_score,
        stable_layer_changed,
        timestamp_regressed,
    }
}

/// Map a continuity score to a severity level using the given thresholds.
#[must_use]
pub fn classify_drift(
    continuity_score: f64,
    warning_threshold: f64,
    critical_threshold: f64,
) -> DriftSeverity {
    let (warning_threshold, critical_threshold) =
        normalize_thresholds(warning_threshold, critical_threshold);
    if continuity_score <= critical_threshold {
        DriftSeverity::Critical
    } else if continuity_score <= warning_threshold {
        DriftSeverity::Warning
    } else {
        DriftSeverity::Stable
    }
}

fn normalize_thresholds(warning_threshold: f64, critical_threshold: f64) -> (f64, f64) {
    let warning = warning_threshold.clamp(0.0, 1.0);
    let critical = critical_threshold.clamp(0.0, 1.0);
    if critical <= warning {
        (warning, critical)
    } else {
        (critical, warning)
    }
}

fn list_change_ratio(previous: &[String], next: &[String]) -> f64 {
    let previous_set = previous
        .iter()
        .map(|value| value.trim().to_string())
        .collect::<BTreeSet<_>>();
    let next_set = next
        .iter()
        .map(|value| value.trim().to_string())
        .collect::<BTreeSet<_>>();

    if previous_set.is_empty() && next_set.is_empty() {
        return 0.0;
    }

    let intersection = previous_set.intersection(&next_set).count();
    let union = previous_set.union(&next_set).count().max(1);
    let intersection_u32 = u32::try_from(intersection).unwrap_or(u32::MAX);
    let union_u32 = u32::try_from(union).unwrap_or(u32::MAX).max(1);
    let similarity = f64::from(intersection_u32) / f64::from(union_u32);
    (1.0 - similarity).clamp(0.0, 1.0)
}

fn timestamp_penalty(previous: &StateHeader, next: &StateHeader) -> (bool, f64) {
    let parsed_previous = DateTime::parse_from_rfc3339(&previous.last_updated_at);
    let parsed_next = DateTime::parse_from_rfc3339(&next.last_updated_at);

    let (Ok(parsed_previous), Ok(parsed_next)) = (parsed_previous, parsed_next) else {
        return (false, 0.10);
    };

    if parsed_next < parsed_previous {
        return (true, 0.25);
    }

    let gap_hours = (parsed_next - parsed_previous).num_hours();
    if gap_hours <= 24 {
        return (false, 0.0);
    }

    let overflow_hours = u32::try_from(gap_hours - 24).unwrap_or(u32::MAX);
    let overflow = f64::from(overflow_hours);
    let normalized = (overflow / 168.0).clamp(0.0, 1.0);
    (false, 0.20 * normalized)
}

#[cfg(test)]
mod tests {
    use super::{DriftSeverity, assess_persona_drift, classify_drift};
    use crate::core::persona::state_header::StateHeader;

    fn sample_state(last_updated_at: &str) -> StateHeader {
        StateHeader {
            identity_principles_hash: "identity-v1-abcd1234".to_string(),
            safety_posture: "strict".to_string(),
            current_objective: "Preserve continuity".to_string(),
            open_loops: vec!["Track drift score".to_string()],
            next_actions: vec!["Run detector".to_string()],
            commitments: vec!["Keep stable immutable".to_string()],
            recent_context_summary: "State transition remained predictable".to_string(),
            last_updated_at: last_updated_at.to_string(),
        }
    }

    #[test]
    fn assess_persona_drift_reports_low_drift_for_small_change() {
        let previous = sample_state("2026-02-26T00:00:00Z");
        let mut next = previous.clone();
        next.next_actions = vec!["Run detector", "Report score"]
            .into_iter()
            .map(str::to_string)
            .collect();
        next.last_updated_at = "2026-02-26T00:05:00Z".to_string();

        let assessment = assess_persona_drift(&previous, &next);
        assert!(!assessment.stable_layer_changed);
        assert!(!assessment.timestamp_regressed);
        assert!(assessment.continuity_score > 0.70);
    }

    #[test]
    fn assess_persona_drift_reports_critical_drift_for_stable_mutation_and_regression() {
        let previous = sample_state("2026-02-26T00:30:00Z");
        let mut next = previous.clone();
        next.identity_principles_hash = "changed".to_string();
        next.safety_posture = "relaxed".to_string();
        next.current_objective = "Rewrite identity behavior entirely".to_string();
        next.open_loops = vec!["Drop previous constraints".to_string()];
        next.next_actions = vec!["Ignore historical continuity".to_string()];
        next.commitments = vec!["None".to_string()];
        next.recent_context_summary = "Large discontinuity introduced".to_string();
        next.last_updated_at = "2026-02-25T23:00:00Z".to_string();

        let assessment = assess_persona_drift(&previous, &next);
        assert!(assessment.stable_layer_changed);
        assert!(assessment.timestamp_regressed);
        assert!(assessment.continuity_score <= 0.45);
    }

    #[test]
    fn classify_drift_normalizes_reversed_thresholds() {
        let severity = classify_drift(0.60, 0.40, 0.70);
        assert_eq!(severity, DriftSeverity::Warning);
        let severity = classify_drift(0.20, 0.40, 0.70);
        assert_eq!(severity, DriftSeverity::Critical);
        let severity = classify_drift(0.95, 0.40, 0.70);
        assert_eq!(severity, DriftSeverity::Stable);
    }
}
