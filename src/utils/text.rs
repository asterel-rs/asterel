//! Text manipulation utilities.
//!
//! Provides truncation with ellipsis, reasoning-block stripping,
//! and other string transformations used across the codebase.

/// Truncate a string to `max_chars` characters, appending `...`
/// if the string was shortened.
#[must_use]
pub(crate) fn truncate_ellipsis(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => {
            let truncated = &s[..idx];
            format!("{}...", truncated.trim_end())
        }
        None => s.to_string(),
    }
}

/// Append a truncated-with-ellipsis view of `s` into an existing `String`
/// buffer. Zero heap allocations beyond the caller's `dst` growth.
pub(crate) fn truncate_ellipsis_into(dst: &mut String, s: &str, max_chars: usize) {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => {
            let truncated = s[..idx].trim_end();
            dst.reserve(truncated.len() + 3);
            dst.push_str(truncated);
            dst.push_str("...");
        }
        None => dst.push_str(s),
    }
}

/// Collapse text that will be embedded in a prompt list item into one line.
#[must_use]
pub(crate) fn sanitize_prompt_line(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\n' | '\r' | '\t' => out.push(' '),
            ch if ch.is_control() => out.push(' '),
            _ => out.push(ch),
        }
    }
    while out.contains("  ") {
        out = out.replace("  ", " ");
    }
    out.trim().to_string()
}

/// Sanitize a raw string into a URL/filesystem-safe slug.
///
/// Non-alphanumeric characters are replaced with hyphens, consecutive
/// hyphens are collapsed, and leading/trailing hyphens are trimmed.
/// Returns `fallback` if the result would be empty.
#[must_use]
pub(crate) fn sanitize_slug(raw: &str, fallback: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_dash = false;

    for ch in raw.trim().chars() {
        let normalized = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '-'
        };

        if normalized == '-' {
            if prev_dash {
                continue;
            }
            prev_dash = true;
            out.push('-');
        } else {
            prev_dash = false;
            out.push(normalized);
        }
    }

    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Return the nearest valid UTF-8 character boundary at or before `index`.
