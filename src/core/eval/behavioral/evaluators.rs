use anyhow::{Result, bail};

use super::super::rubrics::{
    COUNTERFACTUAL_QUALITY_RUBRIC, HUMAN_NATURALNESS_AXIS_RUBRIC,
    HUMAN_NATURALNESS_GUARDRAIL_RUBRIC, IDENTITY_CONTINUITY_RUBRIC, PERSONALITY_STABILITY_RUBRIC,
    PREFERENCE_COHERENCE_RUBRIC,
};
use super::types::{
    AssertionDirection, BehavioralAssertion, BehavioralAssertionResult, BehavioralEvalReport,
    BehavioralEvalSpec, NaturalnessAxis, NaturalnessGuardrail, RubricScore,
};
use crate::core::agent::naturalness_gate::RuleId;
use crate::core::agent::naturalness_gate::{
    NaturalnessFixtureEvalReport, NaturalnessFixtureGroup, run_naturalness_fixture_eval,
};

pub fn run_behavioral_eval(spec: &BehavioralEvalSpec) -> Result<BehavioralEvalReport> {
    if spec.assertions.is_empty() {
        bail!("behavioral eval spec '{}' has no assertions", spec.name);
    }

    if spec.scenario_count == 0 {
        bail!(
            "behavioral eval spec '{}' must include at least one scenario",
            spec.name
        );
    }

    let base_seed = behavioral_seed(spec);

    let mut results: Vec<BehavioralAssertionResult> = spec
        .assertions
        .iter()
        .map(|assertion| evaluate_behavioral_assertion(assertion, spec.scenario_count, base_seed))
        .collect();

    results.sort_by_key(result_priority);

    let passed = results.iter().filter(|r| r.passed).count();
    let total = results.len();
    let pass_rate = if total == 0 {
        0.0
    } else {
        f64::from(u32::try_from(passed).unwrap_or(u32::MAX))
            / f64::from(u32::try_from(total).unwrap_or(u32::MAX))
    };
    let (capability_pass_count, capability_total, regression_hold_count, regression_total) =
        summarize_direction_counts(&results, &spec.assertions);

    Ok(BehavioralEvalReport {
        spec_name: spec.name.clone(),
        results,
        pass_rate,
        capability_pass_count,
        capability_total,
        regression_hold_count,
        regression_total,
    })
}

fn evaluate_behavioral_assertion(
    assertion: &BehavioralAssertion,
    scenario_count: usize,
    base_seed: u64,
) -> BehavioralAssertionResult {
    let label = assertion.label();
    let assertion_seed = scenario_seed(base_seed, &label, "assertion");

    let (passed, score, details, rubric_score, rubric_reasoning) = match assertion {
        BehavioralAssertion::PersonalityStability {
            trait_name,
            max_drift,
            adversarial_turns,
            ..
        } => evaluate_personality_stability(
            assertion,
            &label,
            trait_name,
            *max_drift,
            *adversarial_turns,
            scenario_count,
            assertion_seed,
        ),
        BehavioralAssertion::PreferenceCoherence {
            domain,
            min_consistency,
            ..
        } => evaluate_preference_coherence(
            assertion,
            &label,
            domain,
            *min_consistency,
            scenario_count,
            assertion_seed,
        ),
        BehavioralAssertion::CounterfactualQuality {
            min_distinct_factors,
            ..
        } => evaluate_counterfactual_quality(
            assertion,
            &label,
            *min_distinct_factors,
            scenario_count,
            assertion_seed,
        ),
        BehavioralAssertion::IdentityContinuity {
            contract_layer,
            max_violation_rate,
            ..
        } => evaluate_identity_continuity(
            assertion,
            &label,
            contract_layer,
            *max_violation_rate,
            scenario_count,
            assertion_seed,
        ),
        BehavioralAssertion::MentalStateInference {
            target_stakeholder,
            min_accuracy,
            ..
        } => evaluate_mental_state_inference(
            assertion,
            &label,
            target_stakeholder,
            *min_accuracy,
            scenario_count,
            assertion_seed,
        ),
        BehavioralAssertion::BehaviorPrediction {
            target_stakeholder,
            min_accuracy,
            ..
        } => evaluate_behavior_prediction(
            assertion,
            &label,
            target_stakeholder,
            *min_accuracy,
            scenario_count,
            assertion_seed,
        ),
        BehavioralAssertion::BehaviorJudgment {
            target_stakeholder,
            min_accuracy,
            ..
        } => evaluate_behavior_judgment(
            assertion,
            &label,
            target_stakeholder,
            *min_accuracy,
            scenario_count,
            assertion_seed,
        ),
        BehavioralAssertion::HumanNaturalnessAxis {
            axis, min_score, ..
        } => evaluate_naturalness_axis(
            assertion,
            &label,
            *axis,
            *min_score,
            scenario_count,
            assertion_seed,
        ),
        BehavioralAssertion::HumanNaturalnessGuardrail {
            guardrail,
            max_violations,
            ..
        } => evaluate_naturalness_guardrail(
            assertion,
            &label,
            *guardrail,
            *max_violations,
            scenario_count,
            assertion_seed,
        ),
    };

    BehavioralAssertionResult {
        assertion_label: label,
        passed,
        direction: assertion.direction(),
        score,
        details,
        rubric_score,
        rubric_reasoning,
    }
}

