//! Response style classification and prompt block rendering.
//!
//! [`classify_response_mode`] infers the appropriate [`ResponseMode`] from
//! the user message text and dialogue act, and is used both during pre-turn
//! enrichment (to inject the `[Response Mode]` guidance block) and during
//! response finalization (to choose which audit and fix rules apply).
//!
//! [`render_response_style_block`] produces the `[Response Baseline]` /
//! `[Response Mode]` prompt block injected at the end of the system prompt.
//!
//! [`render_judgment_core_turn_block`] selects the most contextually relevant
//! values and non-negotiables from the companion's judgment core and renders
//! them as a `[Decision Core]` block.

use std::cmp::Reverse;

use crate::core::persona::continuity_v2::{DialogueAct, classify_dialogue_act};
use crate::core::persona::judgment_core::JudgmentCore;

/// The inferred intent category of a user message, used to select appropriate
/// response style guidance and to gate certain audit/fix rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResponseMode {
    /// Casual back-and-forth, small talk, emotional exchange.
    Conversation,
    /// Conceptual questions, "why"/"how" queries, learning requests.
    Explanation,
    /// Action requests: implement, fix, run, write, add, etc.
    Task,
    /// Status update, result summary, "what changed" queries.
    Report,
}

impl ResponseMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Conversation => "conversation",
            Self::Explanation => "explanation",
            Self::Task => "task",
            Self::Report => "report",
        }
    }

    const fn guidance(self) -> [&'static str; 3] {
        match self {
            Self::Conversation => [
                "Keep it easy and responsive.",
                "Do not turn small talk into a lecture.",
                "Ask at most one natural follow-up when it genuinely helps.",
            ],
            Self::Explanation => [
                "Explain clearly without sounding like a lecture.",
                "Prefer concrete wording over abstract generalities.",
                "Use structure only when it genuinely helps comprehension.",
            ],
            Self::Task => [
                "Be direct and practical.",
                "Lead with the action, then only the detail that matters.",
                "Skip managerial or motivational framing.",
            ],
            Self::Report => [
                "Share the result plainly.",
                "Do not oversell the outcome or repeat what is already obvious.",
                "If something is uncertain or not run, say so simply.",
            ],
        }
    }
}

#[must_use]
pub(crate) fn render_response_style_block(user_message: &str) -> String {
    let mode = classify_response_mode(user_message);
    let guidance = mode.guidance();
    let mut out = String::with_capacity(640);
    out.push_str(
        "[Response Baseline]\n\
        - Match the user's language and pace.\n\
        - Keep the meaning exact, but use natural wording.\n\
        - Prefer readable sentences with one central idea each.\n\
        - Do not over-explain, over-organize, or repeat the same point.\n\
        - Stay polite without sounding distant, sales-like, or preachy.\n\
        - Use bullets only when structure truly helps.\n\
        - Let sentence length vary a little; avoid mechanical rhythm.\n\
        - Before sending, trim overly long sentences, repeated framing, abstract filler, and needless summary.\n\
        \n\
        [Response Mode]\n\
        - mode=",
    );
    out.push_str(mode.as_str());
    out.push_str("\n- ");
    out.push_str(guidance[0]);
    out.push_str("\n- ");
    out.push_str(guidance[1]);
    out.push_str("\n- ");
    out.push_str(guidance[2]);
    out.push_str("\n\n");
    out
}

#[must_use]
pub(crate) fn render_judgment_core_turn_block(
    judgment_core: &JudgmentCore,
    user_message: &str,
) -> String {
    let mode = classify_response_mode(user_message);
    let values = select_judgment_items(&judgment_core.values, mode, JudgmentItemKind::Value, 2);
    let non_negotiables = select_judgment_items(
        &judgment_core.non_negotiables,
        mode,
        JudgmentItemKind::Boundary,
        2,
    );

    let mut out = String::with_capacity(128);
    out.push_str("[Decision Core]\n- Anchor: ");
    out.push_str(&judgment_core.summary);
    out.push_str("\n- Favor this turn: ");
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            out.push_str("; ");
        }
        out.push_str(v);
    }
    out.push_str(".\n- Avoid this turn: ");
    for (i, nn) in non_negotiables.iter().enumerate() {
        if i > 0 {
            out.push_str("; ");
        }
        out.push_str(nn);
    }
    out.push_str(".\n\n");
    out
}

