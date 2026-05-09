//! Rule-based heuristic attribution of the likely cause behind a
//! detected user affect state (e.g. agent failure, time pressure).

use super::types::AffectLabel;

/// Likely cause behind a detected user affect state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AffectCause {
    /// The agent itself is blamed for the problem.
    AgentFailure,
    /// An external system or environment issue is the source.
    ExternalProblem,
    /// The user does not understand the situation or explanation.
    Confusion,
    /// The user is under time constraints or urgency.
    TimePressure,
    /// No clear cause could be determined.
    Unknown,
}

const AGENT_BLAME: &[&str] = &["you broke", "you messed up", "your fault", "you failed"];
const TIME_WORDS: &[&str] = &["urgent", "deadline", "asap", "quickly", "hurry"];
const CONFUSION_MARKERS: &[&str] = &["don't understand", "makes no sense", "how does", "i'm lost"];
const EXTERNAL_ISSUES: &[&str] = &["server", "deploy", "production", "client", "boss", "outage"];

/// Attribute the likely cause of a user's emotional state using rule-based
/// keyword heuristics.
///
/// Rules are evaluated in strict priority order:
/// 1. **`AgentFailure`** — explicit blame directed at "you" + blame keywords
///    (highest specificity; "you broke" is unambiguous)
/// 2. **`TimePressure`** — urgency keywords ("asap", "deadline", "urgent")
/// 3. **Confusion** — either the affect label is `Confused`, or confusion
///    markers appear ("don't understand", "makes no sense")
/// 4. **`ExternalProblem`** — system/environment keywords ("server", "outage")
/// 5. **Unknown** — fallback when no pattern matches
///
/// The `_context` parameter is reserved for future use (e.g., conversation
/// history) and is currently ignored.
pub(crate) fn attribute_cause_heuristic(
    user_message: &str,
    affect_label: AffectLabel,
    _context: Option<&str>,
) -> AffectCause {
    let lower = user_message.to_lowercase();

    if lower.contains("you") && AGENT_BLAME.iter().any(|w| lower.contains(w)) {
        return AffectCause::AgentFailure;
    }
    if TIME_WORDS.iter().any(|w| lower.contains(w)) {
        return AffectCause::TimePressure;
    }
    if affect_label == AffectLabel::Confused || CONFUSION_MARKERS.iter().any(|w| lower.contains(w))
    {
        return AffectCause::Confusion;
    }
    if EXTERNAL_ISSUES.iter().any(|w| lower.contains(w)) {
        return AffectCause::ExternalProblem;
    }
    AffectCause::Unknown
}

/// VAD-enhanced cause attribution.
///
/// Uses continuous VAD coordinates for finer-grained heuristics:
/// - Low dominance + low valence → external/environmental cause
/// - High dominance + low valence → agent failure (user feels in control but unhappy)
/// - Low dominance + high arousal → time pressure or anxiety
///
/// Falls back to keyword-based `attribute_cause_heuristic` when VAD
/// does not clearly discriminate.
pub(crate) fn attribute_cause_vad(
    user_message: &str,
    affect_label: AffectLabel,
    valence: f64,
    dominance: f64,
    arousal: f64,
) -> AffectCause {
    // Keyword-based heuristics still take priority for high-confidence patterns.
    let keyword_cause = attribute_cause_heuristic(user_message, affect_label, None);
    if keyword_cause != AffectCause::Unknown {
        return keyword_cause;
    }

    // VAD-based disambiguation for ambiguous cases.
    if valence < -0.3 && dominance > 0.6 {
        return AffectCause::AgentFailure;
    }
    if valence < -0.3 && dominance < 0.3 {
        return AffectCause::ExternalProblem;
    }
    if dominance < 0.3 && arousal > 0.6 {
        return AffectCause::TimePressure;
    }

    AffectCause::Unknown
}

