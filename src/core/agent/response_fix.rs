//! Deterministic response repair.
//!
//! [`apply_deterministic_fixes`] drives the fix pipeline in a fixed order:
//! prefix deletions first, suffix deletions second, then body rewrites
//! (outline scaffolding, repetition compression, intensity weakening, salesy
//! language replacement, bullet collapse). Body rewrites are skipped in
//! `Task` and `Report` modes where they would alter content the user expects.
//!
//! Every function in this module returns `None` to signal "no change needed",
//! which keeps the caller's logic simple: `if let Some(updated) = fix(text)`.
//!
//! The fixes are designed to be idempotent: applying `apply_deterministic_fixes`
//! twice on the same text should produce the same result as applying it once.

use super::response_audit::{
    ResponseFixHint, match_templated_leadin_prefix, repetition_key, split_sentences,
};
use super::response_style::ResponseMode;

const OUTLINE_SUMMARY_PREFIXES: [&str; 6] = [
    "結論から言うと、",
    "結論から言うと",
    "要するに、",
    "要するに",
    "つまり、",
    "つまり",
];

const OUTLINE_STEP_PREFIXES: [&str; 4] = ["まず、", "まず", "最初に、", "最初に"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResponseFixResult {
    pub(crate) text: String,
    pub(crate) applied_actions: Vec<ResponseFixHint>,
}

#[must_use]
pub(crate) fn apply_deterministic_fixes(
    text: &str,
    output_mode: ResponseMode,
) -> ResponseFixResult {
    let mut current = text.to_string();
    let mut applied_actions = Vec::new();

    if let Some(updated) = remove_templated_leadin(current.as_str()) {
        current = updated;
        applied_actions.push(ResponseFixHint::DeletePrefix);
    }

    if let Some(updated) = remove_menu_offer_closing(current.as_str()) {
        current = updated;
        applied_actions.push(ResponseFixHint::DeleteSuffix);
    }

    if let Some(updated) = remove_templated_wrap_up(current.as_str()) {
        current = updated;
        applied_actions.push(ResponseFixHint::DeleteSuffix);
    }

    if allows_body_rewrites(output_mode) {
        if let Some(updated) = remove_outline_scaffolding(current.as_str()) {
            current = updated;
            applied_actions.push(ResponseFixHint::DeleteScaffold);
        }

        if let Some(updated) = compress_repetitive_rephrase(current.as_str()) {
            current = updated;
            applied_actions.push(ResponseFixHint::CompressRedundancy);
        }

        if let Some(updated) = weaken_importance_inflation(current.as_str()) {
            current = updated;
            applied_actions.push(ResponseFixHint::WeakenIntensity);
        }

        if let Some(updated) = replace_salesy_language(current.as_str()) {
            current = updated;
            applied_actions.push(ResponseFixHint::ReplaceWithPlainWord);
        }

        if let Some(updated) = collapse_unneeded_bullets(current.as_str()) {
            current = updated;
            applied_actions.push(ResponseFixHint::CollapseBullets);
        }
    }

    ResponseFixResult {
        text: current,
        applied_actions,
    }
}

/// Remove a canned acknowledgment prefix from the start of `text`.
///
/// Returns `None` when no known prefix is found in the first sentence.
fn remove_templated_leadin(text: &str) -> Option<String> {
    let leading = text.len() - text.trim_start().len();
    let trimmed = &text[leading..];
    let sentences = split_sentences(trimmed);
    let first = sentences.first()?;
    let prefix = match_templated_leadin_prefix(first.text.as_str(), sentences.len())?;
    trimmed
        .strip_prefix(prefix)
        .map(|rest| format!("{}{}", &text[..leading], rest.trim_start()))
}

/// Remove a "必要なら〜できます" closing sentence from the end of `text`.
///
/// Returns `None` when the last sentence is not a menu-offer closing.
fn remove_menu_offer_closing(text: &str) -> Option<String> {
    let sentences = split_sentences(text);
    let last = sentences.last()?;
    if !is_menu_offer_closing(last.text.as_str()) {
        return None;
    }

    Some(text[..last.byte_span.start].trim_end().to_string())
}

/// Remove a boilerplate wrap-up sentence ("以上です。") from the end of `text`.
///
/// Only fires when there are at least two sentences, so a single-sentence
/// response beginning with "以上が…" is left untouched.
fn remove_templated_wrap_up(text: &str) -> Option<String> {
    let sentences = split_sentences(text);
    if sentences.len() < 2 {
        return None;
    }

    let last = sentences.last()?;
    if !matches!(
        last.text.as_str(),
        "以上です。" | "以上です" | "以上になります。" | "以上になります"
    ) {
        return None;
    }

    Some(text[..last.byte_span.start].trim_end().to_string())
}

/// Remove outline-scaffolding prefixes ("結論から言うと、", "まず、" in a
/// single-sentence response, etc.) from the start of `text`.
///
/// Returns `None` when no such prefix is found.
fn remove_outline_scaffolding(text: &str) -> Option<String> {
    let leading = text.len() - text.trim_start().len();
    let trimmed = &text[leading..];

    if let Some(prefix) = OUTLINE_SUMMARY_PREFIXES
        .iter()
        .find(|prefix| trimmed.starts_with(**prefix))
    {
        let rest = trimmed.strip_prefix(prefix)?;
        return Some(format!("{}{}", &text[..leading], rest.trim_start()));
    }

    if split_sentences(trimmed).len() != 1 {
        return None;
    }

    OUTLINE_STEP_PREFIXES.iter().find_map(|prefix| {
        trimmed
            .strip_prefix(prefix)
            .map(|rest| format!("{}{}", &text[..leading], rest.trim_start()))
    })
}

/// Remove the second of two semantically duplicate adjacent sentences.
///
/// Uses [`repetition_key`] for normalisation so that paraphrases ("原因は X
/// です。" and "同じ問題が X で起きています。") are correctly identified as
/// duplicates. Returns `None` when no duplicate pair is found.
fn compress_repetitive_rephrase(text: &str) -> Option<String> {
    let sentences = split_sentences(text);
    let duplicate_span = sentences.windows(2).find_map(|window| {
        let previous_key = repetition_key(window[0].text.as_str());
        let current_key = repetition_key(window[1].text.as_str());
        (!previous_key.is_empty() && previous_key == current_key)
            .then_some(window[1].byte_span.clone())
    })?;

    let before = text[..duplicate_span.start].trim_end();
    let after = text[duplicate_span.end..].trim_start();
    let joined = if before.is_empty() {
        after.to_string()
    } else if after.is_empty() {
        before.to_string()
    } else if before.ends_with('\n') || after.starts_with('\n') || before.ends_with('。') {
        format!("{before}{after}")
    } else {
        format!("{before} {after}")
    };
    Some(joined)
}

/// Replace the first occurrence of a hyperbolic importance intensifier with
/// its plain-language equivalent (e.g. "非常に重要な" → "重要な").
///
/// Returns `None` when no such pattern is present.
fn weaken_importance_inflation(text: &str) -> Option<String> {
    const REPLACEMENTS: [(&str, &str); 5] = [
        ("非常に重要な", "重要な"),
        ("非常に重要です", "重要です"),
        ("極めて重要", "重要"),
        ("とても重要", "重要"),
        ("決定的に重要", "重要"),
    ];

    REPLACEMENTS
        .iter()
        .find_map(|(from, to)| text.contains(from).then(|| text.replacen(from, to, 1)))
}

/// Replace the first occurrence of a salesy/marketing-register phrase with a
/// plain-language equivalent (e.g. "魅力的" → "よい").
///
/// Returns `None` when no such pattern is present.
fn replace_salesy_language(text: &str) -> Option<String> {
    const REPLACEMENTS: [(&str, &str); 6] = [
        ("非常に強力で、圧倒的に優れています", "有効です"),
        ("非常に強力", "有効"),
        ("圧倒的に優れています", "有効です"),
        ("魅力的", "よい"),
        ("素晴らしい", "よい"),
        ("シームレス", "自然"),
    ];

    REPLACEMENTS
        .iter()
        .find_map(|(from, to)| text.contains(from).then(|| text.replacen(from, to, 1)))
}

/// Collapse a bullet list into a flat prose sequence when every bullet is a
/// complete sentence and none contains protected content (code, URLs, paths).
///
/// Bullets separated by ASCII sentence-final punctuation get a space inserted
/// between them; CJK-terminal bullets are joined without extra whitespace.
/// Returns `None` when collapsing is not safe or not applicable.
fn collapse_unneeded_bullets(text: &str) -> Option<String> {
    let bullet_lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();

    if bullet_lines.len() < 2 {
        return None;
    }

    let mut sentences = Vec::new();
    for line in bullet_lines {
        let body = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .map(str::trim)?;

        if body.is_empty() || contains_protected_bullet_markers(body) || !ends_like_sentence(body) {
            return None;
        }
        sentences.push(body.to_string());
    }

    let mut collapsed = String::new();
    for sentence in sentences {
        if !collapsed.is_empty() && ends_with_ascii_sentence_punctuation(collapsed.as_str()) {
            collapsed.push(' ');
        }
        collapsed.push_str(sentence.as_str());
    }

    Some(collapsed)
}

/// Return `true` when the `output_mode` allows body-level rewrites.
///
/// Body rewrites (outline scaffolding removal, repetition compression,
/// intensity weakening, salesy language replacement, bullet collapse) are
/// only safe in `Conversation` and `Explanation` modes. In `Task` and
/// `Report` modes the model is expected to produce structured or action-
/// oriented output where these transforms would be harmful.
const fn allows_body_rewrites(output_mode: ResponseMode) -> bool {
    matches!(
        output_mode,
        ResponseMode::Conversation | ResponseMode::Explanation
    )
}

fn is_menu_offer_closing(sentence: &str) -> bool {
    sentence.starts_with("必要なら")
        && !sentence.contains("してください")
        && (sentence.contains("できます")
            || sentence.contains("可能です")
            || sentence.contains("見ます"))
}

fn contains_protected_bullet_markers(body: &str) -> bool {
    body.contains('`')
        || body.contains("http://")
        || body.contains("https://")
        || body.contains('/')
        || body.contains('[')
        || body.contains('{')
        || body.contains('}')
}

fn ends_like_sentence(body: &str) -> bool {
    body.ends_with('。')
        || body.ends_with('！')
        || body.ends_with('？')
        || body.ends_with('.')
        || body.ends_with('!')
        || body.ends_with('?')
}

fn ends_with_ascii_sentence_punctuation(text: &str) -> bool {
    text.ends_with('.') || text.ends_with('!') || text.ends_with('?')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_fix_deletes_templated_leadin() {
        let fixed = apply_deterministic_fixes(
            "いい質問です。原因は接続順です。",
            ResponseMode::Explanation,
        );
        assert_eq!(fixed.text, "原因は接続順です。");
        assert!(
            fixed
                .applied_actions
                .contains(&ResponseFixHint::DeletePrefix)
        );
    }

    #[test]
    fn response_fix_deletes_outline_scaffold_leadin() {
        let fixed = apply_deterministic_fixes(
            "以下に簡潔に説明します。原因は接続順です。",
            ResponseMode::Explanation,
        );
        assert_eq!(fixed.text, "原因は接続順です。");
        assert!(
            fixed
                .applied_actions
                .contains(&ResponseFixHint::DeletePrefix)
        );
    }

    #[test]
    fn response_fix_strips_single_sentence_mostly_prefix() {
        let fixed =
            apply_deterministic_fixes("まず、原因は接続順です。", ResponseMode::Explanation);
        assert_eq!(fixed.text, "原因は接続順です。");
        assert!(
            fixed
                .applied_actions
                .contains(&ResponseFixHint::DeleteScaffold)
        );
    }

    #[test]
    fn response_fix_keeps_real_step_sequence_prefix() {
        let fixed = apply_deterministic_fixes(
            "まず、依存を止めます。次に、再起動します。",
            ResponseMode::Explanation,
        );
        assert_eq!(fixed.text, "まず、依存を止めます。次に、再起動します。");
    }

    #[test]
    fn response_fix_deletes_menu_offer_closing() {
        let fixed = apply_deterministic_fixes(
            "原因はメモリ不足です。必要なら次に切り分けもできます。",
            ResponseMode::Explanation,
        );
        assert_eq!(fixed.text, "原因はメモリ不足です。");
        assert!(
            fixed
                .applied_actions
                .contains(&ResponseFixHint::DeleteSuffix)
        );
    }

    #[test]
    fn response_fix_deletes_templated_wrap_up() {
        let fixed =
            apply_deterministic_fixes("原因は接続順です。以上です。", ResponseMode::Explanation);
        assert_eq!(fixed.text, "原因は接続順です。");
        assert!(
            fixed
                .applied_actions
                .contains(&ResponseFixHint::DeleteSuffix)
        );
    }

    #[test]
    fn response_fix_keeps_meaningful_above_statement() {
        let fixed = apply_deterministic_fixes("以上が理由です。", ResponseMode::Explanation);
        assert_eq!(fixed.text, "以上が理由です。");
    }

    #[test]
    fn response_fix_compresses_repetitive_rephrase() {
        let fixed = apply_deterministic_fixes(
            "原因はキャッシュ不整合です。同じ問題がキャッシュ不整合で起きています。",
            ResponseMode::Explanation,
        );
        assert_eq!(fixed.text, "原因はキャッシュ不整合です。");
        assert!(
            fixed
                .applied_actions
                .contains(&ResponseFixHint::CompressRedundancy)
        );
    }

    #[test]
    fn response_fix_weakens_importance_inflation() {
        let fixed =
            apply_deterministic_fixes("これは非常に重要な問題です。", ResponseMode::Explanation);
        assert_eq!(fixed.text, "これは重要な問題です。");
        assert!(
            fixed
                .applied_actions
                .contains(&ResponseFixHint::WeakenIntensity)
        );
    }

    #[test]
    fn response_fix_flattens_salesy_language() {
        let fixed = apply_deterministic_fixes(
            "この方法は非常に強力で、圧倒的に優れています。",
            ResponseMode::Explanation,
        );
        assert_eq!(fixed.text, "この方法は有効です。");
        assert!(
            fixed
                .applied_actions
                .contains(&ResponseFixHint::ReplaceWithPlainWord)
        );
    }

    #[test]
    fn response_fix_is_idempotent() {
        let once = apply_deterministic_fixes(
            "いい質問です。原因は接続順です。",
            ResponseMode::Explanation,
        );
        let twice = apply_deterministic_fixes(&once.text, ResponseMode::Explanation);
        assert_eq!(once.text, twice.text);
    }

    #[test]
    fn response_fix_keeps_task_mode_as_trim_only() {
        let fixed = apply_deterministic_fixes(
            "いい質問です。これは非常に重要な問題です。必要なら次に見ます。",
            ResponseMode::Task,
        );
        assert_eq!(fixed.text, "これは非常に重要な問題です。");
    }

    #[test]
    fn response_fix_ignores_non_trailing_menu_offer() {
        let fixed = apply_deterministic_fixes(
            "原因はメモリ不足です。必要なら次に見ます。ログは /tmp/app.log にあります。",
            ResponseMode::Explanation,
        );
        assert_eq!(
            fixed.text,
            "原因はメモリ不足です。必要なら次に見ます。ログは /tmp/app.log にあります。"
        );
    }

    #[test]
    fn response_fix_collapses_unneeded_bullets_in_explanations() {
        let fixed = apply_deterministic_fixes(
            "- 原因は接続順です。\n- 依存は壊れていません。",
            ResponseMode::Explanation,
        );
        assert_eq!(fixed.text, "原因は接続順です。依存は壊れていません。");
        assert!(
            fixed
                .applied_actions
                .contains(&ResponseFixHint::CollapseBullets)
        );
    }

    #[test]
    fn response_fix_keeps_bullets_in_task_mode() {
        let fixed = apply_deterministic_fixes(
            "- 原因は接続順です。\n- 依存は壊れていません。",
            ResponseMode::Task,
        );
        assert_eq!(fixed.text, "- 原因は接続順です。\n- 依存は壊れていません。");
    }

    #[test]
    fn response_fix_ignores_bullets_with_protected_content() {
        let fixed = apply_deterministic_fixes(
            "- コマンドは `cargo test` です。\n- ログは /tmp/app.log です。",
            ResponseMode::Explanation,
        );
        assert_eq!(
            fixed.text,
            "- コマンドは `cargo test` です。\n- ログは /tmp/app.log です。"
        );
    }

    #[test]
    fn response_fix_trims_outline_scaffolding_in_short_explanations() {
        let fixed = apply_deterministic_fixes(
            "結論から言うと、原因は接続順です。",
            ResponseMode::Explanation,
        );
        assert_eq!(fixed.text, "原因は接続順です。");
    }

    #[test]
    fn response_fix_keeps_real_step_progression() {
        let fixed = apply_deterministic_fixes(
            "まず、依存を確認します。次に、テストを回します。",
            ResponseMode::Explanation,
        );
        assert_eq!(
            fixed.text,
            "まず、依存を確認します。次に、テストを回します。"
        );
    }
}
