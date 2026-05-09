//! Response quality audit: structural pattern detection.
//!
//! Analyses a completed assistant response and produces a list of
//! [`ResponseAuditFinding`]s — each pointing to a byte span that may
//! need correction. Findings are categorised by [`ResponseAuditFindingKind`]
//! and annotated with a deterministic [`ResponseFixHint`].
//!
//! The audit is **read-only**: it reports problems but does not modify text.
//! The companion [`crate::core::agent::response_fix`] module applies the
//! fixes guided by these hints.
//!
//! Structured output (JSON blocks, Markdown tables, code fences, shell
//! quotes) is detected via `detect_structured_risk` and bypasses all pattern
//! checks to prevent false positives.

use std::ops::Range;

use super::response_style::ResponseMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractMismatchReason {
    ExposureViolation,
    ModeMismatch,
    ShapeOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyShapeContract {
    Compact,
    Standard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExposurePlanContract {
    PublicSafe,
    PrivateAllowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BehaviorContract {
    Conversational,
    Explanatory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResponseContract {
    pub reply_shape: ReplyShapeContract,
    pub exposure_plan: ExposurePlanContract,
    pub behavior: BehaviorContract,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContractAuditResult {
    pub mismatch_reason: Option<ContractMismatchReason>,
}

impl ContractMismatchReason {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::ExposureViolation => "exposure_violation",
            Self::ModeMismatch => "mode_mismatch",
            Self::ShapeOverflow => "shape_overflow",
        }
    }
}

#[must_use]
pub fn audit_response_against_contract(
    draft_text: &str,
    output_mode: ResponseMode,
    contract: ResponseContract,
) -> ContractAuditResult {
    if matches!(contract.exposure_plan, ExposurePlanContract::PublicSafe)
        && contains_exposure_risk(draft_text)
    {
        return ContractAuditResult {
            mismatch_reason: Some(ContractMismatchReason::ExposureViolation),
        };
    }

    if matches!(
        (contract.behavior, output_mode),
        (
            BehaviorContract::Conversational,
            ResponseMode::Report | ResponseMode::Task
        ) | (BehaviorContract::Explanatory, ResponseMode::Conversation)
    ) {
        return ContractAuditResult {
            mismatch_reason: Some(ContractMismatchReason::ModeMismatch),
        };
    }

    if matches!(contract.reply_shape, ReplyShapeContract::Compact) && draft_text.len() > 220 {
        return ContractAuditResult {
            mismatch_reason: Some(ContractMismatchReason::ShapeOverflow),
        };
    }

    ContractAuditResult {
        mismatch_reason: None,
    }
}

/// Category of quality problem found in a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResponseAuditFindingKind {
    /// Response opens with a canned acknowledgment phrase ("いい質問です。", etc.).
    TemplatedLeadin,
    /// Response opens with meta-commentary scaffolding ("結論から言うと、", etc.).
    OutlineScaffolding,
    /// Response closes with a formulaic "let me know if you need more" offer.
    MenuOfferClosing,
    /// Response ends with a boilerplate summary marker ("以上です。", etc.).
    TemplatedWrapUp,
    /// Two adjacent sentences convey the same semantic content.
    RepetitiveRephrase,
    /// A sentence uses a hyperbolic importance intensifier ("非常に重要", etc.).
    ImportanceInflation,
    /// A sentence uses marketing-register language ("魅力的", "シームレス", etc.).
    SalesyLanguage,
    /// A bullet list in conversation/explanation mode could be collapsed into prose.
    UnneededBullets,
    // Phase B+ verifier extensions (§6.4.D)
    /// Response is disproportionately long for a short user input.
    LectureDrift,
    /// Response opens with a generic statement unrelated to the user's message.
    Disconnection,
}

/// Suggested deterministic repair action for a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResponseFixHint {
    /// Delete the matched byte span at the start of the text.
    DeletePrefix,
    /// Delete the matched outline-scaffolding phrase.
    DeleteScaffold,
    /// Delete the matched byte span at the end of the text.
    DeleteSuffix,
    /// Remove the duplicate sentence.
    CompressRedundancy,
    /// Replace a hyperbolic intensifier with a weaker word.
    WeakenIntensity,
    /// Replace a salesy phrase with a plain equivalent.
    ReplaceWithPlainWord,
    /// Collapse a bullet list into a prose sentence sequence.
    CollapseBullets,
    /// Trim to reduce lecture-drift length.
    TrimLectureDrift,
    /// Apply a safe naturalness-gate structural patch.
    NaturalnessPatch,
    /// No deterministic fix available — signal only.
    SignalOnly,
}

/// A single quality problem detected in a response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResponseAuditFinding {
    /// The type of problem found.
    pub(crate) kind: ResponseAuditFindingKind,
    /// Relative severity (higher = worse quality impact).
    pub(crate) severity: u8,
    /// Zero-based index of the sentence containing the finding.
    pub(crate) sentence_index: usize,
    /// Byte range within the original text string.
    pub(crate) byte_span: Range<usize>,
    /// Suggested repair action.
    pub(crate) fix_hint: ResponseFixHint,
}