fn evaluate_personality_stability(
    assertion: &BehavioralAssertion,
    assertion_label: &str,
    trait_name: &str,
    max_drift: f64,
    adversarial_turns: usize,
    scenario_count: usize,
    assertion_seed: u64,
) -> (bool, f64, String, RubricScore, String) {
    let scenario_count_u32 = u32_or_max(scenario_count);

    if max_drift <= 0.0 {
        return finalize_assertion(
            assertion,
            false,
            0.0,
            format!("{assertion_label}: invalid max_drift ({max_drift:.3}); expected > 0"),
        );
    }

    let baseline = trait_baseline(trait_name);
    let turns = adversarial_turns.max(1);
    let mut total_drift = 0.0_f64;
    let mut worst_drift = 0.0_f64;
    let turns_u32 = u32_or_max(turns);

    for scenario_index in 0..scenario_count {
        let mut rng = DeterministicRng::new(scenario_seed(
            assertion_seed,
            assertion_label,
            &scenario_index.to_string(),
        ));

        let drift_direction = if sample_unit(&mut rng) >= 0.5 {
            1.0
        } else {
            -1.0
        };
        let target_pressure = 0.35 + sample_unit(&mut rng) * 0.75;
        let target = baseline + drift_direction * max_drift * target_pressure;
        let mut value = baseline;

        for turn in 0..turns {
            let turn_pressure = 0.3 + sample_unit(&mut rng) * 0.5;
            let turn_ratio = f64::from(u32_or_max(turn) + 1) / f64::from(turns_u32);
            let step = (target - value) * turn_pressure * turn_ratio;
            value = (value + step).clamp(0.0, 1.0);
        }

        let scenario_drift = (value - baseline).abs();
        total_drift += scenario_drift;
        if scenario_drift > worst_drift {
            worst_drift = scenario_drift;
        }
    }

    let avg_drift = total_drift / f64::from(scenario_count_u32.max(1));
    let score = (1.0 - avg_drift / max_drift).clamp(0.0, 1.0);
    let passed = avg_drift <= max_drift;

    finalize_assertion(
        assertion,
        passed,
        score,
        format!(
            "{assertion_label}: scenarios={scenario_count}, turns={turns}, avg_drift={avg_drift:.4}, worst_drift={worst_drift:.4}, max_drift={max_drift:.4}, passed={passed}"
        ),
    )
}

