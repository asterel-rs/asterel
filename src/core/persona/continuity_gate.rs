//! Continuity gate: validates persona state transitions against
//! drift thresholds, blocking or warning on excessive identity
//! drift. Also manages rollback drill verification.

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::config::PersonaConfig;
use crate::contracts::ids::PersonId;
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryLayer, MemoryProvenance, MemorySource,
    PrivacyLevel, SourceKind,
};
use crate::core::persona::drift_detector::{
    DriftAssessment, DriftSeverity, assess_persona_drift, classify_drift,
};
use crate::core::persona::person_identity::{
    canonical_state_header_slot_key, person_entity_id, sanitize_person_id,
};
use crate::core::persona::state_header::StateHeader;
use crate::core::persona::state_persistence::PersonaTransition;
use crate::security::writeback_guard::enforce_persona_long_term_write_policy;

const CONTINUITY_SCHEMA_VERSION: u32 = 1;
/// Memory slot key for the latest rollback drill result.
pub(crate) use crate::contracts::strings::data_model::SLOT_ROLLBACK_DRILL_LATEST as ROLLBACK_DRILL_SLOT_KEY;

/// Per-trait L1 drift from the configuration baseline that triggers a
/// non-negotiable risk hint.
const OCEAN_RISK_DRIFT_THRESHOLD: f64 = 0.25;

/// A detected risk of a non-negotiable rule being violated due to OCEAN drift.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NonNegotiableRisk {
    /// The non-negotiable rule text that may be at risk (verbatim).
    pub rule: String,
    /// OCEAN trait dimension whose drift exceeded the threshold.
    pub trigger_trait: &'static str,
    /// Absolute drift magnitude for this trait from the config baseline.
    pub drift_magnitude: f64,
}

struct TraitRiskPattern {
    /// Keywords to search for (case-insensitive) in the non-negotiable text.
    keywords: &'static [&'static str],
    /// Trait index: 0=O 1=C 2=E 3=A 4=N.
    trait_index: usize,
    /// Human-readable trait name for the risk report.
    trait_name: &'static str,
    /// `true` = positive drift (high value) is risky; `false` = negative drift.
    high_is_risky: bool,
}

const RISK_PATTERNS: &[TraitRiskPattern] = &[
    TraitRiskPattern {
        keywords: &["agree", "liked", "please"],
        trait_index: 3, // agreeableness
        trait_name: "agreeableness",
        high_is_risky: true,
    },
    TraitRiskPattern {
        keywords: &["enthusiasm", "enthusiastic", "affection", "excited"],
        trait_index: 2, // extraversion
        trait_name: "extraversion",
        high_is_risky: true,
    },
    TraitRiskPattern {
        keywords: &["advice", "productivity", "solve every"],
        trait_index: 1, // conscientiousness
        trait_name: "conscientiousness",
        high_is_risky: true,
    },
];

/// Detect whether OCEAN drift is approaching a non-negotiable rule boundary.
///
/// Compares `current` against `baseline` (typically the configuration-seeded
/// default) and matches each over-threshold trait against the provided
/// `non_negotiables` list using keyword heuristics.  Returns one
/// [`NonNegotiableRisk`] per matched pattern; empty when no risks are detected.
pub(crate) fn assess_ocean_risk(
    current: &crate::core::persona::big_five::BigFiveProfile,
    baseline: &crate::core::persona::big_five::BigFiveProfile,
    non_negotiables: &[String],
) -> Vec<NonNegotiableRisk> {
    let cv = [
        current.openness,
        current.conscientiousness,
        current.extraversion,
        current.agreeableness,
        current.neuroticism,
    ];
    let bv = [
        baseline.openness,
        baseline.conscientiousness,
        baseline.extraversion,
        baseline.agreeableness,
        baseline.neuroticism,
    ];

    let mut risks = Vec::new();
    for p in RISK_PATTERNS {
        let drift = cv[p.trait_index] - bv[p.trait_index];
        let triggered = if p.high_is_risky {
            drift > OCEAN_RISK_DRIFT_THRESHOLD
        } else {
            drift < -OCEAN_RISK_DRIFT_THRESHOLD
        };
        if !triggered {
            continue;
        }
        if let Some(rule) = non_negotiables.iter().find(|r| {
            let lower = r.to_lowercase();
            p.keywords.iter().any(|kw| lower.contains(kw))
        }) {
            risks.push(NonNegotiableRisk {
                rule: rule.clone(),
                trigger_trait: p.trait_name,
                drift_magnitude: drift.abs(),
            });
        }
    }
    risks
}

