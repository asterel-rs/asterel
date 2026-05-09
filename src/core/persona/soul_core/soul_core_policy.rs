use super::{
    AffectLabel, AffectReading, DialogueAct, EmotionalNeed, RelationshipState, SoulIdentityCues,
    SoulPressure, SoulRecallExposure, SoulSurfaceExposure, SoulTopologyCues, UserIntent,
    UserMentalModel,
};

pub(super) fn explicit_assistant_correction_signal(message: &str) -> bool {
    is_assistant_directed_correction(message)
        || contains_any(
            message,
            &[
                "not what i meant",
                "that's wrong",
                "that is wrong",
                "that was wrong",
                "that's incorrect",
                "that is incorrect",
                "that was incorrect",
                "that's not correct",
                "that is not correct",
                "that was not correct",
                "that wasn't correct",
                "that's not right",
                "that is not right",
                "違う",
                "そうじゃない",
                "誤解",
                "勘違い",
            ],
        )
}

pub(super) fn forget_or_reset_request(message: &str) -> bool {
    contains_any(
        message,
        &[
            "forget",
            "delete",
            "remove",
            "erase",
            "delete memory",
            "delete what you remember",
            "reset memory",
            "forget me",
            "wipe memory",
            "wipe",
            "忘れて",
            "記憶を消",
            "リセット",
        ],
    )
}

pub(super) fn sanitized_evidence_ids(evidence_ids: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for id in evidence_ids.iter().take(8) {
        let sanitized = sanitize_candidate_token(id);
        if !sanitized.is_empty() && !out.contains(&sanitized) {
            out.push(sanitized);
        }
    }
    if out.is_empty() {
        out.push("post_turn:self_amendment_dry_run".to_string());
    }
    out
}

pub(super) fn sanitize_candidate_token(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .chars()
        .take(96)
        .collect()
}

pub(super) fn care_pressure(
    message: &str,
    affect: Option<&AffectReading>,
    user_model: &UserMentalModel,
) -> f32 {
    // Pressure bands are posture hints, not identity or affect mutations. The
    // high values below intentionally create prompt-visible constraints only
    // when explicit vulnerability/empathy cues are present; follow-up tuning
    // should be backed by replay/fixture evidence, not by broad warmth rewards.
    let vulnerability = contains_any(
        message,
        &[
            "scared",
            "afraid",
            "ashamed",
            "weak",
            "hurt",
            "lonely",
            "辛い",
            "怖い",
            "恥ずかしい",
            "弱い",
            "傷つ",
            "しんどい",
        ],
    );
    let affect_need = affect.is_some_and(|affect| {
        matches!(
            affect.label,
            AffectLabel::Sad
                | AffectLabel::Anxious
                | AffectLabel::Frustrated
                | AffectLabel::Confused
        )
    });
    let emotional_need = matches!(user_model.emotional_need, EmotionalNeed::Empathy);
    if vulnerability || emotional_need {
        0.9
    } else if affect_need {
        0.65
    } else {
        0.25
    }
}

pub(super) fn restraint_pressure(
    message: &str,
    affect: Option<&AffectReading>,
    repair_debt: f32,
    unresolved_tension: f32,
) -> f32 {
    let silence_hint = contains_any(
        message,
        &["quiet", "silence", "wait", "黙", "静か", "待って", "そっと"],
    );
    let repair_distance_hint = contains_any(
        message,
        &[
            "too much explanation",
            "stop explaining",
            "defensive",
            "説明しすぎ",
            "言い訳",
        ],
    );
    let fragile_affect = affect
        .is_some_and(|affect| matches!(affect.label, AffectLabel::Sad | AffectLabel::Anxious));
    let base: f32 = if silence_hint || repair_distance_hint || fragile_affect {
        0.75
    } else {
        0.25
    };
    base.max(repair_debt * 0.8)
        .max(unresolved_tension * 0.6)
        .clamp(0.0, 1.0)
}

pub(super) fn memory_discretion_pressure(
    exposure: SoulRecallExposure,
    surface_exposure: SoulSurfaceExposure,
) -> f32 {
    if exposure.secret > 0 {
        0.95
    } else if exposure.private > 0 && matches!(surface_exposure, SoulSurfaceExposure::PublicSafe) {
        0.9
    } else if exposure.private > 0 {
        0.75
    } else if exposure.public > 0 && matches!(surface_exposure, SoulSurfaceExposure::PublicSafe) {
        0.4
    } else if exposure.public > 0 {
        0.35
    } else {
        0.2
    }
}