#[must_use]
pub(crate) fn floor_char_boundary(s: &str, index: usize) -> usize {
    let capped = index.min(s.len());
    if s.is_char_boundary(capped) {
        return capped;
    }

    let mut boundary = capped;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

fn starts_with_ascii_ci(haystack: &str, needle: &str) -> bool {
    if haystack.len() < needle.len() {
        return false;
    }

    haystack.as_bytes()[..needle.len()].eq_ignore_ascii_case(needle.as_bytes())
}

const INTERNAL_PROMPT_BLOCK_PREFIXES: &[&str] = &[
    "[Integrated Model]",
    "[Cognitive Scaffolding]",
    "[Attention Focus]",
    "[User Model]",
    "[Relationship Context]",
    "[User Profile]",
    "[User Knowledge]",
    "[Response Baseline]",
    "[Response Mode]",
    "[Decision Core]",
    "[Session Control]",
    "[Affect Topology]",
    "[Desire Objective]",
    "[Taste Contract]",
    "[Taste contract]",
    "[Value Guidance]",
    "[Style profile guidance]",
    "[Personality Guidance]",
    "[Pending Follow-ups]",
    "[Curiosity Signal]",
    "[Counterfactual]",
    "[Self-Contract]",
    "[Self-Model Shadow]",
    "[World Model]",
    "[Narrative Deltas]",
    "[Self-Narrative]",
    "[Memory context]",
    "[Untrusted content]",
    "[Detected Links]",
    "[History]",
    "## Companion State (restored after compaction)",
    "[Conversation state]",
    "[Fact ledger]",
    "[Runtime metadata]",
    "[A2A Provenance]",
    "[A2A Context]",
    "[Channel Style]",
    "[Surface Realization]",
    "[Past Experiences]",
    "[Distilled Principles]",
    "[Companion Memory Graph]",
    "[Behavior Selection]",
    "[Delegation Handoff]",
    "[Extension Contract]",
    "## Available Skills",
    "### Working Memory Focus",
    "[GPE Strategy]",
    "[Current Mood]",
    "[Affect Cause]",
    "[Affect Guidance",
    "[Reasoning:",
];

pub(crate) fn is_internal_prompt_block_header(trimmed_line: &str) -> bool {
    INTERNAL_PROMPT_BLOCK_PREFIXES
        .iter()
        .any(|prefix| trimmed_line.starts_with(prefix))
}

#[must_use]
pub(crate) fn strip_internal_prompt_blocks(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut skipping_block = false;
    let mut skipped_block_content = false;

    for line in text.lines() {
        let trimmed = line.trim();

        if is_internal_prompt_block_header(trimmed) {
            skipping_block = true;
            skipped_block_content = false;
            continue;
        }

        if skipping_block {
            if trimmed.is_empty() && skipped_block_content {
                skipping_block = false;
                skipped_block_content = false;
            } else if !trimmed.is_empty() {
                skipped_block_content = true;
            }
            continue;
        }

        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(line);
    }

    output
}

/// Remove `<think>` and `<reasoning>` tag content from text,
/// preserving content inside code fences and inline code spans.
#[must_use]
pub(crate) fn strip_reasoning(input: &str) -> String {
    const OPEN_THINK: &str = "<think>";
    const CLOSE_THINK: &str = "</think>";
    const OPEN_REASONING: &str = "<reasoning>";
    const CLOSE_REASONING: &str = "</reasoning>";

    let mut output = String::with_capacity(input.len());
    let mut cursor = 0usize;
    let mut redaction_depth = 0usize;
    let mut in_code_fence = false;
    let mut in_inline_code = false;

    while cursor < input.len() {
        let rest = &input[cursor..];

        if !in_inline_code && rest.starts_with("```") {
            if redaction_depth == 0 {
                output.push_str("```");
            }
            in_code_fence = !in_code_fence;
            cursor += 3;
            continue;
        }

        if !in_code_fence && rest.starts_with('`') {
            if redaction_depth == 0 {
                output.push('`');
            }
            in_inline_code = !in_inline_code;
            cursor += 1;
            continue;
        }

        if !in_code_fence && !in_inline_code {
            if starts_with_ascii_ci(rest, OPEN_THINK) || starts_with_ascii_ci(rest, OPEN_REASONING)
            {
                redaction_depth = redaction_depth.saturating_add(1);
                cursor += if starts_with_ascii_ci(rest, OPEN_THINK) {
                    OPEN_THINK.len()
                } else {
                    OPEN_REASONING.len()
                };
                continue;
            }

            if starts_with_ascii_ci(rest, CLOSE_THINK)
                || starts_with_ascii_ci(rest, CLOSE_REASONING)
            {
                redaction_depth = redaction_depth.saturating_sub(1);
                cursor += if starts_with_ascii_ci(rest, CLOSE_THINK) {
                    CLOSE_THINK.len()
                } else {
                    CLOSE_REASONING.len()
                };
                continue;
            }
        }

        let Some(next_char) = rest.chars().next() else {
            break;
        };
        if redaction_depth == 0 {
            output.push(next_char);
        }
        cursor += next_char.len_utf8();
    }

    output
}

/// Remove lines starting with `INFERRED_CLAIM` or
/// `CONTRADICTION_EVENT` markers from the text, and strip internal
/// prompt scaffolding blocks if they leak into model output.
#[must_use]
pub(crate) fn strip_inference_markers(text: &str) -> String {
    let mut filtered = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("INFERRED_CLAIM ") || trimmed.starts_with("CONTRADICTION_EVENT ") {
            continue;
        }
        if !filtered.is_empty() {
            filtered.push('\n');
        }
        filtered.push_str(line);
    }
    let without_blocks = strip_internal_prompt_blocks(&filtered);
    strip_citation_markers(&without_blocks)
}

