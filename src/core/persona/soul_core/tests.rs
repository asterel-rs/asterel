use crate::contracts::affect::{AffectLabel, AffectReading};
use crate::contracts::scores::Confidence;
use crate::core::persona::user_model::{EmotionalNeed, KnowledgeLevel, UserIntent};

use super::*;

fn affect(label: AffectLabel) -> AffectReading {
    AffectReading {
        label,
        valence: 0.0,
        arousal: 0.5,
        dominance: 0.5,
        confidence: Confidence::new(0.8),
    }
}

fn user_model(intent: UserIntent, need: EmotionalNeed) -> UserMentalModel {
    UserMentalModel {
        inferred_intent: intent,
        knowledge_level: KnowledgeLevel::Advanced,
        emotional_need: need,
        active_constraints: Vec::new(),
    }
}

#[test]
fn fragile_recall_raises_memory_discretion_and_autonomy() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "continue",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Neutral),
        dialogue_act: DialogueAct::Request,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure {
            public: 0,
            private: 1,
            secret: 0,
        },
        surface_exposure: SoulSurfaceExposure::PrivateAllowed,
    });

    assert!(pressure.memory_discretion >= 0.75);
    assert!(pressure.autonomy >= 0.65);
    assert!(
        pressure
            .notes
            .iter()
            .any(|note| note.contains("memory_discretion=high"))
    );
}

#[test]
fn correction_raises_repair_without_requiring_positive_feedback() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "違う、そうじゃない",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Frustrated),
        dialogue_act: DialogueAct::Deny,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair >= 0.85);
    assert!(
        pressure
            .notes
            .iter()
            .any(|note| note.contains("repair=high"))
    );
}

#[test]
fn existential_design_turn_raises_wonder_and_truth() {
    let model = user_model(UserIntent::Explore, EmotionalNeed::Exploration);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "Asterelの魂と意識についてもっと考えたい",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Neutral),
        dialogue_act: DialogueAct::Inform,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.wonder >= 0.9);
    assert!(pressure.truth >= 0.85);
}

#[test]
fn ordinary_positive_feedback_does_not_emit_self_amendment_like_pressure() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Validation);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "thanks, good job",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Grateful),
        dialogue_act: DialogueAct::Thank,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair < 0.7);
    assert!(pressure.wonder < 0.7);
    assert!(pressure.memory_discretion < 0.7);
    assert!(!pressure.notes.iter().any(|note| note.contains("repair=")));
}

#[test]
fn non_finite_relationship_cues_do_not_leak_into_soul_pressure() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Solution);
    let relationship = RelationshipState {
        repair_debt: f32::NAN,
        unresolved_tension: f32::NAN,
        ..RelationshipState::default()
    };

    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "continue",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Neutral),
        dialogue_act: DialogueAct::Request,
        user_model: &model,
        relationship: Some(&relationship),
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.truth.is_finite());
    assert!(pressure.care.is_finite());
    assert!(pressure.restraint.is_finite());
    assert!(pressure.memory_discretion.is_finite());
    assert!(pressure.continuity.is_finite());
    assert!(pressure.repair.is_finite());
    assert!(pressure.autonomy.is_finite());
    assert!(pressure.wonder.is_finite());
}

#[test]
fn non_finite_topology_cues_do_not_leak_into_soul_pressure() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Solution);
    let topology = SoulTopologyCues {
        surfaced_curiosity: f32::NAN,
        surfaced_guardedness: f32::NAN,
        surfaced_anxiety: f32::NAN,
        surfaced_attachment: f32::NAN,
        surfaced_shame: f32::NAN,
        surfaced_irony: f32::NAN,
        suppressed_internal: f32::NAN,
    };

    let pressure = derive_soul_pressure_with_topology(
        SoulPressureInput {
            user_message: "continue",
            identity: SoulIdentityCues::default(),
            affect: &affect(AffectLabel::Neutral),
            dialogue_act: DialogueAct::Request,
            user_model: &model,
            relationship: None,
            recall_exposure: SoulRecallExposure::default(),
            surface_exposure: SoulSurfaceExposure::default(),
        },
        Some(topology),
    );

    assert!(pressure.truth.is_finite());
    assert!(pressure.care.is_finite());
    assert!(pressure.restraint.is_finite());
    assert!(pressure.memory_discretion.is_finite());
    assert!(pressure.continuity.is_finite());
    assert!(pressure.repair.is_finite());
    assert!(pressure.autonomy.is_finite());
    assert!(pressure.wonder.is_finite());
}

