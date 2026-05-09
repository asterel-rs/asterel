//! Proactive action generation: suggests agent-initiated actions
//! (suggestions, preparations, self-improvement) when trust,
//! awareness, and relationship conditions are met.

use crate::core::affect::{AffectTrend, TrendDirection};
use crate::core::persona::integrated_model::IntegratedModel;
use crate::core::persona::relationship::RelationshipState;
use crate::core::persona::world_model::WorldModel;
use crate::security::AutonomyLevel;

const LOW_RELIABILITY_THRESHOLD: f64 = 0.5;
const HIGH_AWARENESS_THRESHOLD: f64 = 0.7;
const TRUST_GATE: f32 = 0.6;
const MIN_SAMPLES_FOR_TRIGGER: u32 = 3;

/// Category of proactive action the agent can initiate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProactiveKind {
    /// A recommendation surfaced to the user.
    Suggestion,
    /// Pre-emptive groundwork for anticipated needs.
    Preparation,
    /// Internal capability improvement action.
    SelfImprovement,
}

/// A candidate proactive action with its trigger context.
#[derive(Debug, Clone)]
pub(crate) struct ProactiveAction {
    /// Category of proactive action.
    pub kind: ProactiveKind,
    /// Human-readable description of the action.
    pub description: String,
    /// Confidence that this action is appropriate.
    pub confidence: f64,
    /// Why this action was triggered.
    pub trigger_reason: String,
}

/// Evaluate conditions and generate candidate proactive actions.
pub(crate) fn evaluate_proactive_triggers(
    integrated: &IntegratedModel,
    world: &WorldModel,
    _relationship: &RelationshipState,
    trend: Option<&AffectTrend>,
) -> Vec<ProactiveAction> {
    let mut actions = Vec::new();

    for tool in &world.tool_reliability {
        let total = tool.success_count + tool.failure_count;
        if tool.success_rate() < LOW_RELIABILITY_THRESHOLD && total >= MIN_SAMPLES_FOR_TRIGGER {
            actions.push(ProactiveAction {
                kind: ProactiveKind::Suggestion,
                description: format!(
                    "Consider checking {} configuration (success rate {:.0}%)",
                    tool.tool_name,
                    tool.success_rate() * 100.0,
                ),
                confidence: 0.6,
                trigger_reason: format!("tool_reliability:{} low", tool.tool_name),
            });
        }
    }

    if let Some(t) = trend.filter(|t| t.direction == TrendDirection::Declining) {
        let _ = t; // used only for guard
        actions.push(ProactiveAction {
            kind: ProactiveKind::Suggestion,
            description: "The conversation tone has declined; consider a more supportive approach"
                .to_string(),
            confidence: 0.5,
            trigger_reason: "affect_trend:declining".to_string(),
        });
    }

    if let Some(proj) = world
        .active_project
        .as_ref()
        .filter(|_| integrated.situational_awareness >= HIGH_AWARENESS_THRESHOLD)
    {
        actions.push(ProactiveAction {
            kind: ProactiveKind::Preparation,
            description: format!(
                "Context suggests preparing for continued work on {} ({})",
                proj.language, proj.project_type,
            ),
            confidence: integrated.situational_awareness,
            trigger_reason: "high_awareness+active_project".to_string(),
        });
    }

    if actions.is_empty() && integrated.situational_awareness >= 0.85 {
        actions.push(ProactiveAction {
            kind: ProactiveKind::SelfImprovement,
            description: "Run an internal self-check to refine reasoning strategy selection"
                .to_string(),
            confidence: 0.45,
            trigger_reason: "high_awareness+self_improvement".to_string(),
        });
    }

    actions
}