/// Return a single-sentence guidance string for the given cause.
pub(crate) fn cause_to_guidance(cause: AffectCause) -> &'static str {
    match cause {
        AffectCause::AgentFailure => {
            "Acknowledge the prior mistake, apologize briefly, and provide a corrected solution."
        }
        AffectCause::ExternalProblem => {
            "Focus on actionable help for the external issue without over-empathizing."
        }
        AffectCause::Confusion => {
            "Simplify the explanation, use concrete examples, and check understanding."
        }
        AffectCause::TimePressure => {
            "Prioritize the most direct solution and skip non-essential context."
        }
        AffectCause::Unknown => "Respond empathetically while focusing on the user's stated goal.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(msg: &str, label: AffectLabel) -> AffectCause {
        attribute_cause_heuristic(msg, label, None)
    }

    #[test]
    fn each_cause_detected() {
        assert_eq!(
            h("You broke my config!", AffectLabel::Frustrated),
            AffectCause::AgentFailure
        );
        assert_eq!(
            h("This is urgent asap", AffectLabel::Anxious),
            AffectCause::TimePressure
        );
        assert_eq!(
            h("I'm lost here", AffectLabel::Neutral),
            AffectCause::Confusion
        );
        assert_eq!(
            h("Neutral msg", AffectLabel::Confused),
            AffectCause::Confusion
        );
        assert_eq!(
            h("Production is down", AffectLabel::Frustrated),
            AffectCause::ExternalProblem
        );
        assert_eq!(
            h("Help me with this", AffectLabel::Neutral),
            AffectCause::Unknown
        );
    }

    #[test]
    fn agent_failure_requires_you() {
        assert_ne!(
            h("Something broke", AffectLabel::Frustrated),
            AffectCause::AgentFailure
        );
    }

    #[test]
    fn priority_ordering() {
        assert_eq!(
            h("You broke it, urgent!", AffectLabel::Angry),
            AffectCause::AgentFailure
        );
        assert_eq!(
            h("I don't understand but hurry", AffectLabel::Neutral),
            AffectCause::TimePressure
        );
    }

    #[test]
    fn vad_high_dominance_low_valence_is_agent_failure() {
        let cause = super::attribute_cause_vad(
            "Something went wrong here",
            AffectLabel::Frustrated,
            -0.5, // low valence
            0.7,  // high dominance
            0.5,
        );
        assert_eq!(cause, AffectCause::AgentFailure);
    }

    #[test]
    fn vad_low_dominance_low_valence_is_external() {
        let cause = super::attribute_cause_vad(
            "Everything is falling apart",
            AffectLabel::Sad,
            -0.5, // low valence
            0.2,  // low dominance
            0.3,
        );
        assert_eq!(cause, AffectCause::ExternalProblem);
    }

    #[test]
    fn vad_low_dominance_high_arousal_is_time_pressure() {
        let cause = super::attribute_cause_vad(
            "I need this done",
            AffectLabel::Anxious,
            -0.1,
            0.2, // low dominance
            0.7, // high arousal
        );
        assert_eq!(cause, AffectCause::TimePressure);
    }

    #[test]
    fn vad_keyword_takes_priority() {
        // Even with VAD pointing to ExternalProblem, keyword "you broke" wins
        let cause = super::attribute_cause_vad(
            "You broke my config!",
            AffectLabel::Angry,
            -0.7,
            0.2, // low dominance → would be External
            0.8,
        );
        assert_eq!(cause, AffectCause::AgentFailure);
    }

    #[test]
    fn guidance_nonempty_and_distinct() {
        let all = [
            AffectCause::AgentFailure,
            AffectCause::ExternalProblem,
            AffectCause::Confusion,
            AffectCause::TimePressure,
            AffectCause::Unknown,
        ];
        for c in &all {
            assert!(!cause_to_guidance(*c).is_empty());
        }
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(cause_to_guidance(*a), cause_to_guidance(*b));
            }
        }
    }
}