/// Remove internal grounding citation markers (`[F1]`, `[H2]`, `[C3]`,
/// …) from `text`. The markers exist so `verify_citations` can count
/// them for retrieval-quality measurement, but they have no meaning to
/// the user and must not reach the user-facing surface.
///
/// Pattern is intentionally narrow: only `[F\d+]` / `[H\d+]` /
/// `[C\d+]` is removed. Other bracketed content is untouched.
/// Whitespace runs introduced by the removal are collapsed, and
/// punctuation that ended up with a stray leading space is tidied.
#[must_use]
pub fn strip_citation_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'['
            && i + 2 < bytes.len()
            && matches!(bytes[i + 1], b'F' | b'H' | b'C')
            && bytes[i + 2].is_ascii_digit()
        {
            // Walk past the remaining digits.
            let mut end = i + 3;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end < bytes.len() && bytes[end] == b']' {
                i = end + 1;
                continue;
            }
        }
        // Preserve the next full UTF-8 character.
        let ch = text[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    // Collapse intra-line whitespace runs introduced by removals while
    // preserving line breaks.
    let mut collapsed = String::with_capacity(out.len());
    let mut prev_space = false;
    for ch in out.chars() {
        if ch.is_whitespace() && !matches!(ch, '\n' | '\r') {
            if !prev_space {
                collapsed.push(' ');
            }
            prev_space = true;
        } else {
            collapsed.push(ch);
            prev_space = false;
        }
    }
    let punctuation_tidied = collapsed
        .replace(" .", ".")
        .replace(" ,", ",")
        .replace(" !", "!")
        .replace(" ?", "?")
        .replace(" 。", "。")
        .replace(" 、", "、");
    // Trim trailing whitespace introduced before line breaks (e.g. `"shape [F1]\n"`
    // would otherwise leave `"shape \n"`).
    punctuation_tidied
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Compute keyword overlap score between tokenized message words and a text.
///
/// Returns the fraction of `msg_words` (length > 3) found in `text`
/// (case-insensitive), capped at 1.0. Returns 0.0 for empty `msg_words`.
///
/// The function accepts ALREADY-LOWERCASED words (and lowercases the text once).
/// Callers that hold raw words should use [`keyword_overlap_score_raw`] instead,
/// or, for hot paths, precompute the lowercased word list once per turn and
/// reuse it across many texts.
#[must_use]
pub(crate) fn keyword_overlap_score(msg_words_lower: &[&str], text: &str) -> f64 {
    let lower = text.to_lowercase();
    let mut eligible = 0usize;
    let mut hits = 0usize;
    for w in msg_words_lower {
        if w.len() <= 3 {
            continue;
        }
        eligible += 1;
        if lower.contains(*w) {
            hits += 1;
        }
    }
    if eligible == 0 {
        return 0.0;
    }
    // Cast safety: word counts are far below 2^52.
    #[allow(clippy::cast_precision_loss)]
    {
        (hits as f64 / eligible as f64).min(1.0)
    }
}

/// Build a pre-lowercased word list from a message, preserving words longer
/// than 3 characters. This is the canonical way to amortise the lowercase
/// cost across many `keyword_overlap_score` calls inside a single turn.
#[must_use]
pub(crate) fn lowercase_words_over_len(message: &str, min_len: usize) -> Vec<String> {
    message
        .split_whitespace()
        .filter(|w| w.len() > min_len)
        .map(str::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_ascii_no_truncation() {
        assert_eq!(truncate_ellipsis("hello", 10), "hello");
        assert_eq!(truncate_ellipsis("hello world", 50), "hello world");
    }

    #[test]
    fn truncate_ascii_with_truncation() {
        assert_eq!(truncate_ellipsis("hello world", 5), "hello...");
        assert_eq!(
            truncate_ellipsis("This is a long message", 10),
            "This is a..."
        );
    }

    #[test]
    fn truncate_empty_string() {
        assert_eq!(truncate_ellipsis("", 10), "");
    }

    #[test]
    fn truncate_at_exact_boundary() {
        assert_eq!(truncate_ellipsis("hello", 5), "hello");
    }

    #[test]
    fn truncate_emoji_single() {
        let s = "🦀";
        assert_eq!(truncate_ellipsis(s, 10), s);
        assert_eq!(truncate_ellipsis(s, 1), s);
    }

    #[test]
    fn truncate_emoji_multiple() {
        let s = "😀😀😀😀";
        assert_eq!(truncate_ellipsis(s, 2), "😀😀...");
        assert_eq!(truncate_ellipsis(s, 3), "😀😀😀...");
    }

    #[test]
    fn truncate_mixed_ascii_emoji() {
        assert_eq!(truncate_ellipsis("Hello 🦀 World", 8), "Hello 🦀...");
        assert_eq!(truncate_ellipsis("Hi 😊", 10), "Hi 😊");
    }

    #[test]
    fn truncate_cjk_characters() {
        let s = "这是一个测试消息用来触发崩溃的中文";
        let result = truncate_ellipsis(s, 16);
        assert!(result.ends_with("..."));
        assert!(result.is_char_boundary(result.len() - 1));
    }

    #[test]
    fn truncate_accented_characters() {
        let s = "café résumé naïve";
        assert_eq!(truncate_ellipsis(s, 10), "café résum...");
    }

    #[test]
    fn truncate_unicode_edge_case() {
        let s = "aé你好🦀";
        assert_eq!(truncate_ellipsis(s, 3), "aé你...");
    }

    #[test]
    fn truncate_decomposed_unicode_preserves_char_boundaries() {
        let s = "e\u{0301}cole";
        assert_eq!(truncate_ellipsis(s, 2), "e\u{0301}...");
    }

    #[test]
    fn truncate_with_zero_width_characters_keeps_valid_output() {
        let s = "alpha\u{200B}beta";
        assert_eq!(truncate_ellipsis(s, 6), "alpha\u{200B}...");
    }

    #[test]
    fn truncate_long_string() {
        let s = "a".repeat(200);
        let result = truncate_ellipsis(&s, 50);
        assert_eq!(result.len(), 53);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn truncate_zero_max_chars() {
        assert_eq!(truncate_ellipsis("hello", 0), "...");
    }

    #[test]
    fn strip_reasoning_blocks_removes_think_tag_content() {
        let input = "prefix<think>secret plan</think>suffix";
        assert_eq!(strip_reasoning(input), "prefixsuffix");
    }

    #[test]
    fn strip_reasoning_blocks_removes_reasoning_tag_content_case_insensitive() {
        let input = "a<ReAsOnInG>hidden</rEaSoNiNg>b";
        assert_eq!(strip_reasoning(input), "ab");
    }

    #[test]
    fn strip_reasoning_blocks_preserves_code_fences_and_inline_code() {
        let input = "```xml\n<think>keep in code</think>\n```\n`<reasoning>inline</reasoning>`";
        assert_eq!(strip_reasoning(input), input);
    }

    #[test]
    fn strip_reasoning_blocks_handles_unclosed_tag() {
        let input = "hello<think>secret";
        assert_eq!(strip_reasoning(input), "hello");
    }

    #[test]
    fn strip_reasoning_blocks_preserves_zero_width_chars_outside_tags() {
        let input = "A\u{200B}<think>hidden</think>B\u{200D}C";
        assert_eq!(strip_reasoning(input), "A\u{200B}B\u{200D}C");
    }

    #[test]
    fn strip_inference_markers_removes_only_inference_lines() {
        let input = "A\nINFERRED_CLAIM foo\nB\nCONTRADICTION_EVENT bar\nC";
        assert_eq!(strip_inference_markers(input), "A\nB\nC");
    }

    #[test]
    fn strip_inference_markers_keeps_non_marker_lines() {
        let input = "INFERRED claim not marker\nCONTRADICTION event not marker";
        assert_eq!(strip_inference_markers(input), input);
    }

    #[test]
    fn strip_inference_markers_removes_internal_prompt_blocks() {
        let input = "\
[Integrated Model]
- situational_awareness=0.54
- affordances=none

[Cognitive Scaffolding]
{\"affect\":\"neutral\"}

[Response Baseline]
- Match the user's language and pace.
- Do not over-explain.

夜は静かだけど、
気持ちまで静かとは限らない。";
        assert_eq!(
            strip_inference_markers(input),
            "夜は静かだけど、\n気持ちまで静かとは限らない。"
        );
    }

    #[test]
    fn strip_inference_markers_removes_companion_runtime_prompt_blocks() {
        let input = "\
[Decision Core]
- Anchor: direct answer

[Session Control]
Mode: chitchat

[Affect Topology]
Surface tone: warm

[Desire Objective]
Stay close to the current turn.

[Taste Contract]
- hierarchy: clear

[Value Guidance]
- Prefer grounded detail.

## Companion State (restored after compaction)

Compaction generation: 4

返事だけが残る。";
        assert_eq!(strip_inference_markers(input), "返事だけが残る。");
    }

    #[test]
    fn strip_internal_prompt_blocks_covers_prompt_visible_producer_inventory() {
        let producer_headers = [
            // transport/channels/style_profile.rs::render_channel_style_block
            "[Channel Style]",
            // contracts/channels.rs::SurfaceRealizationPolicy::render_guidance
            "[Surface Realization]",
            // core/experience/presenter.rs
            "[Past Experiences]",
            "[Distilled Principles]",
            // transport/channels/prompt_builder.rs
            "## Available Skills",
            // core/agent/turn_enrichment/turn_enrichment_io.rs::build_working_memory_focus_block
            "### Working Memory Focus",
            // core/memory/graphrag/grounding.rs::render_companion_memory_grounding
            "[Companion Memory Graph]",
            // core/persona/presenter.rs::render_behavior_selection_block
            "[Behavior Selection]",
            // core/subagents/runtime.rs::compose_subagent_task
            "[Delegation Handoff]",
            // plugins/skills/loader/parse.rs::render_extension_contract_block
            "[Extension Contract]",
        ];

        for header in producer_headers {
            assert!(
                is_internal_prompt_block_header(header),
                "prompt producer header must be strip-listed: {header}"
            );
        }

        let mut echoed = String::new();
        for header in producer_headers {
            echoed.push_str(header);
            echoed.push('\n');
            echoed.push_str("internal prompt-only content\n\n");
        }
        echoed.push_str("visible reply");

        let stripped = strip_internal_prompt_blocks(&echoed);
        assert_eq!(stripped, "visible reply");
    }

    #[test]
    fn strip_inference_markers_keeps_non_internal_bracket_sections() {
        let input = "[Playlist]\n- dawn\n- dusk";
        assert_eq!(strip_inference_markers(input), input);
    }

    #[test]
    fn strip_citation_markers_removes_f_h_c_ids() {
        assert_eq!(
            strip_citation_markers("Based on [F1] and [H2], answer follows [C3]."),
            "Based on and, answer follows."
        );
        assert_eq!(strip_citation_markers("[F12] long-form id"), "long-form id");
        assert_eq!(strip_citation_markers("no markers here"), "no markers here");
    }

    #[test]
    fn strip_citation_markers_leaves_other_brackets_alone() {
        assert_eq!(
            strip_citation_markers("[F1] [G1] [Playlist]"),
            "[G1] [Playlist]"
        );
        assert_eq!(strip_citation_markers("[Fa] [F]"), "[Fa] [F]");
    }

    #[test]
    fn strip_citation_markers_preserves_japanese_punctuation() {
        assert_eq!(
            strip_citation_markers("覚えてる [F1]。表紙の色だけ [F2] 。"),
            "覚えてる。表紙の色だけ。"
        );
    }

    #[test]
    fn strip_inference_markers_also_strips_citations() {
        let input = "Listening for the shape [F1]\nINFERRED_CLAIM foo\nGood read [H1].";
        assert_eq!(
            strip_inference_markers(input),
            "Listening for the shape\nGood read."
        );
    }
}