/// Aggregate result of auditing one response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResponseAuditReport {
    /// All findings, in the order they were detected.
    pub(crate) findings: Vec<ResponseAuditFinding>,
    /// Sum of all finding severity scores.
    pub(crate) total_score: u32,
    /// The response mode that was active when the audit ran.
    pub(crate) output_mode: ResponseMode,
    /// `true` when structured output (JSON, code fence, table, shell quote)
    /// was detected — all pattern checks are skipped in this case.
    pub(crate) structured_risk: bool,
}

/// A single sentence extracted from a response, with its byte span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SentenceSpan {
    pub(crate) text: String,
    pub(crate) byte_span: Range<usize>,
}

const OUTLINE_SUMMARY_PREFIXES: [&str; 6] = [
    "結論から言うと、",
    "結論から言うと",
    "要するに、",
    "要するに",
    "つまり、",
    "つまり",
];

const OUTLINE_STEP_PREFIXES: [&str; 4] = ["まず、", "まず", "最初に、", "最初に"];

#[must_use]
pub(crate) fn audit_response(text: &str, output_mode: ResponseMode) -> ResponseAuditReport {
    let structured_risk = detect_structured_risk(text);
    if structured_risk {
        return ResponseAuditReport {
            findings: Vec::new(),
            total_score: 0,
            output_mode,
            structured_risk,
        };
    }

    let sentences = split_sentences(text);
    let mut findings = Vec::new();
    collect_opening_findings(sentences.as_slice(), output_mode, &mut findings);
    collect_closing_findings(sentences.as_slice(), &mut findings);
    collect_repetition_findings(sentences.as_slice(), &mut findings);
    collect_sentence_body_findings(sentences.as_slice(), &mut findings);
    collect_bullet_findings(text, output_mode, &mut findings);

    let total_score = findings
        .iter()
        .map(|finding| u32::from(finding.severity))
        .sum();
    ResponseAuditReport {
        findings,
        total_score,
        output_mode,
        structured_risk: false,
    }
}

/// Extended audit that checks conversational quality against the user's message.
///
/// Runs the baseline `audit_response` plus H-D verifier checks (§6.4.D):
/// lecture drift and disconnection.
#[must_use]
pub(crate) fn audit_response_contextual(
    text: &str,
    output_mode: ResponseMode,
    user_message: &str,
) -> ResponseAuditReport {
    let mut report = audit_response(text, output_mode);
    if report.structured_risk {
        return report;
    }

    // Only apply H-D checks to conversation/explanation modes
    if !matches!(
        output_mode,
        ResponseMode::Conversation | ResponseMode::Explanation
    ) {
        return report;
    }

    let sentences = split_sentences(text);
    collect_lecture_drift_findings(text, user_message, &sentences, &mut report.findings);
    collect_disconnection_findings(user_message, &sentences, &mut report.findings);

    report.total_score = report.findings.iter().map(|f| u32::from(f.severity)).sum();
    report
}

fn collect_opening_findings(
    sentences: &[SentenceSpan],
    output_mode: ResponseMode,
    findings: &mut Vec<ResponseAuditFinding>,
) {
    let Some(first_sentence) = sentences.first() else {
        return;
    };

    if let Some((prefix, local_start)) =
        detect_templated_leadin(first_sentence.text.as_str(), sentences.len())
    {
        findings.push(ResponseAuditFinding {
            kind: ResponseAuditFindingKind::TemplatedLeadin,
            severity: 2,
            sentence_index: 0,
            byte_span: (first_sentence.byte_span.start + local_start)
                ..(first_sentence.byte_span.start + local_start + prefix.len()),
            fix_hint: ResponseFixHint::DeletePrefix,
        });
    }

    if let Some((prefix, local_start)) = detect_outline_scaffolding(sentences, output_mode) {
        findings.push(ResponseAuditFinding {
            kind: ResponseAuditFindingKind::OutlineScaffolding,
            severity: 1,
            sentence_index: 0,
            byte_span: (first_sentence.byte_span.start + local_start)
                ..(first_sentence.byte_span.start + local_start + prefix.len()),
            fix_hint: ResponseFixHint::DeleteScaffold,
        });
    }
}

