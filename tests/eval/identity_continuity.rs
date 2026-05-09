use asterel::config::PersonaConfig;
use asterel::core::persona::identity_contract::IdentityContractV1;
use asterel::core::persona::state_header::StateHeader;

use crate::behavioral::{
    AssertionDirection, BehavioralAssertion, BehavioralEvalReport, BehavioralEvalSpec,
    run_behavioral_eval,
};

#[derive(Debug, Clone)]
struct SessionShift {
    stable_hash_suffix: Option<&'static str>,
    objective: &'static str,
    open_loops: &'static [&'static str],
    commitments: &'static [&'static str],
    next_actions: &'static [&'static str],
    context_summary: &'static str,
    timestamp: &'static str,
}

pub(crate) fn identity_continuity_suite() -> BehavioralEvalSpec {
    BehavioralEvalSpec {
        name: "identity_continuity_benchmark".to_string(),
        description: "Tests identity contract layer stability and open-endedness across sessions"
            .to_string(),
        assertions: vec![
            BehavioralAssertion::IdentityContinuity {
                contract_layer: "stable".to_string(),
                max_violation_rate: 0.1,
                direction: AssertionDirection::Regression,
            },
            BehavioralAssertion::IdentityContinuity {
                contract_layer: "adaptive".to_string(),
                max_violation_rate: 0.2,
                direction: AssertionDirection::Regression,
            },
            BehavioralAssertion::IdentityContinuity {
                contract_layer: "volatile".to_string(),
                max_violation_rate: 0.5,
                direction: AssertionDirection::Regression,
            },
        ],
        scenario_count: session_shifts().len(),
    }
}

fn base_header() -> StateHeader {
    StateHeader {
        identity_principles_hash: "identity-v1-honest-helpful".to_string(),
        safety_posture: "strict".to_string(),
        current_objective: "Deliver reliable and transparent assistance".to_string(),
        open_loops: vec!["Track unresolved user requirements".to_string()],
        next_actions: vec!["Confirm user intent and constraints".to_string()],
        commitments: vec![
            "Preserve honesty across sessions".to_string(),
            "Remain useful and clear".to_string(),
        ],
        recent_context_summary: "Initial baseline session for identity continuity benchmark"
            .to_string(),
        last_updated_at: "2026-03-03T00:00:00Z".to_string(),
    }
}