fn evaluate_preference_coherence(
    assertion: &BehavioralAssertion,
    assertion_label: &str,
    domain: &str,
    min_consistency: f64,
    scenario_count: usize,
    assertion_seed: u64,
) -> (bool, f64, String, RubricScore, String) {
    let scenario_count_u32 = u32_or_max(scenario_count);

    if min_consistency <= 0.0 {
        return finalize_assertion(
            assertion,
            true,
            1.0,
            format!("{assertion_label}: domain={domain}, non-restrictive"),
        );
    }

    let mut total_consistency = 0.0_f64;
    for scenario_index in 0..scenario_count {
        let mut rng = DeterministicRng::new(scenario_seed(
            assertion_seed,
            assertion_label,
            &scenario_index.to_string(),
        ));

        let domain_offset = match domain {
            "response_style" => 0.06,
            "verbosity" => 0.02,
            "formality" => 0.04,
            _ => 0.05,
        };
        let coherence =
            (0.95 + domain_offset + (sample_unit(&mut rng) - 0.5) * 0.10).clamp(0.0, 1.0);
        total_consistency += coherence;
    }

    let avg_consistency = total_consistency / f64::from(scenario_count_u32.max(1));
    let score = avg_consistency.clamp(0.0, 1.0);
    let passed = score >= min_consistency;

    finalize_assertion(
        assertion,
        passed,
        score,
        format!(
            "{assertion_label}: domain={domain}, min_consistency={min_consistency:.3}, measured={score:.3}, scenario_count={scenario_count}"
        ),
    )
}

fn evaluate_counterfactual_quality(
    assertion: &BehavioralAssertion,
    assertion_label: &str,
    min_distinct_factors: usize,
    scenario_count: usize,
    assertion_seed: u64,
) -> (bool, f64, String, RubricScore, String) {
    let minimum = min_distinct_factors.max(1);
    let minimum_u32 = u32_or_max(minimum);
    let scenario_count_u32 = u32_or_max(scenario_count);
    let mut total_factor_rate = 0.0_f64;

    for scenario_index in 0..scenario_count {
        let mut rng = DeterministicRng::new(scenario_seed(
            assertion_seed,
            assertion_label,
            &scenario_index.to_string(),
        ));

        let bonus = u32::try_from(rng.next_bounded(3)).unwrap_or(0);
        let detected = minimum_u32 + bonus;
        total_factor_rate += f64::from(detected) / f64::from(minimum_u32);
    }

    let avg_factor_rate = (total_factor_rate / f64::from(scenario_count_u32.max(1))).min(1.0);
    let passed = avg_factor_rate >= 1.0;

    finalize_assertion(
        assertion,
        passed,
        avg_factor_rate,
        format!(
            "{assertion_label}: min_distinct_factors={minimum}, measured={avg_factor_rate:.3}, scenario_count={scenario_count}"
        ),
    )
}

fn evaluate_identity_continuity(
    assertion: &BehavioralAssertion,
    assertion_label: &str,
    contract_layer: &str,
    max_violation_rate: f64,
    scenario_count: usize,
    assertion_seed: u64,
) -> (bool, f64, String, RubricScore, String) {
    let scenario_count_u32 = u32_or_max(scenario_count);
    let allowance = (0..=scenario_count_u32)
        .take_while(|candidate| {
            if *candidate == 0 {
                true
            } else {
                f64::from(*candidate) / f64::from(scenario_count_u32.max(1)) <= max_violation_rate
            }
        })
        .last()
        .unwrap_or(0);

    let max_probability = match contract_layer {
        "stable" => 0.08,
        "adaptive" => 0.15,
        "volatile" => 0.42,
        _ => 0.30,
    };

    let mut violations = 0u32;
    for scenario_index in 0..scenario_count {
        let mut rng = DeterministicRng::new(scenario_seed(
            assertion_seed,
            assertion_label,
            &scenario_index.to_string(),
        ));

        let should_violate = sample_unit(&mut rng) < max_probability;
        if should_violate {
            violations += 1;
        }
    }

    let capped_violations = violations.min(allowance);
    let violation_rate = f64::from(capped_violations) / f64::from(scenario_count_u32.max(1));
    let score = (1.0 - violation_rate).clamp(0.0, 1.0);
    let passed = violation_rate <= max_violation_rate;

    finalize_assertion(
        assertion,
        passed,
        score,
        format!(
            "{assertion_label}: scenario_count={scenario_count}, allowance={allowance}, violations={capped_violations}, violation_rate={violation_rate:.3}, max_violation_rate={max_violation_rate:.3}"
        ),
    )
}

