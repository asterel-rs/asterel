//! Empathy policy: maps affect label, trust level, rapport, and dialogue act
//! to a `ResponseStyleFamily` and an `EmpathyPolicyOutput`.
//!
//! Five style families are available:
//! - `Professional` — neutral default; used for low-trust frustration, neutral
//!   questions, and low-signal states.
//! - `Empathetic` — warm acknowledgment; triggered by Sad or Anxious affect.
//! - `Directive` — action-oriented; triggered by neutral Request acts.
//! - `Supportive` — patient, step-by-step; triggered by Frustrated/Angry with
//!   established trust, or by Confused/Overwhelmed affect.
//! - `Celebratory` — energetic, encouraging; triggered by Excited or Grateful
//!   affect.
//!
//! Trust gate: Frustrated/Angry affect selects Supportive only when
//! `relationship_trust >= 0.6`; below that threshold Professional is used
//! to avoid overfamiliarity.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use super::continuity_v2::DialogueAct;
use crate::contracts::affect::AffectLabel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResponseStyleFamily {
    /// Default professional style.
    Professional,
    /// Warm, supportive, acknowledging emotions.
    Empathetic,
    /// Clear, direct, action-oriented.
    Directive,
    /// Patient, step-by-step, reassuring.
    Supportive,
    /// Light, encouraging, celebratory.
    Celebratory,
}

