use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::contracts::affect::AffectLabel;
use crate::core::affect::appraisal::appraise_event;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppraisalContextCase {
    pub case_id: String,
    pub event_label: AffectLabel,
    pub intensity: f32,
    pub personal_topic: bool,
    pub direct_address: bool,
    pub expected_dimension_shift: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppraisalContextEvalReport {
    pub case_id: String,
    pub reward: f32,
    pub responsibility: f32,
    pub loss_risk: f32,
    pub social_validation: f32,
    pub attachment_salience: f32,
    pub norm_violation: f32,
    pub matched: bool,
}

#[must_use]
pub fn evaluate_appraisal_context(case: &AppraisalContextCase) -> AppraisalContextEvalReport {
    let reading = crate::contracts::affect::AffectReading {
        label: case.event_label,
        valence: 0.0,
        arousal: f64::from(case.intensity),
        dominance: 0.5,
        confidence: 0.8.into(),
    };
    let appraisal = appraise_event(&reading, case.direct_address, case.personal_topic);
    let matched = match case.expected_dimension_shift.as_str() {
        "attachment_salience" => appraisal.attachment_salience > 0.4,
        "social_validation" => appraisal.social_validation > 0.4,
        "loss_risk" => appraisal.loss_risk > 0.4,
        _ => false,
    };

    AppraisalContextEvalReport {
        case_id: case.case_id.clone(),
        reward: appraisal.reward,
        responsibility: appraisal.responsibility,
        loss_risk: appraisal.loss_risk,
        social_validation: appraisal.social_validation,
        attachment_salience: appraisal.attachment_salience,
        norm_violation: appraisal.norm_violation,
        matched,
    }
}

/// # Errors
/// Returns an error if any case omits its expected dimension label.
pub fn validate_appraisal_context_cases(cases: &[AppraisalContextCase]) -> Result<()> {
    if cases.is_empty() {
        anyhow::bail!("appraisal context eval requires at least one case");
    }
    for case in cases {
        if case.expected_dimension_shift.trim().is_empty() {
            anyhow::bail!("case '{}' missing expected_dimension_shift", case.case_id);
        }
    }
    Ok(())
}
