//! Web summarize tool — extractive summarization of text content.
//!
//! # What it does
//!
//! `web_summarize` selects the `max_sentences` most informative sentences
//! from a body of text using a purely heuristic scoring function — no LLM
//! call is made. Sentences are scored on four axes:
//!
//! * **Position** (30 %) — earlier sentences receive higher scores, with a
//!   bonus for the first three sentences.
//! * **Length** (20 %) — sentences between 50–200 characters score highest;
//!   sentences outside 20–500 characters are filtered out before scoring.
//! * **Keyword density** (30 %) — overlap between sentence tokens and
//!   title-derived keywords (when a `title` is provided).
//! * **Cue phrases** (20 %) — bonus for sentences containing signal words
//!   such as "important", "conclusion", "therefore", etc.
//!
//! Top-scoring sentences are re-sorted by their original position to preserve
//! narrative coherence in the summary.
//!
//! # Middleware integration
//!
//! This tool requires no `Network` capability because it operates on text
//! supplied by the caller. It is typically chained after `web_fetch` or
//! `web_scrape` to compress the output before it is forwarded to the agent's
//! context window.
#![allow(clippy::cast_precision_loss)]

use std::future::Future;
use std::pin::Pin;

use serde::Deserialize;
use serde_json::json;

use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult, ToolSpec};

const DEFAULT_MAX_SENTENCES: usize = 5;
const MAX_SUMMARY_SENTENCES: usize = 15;
const MIN_SENTENCE_LEN: usize = 20;
const MAX_SENTENCE_LEN: usize = 500;
const IDEAL_SENTENCE_MIN: usize = 50;
const IDEAL_SENTENCE_MAX: usize = 200;
const CUE_PHRASES: [&str; 10] = [
    "important",
    "key",
    "significant",
    "conclusion",
    "summary",
    "result",
    "finding",
    "therefore",
    "however",
    "in conclusion",
];

pub struct WebSummarizeTool;

#[derive(Debug, Deserialize)]
struct SummarizeRequest {
    text: String,
    #[serde(default)]
    max_sentences: Option<usize>,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
struct SummaryOutput {
    summary: String,
    sentence_count: usize,
    original_length: usize,
    compression_ratio: f64,
}

#[derive(Debug, Clone)]
struct ScoredSentence {
    index: usize,
    text: String,
    score: f64,
}

impl WebSummarizeTool {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for WebSummarizeTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for WebSummarizeTool {
    fn name(&self) -> &'static str {
        "web_summarize"
    }

    fn description(&self) -> &'static str {
        "Summarize text content using extractive summarization. Selects the most important sentences by position, keyword density, and length. No LLM call — pure heuristic."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "Text content to summarize"
                },
                "max_sentences": {
                    "type": "integer",
                    "description": "Max sentences in summary (default 5, max 15)"
                },
                "title": {
                    "type": "string",
                    "description": "Optional: page title for keyword boosting"
                }
            },
            "required": ["text"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let request: SummarizeRequest = serde_json::from_value(args)?;
            let max_sentences = sanitize_max_sentences(request.max_sentences);
            let summary = summarize_text(&request.text, max_sentences, request.title.as_deref());
            let output = serde_json::to_string(&json!({
                "summary": summary.summary,
                "sentence_count": summary.sentence_count,
                "original_length": summary.original_length,
                "compression_ratio": summary.compression_ratio,
            }))?;

            Ok(ToolResult {
                success: true,
                output,
                error: None,
                attachments: Vec::new(),
                taint_labels: Vec::new(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            })
        })
    }

    fn spec(&self) -> ToolSpec {
        let name = self.name().to_string();
        let effect = crate::contracts::tools::ToolEffect::classify(&name);
        ToolSpec {
            name,
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
            required_capabilities: vec![],
            effect,
        }
    }
}