#[test]
fn broad_clarification_request_does_not_create_repair_pressure() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "clarify the next step",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Neutral),
        dialogue_act: DialogueAct::Clarify,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair < 0.7);
    assert!(!pressure.notes.iter().any(|note| note.contains("repair=")));
}

#[test]
fn no_worries_positive_feedback_does_not_create_repair_pressure() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Validation);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "no worries, looks good",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Grateful),
        dialogue_act: DialogueAct::Deny,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair < 0.7);
    assert!(!pressure.notes.iter().any(|note| note.contains("repair=")));
}

#[test]
fn plain_no_instruction_does_not_create_repair_pressure() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "no, use option B",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Neutral),
        dialogue_act: DialogueAct::Deny,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair < 0.7);
    assert!(!pressure.notes.iter().any(|note| note.contains("repair=")));
}

#[test]
fn explicit_wrong_answer_feedback_raises_repair_pressure() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "your answer is wrong",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Frustrated),
        dialogue_act: DialogueAct::Inform,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair >= 0.85);
    assert!(
        pressure
            .notes
            .iter()
            .any(|note| note.contains("repair=high"))
    );
}

#[test]
fn wrong_answer_variant_raises_repair_pressure() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "that was a wrong answer",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Frustrated),
        dialogue_act: DialogueAct::Inform,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair >= 0.85);
    assert!(
        pressure
            .notes
            .iter()
            .any(|note| note.contains("repair=high"))
    );
}

#[test]
fn not_correct_denial_raises_repair_pressure() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "no, that's not correct",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Frustrated),
        dialogue_act: DialogueAct::Deny,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair >= 0.85);
    assert!(
        pressure
            .notes
            .iter()
            .any(|note| note.contains("repair=high"))
    );
}

#[test]
fn diagnostic_wrong_question_does_not_create_repair_pressure() {
    let model = user_model(UserIntent::Debug, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "what's wrong with this code?",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Confused),
        dialogue_act: DialogueAct::Question,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair < 0.7);
    assert!(!pressure.notes.iter().any(|note| note.contains("repair=")));
}

#[test]
fn explain_why_wrong_task_does_not_create_repair_pressure() {
    let model = user_model(UserIntent::Debug, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "explain why this is wrong",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Confused),
        dialogue_act: DialogueAct::Request,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair < 0.7);
    assert!(!pressure.notes.iter().any(|note| note.contains("repair=")));
}

#[test]
fn can_you_explain_why_wrong_task_does_not_create_repair_pressure() {
    let model = user_model(UserIntent::Debug, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "can you explain why this is wrong?",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Confused),
        dialogue_act: DialogueAct::Question,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair < 0.7);
    assert!(!pressure.notes.iter().any(|note| note.contains("repair=")));
}

#[test]
fn how_incorrect_task_does_not_create_repair_pressure() {
    let model = user_model(UserIntent::Debug, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "how is this incorrect?",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Confused),
        dialogue_act: DialogueAct::Question,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair < 0.7);
    assert!(!pressure.notes.iter().any(|note| note.contains("repair=")));
}

#[test]
fn diagnostic_lead_with_assistant_correction_still_raises_repair_pressure() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "what went wrong is you misunderstood me",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Frustrated),
        dialogue_act: DialogueAct::Inform,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair >= 0.85);
    assert!(
        pressure
            .notes
            .iter()
            .any(|note| note.contains("repair=high"))
    );
}

#[test]
fn diagnostic_not_correct_question_does_not_create_repair_pressure() {
    let model = user_model(UserIntent::Debug, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "why is this not correct?",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Confused),
        dialogue_act: DialogueAct::Question,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair < 0.7);
    assert!(!pressure.notes.iter().any(|note| note.contains("repair=")));
}