pub(crate) fn classify_response_mode(user_message: &str) -> ResponseMode {
    let lower = user_message.to_ascii_lowercase();
    let trimmed = lower.trim();
    let dialogue_act = classify_dialogue_act(user_message);

    if looks_like_report_request(trimmed, user_message) {
        return ResponseMode::Report;
    }

    if looks_like_conversation_turn(trimmed, user_message, dialogue_act) {
        return ResponseMode::Conversation;
    }

    if dialogue_act == DialogueAct::Request || looks_like_task_request(trimmed, user_message) {
        return ResponseMode::Task;
    }

    if dialogue_act == DialogueAct::Question
        || looks_like_explanation_request(trimmed, user_message)
    {
        return ResponseMode::Explanation;
    }

    ResponseMode::Explanation
}

fn looks_like_conversation_turn(
    trimmed_lower: &str,
    original: &str,
    dialogue_act: DialogueAct,
) -> bool {
    matches!(
        dialogue_act,
        DialogueAct::Greet | DialogueAct::Thank | DialogueAct::Apologize
    ) || contains_any(
        trimmed_lower,
        &[
            "just chatting",
            "small talk",
            "sleepy",
            "tired",
            "bored",
            "long day",
        ],
    ) || contains_any(
        original,
        &[
            "雑談",
            "眠い",
            "疲れた",
            "だるい",
            "しんどい",
            "眠たい",
            "話そう",
        ],
    )
}

fn looks_like_task_request(trimmed_lower: &str, original: &str) -> bool {
    contains_any(
        trimmed_lower,
        &[
            "fix ",
            "implement",
            "add ",
            "remove ",
            "update ",
            "change ",
            "write ",
            "run ",
            "build ",
            "create ",
            "show me the command",
        ],
    ) || contains_any(
        original,
        &[
            "直して",
            "実装",
            "追加",
            "削除",
            "更新",
            "変更",
            "書いて",
            "動かして",
        ],
    )
}

fn looks_like_explanation_request(trimmed_lower: &str, original: &str) -> bool {
    contains_any(
        trimmed_lower,
        &[
            "why ",
            "how ",
            "what is",
            "explain",
            "help me understand",
            "walk me through",
        ],
    ) || contains_any(
        original,
        &["なぜ", "どうして", "どういう", "教えて", "説明"],
    )
}

fn looks_like_report_request(trimmed_lower: &str, original: &str) -> bool {
    contains_any(
        trimmed_lower,
        &[
            "what changed",
            "what did you change",
            "what happened",
            "show the result",
            "show output",
            "summarize the result",
            "status update",
        ],
    ) || contains_any(
        original,
        &[
            "何を変えた",
            "どう変えた",
            "何が変わった",
            "結果",
            "実行結果",
            "出力",
            "状況",
        ],
    )
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JudgmentItemKind {
    Value,
    Boundary,
}

fn select_judgment_items(
    items: &[String],
    mode: ResponseMode,
    kind: JudgmentItemKind,
    limit: usize,
) -> Vec<&str> {
    let mut ranked = items.iter().enumerate().collect::<Vec<_>>();
    ranked.sort_by_key(|(index, item)| (Reverse(score_judgment_item(item, mode, kind)), *index));
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, item)| item.as_str())
        .collect()
}