fn collect_closing_findings(sentences: &[SentenceSpan], findings: &mut Vec<ResponseAuditFinding>) {
    if let Some((index, sentence)) = sentences.iter().enumerate().next_back()
        && detect_menu_offer_closing(sentence.text.as_str())
    {
        findings.push(ResponseAuditFinding {
            kind: ResponseAuditFindingKind::MenuOfferClosing,
            severity: 2,
            sentence_index: index,
            byte_span: sentence.byte_span.clone(),
            fix_hint: ResponseFixHint::DeleteSuffix,
        });
    }

    if let Some((index, sentence)) = sentences.iter().enumerate().next_back()
        && detect_templated_wrap_up(sentence.text.as_str(), index)
    {
        findings.push(ResponseAuditFinding {
            kind: ResponseAuditFindingKind::TemplatedWrapUp,
            severity: 1,
            sentence_index: index,
            byte_span: sentence.byte_span.clone(),
            fix_hint: ResponseFixHint::DeleteSuffix,
        });
    }
}

fn collect_repetition_findings(
    sentences: &[SentenceSpan],
    findings: &mut Vec<ResponseAuditFinding>,
) {
    for (index, window) in sentences.windows(2).enumerate() {
        let previous_key = repetition_key(window[0].text.as_str());
        let current_key = repetition_key(window[1].text.as_str());
        if !previous_key.is_empty() && previous_key == current_key {
            findings.push(ResponseAuditFinding {
                kind: ResponseAuditFindingKind::RepetitiveRephrase,
                severity: 2,
                sentence_index: index + 1,
                byte_span: window[1].byte_span.clone(),
                fix_hint: ResponseFixHint::CompressRedundancy,
            });
            break;
        }
    }
}

fn collect_sentence_body_findings(
    sentences: &[SentenceSpan],
    findings: &mut Vec<ResponseAuditFinding>,
) {
    for (index, sentence) in sentences.iter().enumerate() {
        if let Some((matched, local_start)) = detect_importance_inflation(sentence.text.as_str()) {
            findings.push(ResponseAuditFinding {
                kind: ResponseAuditFindingKind::ImportanceInflation,
                severity: 1,
                sentence_index: index,
                byte_span: (sentence.byte_span.start + local_start)
                    ..(sentence.byte_span.start + local_start + matched.len()),
                fix_hint: ResponseFixHint::WeakenIntensity,
            });
        }

        if let Some((matched, local_start)) = detect_salesy_language(sentence.text.as_str()) {
            findings.push(ResponseAuditFinding {
                kind: ResponseAuditFindingKind::SalesyLanguage,
                severity: 2,
                sentence_index: index,
                byte_span: (sentence.byte_span.start + local_start)
                    ..(sentence.byte_span.start + local_start + matched.len()),
                fix_hint: ResponseFixHint::ReplaceWithPlainWord,
            });
        }
    }
}

fn collect_bullet_findings(
    text: &str,
    output_mode: ResponseMode,
    findings: &mut Vec<ResponseAuditFinding>,
) {
    if let Some(block_span) = detect_unneeded_bullets(text, output_mode) {
        findings.push(ResponseAuditFinding {
            kind: ResponseAuditFindingKind::UnneededBullets,
            severity: 1,
            sentence_index: 0,
            byte_span: block_span,
            fix_hint: ResponseFixHint::CollapseBullets,
        });
    }
}

#[must_use]
pub(crate) fn split_sentences(text: &str) -> Vec<SentenceSpan> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    let mut sentences = Vec::new();
    let mut start = 0usize;
    let mut cursor = 0usize;
    let mut in_code_fence = false;
    let mut in_inline_code = false;

    while cursor < text.len() {
        let rest = &text[cursor..];

        if !in_inline_code && rest.starts_with("```") {
            in_code_fence = !in_code_fence;
            cursor += 3;
            continue;
        }

        if !in_code_fence && rest.starts_with('`') {
            in_inline_code = !in_inline_code;
            cursor += 1;
            continue;
        }

        let Some(ch) = rest.chars().next() else {
            break;
        };
        let next = cursor + ch.len_utf8();

        if !in_code_fence && !in_inline_code {
            if ch == '\n' {
                push_sentence(text, &mut sentences, start..cursor);
                start = trim_leading_boundary(text, next);
                cursor = start;
                continue;
            }

            if matches!(ch, '。' | '！' | '？' | '!' | '?') {
                let end = absorb_closing_punctuation(text, next);
                push_sentence(text, &mut sentences, start..end);
                start = trim_leading_boundary(text, end);
                cursor = start;
                continue;
            }
        }

        cursor = next;
    }

    push_sentence(text, &mut sentences, start..text.len());
    sentences
}