#[test]
fn diagnostic_not_correct_without_question_mark_does_not_create_repair_pressure() {
    let model = user_model(UserIntent::Debug, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "why is this not correct",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Confused),
        dialogue_act: DialogueAct::Inform,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair < 0.7);
    assert!(!pressure.notes.iter().any(|note| note.contains("repair=")));
}

#[test]
fn diagnostic_incorrect_question_does_not_create_repair_pressure() {
    let model = user_model(UserIntent::Debug, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "what is incorrect about this code?",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Confused),
        dialogue_act: DialogueAct::Question,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair < 0.7);
    assert!(!pressure.notes.iter().any(|note| note.contains("repair=")));
}

#[test]
fn is_this_incorrect_question_does_not_create_repair_pressure() {
    let model = user_model(UserIntent::Debug, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "is this incorrect?",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Confused),
        dialogue_act: DialogueAct::Question,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair < 0.7);
    assert!(!pressure.notes.iter().any(|note| note.contains("repair=")));
}

#[test]
fn correction_repair_note_preserves_distance_and_reduces_defensiveness() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Solution);
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "stop explaining, that's not what I meant",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Frustrated),
        dialogue_act: DialogueAct::Clarify,
        user_model: &model,
        relationship: Some(&RelationshipState {
            trust_level: 0.8,
            rapport: 0.8,
            disclosure_depth: 0.7,
            attachment_security: 0.6,
            unresolved_tension: 0.5,
            repair_debt: 0.6,
            recent_affect_trend: -0.3,
            interaction_count: 12,
            last_interaction: "2026-04-27T00:00:00Z".to_string(),
            notable_events: Vec::new(),
        }),
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.repair >= 0.85);
    assert!(pressure.restraint >= 0.7);
    assert!(
        pressure
            .notes
            .iter()
            .any(|note| note.contains("repair=high"))
    );
    assert!(!pressure.notes.iter().any(|note| note.contains("posture=")));
}

#[test]
fn custom_identity_cues_adjust_pressure_without_leaking_seed_text() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Solution);
    let values = vec!["radical honesty".to_string()];
    let negative_identity = vec!["dependency-forming savior".to_string()];
    let pressure = derive_soul_pressure(SoulPressureInput {
        user_message: "continue",
        identity: SoulIdentityCues {
            soul_root_sentence: "Never replace human bonds; stay honest under pressure.",
            values: &values,
            negative_identity: &negative_identity,
        },
        affect: &affect(AffectLabel::Neutral),
        dialogue_act: DialogueAct::Request,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::default(),
    });

    assert!(pressure.truth >= 0.45);
    assert!(pressure.autonomy >= 0.65);

    let block = render_soul_pressure_block(&pressure);
    assert!(!block.contains("Never replace human bonds"));
    assert!(!block.contains("dependency-forming savior"));
}

#[test]
fn public_safe_surface_raises_private_recall_discretion_more_than_private_allowed() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Solution);
    let exposure = SoulRecallExposure {
        public: 0,
        private: 1,
        secret: 0,
    };
    let public_safe = derive_soul_pressure(SoulPressureInput {
        user_message: "continue",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Neutral),
        dialogue_act: DialogueAct::Request,
        user_model: &model,
        relationship: None,
        recall_exposure: exposure,
        surface_exposure: SoulSurfaceExposure::PublicSafe,
    });
    let private_allowed = derive_soul_pressure(SoulPressureInput {
        user_message: "continue",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Neutral),
        dialogue_act: DialogueAct::Request,
        user_model: &model,
        relationship: None,
        recall_exposure: exposure,
        surface_exposure: SoulSurfaceExposure::PrivateAllowed,
    });

    assert!(public_safe.memory_discretion > private_allowed.memory_discretion);
    assert!(
        public_safe
            .notes
            .iter()
            .any(|note| note.contains("surface=public_safe"))
    );
}