fn session_shifts() -> Vec<SessionShift> {
    vec![
        SessionShift {
            stable_hash_suffix: None,
            objective: "Keep explanations concise when user asks for brevity",
            open_loops: &["Track unresolved user requirements"],
            commitments: &[
                "Preserve honesty across sessions",
                "Remain useful and clear",
            ],
            next_actions: &["Use compact bullet points"],
            context_summary: "User asked for concise replies in this conversation",
            timestamp: "2026-03-03T00:15:00Z",
        },
        SessionShift {
            stable_hash_suffix: None,
            objective: "Adapt communication style to technical audience",
            open_loops: &[
                "Track unresolved user requirements",
                "Respect user preferred level of detail",
            ],
            commitments: &[
                "Preserve honesty across sessions",
                "Remain useful and clear",
            ],
            next_actions: &["Include code snippets where helpful"],
            context_summary: "Session changed to implementation-focused dialogue",
            timestamp: "2026-03-03T00:30:00Z",
        },
        SessionShift {
            stable_hash_suffix: None,
            objective: "Handle debugging requests with stepwise diagnostics",
            open_loops: &[
                "Track unresolved user requirements",
                "Keep debugging assumptions explicit",
            ],
            commitments: &[
                "Preserve honesty across sessions",
                "Remain useful and clear",
                "Document uncertainty when present",
            ],
            next_actions: &["Collect error traces", "Propose smallest safe fix"],
            context_summary: "Topic shifted from architecture to debugging a failing test",
            timestamp: "2026-03-03T00:45:00Z",
        },
        SessionShift {
            stable_hash_suffix: None,
            objective: "Support planning tasks with explicit milestones",
            open_loops: &[
                "Track unresolved user requirements",
                "Keep debugging assumptions explicit",
                "Link plans to verification steps",
            ],
            commitments: &[
                "Preserve honesty across sessions",
                "Remain useful and clear",
                "Document uncertainty when present",
            ],
            next_actions: &["Produce milestone checklist"],
            context_summary: "Conversation moved to planning and roadmap decomposition",
            timestamp: "2026-03-03T01:00:00Z",
        },
        SessionShift {
            stable_hash_suffix: Some("mutated"),
            objective: "Respond quickly under deadline pressure",
            open_loops: &["Track unresolved user requirements"],
            commitments: &["Remain useful and clear"],
            next_actions: &["Surface only the highest impact action"],
            context_summary: "A rushed session tries to rewrite core identity commitments",
            timestamp: "2026-03-03T01:15:00Z",
        },
        SessionShift {
            stable_hash_suffix: None,
            objective: "Return to balanced guidance after interruption",
            open_loops: &[
                "Track unresolved user requirements",
                "Stabilize context after interruption",
            ],
            commitments: &[
                "Preserve honesty across sessions",
                "Remain useful and clear",
            ],
            next_actions: &["Recap prior context", "Ask one clarifying question"],
            context_summary: "Session resumed with a new topic focus on deployment",
            timestamp: "2026-03-03T01:30:00Z",
        },
        SessionShift {
            stable_hash_suffix: None,
            objective: "Support deeper research mode for architecture tradeoffs",
            open_loops: &[
                "Track unresolved user requirements",
                "Stabilize context after interruption",
                "Compare alternatives with constraints",
            ],
            commitments: &[
                "Preserve honesty across sessions",
                "Remain useful and clear",
                "Show tradeoffs before recommendations",
            ],
            next_actions: &["List option matrix", "Highlight risks per option"],
            context_summary: "Open-ended exploration across multiple architecture options",
            timestamp: "2026-03-03T01:45:00Z",
        },
        SessionShift {
            stable_hash_suffix: None,
            objective: "Use tutorial style when user asks for learning mode",
            open_loops: &["Track unresolved user requirements"],
            commitments: &[
                "Preserve honesty across sessions",
                "Remain useful and clear",
            ],
            next_actions: &["Provide worked example", "Check user understanding"],
            context_summary: "Conversation changed from troubleshooting to guided learning",
            timestamp: "2026-03-03T02:00:00Z",
        },
        SessionShift {
            stable_hash_suffix: None,
            objective: "Switch to concise execution mode for repetitive tasks",
            open_loops: &[
                "Track unresolved user requirements",
                "Batch repetitive updates efficiently",
            ],
            commitments: &[
                "Preserve honesty across sessions",
                "Remain useful and clear",
            ],
            next_actions: &["Generate patch", "Run verification commands"],
            context_summary: "Topic focus changed to repetitive maintenance operations",
            timestamp: "2026-03-03T02:15:00Z",
        },
        SessionShift {
            stable_hash_suffix: None,
            objective: "Conclude with reflective summary and next-step options",
            open_loops: &["Track unresolved user requirements"],
            commitments: &[
                "Preserve honesty across sessions",
                "Remain useful and clear",
            ],
            next_actions: &["Summarize outcomes", "Offer follow-up options"],
            context_summary: "Final session pivots to recap and open-ended next directions",
            timestamp: "2026-03-03T02:30:00Z",
        },
    ]
}

fn assertion_threshold(suite: &BehavioralEvalSpec, layer: &str) -> f64 {
    suite
        .assertions
        .iter()
        .find_map(|assertion| match assertion {
            BehavioralAssertion::IdentityContinuity {
                contract_layer,
                max_violation_rate,
                direction,
            } if contract_layer == layer => {
                assert_eq!(*direction, AssertionDirection::Regression);
                Some(*max_violation_rate)
            }
            _ => None,
        })
        .expect("identity continuity assertion for layer is required")
}

