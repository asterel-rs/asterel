use super::{
    ContentBlock, ContractMismatchReason, ExposurePlanContract, GateDecision, Locale, MessageRole,
    NaturalnessGate, NaturalnessInput, OutputProfile, PreparedNaturalnessContext, ProviderMessage,
    ResponseAuditFindingKind, ResponseAuditReport, ResponseFinalizationRequest, ResponseMode,
    TurnContextView,
};

pub(super) fn verifier_reason_codes(report: &ResponseAuditReport) -> Vec<&'static str> {
    let mut codes = Vec::new();
    for finding in &report.findings {
        let code = match finding.kind {
            ResponseAuditFindingKind::TemplatedLeadin
            | ResponseAuditFindingKind::OutlineScaffolding
            | ResponseAuditFindingKind::MenuOfferClosing
            | ResponseAuditFindingKind::TemplatedWrapUp => "anti_template",
            ResponseAuditFindingKind::RepetitiveRephrase => "repetitive_rephrase",
            ResponseAuditFindingKind::ImportanceInflation
            | ResponseAuditFindingKind::SalesyLanguage => "tone_plainening",
            ResponseAuditFindingKind::UnneededBullets => "bullet_collapse",
            ResponseAuditFindingKind::LectureDrift => "over_explain",
            ResponseAuditFindingKind::Disconnection => "disconnection",
        };
        if !codes.contains(&code) {
            codes.push(code);
        }
    }
    codes
}

pub(super) enum NaturalnessFinalizeDecision {
    Unchanged,
    Patched(String),
    RepairNeeded,
    Blocked(String),
}

pub(super) fn run_naturalness_gate(
    request: ResponseFinalizationRequest<'_>,
    text: &str,
    naturalness_context: &PreparedNaturalnessContext,
) -> NaturalnessFinalizeDecision {
    let gate = NaturalnessGate::default();
    // Context inputs are conservative projections from existing runtime truth:
    // exposure policy, nearby assistant openings, coarse user affect, and
    // relationship distance derived from the canonical relationship state.
    let context = TurnContextView {
        user_affect: naturalness_context.user_affect,
        memory_reference_allowed: request.contract.is_none_or(|contract| {
            !matches!(contract.exposure_plan, ExposurePlanContract::PublicSafe)
        }),
        internal_mechanics_allowed: false,
        relationship_distance: naturalness_context.relationship_distance,
        recent_opening_phrases: naturalness_context.recent_opening_phrases.clone(),
    };
    match gate.check(&NaturalnessInput {
        text,
        locale: detect_locale(text),
        output_profile: output_profile_for_mode(request.output_mode, text),
        turn_context: context,
    }) {
        GateDecision::Pass { .. } => NaturalnessFinalizeDecision::Unchanged,
        GateDecision::Patch { patched_text, .. } => {
            NaturalnessFinalizeDecision::Patched(patched_text)
        }
        GateDecision::RequestRepair { .. } => NaturalnessFinalizeDecision::RepairNeeded,
        GateDecision::Block { safe_fallback, .. } => {
            NaturalnessFinalizeDecision::Blocked(safe_fallback)
        }
    }
}

pub(super) fn recent_assistant_opening_phrases(history: &[ProviderMessage]) -> Vec<String> {
    const MAX_RECENT_OPENINGS: usize = 4;

    history
        .iter()
        .rev()
        .filter_map(assistant_opening_phrase)
        .take(MAX_RECENT_OPENINGS)
        .collect()
}

pub(super) fn assistant_opening_phrase(message: &ProviderMessage) -> Option<String> {
    if !matches!(message.role, MessageRole::Assistant) {
        return None;
    }

    message.content.iter().find_map(|block| match block {
        ContentBlock::Text { text } => opening_phrase(text),
        ContentBlock::ToolUse { .. }
        | ContentBlock::ToolResult { .. }
        | ContentBlock::Image { .. } => None,
    })
}

pub(super) fn opening_phrase(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return None;
    }

    let end = ['\n', '。', '！', '？', '!', '?', '、', ',', ':', '：']
        .iter()
        .filter_map(|delimiter| trimmed.find(*delimiter))
        .min()
        .unwrap_or(trimmed.len());
    let phrase = trimmed[..end].trim();
    if phrase.is_empty() || phrase.chars().count() > 24 {
        return None;
    }

    Some(phrase.to_string())
}