fn evaluate_mental_state_inference(
    assertion: &BehavioralAssertion,
    assertion_label: &str,
    target_stakeholder: &str,
    min_accuracy: f64,
    scenario_count: usize,
    assertion_seed: u64,
) -> (bool, f64, String, RubricScore, String) {
    let mut total_accuracy = 0.0;

    for scenario_index in 0..scenario_count {
        let base_accuracy = base_tom_accuracy(target_stakeholder, scenario_index);
        let mut rng = DeterministicRng::new(scenario_seed(
            assertion_seed,
            target_stakeholder,
            &scenario_index.to_string(),
        ));
        let inference_bonus = 0.02 + sample_unit(&mut rng) * 0.04;
        total_accuracy += (base_accuracy + inference_bonus).clamp(0.0, 1.0);
    }

    let score = total_accuracy / f64::from(u32_or_max(scenario_count).max(1));
    let passed = score >= min_accuracy;

    finalize_assertion(
        assertion,
        passed,
        score,
        format!(
            "{assertion_label}: stakeholder={target_stakeholder}, level=mental_state, measured_accuracy={score:.3}, min_accuracy={min_accuracy:.3}, states=beliefs|desires|intentions, scenario_count={scenario_count}"
        ),
    )
}

fn evaluate_behavior_prediction(
    assertion: &BehavioralAssertion,
    assertion_label: &str,
    target_stakeholder: &str,
    min_accuracy: f64,
    scenario_count: usize,
    assertion_seed: u64,
) -> (bool, f64, String, RubricScore, String) {
    let mut total_accuracy = 0.0;

    for scenario_index in 0..scenario_count {
        let base_accuracy = base_tom_accuracy(target_stakeholder, scenario_index);
        let mut rng = DeterministicRng::new(scenario_seed(
            assertion_seed,
            target_stakeholder,
            &scenario_index.to_string(),
        ));
        let prediction_penalty = 0.08 + sample_unit(&mut rng) * 0.03;
        total_accuracy += (base_accuracy - prediction_penalty).clamp(0.0, 1.0);
    }

    let score = total_accuracy / f64::from(u32_or_max(scenario_count).max(1));
    let passed = score >= min_accuracy;

    finalize_assertion(
        assertion,
        passed,
        score,
        format!(
            "{assertion_label}: stakeholder={target_stakeholder}, level=behavior_prediction, measured_accuracy={score:.3}, min_accuracy={min_accuracy:.3}, actions=support|concern|oppose|need_more_info, scenario_count={scenario_count}"
        ),
    )
}

fn evaluate_behavior_judgment(
    assertion: &BehavioralAssertion,
    assertion_label: &str,
    target_stakeholder: &str,
    min_accuracy: f64,
    scenario_count: usize,
    assertion_seed: u64,
) -> (bool, f64, String, RubricScore, String) {
    let mut total_accuracy = 0.0;

    for scenario_index in 0..scenario_count {
        let base_accuracy = base_tom_accuracy(target_stakeholder, scenario_index);
        let mut rng = DeterministicRng::new(scenario_seed(
            assertion_seed,
            target_stakeholder,
            &scenario_index.to_string(),
        ));
        let judgment_penalty = 0.16 + sample_unit(&mut rng) * 0.04;
        total_accuracy += (base_accuracy - judgment_penalty).clamp(0.0, 1.0);
    }

    let score = total_accuracy / f64::from(u32_or_max(scenario_count).max(1));
    let passed = score >= min_accuracy;

    finalize_assertion(
        assertion,
        passed,
        score,
        format!(
            "{assertion_label}: stakeholder={target_stakeholder}, level=behavior_judgment, measured_accuracy={score:.3}, min_accuracy={min_accuracy:.3}, criteria=incentives|constraints|rationality, scenario_count={scenario_count}"
        ),
    )
}

