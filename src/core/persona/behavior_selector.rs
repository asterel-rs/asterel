//! Composite behavior selection: combines empathy policy, conversation
//! register, expression depth, and OCEAN trait activation into a single
//! `BehaviorSelection` snapshot consumed by prompt assembly.

use serde::{Deserialize, Serialize};

use crate::contracts::affect::{AffectLabel, AffectReading};
use crate::core::persona::big_five::BigFiveProfile;
use crate::core::persona::continuity_v2::DialogueAct;
use crate::core::persona::empathy_policy::{
    EmpathyPolicyInput, ResponseStyleFamily, select_empathy_response_style,
};
use crate::core::persona::style_profile::StyleProfileState;
use crate::core::persona::trait_activation::{
    CueWeights, TraitActivation, activate_traits, classify_situational_cue, derive_cue_weights,
};
use crate::core::persona::user_model::{UserIntent, UserMentalModel};

use super::relationship::RelationshipState;
use super::soul_core::SoulPressure;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConversationRegister {
    Casual,
    Focused,
    Precise,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExpressionDepth {
    Surface,
    Emerging,
    Deepening,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BehaviorPostureConstraint {
    TruthBoundary,
    MemoryDiscretion,
    Autonomy,
    Wonder,
    Repair,
    LowDefensiveness,
    ConciseRepair,
    PreserveDistance,
    Restraint,
    Care,
}

impl BehaviorPostureConstraint {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::TruthBoundary => "truth_boundary",
            Self::MemoryDiscretion => "memory_discretion",
            Self::Autonomy => "autonomy",
            Self::Wonder => "wonder",
            Self::Repair => "repair",
            Self::LowDefensiveness => "low_defensiveness",
            Self::ConciseRepair => "concise_repair",
            Self::PreserveDistance => "preserve_distance",
            Self::Restraint => "restraint",
            Self::Care => "care",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct BehaviorTrace {
    pub cue_weights: CueWeights,
    pub activated_traits: Vec<String>,
    pub register_reason: String,
    pub expression_depth_cap: f64,
    pub posture_constraints: Vec<BehaviorPostureConstraint>,
    pub suppressed_affects: Vec<String>,
    pub style_source: String,
}

#[derive(Debug, Clone)]
pub(crate) struct BehaviorSelection {
    pub empathy_family: ResponseStyleFamily,
    pub acknowledgment_needed: bool,
    pub register: ConversationRegister,
    pub expression_depth: ExpressionDepth,
    pub expression_depth_score: f64,
    pub primary_cue: crate::core::persona::trait_activation::SituationalCue,
    pub trait_activation: TraitActivation,
    pub empathy_rationale: std::borrow::Cow<'static, str>,
    pub trace: BehaviorTrace,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn select_behavior(
    affect: &AffectReading,
    relationship: Option<&RelationshipState>,
    dialogue_act: DialogueAct,
    big_five: &BigFiveProfile,
    style_profile: Option<&StyleProfileState>,
    user_model: &UserMentalModel,
    tier_config: &crate::config::schema::RelationshipTierConfig,
    activation_config: &crate::config::schema::TraitActivationConfig,
    enable_trait_activation: bool,
    soul_pressure: Option<&SoulPressure>,
) -> BehaviorSelection {
    let (trust, rapport, disclosure_depth, attachment_security, unresolved_tension, repair_debt) =
        relationship.map_or((0.3, 0.3, 0.1, 0.5, 0.0, 0.0), |state| {
            (
                state.trust_level,
                state.rapport,
                state.disclosure_depth,
                state.attachment_security,
                state.unresolved_tension,
                state.repair_debt,
            )
        });

    let empathy_output = select_empathy_response_style(&EmpathyPolicyInput {
        affect_label: affect.label,
        affect_confidence: affect.confidence.get(),
        relationship_trust: trust,
        relationship_rapport: rapport,
        dialogue_act,
    });

    let cue_weights = derive_cue_weights(dialogue_act, user_model, affect.label);
    let primary_cue = classify_situational_cue(dialogue_act, user_model, affect.label);
    let trait_activation = if enable_trait_activation {
        activate_traits(&cue_weights, activation_config)
    } else {
        TraitActivation::default()
    };
    let register = derive_register(dialogue_act, user_model, style_profile, big_five);
    let posture_constraints = derive_posture_constraints(soul_pressure);
    let expression_depth_score = apply_posture_depth_cap(
        smooth_expression_depth(
            trust,
            rapport,
            disclosure_depth,
            attachment_security,
            unresolved_tension,
            repair_debt,
            tier_config,
        ),
        &posture_constraints,
        tier_config,
    );
    let expression_depth = classify_expression_depth(expression_depth_score, tier_config);

    BehaviorSelection {
        empathy_family: empathy_output.style_family,
        acknowledgment_needed: empathy_output.acknowledgment_needed,
        register,
        expression_depth,
        expression_depth_score,
        primary_cue,
        trait_activation: trait_activation.clone(),
        empathy_rationale: empathy_output.empathy_rationale,
        trace: BehaviorTrace {
            cue_weights,
            activated_traits: activated_traits(&trait_activation),
            register_reason: register_reason(dialogue_act, user_model, style_profile, big_five),
            expression_depth_cap: expression_depth_score,
            posture_constraints,
            suppressed_affects: suppressed_affects(affect, expression_depth_score),
            style_source: if style_profile.is_some() {
                "style_profile".to_string()
            } else {
                "character_defaults".to_string()
            },
        },
    }
}

fn derive_posture_constraints(
    soul_pressure: Option<&SoulPressure>,
) -> Vec<BehaviorPostureConstraint> {
    let Some(pressure) = soul_pressure else {
        return Vec::new();
    };
    let mut constraints = Vec::new();
    if pressure.truth >= 0.75 {
        constraints.push(BehaviorPostureConstraint::TruthBoundary);
    }
    if pressure.memory_discretion >= 0.7 {
        constraints.push(BehaviorPostureConstraint::MemoryDiscretion);
    }
    if pressure.autonomy >= 0.65 {
        constraints.push(BehaviorPostureConstraint::Autonomy);
    }
    if pressure.wonder >= 0.75 {
        constraints.push(BehaviorPostureConstraint::Wonder);
    }
    if pressure.repair >= 0.7 {
        push_constraint(&mut constraints, BehaviorPostureConstraint::Repair);
        push_constraint(
            &mut constraints,
            BehaviorPostureConstraint::LowDefensiveness,
        );
        push_constraint(&mut constraints, BehaviorPostureConstraint::ConciseRepair);
        push_constraint(
            &mut constraints,
            BehaviorPostureConstraint::PreserveDistance,
        );
    }
    if pressure.restraint >= 0.7 {
        push_constraint(&mut constraints, BehaviorPostureConstraint::Restraint);
        push_constraint(
            &mut constraints,
            BehaviorPostureConstraint::PreserveDistance,
        );
    }
    if pressure.care >= 0.75 {
        constraints.push(BehaviorPostureConstraint::Care);
    }
    constraints
}

fn push_constraint(
    constraints: &mut Vec<BehaviorPostureConstraint>,
    constraint: BehaviorPostureConstraint,
) {
    if !constraints.contains(&constraint) {
        constraints.push(constraint);
    }
}

fn apply_posture_depth_cap(
    expression_depth_score: f64,
    constraints: &[BehaviorPostureConstraint],
    config: &crate::config::schema::RelationshipTierConfig,
) -> f64 {
    if constraints.iter().any(|constraint| {
        matches!(
            constraint,
            BehaviorPostureConstraint::MemoryDiscretion
                | BehaviorPostureConstraint::Repair
                | BehaviorPostureConstraint::Autonomy
                | BehaviorPostureConstraint::Restraint
                | BehaviorPostureConstraint::PreserveDistance
        )
    }) {
        expression_depth_score.min(config.emerging_max)
    } else {
        expression_depth_score
    }
}

fn derive_register(
    dialogue_act: DialogueAct,
    user_model: &UserMentalModel,
    style_profile: Option<&StyleProfileState>,
    big_five: &BigFiveProfile,
) -> ConversationRegister {
    let formality = style_profile.map_or_else(
        || {
            if big_five.conscientiousness > 0.65 {
                65
            } else if big_five.extraversion < 0.35 {
                60
            } else {
                50
            }
        },
        |profile| profile.formality,
    );

    if matches!(dialogue_act, DialogueAct::Greet | DialogueAct::Thank) {
        return ConversationRegister::Casual;
    }

    match user_model.inferred_intent {
        UserIntent::Debug | UserIntent::Instruct if formality >= 55 => {
            ConversationRegister::Precise
        }
        UserIntent::Learn | UserIntent::Explore => ConversationRegister::Focused,
        UserIntent::Vent => ConversationRegister::Casual,
        _ if formality >= 65 => ConversationRegister::Precise,
        _ => ConversationRegister::Focused,
    }
}

fn smooth_expression_depth(
    trust: f32,
    rapport: f32,
    disclosure_depth: f32,
    attachment_security: f32,
    unresolved_tension: f32,
    repair_debt: f32,
    config: &crate::config::schema::RelationshipTierConfig,
) -> f64 {
    let base = (f64::from(trust) * 0.35
        + f64::from(rapport) * 0.20
        + f64::from(disclosure_depth) * 0.20
        + f64::from(attachment_security) * 0.15
        - f64::from(unresolved_tension) * 0.12
        - f64::from(repair_debt) * 0.18)
        .clamp(0.0, 1.0);
    let smoothed = if base <= config.surface_max {
        base / config.surface_max.max(0.0001) * 0.3
    } else if base <= config.emerging_max {
        0.3 + ((base - config.surface_max) / (config.emerging_max - config.surface_max).max(0.0001))
            * 0.2
    } else if base <= config.deepening_max {
        0.5 + ((base - config.emerging_max)
            / (config.deepening_max - config.emerging_max).max(0.0001))
            * 0.2
    } else {
        0.7 + ((base - config.deepening_max) / (1.0 - config.deepening_max).max(0.0001)) * 0.3
    };
    smoothed.clamp(0.0, 1.0)
}

fn classify_expression_depth(
    score: f64,
    config: &crate::config::schema::RelationshipTierConfig,
) -> ExpressionDepth {
    if score > config.deepening_max {
        ExpressionDepth::Full
    } else if score > config.emerging_max {
        ExpressionDepth::Deepening
    } else if score > config.surface_max {
        ExpressionDepth::Emerging
    } else {
        ExpressionDepth::Surface
    }
}

fn activated_traits(activation: &TraitActivation) -> Vec<String> {
    let mut names = Vec::new();
    if activation.openness > 1.01 {
        names.push("openness".to_string());
    }
    if activation.conscientiousness > 1.01 || activation.conscientiousness < 0.99 {
        names.push("conscientiousness".to_string());
    }
    if activation.extraversion > 1.01 || activation.extraversion < 0.99 {
        names.push("extraversion".to_string());
    }
    if activation.agreeableness > 1.01 || activation.agreeableness < 0.99 {
        names.push("agreeableness".to_string());
    }
    if activation.neuroticism > 1.01 || activation.neuroticism < 0.99 {
        names.push("neuroticism".to_string());
    }
    names
}

fn register_reason(
    dialogue_act: DialogueAct,
    user_model: &UserMentalModel,
    style_profile: Option<&StyleProfileState>,
    big_five: &BigFiveProfile,
) -> String {
    if matches!(dialogue_act, DialogueAct::Greet | DialogueAct::Thank) {
        return "greeting-style turn favors casual register".to_string();
    }
    if matches!(
        user_model.inferred_intent,
        UserIntent::Debug | UserIntent::Instruct
    ) {
        return "task/debug intent favors precise register".to_string();
    }
    if let Some(style) = style_profile {
        return format!(
            "style profile formality={} biases register",
            style.formality
        );
    }
    if big_five.conscientiousness > 0.65 {
        return "high conscientiousness biases focused/precise register".to_string();
    }
    "default focused register".to_string()
}

fn suppressed_affects(affect: &AffectReading, expression_depth_score: f64) -> Vec<String> {
    if expression_depth_score < 0.35
        && matches!(
            affect.label,
            AffectLabel::Sad | AffectLabel::Anxious | AffectLabel::Overwhelmed
        )
    {
        vec![affect.label.as_snake_case().to_string()]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::scores::Confidence;
    use crate::core::persona::user_model::{EmotionalNeed, KnowledgeLevel, UserIntent};

    fn affect(label: AffectLabel) -> AffectReading {
        AffectReading {
            label,
            valence: 0.0,
            arousal: 0.0,
            dominance: 0.5,
            confidence: Confidence::new(0.8),
        }
    }

    fn model(intent: UserIntent) -> UserMentalModel {
        UserMentalModel {
            inferred_intent: intent,
            knowledge_level: KnowledgeLevel::Intermediate,
            emotional_need: EmotionalNeed::Solution,
            active_constraints: Vec::new(),
        }
    }

    fn big_five() -> BigFiveProfile {
        BigFiveProfile {
            openness: 0.8,
            conscientiousness: 0.7,
            extraversion: 0.3,
            agreeableness: 0.75,
            neuroticism: 0.25,
        }
    }

    #[test]
    fn expression_depth_score_is_continuous() {
        let config = crate::config::schema::RelationshipTierConfig::default();
        let left = smooth_expression_depth(0.49, 0.49, 0.3, 0.5, 0.0, 0.0, &config);
        let right = smooth_expression_depth(0.51, 0.51, 0.3, 0.5, 0.0, 0.0, &config);
        assert!((right - left).abs() < 0.1);
    }

    #[test]
    fn selector_produces_trace_and_preserves_boundaries() {
        let selection = select_behavior(
            &affect(AffectLabel::Sad),
            None,
            DialogueAct::Inform,
            &big_five(),
            Some(&StyleProfileState {
                formality: 80,
                verbosity: 25,
                temperature: 0.4,
                updated_at: "2026-04-12T00:00:00Z".to_string(),
            }),
            &model(UserIntent::Learn),
            &crate::config::schema::RelationshipTierConfig::default(),
            &crate::config::schema::TraitActivationConfig::default(),
            true,
            None,
        );

        assert_eq!(selection.empathy_family, ResponseStyleFamily::Empathetic);
        assert!(selection.acknowledgment_needed);
        assert!(!selection.trace.register_reason.is_empty());
        assert_eq!(selection.trace.style_source, "style_profile");
    }

    #[test]
    fn selector_can_disable_trait_activation() {
        let selection = select_behavior(
            &affect(AffectLabel::Curious),
            None,
            DialogueAct::Inform,
            &big_five(),
            None,
            &model(UserIntent::Learn),
            &crate::config::schema::RelationshipTierConfig::default(),
            &crate::config::schema::TraitActivationConfig::default(),
            false,
            None,
        );

        assert_eq!(selection.trait_activation.openness, 1.0);
        assert!(selection.trace.activated_traits.is_empty());
    }

    #[test]
    fn selector_applies_soul_pressure_as_posture_constraints_without_trait_drift() {
        let config = crate::config::schema::RelationshipTierConfig::default();
        let activation = crate::config::schema::TraitActivationConfig::default();
        let pressure = SoulPressure {
            memory_discretion: 0.9,
            repair: 0.8,
            autonomy: 0.7,
            ..SoulPressure::default()
        };
        let selection = select_behavior(
            &affect(AffectLabel::Neutral),
            Some(&RelationshipState {
                trust_level: 0.95,
                rapport: 0.95,
                disclosure_depth: 0.95,
                attachment_security: 0.95,
                unresolved_tension: 0.0,
                repair_debt: 0.0,
                recent_affect_trend: 0.0,
                interaction_count: 20,
                last_interaction: "2026-04-27T00:00:00Z".to_string(),
                notable_events: Vec::new(),
            }),
            DialogueAct::Inform,
            &big_five(),
            None,
            &model(UserIntent::Explore),
            &config,
            &activation,
            false,
            Some(&pressure),
        );

        assert!(selection.expression_depth_score <= config.emerging_max);
        assert!(
            selection
                .trace
                .posture_constraints
                .contains(&BehaviorPostureConstraint::MemoryDiscretion)
        );
        assert!(
            selection
                .trace
                .posture_constraints
                .contains(&BehaviorPostureConstraint::Repair)
        );
        assert_eq!(selection.trait_activation.openness, 1.0);
        assert_eq!(selection.trait_activation.conscientiousness, 1.0);
        assert_eq!(selection.trait_activation.extraversion, 1.0);
        assert_eq!(selection.trait_activation.agreeableness, 1.0);
        assert_eq!(selection.trait_activation.neuroticism, 1.0);
    }

    #[test]
    fn repair_pressure_adds_low_defensiveness_and_distance_posture() {
        let config = crate::config::schema::RelationshipTierConfig::default();
        let activation = crate::config::schema::TraitActivationConfig::default();
        let pressure = SoulPressure {
            repair: 0.9,
            restraint: 0.75,
            ..SoulPressure::default()
        };
        let selection = select_behavior(
            &affect(AffectLabel::Frustrated),
            Some(&RelationshipState {
                trust_level: 0.9,
                rapport: 0.9,
                disclosure_depth: 0.9,
                attachment_security: 0.8,
                unresolved_tension: 0.6,
                repair_debt: 0.7,
                recent_affect_trend: -0.2,
                interaction_count: 20,
                last_interaction: "2026-04-27T00:00:00Z".to_string(),
                notable_events: Vec::new(),
            }),
            DialogueAct::Deny,
            &big_five(),
            None,
            &model(UserIntent::Instruct),
            &config,
            &activation,
            false,
            Some(&pressure),
        );

        assert!(selection.expression_depth_score <= config.emerging_max);
        assert!(
            selection
                .trace
                .posture_constraints
                .contains(&BehaviorPostureConstraint::Repair)
        );
        assert!(
            selection
                .trace
                .posture_constraints
                .contains(&BehaviorPostureConstraint::LowDefensiveness)
        );
        assert!(
            selection
                .trace
                .posture_constraints
                .contains(&BehaviorPostureConstraint::ConciseRepair)
        );
        assert!(
            selection
                .trace
                .posture_constraints
                .contains(&BehaviorPostureConstraint::PreserveDistance)
        );
        assert_eq!(selection.trait_activation.openness, 1.0);
    }
}