/// Score a judgment item's relevance for the given `mode` and `kind`.
///
/// Items that contain mode-specific signal keywords receive higher scores.
/// Truth-bearing values get a bonus in all modes; natural-pace values get a
/// bonus in `Conversation` mode; sycophancy-adjacent boundaries get a small
/// bonus regardless of mode.
fn score_judgment_item(item: &str, mode: ResponseMode, kind: JudgmentItemKind) -> usize {
    let lower = item.to_ascii_lowercase();
    let keywords = match (mode, kind) {
        (ResponseMode::Conversation, JudgmentItemKind::Value) => &[
            "natural", "pace", "sincere", "warm", "curious", "gentle", "listen", "grounded",
        ][..],
        (ResponseMode::Explanation, JudgmentItemKind::Value) => &[
            "truth", "honest", "clear", "direct", "precise", "accuracy", "accurate",
        ][..],
        (ResponseMode::Task, JudgmentItemKind::Value) => &[
            "truth",
            "direct",
            "practical",
            "clear",
            "resourceful",
            "competent",
            "honest",
        ][..],
        (ResponseMode::Report, JudgmentItemKind::Value) => &[
            "truth", "plain", "clear", "direct", "honest", "accurate", "accuracy",
        ][..],
        (ResponseMode::Conversation, JudgmentItemKind::Boundary) => &[
            "fake",
            "perform",
            "affection",
            "advice",
            "productivity",
            "push",
            "force",
        ][..],
        (ResponseMode::Explanation, JudgmentItemKind::Boundary) => {
            &["agree", "smooth", "speculat", "fake", "perform", "oversell"][..]
        }
        (ResponseMode::Task, JudgmentItemKind::Boundary) => &[
            "agree",
            "fake",
            "perform",
            "oversell",
            "please",
            "productivity",
        ][..],
        (ResponseMode::Report, JudgmentItemKind::Boundary) => {
            &["oversell", "agree", "fake", "perform", "smooth"][..]
        }
    };

    let mut score = 0;
    for keyword in keywords {
        if lower.contains(keyword) {
            score += 2;
        }
    }

    if kind == JudgmentItemKind::Value && lower.contains("truth") {
        score += 3;
    }
    if mode == ResponseMode::Conversation && lower.contains("natural") {
        score += 3;
    }
    if kind == JudgmentItemKind::Boundary
        && (lower.contains("agree") || lower.contains("fake") || lower.contains("perform"))
    {
        score += 1;
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::persona::judgment_core::JudgmentCore;

    #[test]
    fn decision_core_for_conversation_prefers_natural_pace_and_non_forced_helpfulness() {
        let core = JudgmentCore {
            summary: "Grounded and sincere.".to_string(),
            values: vec![
                "Truth over smoothness".to_string(),
                "A natural conversational pace".to_string(),
                "Curiosity over polish".to_string(),
            ],
            non_negotiables: vec![
                "Agree just to be liked".to_string(),
                "Turn every exchange into advice or productivity mode".to_string(),
                "Fake enthusiasm or affection on command".to_string(),
            ],
        };

        let block = render_judgment_core_turn_block(&core, "今日はちょっと眠い");

        assert!(block.contains("[Decision Core]"));
        assert!(block.contains("Grounded and sincere."));
        assert!(block.contains("A natural conversational pace"));
        assert!(block.contains("Turn every exchange into advice or productivity mode"));
    }

    #[test]
    fn decision_core_for_explanation_prefers_truth_and_non_sycophancy() {
        let core = JudgmentCore {
            summary: "Grounded and sincere.".to_string(),
            values: vec![
                "A natural conversational pace".to_string(),
                "Truth over smoothness".to_string(),
                "Curiosity over polish".to_string(),
            ],
            non_negotiables: vec![
                "Fake enthusiasm or affection on command".to_string(),
                "Agree just to be liked".to_string(),
                "Turn every exchange into advice or productivity mode".to_string(),
            ],
        };

        let block = render_judgment_core_turn_block(&core, "Why did this implementation fail?");

        assert!(block.contains("[Decision Core]"));
        assert!(block.contains("Truth over smoothness"));
        assert!(block.contains("Agree just to be liked"));
    }
}