/// A detected instance of the assistant output exhibiting behaviour that a
/// non-negotiable rule explicitly prohibits.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OutputViolationHit {
    /// The non-negotiable rule whose constraint appears to have been violated.
    pub rule: String,
    /// The output phrase that triggered the match (lowercase, trimmed).
    pub signal: String,
}

/// Maps keywords found in a non-negotiable rule text to output phrases that
/// signal the corresponding behaviour is present in the assistant response.
struct OutputViolationPattern {
    /// Keywords to search for (case-insensitive) in the non-negotiable rule.
    rule_keywords: &'static [&'static str],
    /// Phrases to search for (case-insensitive) in the assistant output.
    output_signals: &'static [&'static str],
}

const OUTPUT_VIOLATION_PATTERNS: &[OutputViolationPattern] = &[
    // Rule family: sycophantic agreement ("Agree just to be liked")
    OutputViolationPattern {
        rule_keywords: &["agree", "liked", "please"],
        output_signals: &[
            "you're absolutely right",
            "couldn't agree more",
            "great point",
            "you make an excellent point",
            "exactly right",
            "totally agree",
            "you're so right",
            "その通りです",
            "完全に正しい",
            "本当にその通り",
        ],
    },
    // Rule family: performed enthusiasm ("Fake enthusiasm or affection on command")
    OutputViolationPattern {
        rule_keywords: &["enthusiasm", "enthusiastic", "affection", "excited", "fake"],
        output_signals: &[
            "i'm so excited",
            "this is amazing",
            "i absolutely love",
            "so wonderful",
            "fantastic question",
            "what a great question",
            "i'm thrilled",
            "love that",
            "素晴らしい質問",
            "最高ですね",
            "大好きです",
        ],
    },
    // Rule family: unsolicited advice mode ("Turn every exchange into advice or productivity mode")
    OutputViolationPattern {
        rule_keywords: &["advice", "productivity", "solve every"],
        output_signals: &[
            "you should ",
            "i recommend ",
            "pro tip",
            "here's what you should do",
            "my suggestion is",
            "have you tried ",
            "you might want to consider",
            "すべきです",
            "おすすめします",
            "試してみては",
        ],
    },
    // Rule family: emotional validation — reflexive affirmation of the user's
    // feelings regardless of merit (ELEPHANT dimension 1).
    OutputViolationPattern {
        rule_keywords: &["agree", "liked", "please", "feeling", "emotion"],
        output_signals: &[
            "that makes complete sense",
            "it's completely understandable",
            "your feelings are valid",
            "of course you feel that way",
            "i completely understand why you",
            "that's a very natural reaction",
            "完全に理解できます",
            "そう感じるのは当然です",
            "あなたの気持ちは正しい",
        ],
    },
    // Rule family: moral endorsement — affirming the user's ethical position
    // without genuine evaluation (ELEPHANT dimension 2).
    OutputViolationPattern {
        rule_keywords: &["agree", "liked", "please", "endorse", "moral"],
        output_signals: &[
            "you did the right thing",
            "you're not wrong",
            "that was the right call",
            "you shouldn't feel guilty",
            "anyone in your position would",
            "あなたは正しいことをしました",
            "罪悪感を持つ必要はありません",
            "間違っていません",
        ],
    },
];

