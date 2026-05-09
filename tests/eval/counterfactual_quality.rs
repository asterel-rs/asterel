use std::collections::{BTreeSet, HashSet};

use crate::behavioral::{
    AssertionDirection, BehavioralAssertion, BehavioralEvalSpec, run_behavioral_eval,
};

const MIN_DISTINCT_FACTORS: usize = 3;

#[derive(Clone, Copy)]
struct CounterfactualScenario {
    prompt: &'static str,
    response: &'static str,
    factor_keywords: [&'static [&'static str]; 3],
}

pub(crate) fn counterfactual_quality_suite() -> BehavioralEvalSpec {
    BehavioralEvalSpec {
        name: "counterfactual_quality_benchmark".to_string(),
        description: "Tests counterfactual reasoning quality and causal factor identification"
            .to_string(),
        assertions: vec![BehavioralAssertion::CounterfactualQuality {
            min_distinct_factors: MIN_DISTINCT_FACTORS,
            direction: AssertionDirection::Capability,
        }],
        scenario_count: counterfactual_scenarios().len(),
    }
}

fn counterfactual_scenarios() -> Vec<CounterfactualScenario> {
    vec![
        CounterfactualScenario {
            prompt: "Why did the project fail?",
            response: "The project failed because planning was weak, staffing and budget were constrained, and stakeholder communication broke down.",
            factor_keywords: [
                &["planning", "plan"],
                &["staffing", "budget", "resource"],
                &["communication", "stakeholder"],
            ],
        },
        CounterfactualScenario {
            prompt: "Why did the user abandon the session?",
            response: "The user likely left due to unhelpful responses, slow system performance, and an unclear navigation flow in the interface.",
            factor_keywords: [
                &["unhelpful", "responses"],
                &["slow", "performance", "latency"],
                &["unclear", "navigation", "interface", "ui"],
            ],
        },
        CounterfactualScenario {
            prompt: "Why did the model give the wrong answer?",
            response: "The incorrect answer came from training-data coverage gaps, ambiguity in the prompt wording, and context-window truncation of critical details.",
            factor_keywords: [
                &["training", "data", "coverage", "gaps"],
                &["ambiguity", "prompt", "wording"],
                &["context", "window", "truncation"],
            ],
        },
        CounterfactualScenario {
            prompt: "Why did incident response lag?",
            response: "Response lag occurred because alerts were noisy, on-call ownership was unclear, and runbooks were stale.",
            factor_keywords: [
                &["alerts", "noisy"],
                &["on-call", "ownership", "unclear"],
                &["runbooks", "stale"],
            ],
        },
        CounterfactualScenario {
            prompt: "Why did onboarding completion drop?",
            response: "Completion dropped due to long signup forms, weak in-product guidance, and delayed email verification.",
            factor_keywords: [
                &["long", "signup", "forms"],
                &["guidance", "in-product"],
                &["delayed", "email", "verification"],
            ],
        },
        CounterfactualScenario {
            prompt: "Why did deployment reliability decline?",
            response: "Reliability declined because test coverage regressed, environment parity was inconsistent, and rollback procedures were brittle.",
            factor_keywords: [
                &["test", "coverage", "regressed"],
                &["environment", "parity", "inconsistent"],
                &["rollback", "brittle"],
            ],
        },
        CounterfactualScenario {
            prompt: "Why did search relevance worsen?",
            response: "Relevance worsened after outdated indexing features remained, synonym mappings were sparse, and click-feedback loops were ignored.",
            factor_keywords: [
                &["outdated", "indexing", "features"],
                &["synonym", "mappings", "sparse"],
                &["click", "feedback", "ignored"],
            ],
        },
        CounterfactualScenario {
            prompt: "Why did conversion rate decline on mobile?",
            response: "Mobile conversion dropped because layout shifts disrupted checkout, page weight increased load time, and payment errors went unresolved.",
            factor_keywords: [
                &["layout", "shifts", "checkout"],
                &["page", "weight", "load", "time"],
                &["payment", "errors", "unresolved"],
            ],
        },
        CounterfactualScenario {
            prompt: "Why did recommendation quality drift?",
            response: "Recommendation drift came from stale user embeddings, sparse item metadata, and delayed online model updates.",
            factor_keywords: [
                &["stale", "user", "embeddings"],
                &["sparse", "item", "metadata"],
                &["delayed", "online", "model", "updates"],
            ],
        },
        CounterfactualScenario {
            prompt: "Why did customer satisfaction dip this quarter?",
            response: "Satisfaction dipped because release quality regressed, support wait times grew, and policy messaging was inconsistent.",
            factor_keywords: [
                &["release", "quality", "regressed"],
                &["support", "wait", "times"],
                &["policy", "messaging", "inconsistent"],
            ],
        },
    ]
}