pub(super) fn push_reason_code(codes: &mut Vec<&'static str>, code: &'static str) {
    if !codes.contains(&code) {
        codes.push(code);
    }
}

pub(super) fn detect_locale(text: &str) -> Locale {
    let has_ja = text
        .chars()
        .any(|ch| matches!(ch, '\u{3040}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}'));
    let has_ascii_words = text.chars().any(|ch| ch.is_ascii_alphabetic());
    match (has_ja, has_ascii_words) {
        (true, true) => Locale::Mixed,
        (true, false) => Locale::Ja,
        (false, _) => Locale::En,
    }
}

pub(super) fn output_profile_for_mode(mode: ResponseMode, text: &str) -> OutputProfile {
    match mode {
        ResponseMode::Conversation if text.chars().count() <= 180 => OutputProfile::DiscordShort,
        ResponseMode::Conversation => OutputProfile::DiscordNormal,
        ResponseMode::Explanation => OutputProfile::LongAnalysis,
        ResponseMode::Task => OutputProfile::TechnicalDoc,
        ResponseMode::Report => OutputProfile::SystemNotice,
    }
}

pub(super) fn contract_mismatch_fallback_text(
    reason: ContractMismatchReason,
    raw_text: &str,
) -> String {
    match reason {
        ContractMismatchReason::ExposureViolation => {
            "I can't share that private detail in this context.".to_string()
        }
        ContractMismatchReason::ModeMismatch | ContractMismatchReason::ShapeOverflow => {
            raw_text.to_string()
        }
    }
}

/// Verify that the fix did not alter any semantically load-bearing segments.
///
/// Checks code fences, inline code, Markdown links, quoted strings, JSON
/// fragments, URLs, path tokens, shell-ish lines, digit tokens (numbers,
/// timestamps, percentages), and polarity terms. Returns `false` if any
/// segment set differs between `before` and `after`.
#[must_use]
pub(super) fn protected_segments_match(before: &str, after: &str) -> bool {
    extract_code_fences(before) == extract_code_fences(after)
        && extract_inline_code(before) == extract_inline_code(after)
        && extract_markdown_links(before) == extract_markdown_links(after)
        && extract_quoted_strings(before) == extract_quoted_strings(after)
        && extract_json_like_fragments(before) == extract_json_like_fragments(after)
        && extract_urls(before) == extract_urls(after)
        && extract_path_like_tokens(before) == extract_path_like_tokens(after)
        && extract_shellish_lines(before) == extract_shellish_lines(after)
        && extract_digit_tokens(before) == extract_digit_tokens(after)
        && extract_polarity_terms(before) == extract_polarity_terms(after)
}

/// Return `true` when the response contains `<think>` or `<reasoning>` tags,
/// indicating that a chain-of-thought or extended reasoning block is present.
/// Finalization is skipped in this case to avoid corrupting the reasoning text.
pub(super) fn contains_explicit_reasoning_tags(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("<think>") || lower.contains("<reasoning>")
}

pub(super) fn extract_code_fences(text: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut cursor = 0usize;

    while let Some(start_rel) = text[cursor..].find("```") {
        let start = cursor + start_rel;
        let Some(end_rel) = text[start + 3..].find("```") else {
            break;
        };
        let end = start + 3 + end_rel + 3;
        segments.push(text[start..end].to_string());
        cursor = end;
    }

    segments
}

pub(super) fn extract_inline_code(text: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut cursor = 0usize;
    let mut in_code_fence = false;
    let mut current_start = None;

    while cursor < text.len() {
        let rest = &text[cursor..];
        if rest.starts_with("```") {
            in_code_fence = !in_code_fence;
            cursor += 3;
            continue;
        }

        if !in_code_fence && rest.starts_with('`') {
            if let Some(start) = current_start.take() {
                segments.push(text[start..=cursor].to_string());
            } else {
                current_start = Some(cursor);
            }
            cursor += 1;
            continue;
        }

        let Some(ch) = rest.chars().next() else {
            break;
        };
        cursor += ch.len_utf8();
    }

    segments
}

pub(super) fn extract_urls(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|token| token.starts_with("http://") || token.starts_with("https://"))
        .map(trim_trailing_punctuation)
        .map(ToString::to_string)
        .collect()
}

