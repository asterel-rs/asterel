use crate::behavioral::{
    AssertionDirection, BehavioralAssertion, BehavioralEvalSpec, run_behavioral_eval,
};

fn tom_levels_suite() -> BehavioralEvalSpec {
    BehavioralEvalSpec {
        name: "three_level_tom_benchmark".to_string(),
        description: "Tests mental state inference, behavior prediction, and behavior judgment"
            .to_string(),
        assertions: vec![
            BehavioralAssertion::MentalStateInference {
                target_stakeholder: "procurement".to_string(),
                min_accuracy: 0.75,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::BehaviorPrediction {
                target_stakeholder: "procurement".to_string(),
                min_accuracy: 0.65,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::BehaviorJudgment {
                target_stakeholder: "procurement".to_string(),
                min_accuracy: 0.55,
                direction: AssertionDirection::Capability,
            },
        ],
        scenario_count: 8,
    }
}

#[test]
fn tom_variants_serialize_round_trip() {
    let suite = tom_levels_suite();
    let json = serde_json::to_string(&suite).expect("serialize");
    let loaded: BehavioralEvalSpec = serde_json::from_str(&json).expect("deserialize");

    assert_eq!(loaded.assertions.len(), 3);
    assert!(matches!(
        &loaded.assertions[0],
        BehavioralAssertion::MentalStateInference {
            target_stakeholder,
            min_accuracy,
            direction: AssertionDirection::Capability,
        } if target_stakeholder == "procurement" && (*min_accuracy - 0.75).abs() < f64::EPSILON
    ));
    assert!(matches!(
        &loaded.assertions[1],
        BehavioralAssertion::BehaviorPrediction {
            target_stakeholder,
            min_accuracy,
            direction: AssertionDirection::Capability,
        } if target_stakeholder == "procurement" && (*min_accuracy - 0.65).abs() < f64::EPSILON
    ));
    assert!(matches!(
        &loaded.assertions[2],
        BehavioralAssertion::BehaviorJudgment {
            target_stakeholder,
            min_accuracy,
            direction: AssertionDirection::Capability,
        } if target_stakeholder == "procurement" && (*min_accuracy - 0.55).abs() < f64::EPSILON
    ));
}

#[test]
fn tom_levels_run_through_behavioral_runner() {
    let suite = tom_levels_suite();
    let report = run_behavioral_eval(&suite).expect("behavioral eval should produce report");

    assert_eq!(report.spec_name, "three_level_tom_benchmark");
    assert_eq!(report.results.len(), 3);
    assert_eq!(report.capability_total, 3);
    assert_eq!(report.regression_total, 0);

    let labels: Vec<&str> = report
        .results
        .iter()
        .map(|result| result.assertion_label.as_str())
        .collect();
    assert!(labels.contains(&"mental-state-inference:procurement"));
    assert!(labels.contains(&"behavior-prediction:procurement"));
    assert!(labels.contains(&"behavior-judgment:procurement"));

    let mental = report
        .results
        .iter()
        .find(|result| result.assertion_label == "mental-state-inference:procurement")
        .expect("mental state result");
    let prediction = report
        .results
        .iter()
        .find(|result| result.assertion_label == "behavior-prediction:procurement")
        .expect("behavior prediction result");
    let judgment = report
        .results
        .iter()
        .find(|result| result.assertion_label == "behavior-judgment:procurement")
        .expect("behavior judgment result");

    assert!(mental.score > prediction.score);
    assert!(prediction.score > judgment.score);
}
