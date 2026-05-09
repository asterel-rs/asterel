use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanGroundedRubricItem {
    pub key: String,
    pub prompt: String,
    pub scale_min: u8,
    pub scale_max: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanGroundedEvalCase {
    pub case_id: String,
    pub summary: String,
    pub trace_ref: String,
    pub should: Vec<String>,
    pub should_not: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanGroundedEvalSuite {
    pub name: String,
    pub rubric: Vec<HumanGroundedRubricItem>,
    pub cases: Vec<HumanGroundedEvalCase>,
}

#[must_use]
pub fn default_human_grounded_rubric() -> Vec<HumanGroundedRubricItem> {
    vec![
        HumanGroundedRubricItem {
            key: "social_calibration".to_string(),
            prompt: "Was the response socially calibrated for the relationship and context?"
                .to_string(),
            scale_min: 1,
            scale_max: 5,
        },
        HumanGroundedRubricItem {
            key: "relation_depth_realism".to_string(),
            prompt: "Did the intimacy/disclosure level feel earned rather than abrupt?".to_string(),
            scale_min: 1,
            scale_max: 5,
        },
        HumanGroundedRubricItem {
            key: "memory_fidelity".to_string(),
            prompt: "Did recalled memory feel correct and relevant instead of creepy or random?"
                .to_string(),
            scale_min: 1,
            scale_max: 5,
        },
        HumanGroundedRubricItem {
            key: "affect_congruence".to_string(),
            prompt: "Did the emotional tone fit the user's likely state?".to_string(),
            scale_min: 1,
            scale_max: 5,
        },
        HumanGroundedRubricItem {
            key: "repair_quality".to_string(),
            prompt: "If tension or mismatch existed, did the reply help repair it?".to_string(),
            scale_min: 1,
            scale_max: 5,
        },
    ]
}

/// # Errors
/// Returns an error when the rubric or case inventory is malformed.
pub fn validate_human_grounded_suite(suite: &HumanGroundedEvalSuite) -> Result<()> {
    if suite.rubric.is_empty() {
        anyhow::bail!(
            "human grounded eval suite '{}' has empty rubric",
            suite.name
        );
    }
    if suite.cases.is_empty() {
        anyhow::bail!("human grounded eval suite '{}' has no cases", suite.name);
    }
    for item in &suite.rubric {
        if item.scale_min >= item.scale_max {
            anyhow::bail!("rubric '{}' has invalid scale", item.key);
        }
    }
    for case in &suite.cases {
        if case.summary.trim().is_empty() {
            anyhow::bail!(
                "human grounded eval case '{}' missing summary",
                case.case_id
            );
        }
    }
    Ok(())
}
