//! Session control state: lightweight per-session tracking of conversational
//! mode, expected density, and behaviors to avoid.
//!
//! This is NOT durable memory — it is session-scoped and not persisted to the
//! semantic store. It is also NOT a replacement for `SessionMood` (PAD emotional
//! inertia). Session control state tracks conversational intent and constraints,
//! while mood tracks emotional dynamics.
//!
//! Design source: `differentiation-strategy.md` §6.4.C

use crate::contracts::affect::{AffectLabel, AffectReading};
use crate::core::persona::continuity_v2::{DialogueAct, classify_dialogue_act};

pub(crate) use crate::contracts::session_control::{
    AvoidBehavior, ConversationMode, ExpectedDensity, SessionControlState,
};

/// Update session control state from the current turn's signals.
pub(crate) fn update_control_state(
    state: &mut SessionControlState,
    user_message: &str,
    affect: &AffectReading,
) {
    let dialogue_act = classify_dialogue_act(user_message);
    let user_len = user_message.len();

    // Derive mode from dialogue act + affect
    let new_mode = derive_mode(dialogue_act, affect, user_len);
    if new_mode != state.mode {
        state.mode = new_mode;
        state.mode_turns = 0;
    }
    state.mode_turns += 1;

    // Derive expected density
    state.density = derive_density(user_len, dialogue_act, state.mode);

    // Derive avoidance set
    state.avoid.clear();
    collect_avoidances(
        state.mode,
        state.density,
        affect,
        dialogue_act,
        &mut state.avoid,
    );
}

fn derive_mode(
    dialogue_act: DialogueAct,
    affect: &AffectReading,
    user_len: usize,
) -> ConversationMode {
    // Emotional distress → empathy
    if matches!(
        affect.label,
        AffectLabel::Sad | AffectLabel::Anxious | AffectLabel::Overwhelmed
    ) && affect.arousal > 0.3
    {
        return ConversationMode::Empathy;
    }

    // Explicit task request
    if matches!(dialogue_act, DialogueAct::Request) && user_len > 30 {
        return ConversationMode::Task;
    }

    // Question with detail → deep dive
    if matches!(dialogue_act, DialogueAct::Question) && user_len > 60 {
        return ConversationMode::DeepDive;
    }

    // Short casual messages → chitchat
    if user_len < 40
        && matches!(
            dialogue_act,
            DialogueAct::Greet | DialogueAct::Inform | DialogueAct::Thank | DialogueAct::Confirm
        )
    {
        return ConversationMode::Chitchat;
    }

    // Default: stay in current-like mode based on dialogue act
    match dialogue_act {
        DialogueAct::Question | DialogueAct::Clarify => ConversationMode::DeepDive,
        DialogueAct::Request => ConversationMode::Task,
        _ => ConversationMode::Chitchat,
    }
}

fn derive_density(
    user_len: usize,
    dialogue_act: DialogueAct,
    mode: ConversationMode,
) -> ExpectedDensity {
    // Very short input → brief response
    if user_len < 20 {
        return ExpectedDensity::Brief;
    }

    // Greeting/thanks → brief
    if matches!(
        dialogue_act,
        DialogueAct::Greet | DialogueAct::Thank | DialogueAct::Confirm
    ) {
        return ExpectedDensity::Brief;
    }

    // Deep dive or long question → expanded
    if mode == ConversationMode::DeepDive && user_len > 80 {
        return ExpectedDensity::Expanded;
    }

    ExpectedDensity::Normal
}

fn collect_avoidances(
    mode: ConversationMode,
    density: ExpectedDensity,
    affect: &AffectReading,
    _dialogue_act: DialogueAct,
    avoid: &mut Vec<AvoidBehavior>,
) {
    // Chitchat → don't over-explain
    if mode == ConversationMode::Chitchat {
        avoid.push(AvoidBehavior::Overexplain);
    }

    // Brief density → don't over-explain or suddenly organize
    if density == ExpectedDensity::Brief {
        if !avoid.contains(&AvoidBehavior::Overexplain) {
            avoid.push(AvoidBehavior::Overexplain);
        }
        avoid.push(AvoidBehavior::SuddenOrganize);
    }

    // Empathy mode → don't lead with analysis
    if mode == ConversationMode::Empathy {
        avoid.push(AvoidBehavior::AnalysisBeforeEmpathy);
        avoid.push(AvoidBehavior::Preachy);
    }

    // Sad/anxious → don't be preachy
    if matches!(affect.label, AffectLabel::Sad | AffectLabel::Anxious)
        && !avoid.contains(&AvoidBehavior::Preachy)
    {
        avoid.push(AvoidBehavior::Preachy);
    }
}

