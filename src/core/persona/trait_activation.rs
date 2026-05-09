//! Situational cue weighting and OCEAN trait activation multipliers.

use serde::{Deserialize, Serialize};

use crate::config::schema::TraitActivationConfig;
use crate::contracts::affect::AffectLabel;
use crate::core::persona::continuity_v2::DialogueAct;
use crate::core::persona::user_model::{UserIntent, UserMentalModel};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SituationalCue {
    Emotional,
    Intellectual,
    Conflict,
    Creative,
    Crisis,
    Routine,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct CueWeights {
    pub emotional: f32,
    pub intellectual: f32,
    pub conflict: f32,
    pub creative: f32,
    pub crisis: f32,
    pub routine: f32,
}

impl Default for CueWeights {
    fn default() -> Self {
        Self {
            emotional: 0.0,
            intellectual: 0.0,
            conflict: 0.0,
            creative: 0.0,
            crisis: 0.0,
            routine: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct TraitActivation {
    pub openness: f64,
    pub conscientiousness: f64,
    pub extraversion: f64,
    pub agreeableness: f64,
    pub neuroticism: f64,
}

impl Default for TraitActivation {
    fn default() -> Self {
        Self {
            openness: 1.0,
            conscientiousness: 1.0,
            extraversion: 1.0,
            agreeableness: 1.0,
            neuroticism: 1.0,
        }
    }
}

pub(crate) fn derive_cue_weights(
    dialogue_act: DialogueAct,
    user_model: &UserMentalModel,
    affect_label: AffectLabel,
) -> CueWeights {
    let mut weights = CueWeights {
        routine: 0.35,
        ..CueWeights::default()
    };

    match dialogue_act {
        DialogueAct::Apologize | DialogueAct::Thank => weights.emotional += 0.6,
        DialogueAct::Deny => weights.conflict += 0.7,
        DialogueAct::Question => weights.intellectual += 0.35,
        DialogueAct::Request => weights.creative += 0.25,
        DialogueAct::Inform => weights.intellectual += 0.2,
        _ => {}
    }

    match user_model.inferred_intent {
        UserIntent::Vent => {
            weights.crisis += 0.7;
            weights.emotional += 0.3;
        }
        UserIntent::Debug | UserIntent::Learn => weights.intellectual += 0.45,
        UserIntent::Explore => {
            weights.intellectual += 0.30;
            weights.creative += 0.35;
        }
        UserIntent::Instruct => {
            weights.intellectual += 0.30;
            weights.conflict += 0.10;
        }
    }

    match affect_label {
        AffectLabel::Sad | AffectLabel::Anxious | AffectLabel::Overwhelmed => {
            weights.emotional += 0.45;
            weights.crisis += 0.25;
        }
        AffectLabel::Frustrated | AffectLabel::Angry => weights.conflict += 0.45,
        AffectLabel::Curious => weights.intellectual += 0.35,
        AffectLabel::Excited | AffectLabel::Grateful => weights.creative += 0.20,
        AffectLabel::Neutral | AffectLabel::Confused => weights.routine += 0.20,
    }

    normalize_weights(weights)
}

pub(crate) fn primary_cue(weights: CueWeights) -> SituationalCue {
    let candidates = [
        (SituationalCue::Emotional, weights.emotional),
        (SituationalCue::Intellectual, weights.intellectual),
        (SituationalCue::Conflict, weights.conflict),
        (SituationalCue::Creative, weights.creative),
        (SituationalCue::Crisis, weights.crisis),
        (SituationalCue::Routine, weights.routine),
    ];

    candidates
        .into_iter()
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .map_or(SituationalCue::Routine, |(cue, _)| cue)
}

pub(crate) fn classify_situational_cue(
    dialogue_act: DialogueAct,
    user_model: &UserMentalModel,
    affect_label: AffectLabel,
) -> SituationalCue {
    primary_cue(derive_cue_weights(dialogue_act, user_model, affect_label))
}

pub(crate) fn activate_traits(
    weights: &CueWeights,
    config: &TraitActivationConfig,
) -> TraitActivation {
    fn blend(current: &mut f64, target: f64, weight: f32) {
        *current += (target - 1.0) * f64::from(weight);
    }

    let mut activation = TraitActivation::default();

    blend(
        &mut activation.agreeableness,
        config.emotional_agreeableness_boost,
        weights.emotional + weights.crisis,
    );
    blend(&mut activation.extraversion, 0.85, weights.emotional);
    blend(
        &mut activation.openness,
        config.intellectual_openness_boost,
        weights.intellectual + weights.creative,
    );
    blend(
        &mut activation.conscientiousness,
        1.15,
        weights.intellectual,
    );
    blend(&mut activation.conscientiousness, 0.85, weights.creative);
    blend(
        &mut activation.agreeableness,
        config.conflict_agreeableness_threshold,
        weights.conflict,
    );
    blend(&mut activation.extraversion, 1.1, weights.conflict);
    blend(&mut activation.neuroticism, 0.8, weights.conflict);
    blend(
        &mut activation.extraversion,
        config.crisis_extraversion_suppression,
        weights.crisis,
    );
    blend(&mut activation.neuroticism, 0.7, weights.crisis);

    activation
}

fn normalize_weights(mut weights: CueWeights) -> CueWeights {
    let total = weights.emotional
        + weights.intellectual
        + weights.conflict
        + weights.creative
        + weights.crisis
        + weights.routine;
    if total <= f32::EPSILON {
        return CueWeights::default();
    }
    weights.emotional /= total;
    weights.intellectual /= total;
    weights.conflict /= total;
    weights.creative /= total;
    weights.crisis /= total;
    weights.routine /= total;
    weights
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::persona::user_model::{EmotionalNeed, KnowledgeLevel};

    fn default_config() -> TraitActivationConfig {
        TraitActivationConfig::default()
    }

    fn model(intent: UserIntent) -> UserMentalModel {
        UserMentalModel {
            inferred_intent: intent,
            knowledge_level: KnowledgeLevel::Intermediate,
            emotional_need: EmotionalNeed::Solution,
            active_constraints: Vec::new(),
        }
    }

    #[test]
    fn cue_weights_normalize_and_capture_blended_turns() {
        let weights = derive_cue_weights(
            DialogueAct::Inform,
            &model(UserIntent::Vent),
            AffectLabel::Anxious,
        );
        let total = weights.emotional
            + weights.intellectual
            + weights.conflict
            + weights.creative
            + weights.crisis
            + weights.routine;
        assert!((total - 1.0).abs() < 0.0001);
        assert!(weights.crisis > 0.0);
        assert!(weights.emotional > 0.0);
    }

    #[test]
    fn crisis_boosts_agreeableness_suppresses_extraversion() {
        let weights = CueWeights {
            crisis: 1.0,
            ..CueWeights::default()
        };
        let activation = activate_traits(&weights, &default_config());
        assert!(activation.agreeableness > 1.0);
        assert!(activation.extraversion < 1.0);
    }

    #[test]
    fn primary_cue_selects_strongest_weight() {
        let weights = CueWeights {
            intellectual: 0.45,
            creative: 0.35,
            routine: 0.20,
            ..CueWeights::default()
        };
        assert_eq!(primary_cue(weights), SituationalCue::Intellectual);
    }

    #[test]
    fn classify_uses_affect_and_intent_together() {
        let cue = classify_situational_cue(
            DialogueAct::Inform,
            &model(UserIntent::Vent),
            AffectLabel::Anxious,
        );
        assert_eq!(cue, SituationalCue::Crisis);
    }
}