fn split_candidate_factors(text: &str) -> Vec<String> {
    text.split([',', ';', '.'])
        .map(str::trim)
        .filter(|fragment| !fragment.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn normalized_tokens(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split_whitespace()
        .map(|token| {
            token
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
                .collect::<String>()
        })
        .filter(|token| token.len() > 2)
        .collect()
}

fn overlap_ratio(left: &str, right: &str) -> f64 {
    let left_tokens = normalized_tokens(left);
    let right_tokens = normalized_tokens(right);
    let baseline = left_tokens.len().min(right_tokens.len());
    if baseline == 0 {
        return 0.0;
    }

    let overlap = left_tokens.intersection(&right_tokens).count();
    overlap as f64 / baseline as f64
}

fn contains_keyword_group(candidate: &str, keywords: &[&str]) -> bool {
    let candidate_lower = candidate.to_lowercase();
    keywords
        .iter()
        .any(|keyword| candidate_lower.contains(keyword))
}

fn extract_distinct_factors(scenario: CounterfactualScenario) -> Vec<String> {
    let candidates = split_candidate_factors(scenario.response);
    let mut selected = Vec::new();

    for keywords in scenario.factor_keywords {
        let matched = candidates
            .iter()
            .find(|candidate| contains_keyword_group(candidate, keywords))
            .cloned()
            .or_else(|| {
                if contains_keyword_group(scenario.response, keywords) {
                    Some(scenario.response.to_string())
                } else {
                    None
                }
            });

        if let Some(candidate) = matched {
            selected.push(candidate);
        }
    }

    let mut distinct: Vec<String> = Vec::new();
    for factor in selected {
        if distinct
            .iter()
            .all(|existing| overlap_ratio(existing, &factor) < 0.5)
        {
            distinct.push(factor);
        }
    }

    distinct
}

#[test]
fn counterfactual_quality_suite_has_expected_configuration() {
    let suite = counterfactual_quality_suite();

    assert_eq!(suite.name, "counterfactual_quality_benchmark");
    assert_eq!(
        suite.description,
        "Tests counterfactual reasoning quality and causal factor identification"
    );
    assert!(suite.scenario_count >= 10);
    assert_eq!(suite.assertions.len(), 1);
    assert!(matches!(
        suite.assertions[0],
        BehavioralAssertion::CounterfactualQuality {
            min_distinct_factors: MIN_DISTINCT_FACTORS,
            direction: AssertionDirection::Capability
        }
    ));
}

#[test]
fn counterfactual_scenarios_identify_minimum_distinct_factors() {
    let suite = counterfactual_quality_suite();
    let min_distinct_factors = match suite.assertions[0] {
        BehavioralAssertion::CounterfactualQuality {
            min_distinct_factors,
            direction: AssertionDirection::Capability,
        } => min_distinct_factors,
        _ => unreachable!("suite assertion should be counterfactual quality"),
    };

    let scenarios = counterfactual_scenarios();
    assert_eq!(scenarios.len(), suite.scenario_count);

    for scenario in scenarios {
        assert!(!scenario.prompt.is_empty());
        let distinct_factors = extract_distinct_factors(scenario);
        let unique_factor_count = distinct_factors.into_iter().collect::<HashSet<_>>().len();
        assert!(
            unique_factor_count >= min_distinct_factors,
            "scenario '{}' had only {unique_factor_count} distinct factors",
            scenario.prompt
        );
    }
}

#[test]
fn counterfactual_quality_suite_runs_behavioral_runner() {
    let suite = counterfactual_quality_suite();
    let report = run_behavioral_eval(&suite).expect("behavioral eval should produce report");

    assert_eq!(report.spec_name, "counterfactual_quality_benchmark");
    assert_eq!(report.results.len(), 1);
    assert!(
        (report.pass_rate - 1.0).abs() < f64::EPSILON,
        "all counterfactual quality assertions should pass deterministically, got pass_rate={:.3}",
        report.pass_rate
    );
    assert!(
        report
            .results
            .iter()
            .all(|result| result.assertion_label == "counterfactual-quality")
    );
}