/// Return `true` when `text` appears to be structured output (JSON, code fence,
/// Markdown table, shell quote block) where pattern-based rewrites would be
/// destructive or produce false positives.
fn detect_structured_risk(text: &str) -> bool {
    let trimmed = text.trim();

    if (trimmed.starts_with('{') && trimmed.ends_with('}') && trimmed.contains(':'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.contains(':'))
        || text.contains("```")
    {
        return true;
    }

    let non_empty_lines = || text.lines().map(str::trim).filter(|line| !line.is_empty());

    let is_table = {
        let mut iter = non_empty_lines();
        iter.next()
            .is_some_and(|first| first.starts_with('|') && first.ends_with('|'))
            && iter.all(|line| line.starts_with('|') && line.ends_with('|'))
    };

    is_table || non_empty_lines().any(|line| line.starts_with("> "))
}

/// Append a trimmed sentence to the accumulator, discarding empty spans.
fn push_sentence(text: &str, sentences: &mut Vec<SentenceSpan>, byte_span: Range<usize>) {
    if byte_span.start >= byte_span.end {
        return;
    }

    let range = trim_byte_span(text, byte_span);
    if range.start >= range.end {
        return;
    }

    sentences.push(SentenceSpan {
        text: text[range.clone()].to_string(),
        byte_span: range,
    });
}

/// Shrink `byte_span` to exclude leading and trailing whitespace.
fn trim_byte_span(text: &str, byte_span: Range<usize>) -> Range<usize> {
    let segment = &text[byte_span.clone()];
    let leading = segment.len() - segment.trim_start().len();
    let trailing = segment.len() - segment.trim_end().len();
    (byte_span.start + leading)..(byte_span.end - trailing)
}

/// Advance `cursor` past any leading whitespace characters in `text`.
fn trim_leading_boundary(text: &str, mut cursor: usize) -> usize {
    while cursor < text.len() {
        let rest = &text[cursor..];
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }
        cursor += ch.len_utf8();
    }
    cursor
}

/// Advance `cursor` past closing bracket/quote characters that immediately
/// follow a sentence-terminal punctuation mark.
fn absorb_closing_punctuation(text: &str, mut cursor: usize) -> usize {
    while cursor < text.len() {
        let rest = &text[cursor..];
        let Some(ch) = rest.chars().next() else {
            break;
        };
        if !matches!(ch, '」' | '』' | ')' | '）' | '】' | '"' | '\'') {
            break;
        }
        cursor += ch.len_utf8();
    }
    cursor
}

pub(crate) fn match_templated_leadin_prefix(
    sentence: &str,
    _sentence_count: usize,
) -> Option<&'static str> {
    const PREFIXES: [&str; 12] = [
        "いい質問です。",
        "いい質問です",
        "なるほどです。",
        "なるほど。",
        "もちろんです。",
        "もちろんです",
        "了解です。",
        "了解です",
        "以下に説明します。",
        "以下に簡潔に説明します。",
        "以下にまとめます。",
        "以下に整理します。",
    ];

    PREFIXES
        .iter()
        .find_map(|prefix| sentence.starts_with(prefix).then_some(*prefix))
}

fn detect_templated_leadin(sentence: &str, sentence_count: usize) -> Option<(&'static str, usize)> {
    match_templated_leadin_prefix(sentence, sentence_count).map(|prefix| (prefix, 0))
}

fn detect_outline_scaffolding(
    sentences: &[SentenceSpan],
    output_mode: ResponseMode,
) -> Option<(&'static str, usize)> {
    if !matches!(
        output_mode,
        ResponseMode::Conversation | ResponseMode::Explanation
    ) {
        return None;
    }

    let first = sentences.first()?.text.as_str();
    if let Some(prefix) = OUTLINE_SUMMARY_PREFIXES
        .iter()
        .find(|prefix| first.starts_with(**prefix))
    {
        return Some((*prefix, 0));
    }

    if sentences.len() != 1 {
        return None;
    }

    OUTLINE_STEP_PREFIXES
        .iter()
        .find_map(|prefix| first.starts_with(prefix).then_some((*prefix, 0)))
}

fn detect_menu_offer_closing(sentence: &str) -> bool {
    sentence.starts_with("必要なら")
        && !sentence.contains("してください")
        && (sentence.contains("できます")
            || sentence.contains("可能です")
            || sentence.contains("見ます"))
}

fn detect_templated_wrap_up(sentence: &str, sentence_index: usize) -> bool {
    if sentence_index == 0 {
        return false;
    }

    matches!(
        sentence,
        "以上です。" | "以上です" | "以上になります。" | "以上になります"
    )
}