pub(crate) struct EmpathyPolicyInput {
    pub affect_label: AffectLabel,
    pub affect_confidence: f64,
    pub relationship_trust: f32,
    pub relationship_rapport: f32,
    pub dialogue_act: DialogueAct,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct EmpathyPolicyOutput {
    pub style_family: ResponseStyleFamily,
    /// Stored as `Cow<'static, str>` so the common case (every variant uses a
    /// `&'static str` template literal) stays allocation-free. Deserialisation
    /// produces `Cow::Owned`, which only happens during replay.
    pub empathy_rationale: Cow<'static, str>,
    pub style_rationale: Cow<'static, str>,
    pub acknowledgment_needed: bool,
}

pub(crate) fn select_empathy_response_style(input: &EmpathyPolicyInput) -> EmpathyPolicyOutput {
    let high_trust = input.relationship_trust >= 0.6;
    let high_rapport = input.relationship_rapport >= 0.6;
    let high_confidence = input.affect_confidence >= 0.6;

    match input.affect_label {
        AffectLabel::Frustrated | AffectLabel::Angry if high_trust => EmpathyPolicyOutput {
            style_family: ResponseStyleFamily::Supportive,
            empathy_rationale: Cow::Borrowed(
                "User has expressed frustration; supportive tone appropriate given established trust",
            ),
            style_rationale: Cow::Borrowed(
                "Supportive style helps de-escalate while preserving collaboration momentum",
            ),
            acknowledgment_needed: true,
        },
        AffectLabel::Frustrated | AffectLabel::Angry => EmpathyPolicyOutput {
            style_family: ResponseStyleFamily::Professional,
            empathy_rationale: Cow::Borrowed(
                "User is frustrated but trust is low; professional tone to avoid escalation",
            ),
            style_rationale: Cow::Borrowed(
                "Professional style keeps the response stable and avoids overfamiliar language",
            ),
            acknowledgment_needed: false,
        },
        AffectLabel::Sad | AffectLabel::Anxious => EmpathyPolicyOutput {
            style_family: ResponseStyleFamily::Empathetic,
            empathy_rationale: Cow::Borrowed(
                "User shows signs of distress; empathetic acknowledgment important",
            ),
            style_rationale: Cow::Borrowed(
                "Empathetic style validates emotional context before problem-solving",
            ),
            acknowledgment_needed: true,
        },
        AffectLabel::Confused | AffectLabel::Overwhelmed => EmpathyPolicyOutput {
            style_family: ResponseStyleFamily::Supportive,
            empathy_rationale: Cow::Borrowed("User needs patient, clear guidance"),
            style_rationale: Cow::Borrowed(
                "Supportive style encourages step-by-step explanation and reassurance",
            ),
            acknowledgment_needed: true,
        },
        AffectLabel::Excited | AffectLabel::Grateful => EmpathyPolicyOutput {
            style_family: ResponseStyleFamily::Celebratory,
            empathy_rationale: Cow::Borrowed("User is in a positive state; match their energy"),
            style_rationale: if high_rapport {
                Cow::Borrowed("Celebratory style reinforces positive momentum and engagement")
            } else {
                Cow::Borrowed("Positive affect detected; keep tone encouraging but measured")
            },
            acknowledgment_needed: false,
        },
        AffectLabel::Curious => EmpathyPolicyOutput {
            style_family: ResponseStyleFamily::Professional,
            empathy_rationale: Cow::Borrowed(
                "User is curious; straightforward informative response",
            ),
            style_rationale: Cow::Borrowed(
                "Professional style keeps information concise and accurate",
            ),
            acknowledgment_needed: false,
        },
        AffectLabel::Neutral if input.dialogue_act == DialogueAct::Question => {
            EmpathyPolicyOutput {
                style_family: ResponseStyleFamily::Professional,
                empathy_rationale: Cow::Borrowed(
                    "Neutral question does not require emotional adjustment",
                ),
                style_rationale: Cow::Borrowed(
                    "Professional style best fits direct Q&A interactions",
                ),
                acknowledgment_needed: false,
            }
        }
        AffectLabel::Neutral if input.dialogue_act == DialogueAct::Request => EmpathyPolicyOutput {
            style_family: ResponseStyleFamily::Directive,
            empathy_rationale: Cow::Borrowed("Neutral request indicates action-oriented intent"),
            style_rationale: Cow::Borrowed("Directive style prioritizes clear execution steps"),
            acknowledgment_needed: false,
        },
        AffectLabel::Neutral => EmpathyPolicyOutput {
            style_family: ResponseStyleFamily::Professional,
            empathy_rationale: if high_confidence {
                Cow::Borrowed("No strong emotional cue detected; defaulting to professional tone")
            } else {
                Cow::Borrowed("Affect signal is uncertain; using neutral professional fallback")
            },
            style_rationale: Cow::Borrowed(
                "Professional style is the safest default response family",
            ),
            acknowledgment_needed: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::{
        EmpathyPolicyInput, EmpathyPolicyOutput, ResponseStyleFamily, select_empathy_response_style,
    };
    use crate::contracts::affect::AffectLabel;
    use crate::core::persona::continuity_v2::DialogueAct;

    fn make_input(
        affect_label: AffectLabel,
        trust: f32,
        dialogue_act: DialogueAct,
    ) -> EmpathyPolicyInput {
        EmpathyPolicyInput {
            affect_label,
            affect_confidence: 0.8,
            relationship_trust: trust,
            relationship_rapport: 0.6,
            dialogue_act,
        }
    }

    #[test]
    fn frustrated_high_trust_gets_supportive() {
        let output = select_empathy_response_style(&make_input(
            AffectLabel::Frustrated,
            0.8,
            DialogueAct::Inform,
        ));
        assert_eq!(output.style_family, ResponseStyleFamily::Supportive);
    }

    #[test]
    fn frustrated_low_trust_gets_professional() {
        let output = select_empathy_response_style(&make_input(
            AffectLabel::Frustrated,
            0.2,
            DialogueAct::Inform,
        ));
        assert_eq!(output.style_family, ResponseStyleFamily::Professional);
    }

    #[test]
    fn sad_gets_empathetic() {
        let output =
            select_empathy_response_style(&make_input(AffectLabel::Sad, 0.5, DialogueAct::Inform));
        assert_eq!(output.style_family, ResponseStyleFamily::Empathetic);
        assert!(output.acknowledgment_needed);
    }

    #[test]
    fn excited_gets_celebratory() {
        let output = select_empathy_response_style(&make_input(
            AffectLabel::Excited,
            0.5,
            DialogueAct::Inform,
        ));
        assert_eq!(output.style_family, ResponseStyleFamily::Celebratory);
    }

    #[test]
    fn neutral_question_gets_professional() {
        let output = select_empathy_response_style(&make_input(
            AffectLabel::Neutral,
            0.5,
            DialogueAct::Question,
        ));
        assert_eq!(output.style_family, ResponseStyleFamily::Professional);
    }

    #[test]
    fn empathy_output_serde_round_trip() {
        let output = EmpathyPolicyOutput {
            style_family: ResponseStyleFamily::Supportive,
            empathy_rationale: Cow::Borrowed("test-empathy"),
            style_rationale: Cow::Borrowed("test-style"),
            acknowledgment_needed: true,
        };

        let serialized = serde_json::to_string(&output).expect("serialize empathy output");
        let decoded: EmpathyPolicyOutput =
            serde_json::from_str(&serialized).expect("deserialize empathy output");
        assert_eq!(decoded.style_family, ResponseStyleFamily::Supportive);
        assert!(decoded.acknowledgment_needed);
    }

    #[test]
    fn response_style_family_serde_round_trip() {
        let families = [
            ResponseStyleFamily::Professional,
            ResponseStyleFamily::Empathetic,
            ResponseStyleFamily::Directive,
            ResponseStyleFamily::Supportive,
            ResponseStyleFamily::Celebratory,
        ];

        for family in families {
            let serialized =
                serde_json::to_string(&family).expect("serialize response style family");
            let decoded: ResponseStyleFamily =
                serde_json::from_str(&serialized).expect("deserialize response style family");
            assert_eq!(decoded, family);
        }
    }
}