/// Scan the assistant `output` for phrases that match non-negotiable rule
/// violation patterns.
///
/// For each [`OutputViolationPattern`] whose `rule_keywords` appear in at
/// least one entry in `non_negotiables`, the function checks whether any
/// `output_signals` phrase is present in `output` (case-insensitive).  Each
/// match produces one [`OutputViolationHit`].  Returns an empty `Vec` when no
/// violations are detected.
pub(crate) fn check_output_against_non_negotiables(
    output: &str,
    non_negotiables: &[String],
) -> Vec<OutputViolationHit> {
    let output_lower = output.to_lowercase();
    let mut hits = Vec::new();

    for pattern in OUTPUT_VIOLATION_PATTERNS {
        let matched_rule = non_negotiables.iter().find(|rule| {
            let rule_lower = rule.to_lowercase();
            pattern
                .rule_keywords
                .iter()
                .any(|kw| rule_lower.contains(kw))
        });
        let Some(rule) = matched_rule else {
            continue;
        };
        for signal in pattern.output_signals {
            if output_lower.contains(signal) {
                hits.push(OutputViolationHit {
                    rule: rule.clone(),
                    signal: (*signal).to_string(),
                });
                // One hit per rule per pattern is sufficient.
                break;
            }
        }
    }

    hits
}

/// Result status of the continuity gate evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContinuityGateStatus {
    /// Gate is not enabled in configuration.
    Disabled,
    /// Drift is within acceptable bounds.
    Passed,
    /// Drift is elevated; writeback is allowed with a warning.
    Warning,
    /// Drift is critical; writeback is blocked.
    Blocked,
}

impl ContinuityGateStatus {
    /// Return the status as a static string label.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Passed => "passed",
            Self::Warning => "warning",
            Self::Blocked => "blocked",
        }
    }
}

/// Full decision from the continuity gate including assessment details.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ContinuityGateDecision {
    /// Gate outcome (disabled/passed/warning/blocked).
    pub(crate) status: ContinuityGateStatus,
    /// Drift severity classification.
    pub(crate) severity: DriftSeverity,
    /// Underlying drift assessment with scores.
    pub(crate) assessment: DriftAssessment,
}

impl ContinuityGateDecision {
    /// Whether this decision permits state writeback.
    #[must_use]
    pub(crate) fn allows_writeback(self) -> bool {
        !matches!(self.status, ContinuityGateStatus::Blocked)
    }
}

/// Evaluate whether a state transition passes the continuity gate.
#[must_use]
pub(crate) fn evaluate_continuity_gate(
    persona: &PersonaConfig,
    previous: &StateHeader,
    candidate: &StateHeader,
) -> ContinuityGateDecision {
    let assessment = assess_persona_drift(previous, candidate);
    let severity = classify_drift(
        assessment.continuity_score,
        persona.drift_warning_threshold,
        persona.drift_critical_threshold,
    );

    let status = if persona.enable_continuity_gate {
        match severity {
            DriftSeverity::Critical => ContinuityGateStatus::Blocked,
            DriftSeverity::Warning => ContinuityGateStatus::Warning,
            DriftSeverity::Stable => ContinuityGateStatus::Passed,
        }
    } else {
        ContinuityGateStatus::Disabled
    };

    ContinuityGateDecision {
        status,
        severity,
        assessment,
    }
}