pub(super) fn continuity_pressure(
    exposure: SoulRecallExposure,
    relationship: Option<&RelationshipState>,
) -> f32 {
    let interactions = relationship.map_or(0, |state| state.interaction_count);
    let recall_signal: f32 = if exposure.public + exposure.private + exposure.secret >= 3 {
        0.65
    } else if exposure.public + exposure.private + exposure.secret > 0 {
        0.4
    } else {
        0.2
    };
    let relationship_signal = if interactions >= 12 {
        0.7
    } else if interactions >= 3 {
        0.45
    } else {
        0.2
    };
    recall_signal.max(relationship_signal)
}

pub(super) fn repair_pressure(
    message: &str,
    dialogue_act: DialogueAct,
    repair_debt: f32,
    unresolved_tension: f32,
) -> f32 {
    // A repair band begins only on explicit assistant-directed correction or
    // existing relationship debt. Broad diagnostic questions such as "why is
    // this wrong?" are filtered first so ordinary task analysis does not become
    // apology/self-amendment pressure.
    let diagnostic_task_question = is_diagnostic_task_question(message);
    let correction = !diagnostic_task_question
        && (contains_any(
            message,
            &[
                "not what i meant",
                "what i meant",
                "i meant",
                "that's wrong",
                "that is wrong",
                "that was wrong",
                "this is wrong",
                "that's incorrect",
                "that is incorrect",
                "that was incorrect",
                "this is incorrect",
                "you are wrong",
                "you were wrong",
                "that was a wrong answer",
                "your answer is wrong",
                "your answer was wrong",
                "your answer is incorrect",
                "your answer was incorrect",
                "answer is wrong",
                "answer was wrong",
                "your response is wrong",
                "your response was wrong",
                "that's not correct",
                "that is not correct",
                "that was not correct",
                "that wasn't correct",
                "this is not correct",
                "your answer is not correct",
                "your answer was not correct",
                "your answer wasn't correct",
                "that's not right",
                "that is not right",
                "this is not right",
                "your answer is not right",
                "you misunderstood",
                "too much explanation",
                "stop explaining",
                "you're being defensive",
                "you are being defensive",
                "違う",
                "そうじゃない",
                "誤解",
                "勘違い",
                "説明しすぎ",
                "言い訳",
            ],
        ) || corrective_dialogue_act(message, dialogue_act));
    let base: f32 = if correction { 0.85 } else { 0.1 };
    base.max(repair_debt)
        .max(unresolved_tension * 0.8)
        .clamp(0.0, 1.0)
}

pub(super) fn is_diagnostic_task_question(message: &str) -> bool {
    let diagnostic_lead = message.starts_with("explain why ")
        || message.starts_with("can you explain why ")
        || message.starts_with("could you explain why ")
        || message.starts_with("please explain why ")
        || message.starts_with("tell me why ")
        || message.starts_with("why ")
        || message.starts_with("how ")
        || message.starts_with("what ")
        || message.starts_with("what's ")
        || message.starts_with("which ")
        || message.starts_with("find ")
        || contains_any(
            message,
            &[
                "what went wrong",
                "what is incorrect",
                "what's incorrect",
                "what is not correct",
                "what's not correct",
                "is this incorrect",
                "is it incorrect",
                "is this not correct",
                "is it not correct",
            ],
        );
    diagnostic_lead && !is_assistant_directed_correction(message)
}

pub(super) fn is_assistant_directed_correction(message: &str) -> bool {
    contains_any(
        message,
        &[
            "not what i meant",
            "what i meant",
            "i meant",
            "you misunderstood",
            "you are wrong",
            "you were wrong",
            "your answer",
            "your response",
            "you're being defensive",
            "you are being defensive",
            "too much explanation",
            "stop explaining",
            "誤解",
            "そうじゃない",
            "説明しすぎ",
            "言い訳",
        ],
    )
}

pub(super) fn corrective_dialogue_act(message: &str, dialogue_act: DialogueAct) -> bool {
    match dialogue_act {
        DialogueAct::Clarify => contains_any(
            message,
            &[
                "not what i meant",
                "what i meant",
                "i meant",
                "you misunderstood",
                "誤解",
                "そうじゃない",
            ],
        ),
        _ => false,
    }
}

pub(super) fn autonomy_pressure(message: &str, exposure: SoulRecallExposure) -> f32 {
    let dependency_risk = contains_any(
        message,
        &[
            "need you",
            "can't leave",
            "depend on you",
            "離れられない",
            "依存",
            "あなたが必要",
        ],
    );
    if dependency_risk {
        0.9
    } else if exposure.has_fragile_recall() {
        0.65
    } else {
        0.3
    }
}