fn summarize_text(text: &str, max_sentences: usize, title: Option<&str>) -> SummaryOutput {
    let original = text.trim();
    let original_length = original.chars().count();

    if original.is_empty() {
        return SummaryOutput {
            summary: String::new(),
            sentence_count: 0,
            original_length,
            compression_ratio: 0.0,
        };
    }

    let sentences = split_sentences(original);
    if sentences.is_empty() {
        return build_passthrough_summary(original, original_length);
    }

    if sentences.len() == 1 && original_length < MIN_SENTENCE_LEN {
        return build_passthrough_summary(original, original_length);
    }

    let filtered_sentences: Vec<(usize, String)> = sentences
        .into_iter()
        .enumerate()
        .filter_map(|(index, sentence)| {
            // Fast lower-bound: byte len < MIN guarantees char count < MIN.
            if sentence.len() < MIN_SENTENCE_LEN {
                return None;
            }
            let length = sentence.chars().count();
            (MIN_SENTENCE_LEN..=MAX_SENTENCE_LEN)
                .contains(&length)
                .then_some((index, sentence))
        })
        .collect();

    if filtered_sentences.is_empty() {
        return build_passthrough_summary(original, original_length);
    }

    let title_keywords = title.map(normalize_keywords).unwrap_or_default();
    let total_sentences = filtered_sentences.len();
    let mut scored = filtered_sentences
        .into_iter()
        .enumerate()
        .map(|(rank, (index, sentence))| ScoredSentence {
            index,
            score: sentence_score(&sentence, rank, total_sentences, &title_keywords),
            text: sentence,
        })
        .collect::<Vec<_>>();

    scored.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.index.cmp(&right.index))
    });

    let limit = max_sentences.min(scored.len());
    let mut selected = scored.into_iter().take(limit).collect::<Vec<_>>();
    selected.sort_by_key(|sentence| sentence.index);

    let mut summary = String::new();
    for sentence in &selected {
        if !summary.is_empty() {
            summary.push('\n');
        }
        summary.push_str(&sentence.text);
    }
    let sentence_count = selected.len();
    let compression_ratio = if original_length == 0 {
        0.0
    } else {
        let summary_length = summary.chars().count() as f64;
        let original_length = original_length as f64;
        summary_length / original_length
    };

    SummaryOutput {
        summary,
        sentence_count,
        original_length,
        compression_ratio,
    }
}

fn build_passthrough_summary(text: &str, original_length: usize) -> SummaryOutput {
    SummaryOutput {
        summary: text.to_string(),
        sentence_count: usize::from(!text.is_empty()),
        original_length,
        compression_ratio: 1.0,
    }
}

fn sanitize_max_sentences(value: Option<usize>) -> usize {
    value
        .unwrap_or(DEFAULT_MAX_SENTENCES)
        .clamp(1, MAX_SUMMARY_SENTENCES)
}

fn split_sentences(text: &str) -> Vec<String> {
    let normalized = text.replace("\r\n", "\n");
    let chars = normalized.chars().collect::<Vec<_>>();
    let mut sentences = Vec::new();
    let mut start = 0usize;
    let mut index = 0usize;

    while index < chars.len() {
        let current = chars[index];
        let next = chars.get(index + 1).copied();
        let should_split = match current {
            '.' | '!' | '?' => next.is_none_or(char::is_whitespace),
            '\n' => true,
            _ => false,
        };

        if should_split {
            let end = if current == '\n' { index } else { index + 1 };
            push_sentence(&chars[start..end], &mut sentences);

            index += 1;
            while index < chars.len() && chars[index].is_whitespace() {
                index += 1;
            }
            start = index;
            continue;
        }

        index += 1;
    }

    if start < chars.len() {
        push_sentence(&chars[start..], &mut sentences);
    }

    sentences
}

fn push_sentence(chars: &[char], sentences: &mut Vec<String>) {
    let sentence = chars.iter().collect::<String>();
    let trimmed = sentence.trim();
    if !trimmed.is_empty() {
        sentences.push(trimmed.to_string());
    }
}

fn sentence_score(
    sentence: &str,
    rank: usize,
    total_sentences: usize,
    title_keywords: &[String],
) -> f64 {
    let position = position_score(rank, total_sentences);
    let length = length_score(sentence);
    let keyword = keyword_density_score(sentence, title_keywords);
    let cue = cue_phrase_score(sentence);

    (position * 0.3) + (length * 0.2) + (keyword * 0.3) + (cue * 0.2)
}

fn position_score(rank: usize, total_sentences: usize) -> f64 {
    if total_sentences <= 1 {
        return 1.0;
    }

    let denominator = (total_sentences - 1) as f64;
    let decay = 1.0 - ((rank as f64) / denominator);
    let early_bonus = if rank < 3 { 0.3 } else { 0.0 };
    (decay + early_bonus).clamp(0.0, 1.0)
}