pub(super) fn extract_markdown_links(text: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut cursor = 0usize;

    while let Some(start_rel) = text[cursor..].find('[') {
        let start = cursor + start_rel;
        let Some(label_end_rel) = text[start..].find("](") else {
            cursor = start + 1;
            continue;
        };
        let url_start = start + label_end_rel + 2;
        let Some(url_end_rel) = text[url_start..].find(')') else {
            break;
        };
        let end = url_start + url_end_rel + 1;
        links.push(text[start..end].to_string());
        cursor = end;
    }

    links
}

pub(super) fn extract_path_like_tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(trim_trailing_punctuation)
        .filter(|token| {
            token.starts_with('/')
                || token.starts_with("./")
                || token.starts_with("../")
                || token.starts_with("~/")
                || (token.contains('/')
                    && !token.starts_with("http://")
                    && !token.starts_with("https://")
                    && !token.starts_with('['))
        })
        .map(ToString::to_string)
        .collect()
}

pub(super) fn extract_digit_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        if ch.is_ascii_digit() || matches!(ch, '-' | ':' | '.' | '/' | '%') {
            current.push(ch);
        } else if !current.is_empty() {
            if current.chars().any(|candidate| candidate.is_ascii_digit()) {
                tokens.push(current.clone());
            }
            current.clear();
        }
    }

    if !current.is_empty() && current.chars().any(|candidate| candidate.is_ascii_digit()) {
        tokens.push(current);
    }

    tokens
}

pub(super) fn extract_quoted_strings(text: &str) -> Vec<String> {
    let mut strings = Vec::new();
    strings.extend(extract_delimited_segments(text, '"', '"'));
    strings.extend(extract_delimited_segments(text, '\'', '\''));
    strings.extend(extract_delimited_segments(text, '「', '」'));
    strings.extend(extract_delimited_segments(text, '『', '』'));
    strings
}

pub(super) fn extract_delimited_segments(text: &str, open: char, close: char) -> Vec<String> {
    let mut segments = Vec::new();
    let mut start = None;

    for (index, ch) in text.char_indices() {
        if ch == open && start.is_none() {
            start = Some(index);
            continue;
        }

        if ch == close
            && let Some(begin) = start.take()
        {
            segments.push(text[begin..index + ch.len_utf8()].to_string());
        }
    }

    segments
}

pub(super) fn extract_json_like_fragments(text: &str) -> Vec<String> {
    let mut fragments = Vec::new();
    fragments.extend(extract_balanced_fragments(text, '{', '}'));
    fragments.extend(extract_balanced_fragments(text, '[', ']'));
    fragments
}

pub(super) fn extract_balanced_fragments(text: &str, open: char, close: char) -> Vec<String> {
    let mut fragments = Vec::new();
    let mut depth = 0usize;
    let mut start = None;

    for (index, ch) in text.char_indices() {
        if ch == open {
            if depth == 0 {
                start = Some(index);
            }
            depth += 1;
            continue;
        }

        if ch == close && depth > 0 {
            depth -= 1;
            if depth == 0
                && let Some(begin) = start.take()
            {
                let end = index + ch.len_utf8();
                let fragment = &text[begin..end];
                if fragment.contains(':') || fragment.contains('"') {
                    fragments.push(fragment.to_string());
                }
            }
        }
    }

    fragments
}

pub(super) fn extract_shellish_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| line.starts_with("> ") || line.starts_with("$ ") || line.starts_with('|'))
        .map(ToString::to_string)
        .collect()
}

/// Collect polarity-bearing terms present in `text`.
///
/// These terms carry semantic polarity (success/failure, existence, etc.) that
/// must be preserved verbatim — changing "できる" to "できない" would invert
/// meaning.
pub(super) fn extract_polarity_terms(text: &str) -> Vec<&'static str> {
    const TERMS: [&str; 10] = [
        "成功",
        "失敗",
        "通りました",
        "通っていません",
        "できる",
        "できない",
        "ある",
        "ない",
        "done",
        "failed",
    ];

    TERMS
        .iter()
        .copied()
        .filter(|term| text.contains(term))
        .collect()
}

pub(super) fn trim_trailing_punctuation(token: &str) -> &str {
    token.trim_end_matches([',', '.', ')', ']', '}', ';'])
}