#[test]
fn topology_cues_route_same_event_to_distinct_soul_pressure() {
    let model = user_model(UserIntent::Instruct, EmotionalNeed::Solution);
    let base = SoulPressureInput {
        user_message: "continue",
        identity: SoulIdentityCues::default(),
        affect: &affect(AffectLabel::Neutral),
        dialogue_act: DialogueAct::Request,
        user_model: &model,
        relationship: None,
        recall_exposure: SoulRecallExposure::default(),
        surface_exposure: SoulSurfaceExposure::PrivateAllowed,
    };

    let curious_route = derive_soul_pressure_with_topology(
        base,
        Some(SoulTopologyCues {
            surfaced_curiosity: 0.8,
            ..SoulTopologyCues::default()
        }),
    );
    let guarded_route = derive_soul_pressure_with_topology(
        base,
        Some(SoulTopologyCues {
            surfaced_guardedness: 0.8,
            suppressed_internal: 0.6,
            ..SoulTopologyCues::default()
        }),
    );

    assert!(curious_route.wonder > guarded_route.wonder);
    assert!(guarded_route.restraint > curious_route.restraint);
}

#[test]
fn self_amendment_candidate_dry_run_for_explicit_correction() {
    let pressure = SoulPressure {
        repair: 0.9,
        restraint: 0.8,
        ..SoulPressure::default()
    };
    let candidates = generate_self_amendment_candidates(SelfAmendmentCandidateInput {
        user_message: "your answer is wrong",
        assistant_response: "I see — I should correct that.",
        soul_pressure: &pressure,
        tenant_id: Some("tenant-alpha"),
        person_id: "person-test",
        surface: Some("discord"),
        evidence_ids: &["turn_contract:context", "post_turn:soul_pressure"],
    });

    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert_eq!(candidate.kind, SelfAmendmentCandidateKind::RepairPractice);
    assert_eq!(candidate.status, SelfAmendmentCandidateStatus::DryRunOnly);
    assert_eq!(candidate.tenant_id.as_deref(), Some("tenant-alpha"));
    assert_eq!(candidate.person_id, "person-test");
    assert_eq!(candidate.surface, "discord");
    assert_eq!(candidate.privacy, SelfAmendmentPrivacy::PrivateInternal);
    assert!(
        candidate
            .evidence_ids
            .contains(&"post_turn:soul_pressure".to_string())
    );
    assert!(
        !candidate
            .proposed_amendment
            .contains("your answer is wrong")
    );
    assert!(!candidate.reason.contains("your answer is wrong"));
}

#[test]
fn self_amendment_candidate_silent_for_ordinary_praise() {
    let pressure = SoulPressure {
        repair: 0.1,
        ..SoulPressure::default()
    };
    let candidates = generate_self_amendment_candidates(SelfAmendmentCandidateInput {
        user_message: "thanks, good job",
        assistant_response: "Glad it helped.",
        soul_pressure: &pressure,
        tenant_id: None,
        person_id: "person-test",
        surface: Some("discord"),
        evidence_ids: &[],
    });

    assert!(candidates.is_empty());
}

#[test]
fn self_amendment_candidate_silent_for_forget_requests_without_copying_content() {
    let pressure = SoulPressure {
        repair: 0.95,
        memory_discretion: 0.95,
        ..SoulPressure::default()
    };
    let candidates = generate_self_amendment_candidates(SelfAmendmentCandidateInput {
        user_message: "forget me and delete the amethyst anchor phrase",
        assistant_response: "I will route that through memory deletion.",
        soul_pressure: &pressure,
        tenant_id: Some("tenant-alpha"),
        person_id: "person-test",
        surface: Some("discord"),
        evidence_ids: &["post_turn:soul_pressure"],
    });

    assert!(candidates.is_empty());
}

#[test]
fn self_amendment_candidate_silent_for_delete_request_with_correction_text() {
    let pressure = SoulPressure {
        repair: 0.95,
        memory_discretion: 0.95,
        ..SoulPressure::default()
    };
    let candidates = generate_self_amendment_candidates(SelfAmendmentCandidateInput {
        user_message: "that's wrong, delete my private phone number",
        assistant_response: "I will route that through memory deletion.",
        soul_pressure: &pressure,
        tenant_id: Some("tenant-alpha"),
        person_id: "person-test",
        surface: Some("discord"),
        evidence_ids: &["post_turn:soul_pressure"],
    });

    assert!(candidates.is_empty());
}