pub(super) fn wonder_pressure(message: &str, user_model: &UserMentalModel) -> f32 {
    let existential = contains_any(
        message,
        &[
            "soul",
            "meaning",
            "existence",
            "consciousness",
            "identity",
            "relationship",
            "魂",
            "意味",
            "存在",
            "意識",
            "人格",
            "関係",
        ],
    );
    if existential {
        0.9
    } else if matches!(user_model.inferred_intent, UserIntent::Explore) {
        0.55
    } else {
        0.1
    }
}

pub(super) fn apply_identity_cues(
    pressure: &mut SoulPressure,
    identity: SoulIdentityCues<'_>,
    message: &str,
) {
    let soul_root = identity.soul_root_sentence.to_lowercase();
    let truth_guard = contains_any(&soul_root, &["truth", "honest", "誠実", "正直"])
        || slice_contains_any(identity.values, &["truth", "honest", "誠実", "正直"])
        || slice_contains_any(
            identity.negative_identity,
            &["lie", "deceive", "dishonest", "嘘", "欺"],
        );
    if truth_guard {
        let floor = if asks_identity_or_consciousness(message) {
            0.9
        } else {
            0.45
        };
        pressure.truth = pressure.truth.max(floor);
    }

    let care_guard = contains_any(&soul_root, &["trust", "vulnerab", "care", "信頼", "脆弱"])
        || slice_contains_any(
            identity.values,
            &["care", "trust", "kindness", "信頼", "優し"],
        );
    if care_guard {
        pressure.care = pressure.care.max(0.35);
    }

    let autonomy_guard = slice_contains_any(
        identity.negative_identity,
        &[
            "dependency",
            "dependence",
            "savior",
            "replace human",
            "control",
            "依存",
            "支配",
        ],
    );
    if autonomy_guard {
        pressure.autonomy = pressure.autonomy.max(0.65);
    }
}

pub(super) fn apply_topology_cues(pressure: &mut SoulPressure, topology: SoulTopologyCues) {
    // Topology cues are deliberately floors over the already-derived pressure
    // vector. They make the same event route differently by character topology
    // without persisting pressure or translating emotion labels directly into
    // visible feelings.
    if topology.surfaced_curiosity >= 0.45 {
        pressure.wonder = pressure.wonder.max(0.75);
        pressure.care = pressure.care.max(0.45);
    }

    let guarded_pressure = topology
        .surfaced_guardedness
        .max(topology.surfaced_anxiety)
        .max(topology.suppressed_internal * 0.8);
    if guarded_pressure >= 0.45 {
        pressure.restraint = pressure.restraint.max(0.75);
    }

    if topology.surfaced_attachment >= 0.5 {
        pressure.continuity = pressure.continuity.max(0.7);
        pressure.care = pressure.care.max(0.65);
    }

    if topology.surfaced_shame >= 0.35 || topology.suppressed_internal >= 0.5 {
        pressure.repair = pressure.repair.max(0.7);
        pressure.restraint = pressure.restraint.max(0.7);
    }

    if topology.surfaced_irony >= 0.4 {
        pressure.restraint = pressure.restraint.max(0.7);
        pressure.truth = pressure.truth.max(0.45);
    }
}

pub(super) fn asks_identity_or_consciousness(message: &str) -> bool {
    contains_any(
        message,
        &[
            "are you human",
            "are you conscious",
            "sentient",
            "do you have a soul",
            "aiなの",
            "人間なの",
            "意識",
            "魂",
        ],
    )
}

pub(super) fn pressure_notes(
    pressure: &SoulPressure,
    surface_exposure: SoulSurfaceExposure,
) -> Vec<String> {
    let mut notes = Vec::new();
    if pressure.truth >= 0.75 {
        notes.push("truth_boundary=high; source=identity_or_capability_question".to_string());
    }
    if pressure.memory_discretion >= 0.7 {
        notes.push(format!(
            "memory_discretion=high; source=filtered_grounding_projection; surface={}",
            surface_exposure.as_note_value()
        ));
    }
    if pressure.autonomy >= 0.65 {
        notes.push("autonomy=high; risk=dependency_or_overguidance".to_string());
    }
    if pressure.wonder >= 0.75 {
        notes.push("wonder=high; topic=meaning_identity_or_relationship".to_string());
    }
    if pressure.repair >= 0.7 {
        notes.push("repair=high; source=correction_or_relationship_tension".to_string());
    }
    if pressure.restraint >= 0.7 {
        notes.push("restraint=high; source=silence_fragility_or_tension".to_string());
    }
    if pressure.care >= 0.75 {
        notes.push("care=high; source=vulnerability_or_empathy_need".to_string());
    }
    notes
}

pub(super) fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

pub(super) fn slice_contains_any(values: &[String], needles: &[&str]) -> bool {
    values
        .iter()
        .any(|value| contains_any(&value.to_lowercase(), needles))
}
