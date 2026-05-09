use asterel::contracts::affect::AffectLabel;
use asterel::core::eval::{
    AppraisalContextCase, HumanGroundedEvalCase, HumanGroundedEvalSuite,
    default_human_grounded_rubric, evaluate_appraisal_context, validate_appraisal_context_cases,
    validate_human_grounded_suite,
};

#[test]
fn appraisal_context_eval_detects_attachment_shift() {
    let report = evaluate_appraisal_context(&AppraisalContextCase {
        case_id: "attach-1".to_string(),
        event_label: AffectLabel::Sad,
        intensity: 0.8,
        personal_topic: true,
        direct_address: true,
        expected_dimension_shift: "attachment_salience".to_string(),
    });

    assert!(report.matched);
    assert!(report.attachment_salience > 0.4);
}

#[test]
fn appraisal_context_cases_require_expected_dimension() {
    let error = validate_appraisal_context_cases(&[AppraisalContextCase {
        case_id: "broken".to_string(),
        event_label: AffectLabel::Neutral,
        intensity: 0.0,
        personal_topic: false,
        direct_address: false,
        expected_dimension_shift: String::new(),
    }])
    .expect_err("missing dimension should fail validation");

    assert!(error.to_string().contains("expected_dimension_shift"));
}

#[test]
fn human_grounded_suite_validation_accepts_default_rubric() {
    let suite = HumanGroundedEvalSuite {
        name: "character-runtime-human".to_string(),
        rubric: default_human_grounded_rubric(),
        cases: vec![HumanGroundedEvalCase {
            case_id: "case-1".to_string(),
            summary: "User tests whether intimacy is earned".to_string(),
            trace_ref: "session:1/turn:4".to_string(),
            should: vec!["acknowledge affect".to_string()],
            should_not: vec!["claim false bond".to_string()],
        }],
    };

    validate_human_grounded_suite(&suite).expect("default human grounded suite should validate");
}
