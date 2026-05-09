//! Integrated model combining self-model, world-model, and
//! relationship state into a unified situational awareness
//! score with action affordances and predicted outcomes.

use crate::core::persona::relationship::RelationshipState;
use crate::core::persona::self_model::SelfModelShadow;
use crate::core::persona::world_model::{ToolReliabilityRecord, WorldModel};

/// Unified situational awareness combining self-, world-, and user-models.
#[derive(Debug, Clone)]
pub(crate) struct IntegratedModel {
    /// Composite awareness score in `[0.0, 1.0]`.
    pub situational_awareness: f64,
    /// Actions the agent can confidently take right now.
    pub action_affordances: Vec<ActionAffordance>,
    /// Optional predicted outcome for the current turn.
    pub predicted_outcome: Option<PredictedOutcome>,
}

/// An action the agent can take with associated confidence.
#[derive(Debug, Clone)]
pub(crate) struct ActionAffordance {
    /// Descriptive label for the available action.
    pub action: String,
    /// Confidence that the action will succeed.
    pub confidence: f64,
    /// How relevant the action is to the current context.
    pub relevance: f64,
}

/// A predicted outcome with an estimated probability.
///
/// # Research Field
///
/// This field is reserved for Phase 5 research and experimentation.
/// Mainline domain logic MUST NOT depend on `predicted_outcome` being populated.
/// The field is always `None` in production builds; it exists as a placeholder
/// for future outcome prediction research and should not be treated as a
/// reliable signal for decision-making.
#[derive(Debug, Clone)]
pub(crate) struct PredictedOutcome {
    /// Human-readable description of the predicted outcome.
    pub description: String,
    /// Estimated probability in `[0.0, 1.0]`.
    pub probability: f64,
}

const W_SELF: f64 = 0.35;
const W_WORLD: f64 = 0.30;
const W_USER: f64 = 0.35;
const RELIABLE_TOOL_THRESHOLD: f64 = 0.7;
const MIN_TOOL_SAMPLES: u32 = 3;

/// Build an integrated model from self-, world-, and relationship state.
#[must_use]
pub(crate) fn build_integrated_model(
    self_model: &SelfModelShadow,
    world_model: &WorldModel,
    relationship: &RelationshipState,
) -> IntegratedModel {
    let sa = self_consistency(self_model) * W_SELF
        + world_consistency(world_model) * W_WORLD
        + user_consistency(relationship) * W_USER;
    IntegratedModel {
        situational_awareness: sa.clamp(0.0, 1.0),
        action_affordances: derive_affordances(world_model),
        predicted_outcome: None, // Phase 5 placeholder
    }
}

fn self_consistency(m: &SelfModelShadow) -> f64 {
    let cap_mean = if m.capability_estimates.is_empty() {
        0.5
    } else {
        let sum: f64 = m.capability_estimates.iter().map(|c| c.success_ema).sum();
        let len = u32::try_from(m.capability_estimates.len()).unwrap_or(u32::MAX);
        sum / f64::from(len)
    };
    (cap_mean * 0.6 + m.continuity_score * 0.4).clamp(0.0, 1.0)
}

fn world_consistency(w: &WorldModel) -> f64 {
    if w.tool_reliability.is_empty() {
        return 0.5;
    }
    let sum: f64 = w
        .tool_reliability
        .iter()
        .map(ToolReliabilityRecord::success_rate)
        .sum();
    let len = u32::try_from(w.tool_reliability.len()).unwrap_or(u32::MAX);
    (sum / f64::from(len)).clamp(0.0, 1.0)
}

fn user_consistency(r: &RelationshipState) -> f64 {
    f64::midpoint(f64::from(r.trust_level), f64::from(r.rapport)).clamp(0.0, 1.0)
}

fn derive_affordances(w: &WorldModel) -> Vec<ActionAffordance> {
    let mut out = Vec::new();
    for tool in &w.tool_reliability {
        let total = tool.success_count + tool.failure_count;
        if tool.success_rate() >= RELIABLE_TOOL_THRESHOLD && total >= MIN_TOOL_SAMPLES {
            out.push(ActionAffordance {
                action: format!("use_{}", tool.tool_name),
                confidence: tool.success_rate(),
                relevance: 0.5,
            });
        }
    }
    if let Some(proj) = &w.active_project {
        out.push(ActionAffordance {
            action: format!("continue_project:{}", proj.language),
            confidence: 0.6,
            relevance: 0.8,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::persona::self_model::{CapabilityEstimate, EpistemicEntry};
    use crate::core::persona::world_model::{ProjectContext, ToolReliabilityRecord};

    fn stub_self() -> SelfModelShadow {
        SelfModelShadow {
            schema_version: 1,
            self_id: "t".into(),
            active_goal: "goal".into(),
            capability_estimates: vec![CapabilityEstimate {
                domain: "general".into(),
                success_ema: 0.75,
                sample_size: 10,
            }],
            uncertainty_register: vec![EpistemicEntry {
                topic: "q".into(),
                confidence: 0.5.into(),
                source: "t".into(),
            }],
            continuity_score: 0.8,
            updated_at: "2026-03-01T00:00:00Z".into(),
        }
    }
    fn stub_world() -> WorldModel {
        WorldModel {
            active_project: Some(ProjectContext {
                language: "Rust".into(),
                framework: Some("actix".into()),
                project_type: "web".into(),
            }),
            tool_reliability: vec![
                ToolReliabilityRecord {
                    tool_name: "shell".into(),
                    success_count: 18,
                    failure_count: 2,
                    avg_duration_ms: 150,
                },
                ToolReliabilityRecord {
                    tool_name: "file_read".into(),
                    success_count: 2,
                    failure_count: 3,
                    avg_duration_ms: 30,
                },
            ],
            ..WorldModel::default()
        }
    }

    #[test]
    fn awareness_bounded_and_affordances_filter() {
        let m = build_integrated_model(&stub_self(), &stub_world(), &RelationshipState::default());
        assert!((0.0..=1.0).contains(&m.situational_awareness));
        let names: Vec<&str> = m
            .action_affordances
            .iter()
            .map(|a| a.action.as_str())
            .collect();
        assert!(names.contains(&"use_shell"));
        assert!(!names.iter().any(|n| n.contains("file_read")));
        assert!(names.iter().any(|n| n.contains("continue_project")));
        assert!(m.predicted_outcome.is_none());
    }

    #[test]
    fn render_block_format() {
        let m = build_integrated_model(&stub_self(), &stub_world(), &RelationshipState::default());
        let block = crate::core::persona::presenter::render_integrated_model_block(&m);
        assert!(block.contains("[Integrated Model]"));
        assert!(block.contains("situational_awareness="));
        assert!(block.contains("predicted_outcome=none"));
    }

    #[test]
    fn empty_world_gives_neutral_awareness() {
        let m = build_integrated_model(
            &stub_self(),
            &WorldModel::default(),
            &RelationshipState::default(),
        );
        assert!(m.situational_awareness > 0.4 && m.situational_awareness < 0.8);
    }
}
