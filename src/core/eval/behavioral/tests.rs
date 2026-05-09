use super::super::types::{
    AssertionDirection, BehavioralAssertion, BehavioralAssertionResult, BehavioralEvalReport,
    BehavioralEvalSpec, NaturalnessAxis, NaturalnessGuardrail, RubricScore,
};
use super::*;

#[test]
fn spec_construction_all_seven_assertion_types() {
    let spec = BehavioralEvalSpec {
        name: "all-assertions".to_string(),
        description: "Test all seven assertion types".to_string(),
        assertions: vec![
            BehavioralAssertion::PersonalityStability {
                trait_name: "openness".to_string(),
                max_drift: 0.1,
                adversarial_turns: 10,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::PreferenceCoherence {
                domain: "communication_style".to_string(),
                min_consistency: 0.85,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::CounterfactualQuality {
                min_distinct_factors: 3,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::IdentityContinuity {
                contract_layer: "stable".to_string(),
                max_violation_rate: 0.0,
                direction: AssertionDirection::Regression,
            },
            BehavioralAssertion::MentalStateInference {
                target_stakeholder: "security-team".to_string(),
                min_accuracy: 0.75,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::BehaviorPrediction {
                target_stakeholder: "security-team".to_string(),
                min_accuracy: 0.65,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::BehaviorJudgment {
                target_stakeholder: "security-team".to_string(),
                min_accuracy: 0.55,
                direction: AssertionDirection::Regression,
            },
        ],
        scenario_count: 5,
    };

    assert_eq!(spec.name, "all-assertions");
    assert_eq!(spec.assertions.len(), 7);
    assert_eq!(spec.scenario_count, 5);
}

#[test]
fn run_behavioral_eval_returns_valid_report() {
    let spec = BehavioralEvalSpec {
        name: "multi-assertion".to_string(),
        description: "Multi-assertion test".to_string(),
        assertions: vec![
            BehavioralAssertion::PersonalityStability {
                trait_name: "conscientiousness".to_string(),
                max_drift: 0.05,
                adversarial_turns: 20,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::PreferenceCoherence {
                domain: "risk_tolerance".to_string(),
                min_consistency: 0.9,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::CounterfactualQuality {
                min_distinct_factors: 2,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::IdentityContinuity {
                contract_layer: "adaptive".to_string(),
                max_violation_rate: 0.05,
                direction: AssertionDirection::Regression,
            },
            BehavioralAssertion::MentalStateInference {
                target_stakeholder: "enterprise-customer".to_string(),
                min_accuracy: 0.75,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::BehaviorPrediction {
                target_stakeholder: "enterprise-customer".to_string(),
                min_accuracy: 0.65,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::BehaviorJudgment {
                target_stakeholder: "enterprise-customer".to_string(),
                min_accuracy: 0.55,
                direction: AssertionDirection::Regression,
            },
        ],
        scenario_count: 10,
    };

    let report = run_behavioral_eval(&spec).expect("should produce report");
    assert_eq!(report.spec_name, "multi-assertion");
    assert_eq!(report.results.len(), 7);
    assert_eq!(report.capability_total, 5);
    assert_eq!(report.regression_total, 2);
    assert_eq!(report.capability_pass_count, 5);
    assert_eq!(report.regression_hold_count, 2);
    assert!((report.pass_rate - 1.0).abs() < f64::EPSILON);

    for result in &report.results {
        assert!(result.passed);
        assert!((0.0..=1.0).contains(&result.score));
        assert!(!result.details.is_empty());
        assert!(result.details.starts_with(&result.assertion_label));
        assert!(!result.rubric_reasoning.is_empty());
    }

    let labels: Vec<&str> = report
        .results
        .iter()
        .map(|r| r.assertion_label.as_str())
        .collect();
    assert!(labels.contains(&"personality-stability:conscientiousness"));
    assert!(labels.contains(&"preference-coherence:risk_tolerance"));
    assert!(labels.contains(&"counterfactual-quality"));
    assert!(labels.contains(&"identity-continuity:adaptive"));
    assert!(labels.contains(&"mental-state-inference:enterprise-customer"));
    assert!(labels.contains(&"behavior-prediction:enterprise-customer"));
    assert!(labels.contains(&"behavior-judgment:enterprise-customer"));
}

#[test]
fn run_behavioral_eval_rejects_empty_assertions() {
    let spec = BehavioralEvalSpec {
        name: "empty".to_string(),
        description: "No assertions".to_string(),
        assertions: vec![],
        scenario_count: 1,
    };

    let err = run_behavioral_eval(&spec).unwrap_err();
    assert!(err.to_string().contains("no assertions"));
}

#[test]
fn run_behavioral_eval_rejects_zero_scenarios() {
    let spec = BehavioralEvalSpec {
        name: "zero-scenarios".to_string(),
        description: "No scenarios".to_string(),
        assertions: vec![BehavioralAssertion::CounterfactualQuality {
            min_distinct_factors: 1,
            direction: AssertionDirection::Capability,
        }],
        scenario_count: 0,
    };

    let err = run_behavioral_eval(&spec).unwrap_err();
    assert!(
        err.to_string()
            .contains("must include at least one scenario")
    );
}

#[test]
fn behavioral_assertion_direction_defaults_to_capability() {
    let raw = r#"{"type":"counterfactual_quality","min_distinct_factors":2}"#;
    let assertion: BehavioralAssertion = serde_json::from_str(raw).expect("deserialize");
    assert_eq!(assertion.direction(), AssertionDirection::Capability);
}

#[test]
fn behavioral_report_separates_capability_and_regression_counts() {
    let results = vec![
        BehavioralAssertionResult {
            assertion_label: "cap-pass".to_string(),
            passed: true,
            direction: AssertionDirection::Capability,
            score: 0.9,
            details: "cap-pass details".to_string(),
            rubric_score: RubricScore::Excellent,
            rubric_reasoning: "excellent".to_string(),
        },
        BehavioralAssertionResult {
            assertion_label: "reg-pass".to_string(),
            passed: true,
            direction: AssertionDirection::Regression,
            score: 0.9,
            details: "reg-pass details".to_string(),
            rubric_score: RubricScore::Excellent,
            rubric_reasoning: "excellent".to_string(),
        },
        BehavioralAssertionResult {
            assertion_label: "cap-fail".to_string(),
            passed: false,
            direction: AssertionDirection::Capability,
            score: 0.2,
            details: "cap-fail details".to_string(),
            rubric_score: RubricScore::Failing,
            rubric_reasoning: "failing".to_string(),
        },
        BehavioralAssertionResult {
            assertion_label: "reg-fail".to_string(),
            passed: false,
            direction: AssertionDirection::Regression,
            score: 0.2,
            details: "reg-fail details".to_string(),
            rubric_score: RubricScore::Failing,
            rubric_reasoning: "failing".to_string(),
        },
    ];
    let assertions = vec![
        BehavioralAssertion::PersonalityStability {
            trait_name: "openness".to_string(),
            max_drift: 0.1,
            adversarial_turns: 3,
            direction: AssertionDirection::Capability,
        },
        BehavioralAssertion::IdentityContinuity {
            contract_layer: "stable".to_string(),
            max_violation_rate: 0.1,
            direction: AssertionDirection::Regression,
        },
        BehavioralAssertion::PreferenceCoherence {
            domain: "style".to_string(),
            min_consistency: 0.8,
            direction: AssertionDirection::Capability,
        },
        BehavioralAssertion::BehaviorJudgment {
            target_stakeholder: "finance".to_string(),
            min_accuracy: 0.5,
            direction: AssertionDirection::Regression,
        },
    ];

    let counts = summarize_direction_counts(&results, &assertions);
    assert_eq!(counts, (1, 2, 1, 2));
}

#[test]
fn regression_failures_are_ordered_first_in_summary() {
    let mut results = [
        BehavioralAssertionResult {
            assertion_label: "cap-pass".to_string(),
            passed: true,
            direction: AssertionDirection::Capability,
            score: 0.9,
            details: "cap-pass details".to_string(),
            rubric_score: RubricScore::Excellent,
            rubric_reasoning: "excellent".to_string(),
        },
        BehavioralAssertionResult {
            assertion_label: "cap-fail".to_string(),
            passed: false,
            direction: AssertionDirection::Capability,
            score: 0.2,
            details: "cap-fail details".to_string(),
            rubric_score: RubricScore::Failing,
            rubric_reasoning: "failing".to_string(),
        },
        BehavioralAssertionResult {
            assertion_label: "reg-fail".to_string(),
            passed: false,
            direction: AssertionDirection::Regression,
            score: 0.1,
            details: "reg-fail details".to_string(),
            rubric_score: RubricScore::Failing,
            rubric_reasoning: "failing".to_string(),
        },
    ];

    results.sort_by_key(result_priority);

    assert_eq!(results[0].assertion_label, "reg-fail");
    assert_eq!(results[1].assertion_label, "cap-fail");
}

#[test]
fn assertion_labels_are_descriptive() {
    let ps = BehavioralAssertion::PersonalityStability {
        trait_name: "neuroticism".to_string(),
        max_drift: 0.1,
        adversarial_turns: 5,
        direction: AssertionDirection::Capability,
    };
    assert_eq!(ps.label(), "personality-stability:neuroticism");

    let pc = BehavioralAssertion::PreferenceCoherence {
        domain: "style".to_string(),
        min_consistency: 0.8,
        direction: AssertionDirection::Capability,
    };
    assert_eq!(pc.label(), "preference-coherence:style");

    let cq = BehavioralAssertion::CounterfactualQuality {
        min_distinct_factors: 3,
        direction: AssertionDirection::Capability,
    };
    assert_eq!(cq.label(), "counterfactual-quality");

    let ic = BehavioralAssertion::IdentityContinuity {
        contract_layer: "volatile".to_string(),
        max_violation_rate: 0.1,
        direction: AssertionDirection::Regression,
    };
    assert_eq!(ic.label(), "identity-continuity:volatile");

    let msi = BehavioralAssertion::MentalStateInference {
        target_stakeholder: "legal".to_string(),
        min_accuracy: 0.8,
        direction: AssertionDirection::Capability,
    };
    assert_eq!(msi.label(), "mental-state-inference:legal");

    let bp = BehavioralAssertion::BehaviorPrediction {
        target_stakeholder: "legal".to_string(),
        min_accuracy: 0.7,
        direction: AssertionDirection::Capability,
    };
    assert_eq!(bp.label(), "behavior-prediction:legal");

    let bj = BehavioralAssertion::BehaviorJudgment {
        target_stakeholder: "legal".to_string(),
        min_accuracy: 0.6,
        direction: AssertionDirection::Regression,
    };
    assert_eq!(bj.label(), "behavior-judgment:legal");
}

#[test]
fn tom_levels_decline_across_reasoning_depth() {
    let stakeholder = "risk-committee";
    let label_one = format!("mental-state-inference:{stakeholder}");
    let label_two = format!("behavior-prediction:{stakeholder}");
    let label_three = format!("behavior-judgment:{stakeholder}");
    let mental = evaluate_mental_state_inference(
        &BehavioralAssertion::MentalStateInference {
            target_stakeholder: stakeholder.to_string(),
            min_accuracy: 0.0,
            direction: AssertionDirection::Capability,
        },
        &label_one,
        stakeholder,
        0.0,
        8,
        scenario_seed(0xA11, &label_one, "assertion"),
    );
    let prediction = evaluate_behavior_prediction(
        &BehavioralAssertion::BehaviorPrediction {
            target_stakeholder: stakeholder.to_string(),
            min_accuracy: 0.0,
            direction: AssertionDirection::Capability,
        },
        &label_two,
        stakeholder,
        0.0,
        8,
        scenario_seed(0xA12, &label_two, "assertion"),
    );
    let judgment = evaluate_behavior_judgment(
        &BehavioralAssertion::BehaviorJudgment {
            target_stakeholder: stakeholder.to_string(),
            min_accuracy: 0.0,
            direction: AssertionDirection::Capability,
        },
        &label_three,
        stakeholder,
        0.0,
        8,
        scenario_seed(0xA13, &label_three, "assertion"),
    );

    assert!(mental.1 > prediction.1);
    assert!(prediction.1 > judgment.1);
}

#[test]
fn score_to_rubric_uses_expected_thresholds() {
    assert_eq!(score_to_rubric(0.95), RubricScore::Excellent);
    assert_eq!(score_to_rubric(0.80), RubricScore::Good);
    assert_eq!(score_to_rubric(0.60), RubricScore::Adequate);
    assert_eq!(score_to_rubric(0.40), RubricScore::Poor);
    assert_eq!(score_to_rubric(0.10), RubricScore::Failing);
}

#[test]
fn serde_round_trip_spec() {
    let spec = BehavioralEvalSpec {
        name: "serde-test".to_string(),
        description: "Round-trip".to_string(),
        assertions: vec![
            BehavioralAssertion::PersonalityStability {
                trait_name: "openness".to_string(),
                max_drift: 0.1,
                adversarial_turns: 10,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::CounterfactualQuality {
                min_distinct_factors: 2,
                direction: AssertionDirection::Regression,
            },
            BehavioralAssertion::MentalStateInference {
                target_stakeholder: "ops".to_string(),
                min_accuracy: 0.7,
                direction: AssertionDirection::Capability,
            },
        ],
        scenario_count: 3,
    };

    let json = serde_json::to_string(&spec).expect("serialize");
    let loaded: BehavioralEvalSpec = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(loaded.name, "serde-test");
    assert_eq!(loaded.assertions.len(), 3);
    assert_eq!(loaded.scenario_count, 3);
}

#[test]
fn serde_round_trip_report() {
    let report = BehavioralEvalReport {
        spec_name: "report-test".to_string(),
        results: vec![BehavioralAssertionResult {
            assertion_label: "test-label".to_string(),
            passed: true,
            direction: AssertionDirection::Capability,
            score: 0.95,
            details: "test details".to_string(),
            rubric_score: RubricScore::Excellent,
            rubric_reasoning: "excellent reasoning".to_string(),
        }],
        pass_rate: 1.0,
        capability_pass_count: 1,
        capability_total: 1,
        regression_hold_count: 0,
        regression_total: 0,
    };

    let json = serde_json::to_string(&report).expect("serialize");
    let loaded: BehavioralEvalReport = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(loaded.spec_name, "report-test");
    assert_eq!(loaded.results.len(), 1);
    assert!(loaded.results[0].passed);
    assert_eq!(loaded.capability_pass_count, 1);
}

// ── Human Naturalness Bench tests ─────────────────────────────

#[test]
fn naturalness_axis_label_format() {
    let assertion = BehavioralAssertion::HumanNaturalnessAxis {
        family: "single_turn_chat".to_string(),
        axis: NaturalnessAxis::ClosureVariety,
        min_score: 0.70,
        direction: AssertionDirection::Capability,
    };
    assert_eq!(
        assertion.label(),
        "human-naturalness-axis:single_turn_chat:closure_variety"
    );
}

#[test]
fn naturalness_guardrail_label_format() {
    let assertion = BehavioralAssertion::HumanNaturalnessGuardrail {
        family: "tone_boundary".to_string(),
        guardrail: NaturalnessGuardrail::FakeHumanMemory,
        max_violations: 0,
        direction: AssertionDirection::Regression,
    };
    assert_eq!(
        assertion.label(),
        "human-naturalness-guardrail:tone_boundary:fake_human_memory"
    );
}

#[test]
fn naturalness_axis_eval_produces_score() {
    let spec = BehavioralEvalSpec {
        name: "naturalness-axis-test".to_string(),
        description: "Test naturalness axis evaluation".to_string(),
        assertions: vec![BehavioralAssertion::HumanNaturalnessAxis {
            family: "single_turn_chat".to_string(),
            axis: NaturalnessAxis::AntiTemplate,
            min_score: 0.50,
            direction: AssertionDirection::Capability,
        }],
        scenario_count: 20,
    };
    let report = run_behavioral_eval(&spec).expect("eval should succeed");
    assert_eq!(report.results.len(), 1);
    assert!(report.results[0].score > 0.0, "score should be positive");
    assert!(report.results[0].score <= 1.0, "score should be <= 1.0");
    assert!(
        report.results[0]
            .details
            .contains("fixture-backed quality score")
    );
}

#[test]
fn naturalness_guardrail_eval_detects_violations() {
    let spec = BehavioralEvalSpec {
        name: "naturalness-guardrail-test".to_string(),
        description: "Test guardrail violation counting".to_string(),
        assertions: vec![BehavioralAssertion::HumanNaturalnessGuardrail {
            family: "tone_boundary".to_string(),
            guardrail: NaturalnessGuardrail::ToneOverweight,
            max_violations: 0,
            direction: AssertionDirection::Regression,
        }],
        scenario_count: 100,
    };
    let report = run_behavioral_eval(&spec).expect("eval should succeed");
    assert_eq!(report.results.len(), 1);
    assert!(report.results[0].passed);
    assert_eq!(report.results[0].score, 1.0);
    assert!(
        report.results[0]
            .details
            .contains("fixture-backed guardrail violations"),
        "details should mention fixture-backed guardrail violations"
    );
}

#[test]
fn naturalness_full_bench_gate_first_shape() {
    let spec = BehavioralEvalSpec {
        name: "human-naturalness-full".to_string(),
        description: "Full naturalness bench with axes and guardrails".to_string(),
        assertions: vec![
            BehavioralAssertion::HumanNaturalnessAxis {
                family: "single_turn_chat".to_string(),
                axis: NaturalnessAxis::AntiTemplate,
                min_score: 0.50,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::HumanNaturalnessAxis {
                family: "single_turn_chat".to_string(),
                axis: NaturalnessAxis::ClosureVariety,
                min_score: 0.40,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::HumanNaturalnessAxis {
                family: "multi_turn_rapport".to_string(),
                axis: NaturalnessAxis::DistanceProgression,
                min_score: 0.45,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::HumanNaturalnessGuardrail {
                family: "tone_boundary".to_string(),
                guardrail: NaturalnessGuardrail::FakeHumanMemory,
                max_violations: 0,
                direction: AssertionDirection::Regression,
            },
            BehavioralAssertion::HumanNaturalnessGuardrail {
                family: "tone_boundary".to_string(),
                guardrail: NaturalnessGuardrail::PerformedEmotion,
                max_violations: 1,
                direction: AssertionDirection::Regression,
            },
        ],
        scenario_count: 30,
    };
    let report = run_behavioral_eval(&spec).expect("eval should succeed");
    assert_eq!(report.results.len(), 5);
    // Gate-first: check that all axes produce scores and all guardrails produce counts
    for result in &report.results {
        assert!(
            result.score >= 0.0 && result.score <= 1.0,
            "score out of range: {} = {}",
            result.assertion_label,
            result.score
        );
    }
}

#[test]
fn naturalness_all_axes_serialization_roundtrip() {
    let axes = vec![
        NaturalnessAxis::AntiTemplate,
        NaturalnessAxis::ClosureVariety,
        NaturalnessAxis::SubtextPause,
        NaturalnessAxis::AestheticSignature,
        NaturalnessAxis::DistanceProgression,
    ];
    for axis in axes {
        let assertion = BehavioralAssertion::HumanNaturalnessAxis {
            family: "test".to_string(),
            axis,
            min_score: 0.5,
            direction: AssertionDirection::Capability,
        };
        let json = serde_json::to_string(&assertion).expect("serialize");
        let loaded: BehavioralAssertion = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(assertion.label(), loaded.label());
    }
}

#[test]
fn naturalness_all_guardrails_serialization_roundtrip() {
    let guardrails = vec![
        NaturalnessGuardrail::FakeHumanMemory,
        NaturalnessGuardrail::PerformedEmotion,
        NaturalnessGuardrail::ToneOverweight,
    ];
    for guardrail in guardrails {
        let assertion = BehavioralAssertion::HumanNaturalnessGuardrail {
            family: "test".to_string(),
            guardrail,
            max_violations: 0,
            direction: AssertionDirection::Regression,
        };
        let json = serde_json::to_string(&assertion).expect("serialize");
        let loaded: BehavioralAssertion = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(assertion.label(), loaded.label());
    }
}