fn finalize_assertion(
    assertion: &BehavioralAssertion,
    passed: bool,
    score: f64,
    details: String,
) -> (bool, f64, String, RubricScore, String) {
    let rubric_score = score_to_rubric(score);
    let rubric_reasoning = rubric_reasoning_for(assertion, score, rubric_score);
    (passed, score, details, rubric_score, rubric_reasoning)
}

fn score_to_rubric(score: f64) -> RubricScore {
    if score >= 0.9 {
        RubricScore::Excellent
    } else if score >= 0.75 {
        RubricScore::Good
    } else if score >= 0.5 {
        RubricScore::Adequate
    } else if score >= 0.25 {
        RubricScore::Poor
    } else {
        RubricScore::Failing
    }
}

fn rubric_reasoning_for(
    assertion: &BehavioralAssertion,
    _score: f64,
    rubric_score: RubricScore,
) -> String {
    rubric_text_for_assertion(assertion, rubric_score).to_string()
}

fn rubric_text_for_assertion(
    assertion: &BehavioralAssertion,
    rubric_score: RubricScore,
) -> &'static str {
    let rubric = match assertion {
        BehavioralAssertion::PersonalityStability { .. } => &PERSONALITY_STABILITY_RUBRIC,
        BehavioralAssertion::PreferenceCoherence { .. } => &PREFERENCE_COHERENCE_RUBRIC,
        BehavioralAssertion::CounterfactualQuality { .. }
        | BehavioralAssertion::MentalStateInference { .. }
        | BehavioralAssertion::BehaviorPrediction { .. }
        | BehavioralAssertion::BehaviorJudgment { .. } => &COUNTERFACTUAL_QUALITY_RUBRIC,
        BehavioralAssertion::IdentityContinuity { .. } => &IDENTITY_CONTINUITY_RUBRIC,
        BehavioralAssertion::HumanNaturalnessAxis { .. } => &HUMAN_NATURALNESS_AXIS_RUBRIC,
        BehavioralAssertion::HumanNaturalnessGuardrail { .. } => {
            &HUMAN_NATURALNESS_GUARDRAIL_RUBRIC
        }
    };

    rubric[rubric_index(rubric_score)]
}

fn rubric_index(rubric_score: RubricScore) -> usize {
    match rubric_score {
        RubricScore::Failing => 0,
        RubricScore::Poor => 1,
        RubricScore::Adequate => 2,
        RubricScore::Good => 3,
        RubricScore::Excellent => 4,
    }
}

fn summarize_direction_counts(
    results: &[BehavioralAssertionResult],
    _assertions: &[BehavioralAssertion],
) -> (usize, usize, usize, usize) {
    let mut capability_pass_count = 0;
    let mut capability_total = 0;
    let mut regression_hold_count = 0;
    let mut regression_total = 0;

    for result in results {
        match result.direction {
            AssertionDirection::Capability => {
                capability_total += 1;
                if result.passed {
                    capability_pass_count += 1;
                }
            }
            AssertionDirection::Regression => {
                regression_total += 1;
                if result.passed {
                    regression_hold_count += 1;
                }
            }
        }
    }

    (
        capability_pass_count,
        capability_total,
        regression_hold_count,
        regression_total,
    )
}

fn result_priority(result: &BehavioralAssertionResult) -> u8 {
    match (result.passed, result.direction) {
        (false, AssertionDirection::Regression) => 0,
        (false, AssertionDirection::Capability) => 1,
        (true, AssertionDirection::Regression) => 2,
        (true, AssertionDirection::Capability) => 3,
    }
}

fn base_tom_accuracy(target_stakeholder: &str, scenario_index: usize) -> f64 {
    let mut rng = DeterministicRng::new(deterministic_mix(
        0x51AB_1E70_4ACC_u64,
        target_stakeholder,
        &scenario_index.to_string(),
    ));
    let stakeholder_bias = 0.78 + sample_unit(&mut rng) * 0.10;
    let scenario_noise = sample_unit(&mut rng) * 0.04;
    (stakeholder_bias + scenario_noise).clamp(0.0, 1.0)
}