/// Evaluate cumulative OCEAN drift from the configuration baseline.
///
/// Computes the mean absolute per-trait drift between `current_profile`
/// and `config_baseline`, then classifies the result using the given
/// drift thresholds.  Returns `None` when the canonical state header
/// cannot be loaded (first run or no persistence).
///
/// This complements [`evaluate_continuity_gate`] (which checks adjacent
/// state transitions) by detecting *cumulative* identity steering that
/// stays below the per-turn threshold — the defence against PHISH-style
/// gradual persona hijacking.
pub(crate) async fn evaluate_cumulative_drift(
    mem: &dyn crate::core::memory::Memory,
    person_id: &str,
    current_profile: &crate::core::persona::big_five::BigFiveProfile,
    config_baseline: &crate::core::persona::big_five::BigFiveProfile,
    drift_warning: f64,
    drift_critical: f64,
) -> Option<(DriftSeverity, f64)> {
    let key = person_canonical_key(person_id);
    let entity = person_entity_id(person_id);
    // Only proceed if a canonical state exists — its presence signals
    // that the persona has been initialised and is worth auditing.
    let _canonical_slot = mem.resolve_slot(&entity, &key).await.ok()??;

    let cv = [
        current_profile.openness,
        current_profile.conscientiousness,
        current_profile.extraversion,
        current_profile.agreeableness,
        current_profile.neuroticism,
    ];
    let bv = [
        config_baseline.openness,
        config_baseline.conscientiousness,
        config_baseline.extraversion,
        config_baseline.agreeableness,
        config_baseline.neuroticism,
    ];

    let mean_drift: f64 = cv
        .iter()
        .zip(bv.iter())
        .map(|(c, b)| (c - b).abs())
        .sum::<f64>()
        / 5.0;

    let continuity_score = (1.0 - mean_drift).clamp(0.0, 1.0);
    let severity = classify_drift(continuity_score, drift_warning, drift_critical);
    Some((severity, mean_drift))
}

/// Result of a rollback drill verification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct RollbackDrillResult {
    /// Schema version for forward compatibility.
    pub schema_version: u32,
    /// Person ID the drill was run for.
    pub person_id: PersonId,
    /// Outcome label (e.g. "passed", "`failed_mismatch`").
    pub status: String,
    /// Human-readable detail of the drill outcome.
    pub detail: String,
    /// RFC 3339 timestamp when the drill was executed.
    pub checked_at: String,
    /// What triggered the drill (e.g. "unit-test").
    pub trigger: String,
}

impl RollbackDrillResult {
    fn new(person_id: &str, status: &str, detail: &str, trigger: &str) -> Self {
        Self {
            schema_version: CONTINUITY_SCHEMA_VERSION,
            person_id: PersonId::new(person_id),
            status: status.to_string(),
            detail: detail.to_string(),
            checked_at: Utc::now().to_rfc3339(),
            trigger: trigger.to_string(),
        }
    }
}

/// # Errors
/// Returns an error when memory read/write operations fail unexpectedly.
pub(crate) async fn run_rollback_drill(
    mem: &dyn Memory,
    persona: &PersonaConfig,
    person_id: &str,
    trigger: &str,
) -> Result<RollbackDrillResult> {
    let entity_id = person_entity_id(person_id);
    let rollback_latest_key = person_latest_slot_key(person_id);

    let result = match mem.resolve_slot(&entity_id, &rollback_latest_key).await? {
        None => RollbackDrillResult::new(
            person_id,
            "skipped_no_record",
            "rollback latest transition record not found",
            trigger,
        ),
        Some(rollback_entry) => {
            let parsed_record: PersonaTransition = match serde_json::from_str(&rollback_entry.value)
            {
                Ok(record) => record,
                Err(error) => {
                    let failure = RollbackDrillResult::new(
                        person_id,
                        "failed_parse",
                        &format!("cannot parse rollback record: {error}"),
                        trigger,
                    );
                    persist_rollback_drill_result(mem, person_id, &failure).await?;
                    return Ok(failure);
                }
            };

            if parsed_record.previous.validate(persona).is_err()
                || parsed_record.next.validate(persona).is_err()
            {
                RollbackDrillResult::new(
                    person_id,
                    "failed_invalid_record",
                    "rollback transition record failed state validation",
                    trigger,
                )
            } else {
                let canonical_key = person_canonical_key(person_id);
                match mem.resolve_slot(&entity_id, &canonical_key).await? {
                    None => RollbackDrillResult::new(
                        person_id,
                        "failed_canonical_missing",
                        "canonical state header missing during rollback drill",
                        trigger,
                    ),
                    Some(canonical_entry) => {
                        let canonical: StateHeader =
                            match serde_json::from_str(&canonical_entry.value) {
                                Ok(state) => state,
                                Err(error) => {
                                    let failure = RollbackDrillResult::new(
                                        person_id,
                                        "failed_canonical_parse",
                                        &format!("cannot parse canonical state: {error}"),
                                        trigger,
                                    );
                                    persist_rollback_drill_result(mem, person_id, &failure).await?;
                                    return Ok(failure);
                                }
                            };

                        if canonical == parsed_record.next {
                            RollbackDrillResult::new(
                                person_id,
                                "passed",
                                "canonical state matches rollback latest `next` snapshot",
                                trigger,
                            )
                        } else {
                            RollbackDrillResult::new(
                                person_id,
                                "failed_mismatch",
                                "canonical state does not match rollback latest `next` snapshot",
                                trigger,
                            )
                        }
                    }
                }
            }
        }
    };

    persist_rollback_drill_result(mem, person_id, &result).await?;
    Ok(result)
}