/// Render the session control state as a prompt guidance block.
#[must_use]
pub(crate) fn render_session_control_block(state: &SessionControlState) -> String {
    let mode_label = match state.mode {
        ConversationMode::Chitchat => "chitchat (keep it light)",
        ConversationMode::Empathy => "empathy (listen, don't fix)",
        ConversationMode::Task => "task (focused, get it done)",
        ConversationMode::DeepDive => "deep dive (explore thoroughly)",
    };
    let density_label = match state.density {
        ExpectedDensity::Brief => "brief (1-2 sentences)",
        ExpectedDensity::Normal => "normal",
        ExpectedDensity::Expanded => "expanded (elaborate)",
    };
    let mut out = format!("[Session Control]\nMode: {mode_label}\nDensity: {density_label}");
    if !state.avoid.is_empty() {
        out.push_str("\nAvoid: ");
        let mut first = true;
        for a in &state.avoid {
            if !first {
                out.push_str(", ");
            }
            first = false;
            out.push_str(match a {
                AvoidBehavior::Overexplain => "over-explaining",
                AvoidBehavior::Preachy => "preachy tone",
                AvoidBehavior::SuddenOrganize => "sudden organizing",
                AvoidBehavior::AnalysisBeforeEmpathy => "analysis before empathy",
            });
        }
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::scores::Confidence;

    fn reading(label: AffectLabel, arousal: f64) -> AffectReading {
        AffectReading {
            label,
            valence: 0.0,
            arousal,
            dominance: 0.5,
            confidence: Confidence::new(0.8),
        }
    }

    #[test]
    fn short_greeting_yields_chitchat_brief() {
        let mut state = SessionControlState::default();
        update_control_state(&mut state, "おはよう", &reading(AffectLabel::Neutral, 0.2));
        assert_eq!(state.mode, ConversationMode::Chitchat);
        assert_eq!(state.density, ExpectedDensity::Brief);
        assert!(state.avoid.contains(&AvoidBehavior::Overexplain));
    }

    #[test]
    fn sad_message_triggers_empathy_mode() {
        let mut state = SessionControlState::default();
        update_control_state(
            &mut state,
            "最近ちょっとつらいことがあって",
            &reading(AffectLabel::Sad, 0.5),
        );
        assert_eq!(state.mode, ConversationMode::Empathy);
        assert!(state.avoid.contains(&AvoidBehavior::AnalysisBeforeEmpathy));
        assert!(state.avoid.contains(&AvoidBehavior::Preachy));
    }

    #[test]
    fn long_request_triggers_task_mode() {
        let mut state = SessionControlState::default();
        // "fix" is an imperative start → classified as Request
        update_control_state(
            &mut state,
            "fix the session management module to use the new binding table design",
            &reading(AffectLabel::Neutral, 0.3),
        );
        assert_eq!(state.mode, ConversationMode::Task);
    }

    #[test]
    fn detailed_question_triggers_deep_dive() {
        let mut state = SessionControlState::default();
        update_control_state(
            &mut state,
            "How does the affect topology system route emotions through the character-specific graph? What happens during diffusion?",
            &reading(AffectLabel::Curious, 0.4),
        );
        assert_eq!(state.mode, ConversationMode::DeepDive);
        assert_eq!(state.density, ExpectedDensity::Expanded);
    }

    #[test]
    fn mode_turns_resets_on_mode_change() {
        let mut state = SessionControlState::default();
        update_control_state(&mut state, "hey", &reading(AffectLabel::Neutral, 0.1));
        assert_eq!(state.mode_turns, 1);
        update_control_state(&mut state, "yo", &reading(AffectLabel::Neutral, 0.1));
        assert_eq!(state.mode_turns, 2);
        // Switch to task mode ("implement" is an imperative start)
        update_control_state(
            &mut state,
            "implement the topology diffusion algorithm correctly for the new config",
            &reading(AffectLabel::Neutral, 0.3),
        );
        assert_eq!(state.mode, ConversationMode::Task);
        assert_eq!(state.mode_turns, 1);
    }

    #[test]
    fn render_block_includes_all_sections() {
        let state = SessionControlState {
            mode: ConversationMode::Empathy,
            density: ExpectedDensity::Brief,
            avoid: vec![AvoidBehavior::AnalysisBeforeEmpathy, AvoidBehavior::Preachy],
            mode_turns: 3,
        };
        let block = render_session_control_block(&state);
        assert!(block.contains("[Session Control]"));
        assert!(block.contains("empathy"));
        assert!(block.contains("brief"));
        assert!(block.contains("analysis before empathy"));
        assert!(block.contains("preachy tone"));
    }
}