fn length_score(sentence: &str) -> f64 {
    // Fast lower-bound: byte len < MIN guarantees char count < MIN.
    if sentence.len() < MIN_SENTENCE_LEN {
        return 0.0;
    }
    let length = sentence.chars().count();

    if !(MIN_SENTENCE_LEN..=MAX_SENTENCE_LEN).contains(&length) {
        return 0.0;
    }

    if (IDEAL_SENTENCE_MIN..=IDEAL_SENTENCE_MAX).contains(&length) {
        return 1.0;
    }

    if length < IDEAL_SENTENCE_MIN {
        return (length as f64 / IDEAL_SENTENCE_MIN as f64).clamp(0.0, 1.0);
    }

    let overage = length.saturating_sub(IDEAL_SENTENCE_MAX);
    let penalty_window = (MAX_SENTENCE_LEN - IDEAL_SENTENCE_MAX) as f64;
    (1.0 - (overage as f64 / penalty_window)).clamp(0.0, 1.0)
}

fn keyword_density_score(sentence: &str, title_keywords: &[String]) -> f64 {
    if title_keywords.is_empty() {
        return 0.0;
    }

    let sentence_keywords = normalize_keywords(sentence);
    if sentence_keywords.is_empty() {
        return 0.0;
    }

    let matches = title_keywords
        .iter()
        .filter(|keyword| {
            sentence_keywords
                .iter()
                .any(|candidate| candidate == *keyword)
        })
        .count();

    (matches as f64 / title_keywords.len() as f64).clamp(0.0, 1.0)
}

fn cue_phrase_score(sentence: &str) -> f64 {
    let normalized = sentence.to_ascii_lowercase();
    if CUE_PHRASES.iter().any(|phrase| normalized.contains(phrase)) {
        1.0
    } else {
        0.0
    }
}

fn normalize_keywords(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .map(str::trim)
        .filter(|word| word.len() >= 3)
        .map(str::to_ascii_lowercase)
        .fold(Vec::new(), |mut keywords, keyword| {
            if !keywords.contains(&keyword) {
                keywords.push(keyword);
            }
            keywords
        })
}

#[cfg(test)]
mod tests {
    use super::{WebSummarizeTool, summarize_text};
    use crate::core::tools::traits::Tool;

    #[test]
    fn basic_summarization_produces_fewer_sentences_than_input() {
        let text = concat!(
            "Rust is a systems programming language focused on safety and performance. ",
            "It helps developers prevent memory bugs through ownership rules. ",
            "The compiler provides detailed feedback during development. ",
            "Teams often adopt Rust for reliable infrastructure and networking services."
        );

        let summary = summarize_text(text, 2, None);

        assert_eq!(summary.sentence_count, 2);
        assert!(summary.summary.lines().count() < 4);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn empty_input_returns_empty_summary() {
        let summary = summarize_text("   ", 5, None);

        assert!(summary.summary.is_empty());
        assert_eq!(summary.sentence_count, 0);
        assert_eq!(summary.compression_ratio, 0.0);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn very_short_input_returns_input_as_is() {
        let summary = summarize_text("Tiny update.", 5, None);

        assert_eq!(summary.summary, "Tiny update.");
        assert_eq!(summary.sentence_count, 1);
        assert_eq!(summary.compression_ratio, 1.0);
    }

    #[test]
    fn position_bias_prefers_early_sentences() {
        let text = concat!(
            "This opening sentence explains the main topic clearly and gives important framing for the article. ",
            "This second sentence adds essential context that remains central to understanding the rest. ",
            "A later sentence mentions a minor tangent about a small implementation detail. ",
            "The final sentence closes with an aside that is less useful than the introduction."
        );

        let summary = summarize_text(text, 2, None);

        assert!(
            summary
                .summary
                .contains("This opening sentence explains the main topic clearly")
        );
        assert!(
            summary
                .summary
                .contains("This second sentence adds essential context")
        );
    }

    #[test]
    fn tool_spec_has_no_required_capabilities() {
        let tool = WebSummarizeTool::new();
        assert!(tool.spec().required_capabilities.is_empty());
    }
}