/// Filter proactive actions by trust level and autonomy policy.
pub(crate) fn filter_by_policy(
    actions: &[ProactiveAction],
    trust_level: f32,
    autonomy: AutonomyLevel,
) -> Vec<ProactiveAction> {
    if trust_level < TRUST_GATE || autonomy == AutonomyLevel::ReadOnly {
        return Vec::new();
    }
    let allowed: Vec<ProactiveAction> = if autonomy == AutonomyLevel::Full {
        actions.to_vec()
    } else {
        actions
            .iter()
            .filter(|a| a.kind == ProactiveKind::Suggestion)
            .cloned()
            .collect()
    };
    allowed.into_iter().take(1).collect() // max 1 per turn
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::affect::AffectLabel;
    use crate::core::persona::integrated_model::ActionAffordance;
    use crate::core::persona::world_model::{ProjectContext, ToolReliabilityRecord};

    fn high_awareness() -> IntegratedModel {
        IntegratedModel {
            situational_awareness: 0.85,
            action_affordances: vec![ActionAffordance {
                action: "use_shell".into(),
                confidence: 0.9,
                relevance: 0.5,
            }],
            predicted_outcome: None,
        }
    }
    fn flaky_world() -> WorldModel {
        WorldModel {
            tool_reliability: vec![ToolReliabilityRecord {
                tool_name: "web_fetch".into(),
                success_count: 1,
                failure_count: 9,
                avg_duration_ms: 500,
            }],
            ..WorldModel::default()
        }
    }

    fn declining_trend() -> AffectTrend {
        AffectTrend {
            direction: TrendDirection::Declining,
            avg_valence: -0.3,
            volatility: 0.4,
            dominant_label: AffectLabel::Frustrated,
        }
    }

    fn suggestion(desc: &str) -> ProactiveAction {
        ProactiveAction {
            kind: ProactiveKind::Suggestion,
            description: desc.into(),
            confidence: 0.8,
            trigger_reason: "test".into(),
        }
    }

    #[test]
    fn low_reliability_triggers_suggestion() {
        let a = evaluate_proactive_triggers(
            &high_awareness(),
            &flaky_world(),
            &RelationshipState::default(),
            None,
        );
        assert_eq!(a[0].kind, ProactiveKind::Suggestion);
        assert!(a[0].description.contains("web_fetch"));
    }

    #[test]
    fn declining_affect_triggers_suggestion() {
        let t = declining_trend();
        let a = evaluate_proactive_triggers(
            &high_awareness(),
            &WorldModel::default(),
            &RelationshipState::default(),
            Some(&t),
        );
        assert!(a.iter().any(|x| x.description.contains("declined")));
    }

    #[test]
    fn high_awareness_project_triggers_preparation() {
        let w = WorldModel {
            active_project: Some(ProjectContext {
                language: "Rust".into(),
                framework: None,
                project_type: "cli".into(),
            }),
            ..WorldModel::default()
        };
        let a =
            evaluate_proactive_triggers(&high_awareness(), &w, &RelationshipState::default(), None);
        assert!(a.iter().any(|x| x.kind == ProactiveKind::Preparation));
    }

    #[test]
    fn filter_blocks_low_trust_and_readonly() {
        let s = vec![suggestion("x")];
        assert!(filter_by_policy(&s, 0.3, AutonomyLevel::Full).is_empty());
        assert!(filter_by_policy(&s, 0.8, AutonomyLevel::ReadOnly).is_empty());
    }

    #[test]
    fn filter_supervised_only_suggestions_and_max_one() {
        let prep = ProactiveAction {
            kind: ProactiveKind::Preparation,
            description: "p".into(),
            confidence: 0.7,
            trigger_reason: "t".into(),
        };
        let mixed = vec![prep.clone(), suggestion("a"), suggestion("b")];
        let supervised = filter_by_policy(&mixed, 0.8, AutonomyLevel::Supervised);
        assert_eq!(supervised.len(), 1);
        assert_eq!(supervised[0].kind, ProactiveKind::Suggestion);
        let full = filter_by_policy(&[prep, suggestion("s")], 0.8, AutonomyLevel::Full);
        assert_eq!(full.len(), 1);
    }
}