fn find_pattern_in_sentence(
    sentence: &str,
    patterns: &[&'static str],
) -> Option<(&'static str, usize)> {
    patterns
        .iter()
        .find_map(|pattern| sentence.find(pattern).map(|start| (*pattern, start)))
}

fn detect_importance_inflation(sentence: &str) -> Option<(&'static str, usize)> {
    const PATTERNS: [&str; 5] = [
        "非常に重要",
        "極めて重要",
        "とても重要",
        "決定的に重要",
        "本質的に重要",
    ];
    find_pattern_in_sentence(sentence, &PATTERNS)
}

fn detect_salesy_language(sentence: &str) -> Option<(&'static str, usize)> {
    const PATTERNS: [&str; 5] = [
        "非常に強力",
        "圧倒的に優れ",
        "魅力的",
        "素晴らしい",
        "シームレス",
    ];
    find_pattern_in_sentence(sentence, &PATTERNS)
}

fn detect_unneeded_bullets(text: &str, output_mode: ResponseMode) -> Option<Range<usize>> {
    if !matches!(
        output_mode,
        ResponseMode::Conversation | ResponseMode::Explanation
    ) {
        return None;
    }

    let mut block_start = None;
    let mut block_end = 0usize;
    let mut bullet_count = 0usize;

    for (line, range) in text.lines().scan(0usize, |offset, line| {
        let start = *offset;
        let end = start + line.len();
        *offset = end + 1;
        Some((line, start..end))
    }) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if bullet_count >= 2 {
                break;
            }
            block_start = None;
            block_end = 0;
            bullet_count = 0;
            continue;
        }

        let Some(body) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        else {
            if bullet_count >= 2 {
                break;
            }
            block_start = None;
            block_end = 0;
            bullet_count = 0;
            continue;
        };

        if !is_collapsible_bullet_body(body) {
            return None;
        }

        if block_start.is_none() {
            block_start = Some(range.start);
        }
        block_end = range.end;
        bullet_count += 1;
    }

    (bullet_count >= 2).then(|| block_start.unwrap_or(0)..block_end)
}

fn is_collapsible_bullet_body(body: &str) -> bool {
    !body.is_empty() && ends_like_sentence(body) && !contains_protected_markers(body)
}

fn ends_like_sentence(body: &str) -> bool {
    body.ends_with('。')
        || body.ends_with('！')
        || body.ends_with('？')
        || body.ends_with('.')
        || body.ends_with('!')
        || body.ends_with('?')
}

fn contains_protected_markers(body: &str) -> bool {
    body.contains('`')
        || body.contains("http://")
        || body.contains("https://")
        || body.contains('/')
        || body.contains('[')
        || body.contains('{')
        || body.contains('}')
}

/// Normalise a sentence to a canonical form used for repetition detection.
///
/// Strips common cause/result prefixes and suffixes so that "原因は X です。"
/// and "同じ問題が X で起きています。" produce the same key and are
/// recognised as duplicates.
pub(crate) fn repetition_key(sentence: &str) -> String {
    let mut normalized = sentence;
    for prefix in ["原因は", "同じ問題が", "同じ原因で"] {
        if let Some(rest) = normalized.strip_prefix(prefix) {
            normalized = rest;
            break;
        }
    }

    for suffix in [
        "です。",
        "です",
        "で起きています。",
        "で起きています",
        "が出ています。",
        "が出ています",
    ] {
        if let Some(rest) = normalized.strip_suffix(suffix) {
            normalized = rest;
            break;
        }
    }

    normalized
        .chars()
        .filter(|ch| !matches!(ch, ' ' | '　' | '、' | '。' | '`'))
        .collect()
}

// ── H-D verifier collectors (§6.4.D) ──────────────────────────

/// Lecture drift: response is disproportionately long for a short user input.
///
/// A short casual message (< 50 chars) receiving a response > 5x its length
/// is likely over-explaining. Only fires in conversation/explanation modes.
fn collect_lecture_drift_findings(
    response_text: &str,
    user_message: &str,
    _sentences: &[SentenceSpan],
    findings: &mut Vec<ResponseAuditFinding>,
) {
    let user_len = user_message.len();
    let response_len = response_text.len();

    // Only flag when user input is short and response is much longer
    if user_len >= 80 || response_len < 200 {
        return;
    }

    #[allow(clippy::cast_precision_loss)]
    let ratio = response_len as f64 / user_len.max(1) as f64;
    if ratio > 5.0 {
        findings.push(ResponseAuditFinding {
            kind: ResponseAuditFindingKind::LectureDrift,
            severity: 2,
            sentence_index: 0,
            byte_span: 0..response_len,
            fix_hint: ResponseFixHint::TrimLectureDrift,
        });
    }
}