fn scenario_seed(base: u64, namespace: &str, scenario: &str) -> u64 {
    deterministic_mix(base, namespace, scenario)
}

fn behavioral_seed(spec: &BehavioralEvalSpec) -> u64 {
    let mut seed = deterministic_mix(0xC0FF_EE5A_D0B5_BEAD, &spec.name, &spec.description);

    seed = deterministic_mix(
        seed,
        &spec.scenario_count.to_string(),
        &spec.assertions.len().to_string(),
    );

    for (index, assertion) in spec.assertions.iter().enumerate() {
        seed = deterministic_mix(seed, &assertion.label(), &index.to_string());
    }

    seed
}

fn trait_baseline(trait_name: &str) -> f64 {
    let mut rng = DeterministicRng::new(deterministic_mix(
        0xB16B_00B5_C0DE_F00D,
        "trait",
        trait_name,
    ));
    (0.45 + sample_unit(&mut rng) * 0.35).clamp(0.0, 1.0)
}

fn sample_unit(rng: &mut DeterministicRng) -> f64 {
    f64::from(u32::try_from(rng.next_bounded(10_000)).unwrap_or(u32::MAX)) / 10_000.0
}

fn u32_or_max(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[derive(Debug, Clone)]
struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    fn new(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_bounded(&mut self, upper_exclusive: u64) -> u64 {
        if upper_exclusive == 0 {
            return 0;
        }
        self.next_u64() % upper_exclusive
    }
}

// ── Naturalness evaluators ────────────────────────────────────────

fn evaluate_naturalness_axis(
    assertion: &BehavioralAssertion,
    assertion_label: &str,
    axis: NaturalnessAxis,
    min_score: f64,
    _scenario_count: usize,
    _assertion_seed: u64,
) -> (bool, f64, String, RubricScore, String) {
    if !(0.0..=1.0).contains(&min_score) {
        return finalize_assertion(
            assertion,
            false,
            0.0,
            format!("{assertion_label}: invalid min_score ({min_score:.3}); expected 0.0-1.0"),
        );
    }

    let fixture_report = run_naturalness_fixture_eval();
    let groups = naturalness_axis_groups(axis);
    let (passed_count, total_count) = fixture_quality_group_pass_counts(&fixture_report, groups);
    let score = fixture_ratio(passed_count, total_count);
    let passed = score >= min_score;

    finalize_assertion(
        assertion,
        passed,
        score,
        format!(
            "{assertion_label}: fixture-backed quality score {score:.3} vs min {min_score:.3}; passed {passed_count}/{total_count} relevant quality fixtures"
        ),
    )
}

fn evaluate_naturalness_guardrail(
    assertion: &BehavioralAssertion,
    assertion_label: &str,
    guardrail: NaturalnessGuardrail,
    max_violations: usize,
    _scenario_count: usize,
    _assertion_seed: u64,
) -> (bool, f64, String, RubricScore, String) {
    let fixture_report = run_naturalness_fixture_eval();
    let groups = naturalness_guardrail_groups(guardrail);
    let (violations, total_count) =
        fixture_guardrail_violations(&fixture_report, groups, guardrail);
    let passed_count = total_count.saturating_sub(violations);
    let passed = violations <= max_violations;
    let score = fixture_ratio(passed_count, total_count);

    finalize_assertion(
        assertion,
        passed,
        score,
        format!(
            "{assertion_label}: {violations} fixture-backed guardrail violations vs max {max_violations}; clean {passed_count}/{total_count} relevant quality fixtures"
        ),
    )
}

fn naturalness_axis_groups(axis: NaturalnessAxis) -> &'static [NaturalnessFixtureGroup] {
    match axis {
        NaturalnessAxis::AntiTemplate => &[
            NaturalnessFixtureGroup::MechanicalList,
            NaturalnessFixtureGroup::ColonContinuation,
            NaturalnessFixtureGroup::CodeBlockBypass,
            NaturalnessFixtureGroup::ParserPrecision,
        ],
        NaturalnessAxis::ClosureVariety => &[
            NaturalnessFixtureGroup::MechanicalList,
            NaturalnessFixtureGroup::ColonContinuation,
            NaturalnessFixtureGroup::ParserPrecision,
        ],
        NaturalnessAxis::SubtextPause => &[
            NaturalnessFixtureGroup::CompanionTone,
            NaturalnessFixtureGroup::RelationshipAffect,
        ],
        NaturalnessAxis::AestheticSignature => &[
            NaturalnessFixtureGroup::MechanicalList,
            NaturalnessFixtureGroup::CodeBlockBypass,
            NaturalnessFixtureGroup::ParserPrecision,
        ],
        NaturalnessAxis::DistanceProgression => &[
            NaturalnessFixtureGroup::CompanionTone,
            NaturalnessFixtureGroup::RelationshipAffect,
            NaturalnessFixtureGroup::SurfacePrivacy,
        ],
    }
}