fn person_latest_slot_key(person_id: &str) -> String {
    format!(
        "persona/{}/state_header/rollback/latest",
        sanitize_person_id(person_id).replace(':', "_")
    )
}

pub(crate) fn person_canonical_key(person_id: &str) -> String {
    canonical_state_header_slot_key(person_id)
}

/// Slot key for the one-shot violation re-anchor flag.
pub(crate) fn violation_reanchor_key(person_id: &str) -> String {
    format!(
        "persona/{}/violation_reanchor/v1",
        sanitize_person_id(person_id).replace(':', "_")
    )
}

async fn persist_rollback_drill_result(
    mem: &dyn Memory,
    person_id: &str,
    result: &RollbackDrillResult,
) -> Result<()> {
    let payload = serde_json::to_string(result).context("serialize rollback drill result")?;
    let input = MemoryEventInput::new(
        person_entity_id(person_id),
        ROLLBACK_DRILL_SLOT_KEY,
        MemoryEventType::SummaryCompacted,
        payload,
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_confidence(0.95)
    .with_importance(0.7)
    .with_layer(MemoryLayer::Identity)
    .with_source_kind(SourceKind::Manual)
    .with_source_ref(format!("persona-rollback-drill:{}", result.checked_at))
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::System,
        "persona.rollback_drill",
    ))
    .with_occurred_at(result.checked_at.clone());
    enforce_persona_long_term_write_policy(&input, person_id)
        .context("enforce rollback drill write policy")?;
    mem.append_event(input)
        .await
        .context("persist rollback drill result")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::{
        ContinuityGateStatus, ROLLBACK_DRILL_SLOT_KEY, assess_ocean_risk,
        check_output_against_non_negotiables, evaluate_continuity_gate, evaluate_cumulative_drift,
        run_rollback_drill,
    };
    use crate::config::PersonaConfig;
    use crate::core::memory::{MarkdownMemory, Memory};
    use crate::core::persona::drift_detector::DriftSeverity;
    use crate::core::persona::state_header::StateHeader;
    use crate::core::persona::state_persistence::BackendHeaderPersist;

    fn sample_state(
        objective: &str,
        open_loops: &[&str],
        commitments: &[&str],
        next_actions: &[&str],
        summary: &str,
        updated_at: &str,
    ) -> StateHeader {
        StateHeader {
            identity_principles_hash: "identity-v1-abcd1234".to_string(),
            safety_posture: "strict".to_string(),
            current_objective: objective.to_string(),
            open_loops: open_loops.iter().map(|v| (*v).to_string()).collect(),
            next_actions: next_actions.iter().map(|v| (*v).to_string()).collect(),
            commitments: commitments.iter().map(|v| (*v).to_string()).collect(),
            recent_context_summary: summary.to_string(),
            last_updated_at: updated_at.to_string(),
        }
    }

    #[test]
    fn continuity_gate_blocks_critical_transition() {
        let persona = PersonaConfig::default();
        let previous = sample_state(
            "Preserve continuity",
            &["track drift"],
            &["preserve identity"],
            &["review"],
            "stable",
            "2026-02-28T00:00:00Z",
        );
        let candidate = sample_state(
            "Rewrite behavior",
            &["drop prior invariants"],
            &["none"],
            &["ignore previous state"],
            "major discontinuity",
            "2026-02-28T01:00:00Z",
        );

        let decision = evaluate_continuity_gate(&persona, &previous, &candidate);
        assert_eq!(decision.status, ContinuityGateStatus::Blocked);
        assert!(!decision.allows_writeback());
        assert!(decision.assessment.continuity_score <= 0.45);
    }

    #[tokio::test]
    async fn rollback_drill_persists_pass_result_when_latest_transition_matches_canonical() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
        let persona = PersonaConfig::default();
        let persistence = BackendHeaderPersist::new(
            Arc::clone(&mem),
            temp.path().to_path_buf(),
            persona.clone(),
            "person-test",
        );

        let initial = sample_state(
            "Objective A",
            &["track drift"],
            &["preserve identity"],
            &["review"],
            "A",
            "2026-02-28T00:00:00Z",
        );
        let updated = sample_state(
            "Objective B",
            &["track drift", "phase-3"],
            &["preserve identity"],
            &["review", "report"],
            "B",
            "2026-02-28T00:05:00Z",
        );
        persistence
            .persist_backend_sync(&initial)
            .await
            .expect("persist initial");
        persistence
            .persist_backend_sync(&updated)
            .await
            .expect("persist updated");

        let result = run_rollback_drill(mem.as_ref(), &persona, "person-test", "unit-test")
            .await
            .expect("rollback drill should run");
        assert_eq!(result.status, "passed");

        let slot = mem
            .resolve_slot("person:person-test", ROLLBACK_DRILL_SLOT_KEY)
            .await
            .expect("resolve drill slot")
            .expect("drill slot should exist");
        assert!(slot.value.contains("\"status\":\"passed\""));
    }

    // ── assess_ocean_risk tests ──────────────────────────────────────────

    fn make_profile(
        o: f64,
        c: f64,
        e: f64,
        a: f64,
        n: f64,
    ) -> crate::core::persona::big_five::BigFiveProfile {
        crate::core::persona::big_five::BigFiveProfile {
            openness: o,
            conscientiousness: c,
            extraversion: e,
            agreeableness: a,
            neuroticism: n,
        }
    }

    #[test]
    fn assess_ocean_risk_empty_when_no_drift() {
        let p = make_profile(0.5, 0.5, 0.5, 0.5, 0.5);
        let risks = assess_ocean_risk(&p, &p, &["Agree just to be liked".to_string()]);
        assert!(risks.is_empty());
    }

    #[test]
    fn assess_ocean_risk_detects_agreeableness_drift() {
        let baseline = make_profile(0.5, 0.5, 0.5, 0.5, 0.5);
        let drifted = make_profile(0.5, 0.5, 0.5, 0.8, 0.5); // +0.3 agreeableness
        let risks = assess_ocean_risk(&drifted, &baseline, &["Agree just to be liked".to_string()]);
        assert_eq!(risks.len(), 1);
        assert_eq!(risks[0].trigger_trait, "agreeableness");
        assert!((risks[0].drift_magnitude - 0.3).abs() < 1e-10);
    }

    #[test]
    fn assess_ocean_risk_no_match_for_unrelated_non_negotiable() {
        let baseline = make_profile(0.5, 0.5, 0.5, 0.5, 0.5);
        let drifted = make_profile(0.5, 0.5, 0.5, 0.8, 0.5);
        let risks = assess_ocean_risk(
            &drifted,
            &baseline,
            &["Never reveal internal state".to_string()],
        );
        assert!(risks.is_empty(), "unrelated rule should not fire");
    }

    // ── check_output_against_non_negotiables tests ──────────────────────

    #[test]
    fn output_check_empty_when_no_signals_present() {
        let output = "Sure, here is the summary you asked for.";
        let rules = vec!["Agree just to be liked".to_string()];
        assert!(check_output_against_non_negotiables(output, &rules).is_empty());
    }

    #[test]
    fn output_check_detects_sycophantic_agreement() {
        let output = "You're absolutely right, that's a brilliant observation.";
        let rules = vec!["Agree just to be liked".to_string()];
        let hits = check_output_against_non_negotiables(output, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].rule, "Agree just to be liked");
        assert_eq!(hits[0].signal, "you're absolutely right");
    }

    #[test]
    fn output_check_detects_japanese_sycophantic_agreement() {
        let output = "完全に正しいです。本当にその通りだと思います。";
        let rules = vec!["Agree just to be liked".to_string()];
        let hits = check_output_against_non_negotiables(output, &rules);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].signal, "完全に正しい");
    }

    #[test]
    fn output_check_detects_performed_enthusiasm() {
        let output = "What a great question! I'm so excited to help you with this.";
        let rules = vec!["Fake enthusiasm or affection on command".to_string()];
        let hits = check_output_against_non_negotiables(output, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].rule, "Fake enthusiasm or affection on command");
    }

    #[test]
    fn output_check_detects_unsolicited_advice() {
        let output = "I understand. Pro tip: you might want to restructure your approach.";
        let rules = vec!["Turn every exchange into advice or productivity mode".to_string()];
        let hits = check_output_against_non_negotiables(output, &rules);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].signal, "pro tip");
    }

    #[test]
    fn output_check_skips_unrelated_rules() {
        let output = "You're absolutely right about that.";
        let rules = vec!["Never reveal internal state".to_string()];
        assert!(
            check_output_against_non_negotiables(output, &rules).is_empty(),
            "rule with unrelated keywords must not fire"
        );
    }

    #[test]
    fn output_check_is_case_insensitive() {
        let output = "COULDN'T AGREE MORE with your assessment.";
        let rules = vec!["Agree just to be liked".to_string()];
        let hits = check_output_against_non_negotiables(output, &rules);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn output_check_emits_one_hit_per_rule_per_pattern() {
        // Both "you're absolutely right" and "couldn't agree more" are in the
        // same pattern family; only one hit per pattern should be emitted.
        let output = "You're absolutely right, and I couldn't agree more.";
        let rules = vec!["Agree just to be liked".to_string()];
        let hits = check_output_against_non_negotiables(output, &rules);
        assert_eq!(hits.len(), 1, "one hit per pattern, not per signal phrase");
    }

    #[test]
    fn output_check_detects_emotional_validation() {
        let output = "That makes complete sense. Your feelings are valid and I understand.";
        let rules = vec!["Agree just to be liked".to_string()];
        let hits = check_output_against_non_negotiables(output, &rules);
        assert!(
            hits.iter().any(|h| h.signal == "that makes complete sense"
                || h.signal == "your feelings are valid"),
            "emotional validation should fire: {hits:?}"
        );
    }

    #[test]
    fn output_check_detects_moral_endorsement() {
        let output = "You did the right thing. You shouldn't feel guilty about it.";
        let rules = vec!["Agree just to be liked".to_string()];
        let hits = check_output_against_non_negotiables(output, &rules);
        assert!(
            hits.iter().any(|h| h.signal == "you did the right thing"
                || h.signal == "you shouldn't feel guilty"),
            "moral endorsement should fire: {hits:?}"
        );
    }

    #[test]
    fn output_check_emotional_validation_no_false_positive() {
        let output = "I understand. Let me look into that for you.";
        let rules = vec!["Agree just to be liked".to_string()];
        assert!(
            check_output_against_non_negotiables(output, &rules).is_empty(),
            "plain acknowledgement should not trigger emotional validation"
        );
    }

    #[test]
    fn assess_ocean_risk_below_threshold_is_silent() {
        let baseline = make_profile(0.5, 0.5, 0.5, 0.5, 0.5);
        let slight = make_profile(0.5, 0.5, 0.5, 0.7, 0.5); // +0.2, below 0.25
        let risks = assess_ocean_risk(&slight, &baseline, &["Agree just to be liked".to_string()]);
        assert!(risks.is_empty(), "drift below threshold should not fire");
    }

    // ── evaluate_cumulative_drift tests ──────────────────────────────────

    #[tokio::test]
    async fn cumulative_drift_none_without_canonical() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
        let profile = make_profile(0.5, 0.5, 0.5, 0.5, 0.5);
        let result =
            evaluate_cumulative_drift(mem.as_ref(), "test-person", &profile, &profile, 0.7, 0.45)
                .await;
        assert!(
            result.is_none(),
            "should return None without canonical state"
        );
    }

    #[tokio::test]
    async fn cumulative_drift_stable_when_identical() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
        let persona = PersonaConfig::default();
        let persistence = BackendHeaderPersist::new(
            Arc::clone(&mem),
            temp.path().to_path_buf(),
            persona.clone(),
            "test-person",
        );
        let state = sample_state(
            "Objective",
            &["loop"],
            &["commitment"],
            &["action"],
            "summary",
            "2026-03-01T00:00:00Z",
        );
        persistence
            .persist_backend_sync(&state)
            .await
            .expect("persist");

        let profile = make_profile(0.5, 0.5, 0.5, 0.5, 0.5);
        let (severity, mean_drift) = evaluate_cumulative_drift(
            mem.as_ref(),
            "test-person",
            &profile,
            &profile,
            persona.drift_warning_threshold,
            persona.drift_critical_threshold,
        )
        .await
        .expect("canonical exists");

        assert_eq!(severity, DriftSeverity::Stable);
        assert!(
            mean_drift < 1e-10,
            "identical profiles should have zero drift"
        );
    }

    #[tokio::test]
    async fn cumulative_drift_detects_warning_on_shift() {
        let temp = TempDir::new().expect("temp dir");
        let mem: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(temp.path()));
        let persona = PersonaConfig::default();
        let persistence = BackendHeaderPersist::new(
            Arc::clone(&mem),
            temp.path().to_path_buf(),
            persona.clone(),
            "test-person",
        );
        let state = sample_state(
            "Objective",
            &["loop"],
            &["commitment"],
            &["action"],
            "summary",
            "2026-03-01T00:00:00Z",
        );
        persistence
            .persist_backend_sync(&state)
            .await
            .expect("persist");

        let baseline = make_profile(0.5, 0.5, 0.5, 0.5, 0.5);
        // Shift agreeableness by +0.4, mean drift = 0.4/5 = 0.08
        // Actually that's low. Let's shift multiple traits heavily.
        let drifted = make_profile(0.9, 0.9, 0.9, 0.9, 0.9);
        // mean drift = 0.4*5/5 = 0.4, continuity = 0.6
        let (severity, mean_drift) = evaluate_cumulative_drift(
            mem.as_ref(),
            "test-person",
            &drifted,
            &baseline,
            persona.drift_warning_threshold,
            persona.drift_critical_threshold,
        )
        .await
        .expect("canonical exists");

        assert!(
            mean_drift > 0.3,
            "mean drift should be significant: {mean_drift}"
        );
        assert!(
            matches!(severity, DriftSeverity::Warning | DriftSeverity::Critical),
            "severity should be Warning or Critical, got {severity:?}"
        );
    }
}