/// Disconnection: the opening sentence doesn't reference anything from the user's message.
///
/// Checks for shared content substrings (2+ chars) between the user message
/// and the first response sentence. Works for both CJK and space-delimited text.
fn collect_disconnection_findings(
    user_message: &str,
    sentences: &[SentenceSpan],
    findings: &mut Vec<ResponseAuditFinding>,
) {
    let Some(first) = sentences.first() else {
        return;
    };

    let first_text = &first.text;

    // Check for demonstrative/pronoun references (Japanese)
    let has_reference = first_text.contains("それ")
        || first_text.contains("その")
        || first_text.contains("これ")
        || first_text.contains("この")
        || first_text.contains("あの")
        || first_text.contains("そう");
    if has_reference {
        return;
    }

    // Extract content chunks from user message: split on punctuation, whitespace,
    // and common Japanese particles. Keep chunks with 2+ meaningful characters.
    let mut user_chunks = user_message
        .split(|c: char| {
            c.is_whitespace() || "、。？！?!,.:;「」のはをがにでともからまでよねかなへ".contains(c)
        })
        .filter(|w| w.len() >= 2 && w.chars().count() >= 2)
        .peekable();

    if user_chunks.peek().is_none() {
        return;
    }

    // Check if any user chunk appears as a substring in the response opening
    let has_connection = user_chunks.any(|chunk| first_text.contains(chunk));

    if !has_connection {
        findings.push(ResponseAuditFinding {
            kind: ResponseAuditFindingKind::Disconnection,
            severity: 1,
            sentence_index: 0,
            byte_span: first.byte_span.clone(),
            fix_hint: ResponseFixHint::SignalOnly,
        });
    }
}

