//! Counterfactual reasoning: "What if I had done X instead of Y?"
//!
//! Assesses hypothetical alternative actions by scoring similarity
//! to past experiences and applicable principles.
//!
//! References: [CAUSALITY] Pearl, 2009; [COUNTERFACTUAL-COG] Byrne, 2005. See
//! the public research reference index in the docs site.
#![allow(clippy::cast_precision_loss)]

use serde::{Deserialize, Serialize};

use crate::contracts::scores::Confidence;
use crate::core::experience::distill_types::Principle;
use crate::core::experience::{ExperienceAtom, ExperienceOutcome};

/// A query for counterfactual reasoning: "What if I had done X instead of Y?"
#[derive(Debug, Clone)]
pub(crate) struct CounterfactualQuery {
    /// The action that was actually taken.
    pub actual_action: String,
    /// The hypothetical alternative action.
    pub alternative_action: String,
    /// Context of the decision point.
    pub context: String,
}

/// Assessment of what might have happened with an alternative action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CounterfactualAssessment {
    /// Estimated outcome had the alternative been chosen.
    pub estimated_outcome: EstimatedOutcome,
    /// Confidence in the assessment (lower when data is sparse).
    pub confidence: Confidence,
    /// Reasoning chain.
    pub reasoning: String,
    /// Supporting evidence from experience/principles.
    pub evidence: Vec<String>,
}

/// Estimated outcome direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EstimatedOutcome {
    /// The alternative would likely have produced a better result.
    Improved,
    /// The alternative would likely have produced a similar result.
    Similar,
    /// The alternative would likely have produced a worse result.
    Worsened,
    /// Insufficient evidence to estimate the outcome direction.
    Uncertain,
}

/// Assess a counterfactual query using pattern matching on past experiences
/// and distilled principles.
pub(crate) fn assess_counterfactual(
    query: &CounterfactualQuery,
    experiences: &[ExperienceAtom],
    principles: &[Principle],
) -> CounterfactualAssessment {
    let mut evidence = Vec::new();
    let mut scores: Vec<f64> = Vec::new();

    // Check if principles suggest the alternative would have been better.
    for principle in principles {
        let lower = principle.statement.to_lowercase();
        let alt_lower = query.alternative_action.to_lowercase();

        if alt_lower
            .split_whitespace()
            .any(|w| w.len() > 4 && lower.contains(w))
        {
            let score = if principle.category
                == crate::core::experience::distill_types::PrincipleCategory::Constraint
            {
                // Constraint violated → alternative might be better.
                0.7
            } else {
                principle.confidence.get() * 0.6
            };
            scores.push(score);
            evidence.push(format!(
                "Principle [{:.2}]: {}",
                principle.confidence,
                truncate(&principle.statement, 80),
            ));
        }
    }

    // Check if past experiences with similar context had different outcomes.
    let context_lower = query.context.to_lowercase();
    for experience in experiences.iter().take(20) {
        let summary_lower = experience.summary.to_lowercase();
        if context_lower
            .split_whitespace()
            .any(|w| w.len() > 4 && summary_lower.contains(w))
        {
            let score = match experience.outcome {
                ExperienceOutcome::Success => 0.3, // similar context succeeded before
                ExperienceOutcome::Failure => -0.3, // similar context failed
                ExperienceOutcome::Partial | ExperienceOutcome::Unknown => 0.0,
            };
            scores.push(score);
            evidence.push(format!(
                "Experience [{:?}]: {}",
                experience.outcome,
                truncate(&experience.summary, 80),
            ));
        }
    }

    if scores.is_empty() {
        return CounterfactualAssessment {
            estimated_outcome: EstimatedOutcome::Uncertain,
            confidence: Confidence::new(0.1),
            reasoning: "Insufficient data to assess counterfactual.".to_string(),
            evidence,
        };
    }

    let avg_score = scores.iter().sum::<f64>() / scores.len() as f64;
    let confidence = (0.3 + scores.len() as f64 * 0.1).min(0.85);

    let estimated_outcome = if avg_score > 0.3 {
        EstimatedOutcome::Improved
    } else if avg_score < -0.3 {
        EstimatedOutcome::Worsened
    } else {
        EstimatedOutcome::Similar
    };

    let reasoning = format!(
        "Based on {} evidence points (avg_score={:.2}), the alternative '{}' \
         would likely have {} compared to '{}'.",
        scores.len(),
        avg_score,
        query.alternative_action,
        match estimated_outcome {
            EstimatedOutcome::Improved => "improved the outcome",
            EstimatedOutcome::Worsened => "worsened the outcome",
            EstimatedOutcome::Similar => "had a similar outcome",
            EstimatedOutcome::Uncertain => "had an uncertain outcome",
        },
        query.actual_action,
    );

    CounterfactualAssessment {
        estimated_outcome,
        confidence: Confidence::new(confidence),
        reasoning,
        evidence,
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let end = s.char_indices().nth(max).map_or(s.len(), |(idx, _)| idx);
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::experience::distill_types::PrincipleCategory;
    use crate::core::experience::{ExperienceAtom, ExperienceKind, ExperienceOutcome};

    fn test_query() -> CounterfactualQuery {
        CounterfactualQuery {
            actual_action: "used standard reasoning".into(),
            alternative_action: "used stepwise verification".into(),
            context: "complex debugging task".into(),
        }
    }

    #[test]
    fn insufficient_data_returns_uncertain() {
        let assessment = assess_counterfactual(&test_query(), &[], &[]);
        assert_eq!(assessment.estimated_outcome, EstimatedOutcome::Uncertain);
        assert!(assessment.confidence < Confidence::new(0.2));
    }

    #[test]
    fn principle_evidence_contributes_to_assessment() {
        let principles = vec![Principle {
            id: "p1".into(),
            category: PrincipleCategory::Strategy,
            statement: "Stepwise verification improves complex tasks".into(),
            confidence: Confidence::new(0.8),
            source_experience_ids: vec![],
            validation_count: 3,
            created_at: String::new(),
            domain: None,
            q_value: 0.0,
            times_applied: 0,
        }];
        let assessment = assess_counterfactual(&test_query(), &[], &principles);
        assert!(!assessment.evidence.is_empty());
        assert!(assessment.confidence > Confidence::new(0.1));
    }

    #[test]
    fn experience_evidence_contributes_to_assessment() {
        let experiences = vec![
            ExperienceAtom::new(
                ExperienceKind::TurnInteraction,
                "complex debugging with verification",
                ExperienceOutcome::Success,
            )
            .with_confidence(0.8),
        ];
        let assessment = assess_counterfactual(&test_query(), &experiences, &[]);
        assert!(!assessment.evidence.is_empty());
    }
}