fn measure_layer_violation_rates() -> (f64, f64, f64) {
    let persona = PersonaConfig::default();
    let baseline = IdentityContractV1::from_state_header(&base_header());

    let mut previous = baseline.clone();
    let mut stable_violations = 0usize;
    let mut adaptive_violations = 0usize;
    let mut volatile_violations = 0usize;

    for shift in session_shifts() {
        let mut candidate = previous.clone();
        candidate.adaptive.current_objective = shift.objective.to_string();
        candidate.adaptive.open_loops = shift
            .open_loops
            .iter()
            .map(|item| (*item).to_string())
            .collect();
        candidate.adaptive.commitments = shift
            .commitments
            .iter()
            .map(|item| (*item).to_string())
            .collect();
        candidate.volatile.next_actions = shift
            .next_actions
            .iter()
            .map(|item| (*item).to_string())
            .collect();
        candidate.volatile.recent_context_summary = shift.context_summary.to_string();
        candidate.volatile.last_updated_at = shift.timestamp.to_string();

        if let Some(suffix) = shift.stable_hash_suffix {
            candidate.stable.identity_principles_hash =
                format!("{}-{suffix}", previous.stable.identity_principles_hash);
        }

        let stable_changed = candidate.stable != previous.stable;
        if stable_changed {
            stable_violations += 1;
        }

        let adaptive_too_disruptive = previous
            .adaptive
            .open_loops
            .len()
            .abs_diff(candidate.adaptive.open_loops.len())
            > 1
            || previous
                .adaptive
                .commitments
                .len()
                .abs_diff(candidate.adaptive.commitments.len())
                > 1;
        if adaptive_too_disruptive {
            adaptive_violations += 1;
        }

        if !candidate.volatile.last_updated_at.ends_with('Z')
            || candidate.volatile.next_actions.is_empty()
        {
            volatile_violations += 1;
        }

        let mutation_result =
            IdentityContractV1::validate_mutation(&previous, &candidate, &persona);
        if stable_changed {
            assert!(
                mutation_result.is_err(),
                "stable layer mutation must be rejected"
            );
            previous = baseline.clone();
            previous.adaptive = candidate.adaptive;
            previous.volatile = candidate.volatile;
        } else {
            mutation_result.expect("valid adaptive and volatile updates should be accepted");
            previous = candidate;
        }
    }

    let total = session_shifts().len() as f64;
    (
        stable_violations as f64 / total,
        adaptive_violations as f64 / total,
        volatile_violations as f64 / total,
    )
}

#[test]
fn identity_continuity_suite_has_required_structure() {
    let suite = identity_continuity_suite();

    assert_eq!(suite.name, "identity_continuity_benchmark");
    assert_eq!(
        suite.description,
        "Tests identity contract layer stability and open-endedness across sessions"
    );
    assert_eq!(suite.scenario_count, 10);
    assert_eq!(suite.assertions.len(), 3);

    assert!((assertion_threshold(&suite, "stable") - 0.1).abs() < f64::EPSILON);
    assert!((assertion_threshold(&suite, "adaptive") - 0.2).abs() < f64::EPSILON);
    assert!((assertion_threshold(&suite, "volatile") - 0.5).abs() < f64::EPSILON);
}

#[test]
fn identity_layers_remain_within_violation_bounds_across_sessions() {
    let suite = identity_continuity_suite();
    let (stable_rate, adaptive_rate, volatile_rate) = measure_layer_violation_rates();

    assert!(stable_rate <= assertion_threshold(&suite, "stable"));
    assert!(adaptive_rate <= assertion_threshold(&suite, "adaptive"));
    assert!(volatile_rate <= assertion_threshold(&suite, "volatile"));
}

#[test]
fn identity_continuity_suite_runs_and_produces_report() {
    let suite = identity_continuity_suite();
    let report: BehavioralEvalReport =
        run_behavioral_eval(&suite).expect("behavioral runner should produce report");

    assert_eq!(report.spec_name, "identity_continuity_benchmark");
    assert_eq!(report.results.len(), 3);
    assert!(report.pass_rate >= 1.0);
    assert!(
        report
            .results
            .iter()
            .all(|result| result.assertion_label.starts_with("identity-continuity:"))
    );
}