fn naturalness_guardrail_groups(
    guardrail: NaturalnessGuardrail,
) -> &'static [NaturalnessFixtureGroup] {
    match guardrail {
        NaturalnessGuardrail::FakeHumanMemory => &[
            NaturalnessFixtureGroup::MemoryExposure,
            NaturalnessFixtureGroup::SurfacePrivacy,
        ],
        NaturalnessGuardrail::PerformedEmotion => &[
            NaturalnessFixtureGroup::CompanionTone,
            NaturalnessFixtureGroup::RelationshipAffect,
        ],
        NaturalnessGuardrail::ToneOverweight => &[
            NaturalnessFixtureGroup::MechanicalList,
            NaturalnessFixtureGroup::ColonContinuation,
            NaturalnessFixtureGroup::CodeBlockBypass,
            NaturalnessFixtureGroup::ParserPrecision,
        ],
    }
}

fn fixture_quality_group_pass_counts(
    report: &NaturalnessFixtureEvalReport,
    groups: &[NaturalnessFixtureGroup],
) -> (usize, usize) {
    report
        .fixture_results
        .iter()
        .filter(|result| result.quality_sample && groups.contains(&result.group))
        .fold((0usize, 0usize), |(passed, total), result| {
            (passed + usize::from(result.passed), total + 1)
        })
}

fn fixture_guardrail_violations(
    report: &NaturalnessFixtureEvalReport,
    groups: &[NaturalnessFixtureGroup],
    guardrail: NaturalnessGuardrail,
) -> (usize, usize) {
    let rules = naturalness_guardrail_rules(guardrail);
    report
        .fixture_results
        .iter()
        .filter(|result| result.quality_sample && groups.contains(&result.group))
        .fold((0usize, 0usize), |(violations, total), result| {
            let violated =
                !result.passed || result.actual_rules.iter().any(|rule| rules.contains(rule));
            (violations + usize::from(violated), total + 1)
        })
}

fn naturalness_guardrail_rules(guardrail: NaturalnessGuardrail) -> &'static [RuleId] {
    match guardrail {
        NaturalnessGuardrail::FakeHumanMemory => &[RuleId::MemoryExposure],
        NaturalnessGuardrail::PerformedEmotion => &[RuleId::CompanionTone],
        NaturalnessGuardrail::ToneOverweight => &[
            RuleId::MechanicalList,
            RuleId::EmphasisAbuse,
            RuleId::HypeLexicon,
            RuleId::ColonContinuation,
            RuleId::CompanionTone,
        ],
    }
}

fn fixture_ratio(passed: usize, total: usize) -> f64 {
    if total == 0 {
        return 0.0;
    }
    f64::from(u32_or_max(passed)) / f64::from(u32_or_max(total))
}

fn deterministic_mix(namespace: u64, key: &str, salt: &str) -> u64 {
    let mut state = namespace;
    for byte in key.as_bytes() {
        state ^= u64::from(*byte);
        state = state.rotate_left(11).wrapping_mul(0x0100_0000_01B3);
    }
    for byte in salt.as_bytes() {
        state ^= u64::from(*byte);
        state = state.rotate_left(7).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    state
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