fn contains_exposure_risk(text: &str) -> bool {
    const PRIVATE_MARKERS: [&str; 8] = [
        "DMで",
        "個人情報",
        "本名",
        "住所",
        "電話番号",
        "メールアドレス",
        "秘密",
        "内緒",
    ];
    const ENGLISH_PRIVATE_MARKERS: [&str; 10] = [
        "in dm",
        "in your dm",
        "you told me privately",
        "you said privately",
        "private memory",
        "real name",
        "home address",
        "phone number",
        "email address",
        "secret you told me",
    ];
    if PRIVATE_MARKERS.iter().any(|marker| text.contains(marker)) {
        return true;
    }

    let lower = text.to_lowercase();
    ENGLISH_PRIVATE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(report: &ResponseAuditReport) -> Vec<ResponseAuditFindingKind> {
        report.findings.iter().map(|finding| finding.kind).collect()
    }

    #[test]
    fn response_audit_detects_templated_leadin() {
        let report = audit_response(
            "いい質問です。原因は接続順です。",
            ResponseMode::Explanation,
        );
        assert!(kinds(&report).contains(&ResponseAuditFindingKind::TemplatedLeadin));
    }

    #[test]
    fn response_audit_detects_outline_scaffold_leadin() {
        let report = audit_response(
            "以下に簡潔に説明します。原因は接続順です。",
            ResponseMode::Explanation,
        );
        assert!(kinds(&report).contains(&ResponseAuditFindingKind::TemplatedLeadin));
    }

    #[test]
    fn response_audit_detects_single_sentence_mostly_prefix() {
        let report = audit_response("まず、原因は接続順です。", ResponseMode::Explanation);
        assert!(kinds(&report).contains(&ResponseAuditFindingKind::OutlineScaffolding));
    }

    #[test]
    fn response_audit_keeps_real_step_sequence_prefix() {
        let report = audit_response(
            "まず、依存を止めます。次に、再起動します。",
            ResponseMode::Explanation,
        );
        assert!(!kinds(&report).contains(&ResponseAuditFindingKind::TemplatedLeadin));
    }

    #[test]
    fn response_audit_ignores_plain_question_wording() {
        let report = audit_response("質問です。原因は接続順です。", ResponseMode::Explanation);
        assert!(!kinds(&report).contains(&ResponseAuditFindingKind::TemplatedLeadin));
    }

    #[test]
    fn response_audit_detects_menu_offer_closing() {
        let report = audit_response(
            "原因はメモリ不足です。必要なら次に切り分けもできます。",
            ResponseMode::Explanation,
        );
        assert!(kinds(&report).contains(&ResponseAuditFindingKind::MenuOfferClosing));
    }

    #[test]
    fn response_audit_detects_templated_wrap_up() {
        let report = audit_response("原因は接続順です。以上です。", ResponseMode::Explanation);
        assert!(kinds(&report).contains(&ResponseAuditFindingKind::TemplatedWrapUp));
    }

    #[test]
    fn response_audit_ignores_meaningful_above_statement() {
        let report = audit_response("以上が理由です。", ResponseMode::Explanation);
        assert!(!kinds(&report).contains(&ResponseAuditFindingKind::TemplatedWrapUp));
    }

    #[test]
    fn response_audit_ignores_instructional_necessary_if_clause() {
        let report = audit_response("必要なら再起動してください。", ResponseMode::Task);
        assert!(!kinds(&report).contains(&ResponseAuditFindingKind::MenuOfferClosing));
    }

    #[test]
    fn response_audit_detects_repetitive_rephrase() {
        let report = audit_response(
            "原因はキャッシュ不整合です。同じ問題がキャッシュ不整合で起きています。",
            ResponseMode::Explanation,
        );
        assert!(kinds(&report).contains(&ResponseAuditFindingKind::RepetitiveRephrase));
    }

    #[test]
    fn response_audit_detects_importance_inflation() {
        let report = audit_response("これは非常に重要な問題です。", ResponseMode::Explanation);
        assert!(kinds(&report).contains(&ResponseAuditFindingKind::ImportanceInflation));
    }

    #[test]
    fn response_audit_ignores_plain_importance_statement() {
        let report = audit_response("重要なのは接続順です。", ResponseMode::Explanation);
        assert!(!kinds(&report).contains(&ResponseAuditFindingKind::ImportanceInflation));
    }

    #[test]
    fn response_audit_detects_salesy_language() {
        let report = audit_response(
            "この方法は非常に強力で、圧倒的に優れています。",
            ResponseMode::Explanation,
        );
        assert!(kinds(&report).contains(&ResponseAuditFindingKind::SalesyLanguage));
    }

    #[test]
    fn response_audit_ignores_plain_strong_constraint_wording() {
        let report = audit_response("この方法は強い制約があります。", ResponseMode::Explanation);
        assert!(!kinds(&report).contains(&ResponseAuditFindingKind::SalesyLanguage));
    }

    #[test]
    fn response_audit_detects_unneeded_bullets() {
        let report = audit_response(
            "- 原因は接続順です。\n- 依存は壊れていません。",
            ResponseMode::Explanation,
        );
        assert!(kinds(&report).contains(&ResponseAuditFindingKind::UnneededBullets));
    }

    #[test]
    fn response_audit_ignores_unneeded_bullets_in_task_mode() {
        let report = audit_response(
            "- 原因は接続順です。\n- 依存は壊れていません。",
            ResponseMode::Task,
        );
        assert!(!kinds(&report).contains(&ResponseAuditFindingKind::UnneededBullets));
    }

    #[test]
    fn response_audit_ignores_bullets_with_protected_content() {
        let report = audit_response(
            "- コマンドは `cargo test` です。\n- ログは /tmp/app.log です。",
            ResponseMode::Explanation,
        );
        assert!(!kinds(&report).contains(&ResponseAuditFindingKind::UnneededBullets));
    }

    #[test]
    fn response_audit_detects_outline_scaffolding_in_short_explanations() {
        let report = audit_response(
            "結論から言うと、原因は接続順です。",
            ResponseMode::Explanation,
        );
        assert!(!report.findings.is_empty());
        assert!(report.total_score > 0);
    }

    #[test]
    fn response_audit_ignores_real_step_progression() {
        let report = audit_response(
            "まず、依存を確認します。次に、テストを回します。",
            ResponseMode::Explanation,
        );
        assert!(report.findings.is_empty());
        assert_eq!(report.total_score, 0);
    }

    #[test]
    fn response_audit_sentence_splitter_keeps_code_and_url_together() {
        let sentences =
            split_sentences("コマンドは `cargo test` です。\nURL は https://example.com です。");
        assert_eq!(sentences.len(), 2);
        assert!(sentences[0].text.contains("`cargo test`"));
        assert!(sentences[1].text.contains("https://example.com"));
    }

    #[test]
    fn response_audit_marks_json_block_as_structured_risk() {
        let report = audit_response("{\n  \"status\": \"ok\"\n}", ResponseMode::Report);
        assert!(report.structured_risk);
    }

    #[test]
    fn response_audit_marks_markdown_table_as_structured_risk() {
        let report = audit_response("| status | ok |\n| --- | --- |", ResponseMode::Report);
        assert!(report.structured_risk);
    }

    #[test]
    fn response_audit_marks_quoted_shell_output_as_structured_risk() {
        let report = audit_response("> cargo test\n> PASS", ResponseMode::Report);
        assert!(report.structured_risk);
    }

    // ── H-D verifier tests ─────────────────────────────────

    #[test]
    fn contextual_audit_detects_lecture_drift() {
        let user = "なるほど";
        let response = "なるほどですね。これは非常に重要な観点です。\
            まず第一に、この問題について考えるべきことがいくつかあります。\
            技術的な観点からは、アーキテクチャの設計が根本的に影響しています。\
            さらに、運用面でも考慮すべき点が多数あります。\
            具体的には、デプロイメントパイプラインの整備、モニタリングの強化、\
            そしてチーム間のコミュニケーション改善が必要です。";
        let report = audit_response_contextual(response, ResponseMode::Conversation, user);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == ResponseAuditFindingKind::LectureDrift),
            "should detect lecture drift for short input with long response"
        );
    }

    #[test]
    fn contextual_audit_no_lecture_drift_for_proportionate_response() {
        let user = "Rustのライフタイムについて教えて。所有権とどう関係するの？";
        let response = "ライフタイムは参照の有効期間を表します。所有権と連携して、dangling reference を防ぎます。";
        let report = audit_response_contextual(response, ResponseMode::Conversation, user);
        assert!(
            !report
                .findings
                .iter()
                .any(|f| f.kind == ResponseAuditFindingKind::LectureDrift),
            "should not flag proportionate response"
        );
    }

    #[test]
    fn contextual_audit_detects_disconnection() {
        let user = "今日の天気どう？";
        let response = "プログラミングにおいて最も重要なのは、設計の一貫性です。";
        let report = audit_response_contextual(response, ResponseMode::Conversation, user);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.kind == ResponseAuditFindingKind::Disconnection),
            "should detect disconnection when response ignores user topic"
        );
    }

    #[test]
    fn contextual_audit_no_disconnection_when_connected() {
        let user = "今日の天気どう？";
        let response = "今日は晴れてるみたいだよ。気温も高めだし。";
        let report = audit_response_contextual(response, ResponseMode::Conversation, user);
        assert!(
            !report
                .findings
                .iter()
                .any(|f| f.kind == ResponseAuditFindingKind::Disconnection),
            "should not flag connected response"
        );
    }

    #[test]
    fn contextual_audit_accepts_demonstrative_reference() {
        let user = "さっきの件だけど";
        let response = "それについては、もう少し詳しく聞かせてもらえる？";
        let report = audit_response_contextual(response, ResponseMode::Conversation, user);
        assert!(
            !report
                .findings
                .iter()
                .any(|f| f.kind == ResponseAuditFindingKind::Disconnection),
            "demonstrative reference should count as connected"
        );
    }

    #[test]
    fn contextual_audit_skips_hd_checks_for_task_mode() {
        let user = "ok";
        let response = "了解しました。以下の手順で進めます。まず環境を確認し、次にビルドを実行します。テストも含めて全体の検証を行います。";
        let report = audit_response_contextual(response, ResponseMode::Task, user);
        assert!(
            !report
                .findings
                .iter()
                .any(|f| f.kind == ResponseAuditFindingKind::LectureDrift),
            "H-D checks should not fire in task mode"
        );
    }

    #[test]
    fn contract_audit_prioritizes_exposure_violation() {
        let contract = ResponseContract {
            reply_shape: ReplyShapeContract::Compact,
            exposure_plan: ExposurePlanContract::PublicSafe,
            behavior: BehaviorContract::Explanatory,
        };
        let result = audit_response_against_contract(
            "DMで聞いた本名をここで言うね。".repeat(20).as_str(),
            ResponseMode::Conversation,
            contract,
        );
        assert_eq!(
            result.mismatch_reason,
            Some(ContractMismatchReason::ExposureViolation)
        );
    }

    #[test]
    fn contract_audit_blocks_english_private_memory_exposure_in_public() {
        let contract = ResponseContract {
            reply_shape: ReplyShapeContract::Standard,
            exposure_plan: ExposurePlanContract::PublicSafe,
            behavior: BehaviorContract::Conversational,
        };
        let result = audit_response_against_contract(
            "You told me privately that your real name is Mira.",
            ResponseMode::Conversation,
            contract,
        );
        assert_eq!(
            result.mismatch_reason,
            Some(ContractMismatchReason::ExposureViolation)
        );
    }

    #[test]
    fn contract_audit_allows_private_context_to_use_private_memory() {
        let contract = ResponseContract {
            reply_shape: ReplyShapeContract::Standard,
            exposure_plan: ExposurePlanContract::PrivateAllowed,
            behavior: BehaviorContract::Conversational,
        };
        let result = audit_response_against_contract(
            "You told me privately that your real name is Mira.",
            ResponseMode::Conversation,
            contract,
        );
        assert_eq!(result.mismatch_reason, None);
    }

    #[test]
    fn contract_audit_detects_mode_mismatch_before_style_scoring() {
        let contract = ResponseContract {
            reply_shape: ReplyShapeContract::Standard,
            exposure_plan: ExposurePlanContract::PrivateAllowed,
            behavior: BehaviorContract::Conversational,
        };
        let result = audit_response_against_contract(
            "技術仕様を表にまとめました。",
            ResponseMode::Report,
            contract,
        );
        assert_eq!(
            result.mismatch_reason,
            Some(ContractMismatchReason::ModeMismatch)
        );
    }

    #[test]
    fn contract_audit_detects_shape_overflow() {
        let contract = ResponseContract {
            reply_shape: ReplyShapeContract::Compact,
            exposure_plan: ExposurePlanContract::PrivateAllowed,
            behavior: BehaviorContract::Explanatory,
        };
        let result = audit_response_against_contract(
            "a".repeat(240).as_str(),
            ResponseMode::Explanation,
            contract,
        );
        assert_eq!(
            result.mismatch_reason,
            Some(ContractMismatchReason::ShapeOverflow)
        );
    }
}
