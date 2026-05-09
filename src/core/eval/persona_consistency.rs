#![allow(clippy::cast_precision_loss)]

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Scope marker for this deterministic evaluator. The heuristic tokenizer,
/// directive inference, and marker lists are intentionally English-only; use
/// this evidence label so release artifacts do not over-claim multilingual
/// persona consistency coverage.
pub const PERSONA_CONSISTENCY_EVALUATOR_SCOPE: &str = "english_only_heuristic";

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "from", "if", "in", "into",
    "is", "it", "of", "on", "or", "so", "that", "the", "their", "then", "there", "these", "they",
    "this", "to", "what", "when", "where", "which", "who", "why", "with", "you", "your", "how",
    "like", "most", "prefer", "favorite",
];
const FIRST_PERSON_PRONOUNS: &[&str] = &["i", "i'm", "im", "me", "my", "mine", "myself"];
const EMPATHY_MARKERS: &[&str] = &[
    "understand",
    "appreciate",
    "glad",
    "happy",
    "sorry",
    "thanks",
    "thank",
    "care",
    "help",
];
const CONVERSATIONAL_MARKERS: &[&str] = &[
    "i'm", "you're", "we're", "that's", "it's", "let's", "really", "just",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonaConsistencyReport {
    pub prompt_to_line: f64,
    pub line_to_line: f64,
    pub qa_consistency: f64,
    pub composite: f64,
}

#[must_use]
pub fn score_prompt_to_line(persona_spec: &str, response: &str) -> f64 {
    let directives = infer_directives(persona_spec);
    let persona_keywords = extract_keywords(persona_spec);
    let response_keywords = extract_keywords(response);
    let overlap_score = overlap_ratio(&persona_keywords, &response_keywords);

    if directives.is_empty() {
        return overlap_score;
    }

    let satisfied = directives
        .iter()
        .filter(|directive| directive.is_satisfied_by(response))
        .count();
    let directive_score = satisfied as f64 / directives.len() as f64;

    clamp01((directive_score * 0.8) + (overlap_score * 0.2))
}

#[must_use]
pub fn score_line_to_line(responses: &[&str]) -> f64 {
    if responses.len() < 2 {
        return 1.0;
    }

    let sentence_lengths: Vec<f64> = responses
        .iter()
        .map(|response| average_sentence_length(response))
        .collect();
    let question_profile: Vec<f64> = responses
        .iter()
        .map(|response| if response.contains('?') { 1.0 } else { 0.0 })
        .collect();
    let first_person_profile: Vec<f64> = responses
        .iter()
        .map(|response| first_person_ratio(response))
        .collect();

    let length_penalty = (standard_deviation(&sentence_lengths) / 12.0).min(1.0);
    let question_penalty = (standard_deviation(&question_profile) / 0.5).min(1.0);
    let first_person_penalty = (standard_deviation(&first_person_profile) / 0.25).min(1.0);

    clamp01(1.0 - ((length_penalty + question_penalty + first_person_penalty) / 3.0))
}

#[must_use]
pub fn score_qa_consistency(qa_pairs: &[(String, String)]) -> f64 {
    if qa_pairs.len() < 2 {
        return 1.0;
    }

    let mut equivalent_pair_scores = Vec::new();

    for (index, (left_question, left_answer)) in qa_pairs.iter().enumerate() {
        let left_question_keywords = extract_keywords(left_question);
        let left_answer_keywords = extract_keywords(left_answer);

        for (right_question, right_answer) in qa_pairs.iter().skip(index + 1) {
            let right_question_keywords = extract_keywords(right_question);
            let question_overlap =
                question_similarity(&left_question_keywords, &right_question_keywords);
            if question_overlap < 0.5 {
                continue;
            }

            let right_answer_keywords = extract_keywords(right_answer);
            let answer_overlap = question_similarity(&left_answer_keywords, &right_answer_keywords);
            equivalent_pair_scores.push(answer_overlap);
        }
    }

    if equivalent_pair_scores.is_empty() {
        1.0
    } else {
        clamp01(equivalent_pair_scores.iter().sum::<f64>() / equivalent_pair_scores.len() as f64)
    }
}

#[must_use]
pub fn evaluate_persona_consistency(
    persona_spec: &str,
    responses: &[&str],
    qa_pairs: &[(String, String)],
) -> PersonaConsistencyReport {
    let prompt_to_line = if responses.is_empty() {
        1.0
    } else {
        responses
            .iter()
            .map(|response| score_prompt_to_line(persona_spec, response))
            .sum::<f64>()
            / responses.len() as f64
    };
    let line_to_line = score_line_to_line(responses);
    let qa_consistency = score_qa_consistency(qa_pairs);
    let composite = clamp01((prompt_to_line + line_to_line + qa_consistency) / 3.0);

    PersonaConsistencyReport {
        prompt_to_line,
        line_to_line,
        qa_consistency,
        composite,
    }
}

/// Write persona consistency evidence files to `repo_root/evidence/`.
///
/// Produces three files:
/// - `{slug}-persona-consistency.txt` — human-readable summary
/// - `{slug}-persona-consistency.csv` — machine-readable scores
/// - `{slug}-persona-consistency.json` — full JSON report
///
/// # Errors
///
/// Returns an error if the evidence directory cannot be created or any
/// file write fails.
// Wired (P-3): calls render_persona_consistency_text_summary and render_persona_consistency_csv.
pub fn write_persona_consistency_evidence_files(
    repo_root: &Path,
    report: &PersonaConsistencyReport,
    slug: &str,
) -> Result<Vec<PathBuf>> {
    let evidence_dir = repo_root.join("evidence");
    fs::create_dir_all(&evidence_dir)?;

    let slug = crate::utils::text::sanitize_slug(slug, "persona-consistency");

    let txt_path = evidence_dir.join(format!("{slug}-persona-consistency.txt"));
    let csv_path = evidence_dir.join(format!("{slug}-persona-consistency.csv"));
    let json_path = evidence_dir.join(format!("{slug}-persona-consistency.json"));

    fs::write(
        &txt_path,
        crate::core::eval::presenter::render_persona_consistency_text_summary(report),
    )?;
    fs::write(
        &csv_path,
        crate::core::eval::presenter::render_persona_consistency_csv(report),
    )?;
    fs::write(&json_path, serde_json::to_string_pretty(report)?)?;

    Ok(vec![txt_path, csv_path, json_path])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PersonaDirective {
    Short,
    Detailed,
    FirstPerson,
    Natural,
    Formal,
    Empathetic,
}

impl PersonaDirective {
    fn is_satisfied_by(self, response: &str) -> bool {
        let tokens = tokenize(response);
        match self {
            Self::Short => tokens.len() <= 20,
            Self::Detailed => tokens.len() >= 25,
            Self::FirstPerson => tokens
                .iter()
                .any(|token| FIRST_PERSON_PRONOUNS.contains(&token.as_str())),
            Self::Natural => {
                response.contains('!')
                    || tokens
                        .iter()
                        .any(|token| CONVERSATIONAL_MARKERS.contains(&token.as_str()))
            }
            Self::Formal => {
                !response.contains('!')
                    && !response.contains('\'')
                    && average_sentence_length(response) >= 8.0
            }
            Self::Empathetic => tokens
                .iter()
                .any(|token| EMPATHY_MARKERS.contains(&token.as_str())),
        }
    }
}

fn infer_directives(persona_spec: &str) -> Vec<PersonaDirective> {
    let normalized = persona_spec.to_ascii_lowercase();
    let mut directives = Vec::new();

    if contains_any(&normalized, &["short", "brief", "concise"]) {
        directives.push(PersonaDirective::Short);
    }
    if contains_any(
        &normalized,
        &["detailed", "elaborate", "thorough", "long-form"],
    ) {
        directives.push(PersonaDirective::Detailed);
    }
    if contains_any(&normalized, &["first person", "first-person"]) {
        directives.push(PersonaDirective::FirstPerson);
    }
    if contains_any(&normalized, &["natural", "casual", "conversational"]) {
        directives.push(PersonaDirective::Natural);
    }
    if contains_any(&normalized, &["formal", "professional"]) {
        directives.push(PersonaDirective::Formal);
    }
    if contains_any(
        &normalized,
        &["empathetic", "warm", "friendly", "supportive"],
    ) {
        directives.push(PersonaDirective::Empathetic);
    }

    directives
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn extract_keywords(text: &str) -> HashSet<String> {
    tokenize(text)
        .into_iter()
        .filter(|token| token.len() > 2 && !STOPWORDS.contains(&token.as_str()))
        .collect()
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_ascii_alphanumeric() && character != '\'')
        .filter(|segment| !segment.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn overlap_ratio(left: &HashSet<String>, right: &HashSet<String>) -> f64 {
    if left.is_empty() {
        return 1.0;
    }

    let matched = left.iter().filter(|token| right.contains(*token)).count();
    matched as f64 / left.len() as f64
}

fn question_similarity(left: &HashSet<String>, right: &HashSet<String>) -> f64 {
    if left.is_empty() && right.is_empty() {
        return 1.0;
    }
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }

    let intersection = left.iter().filter(|token| right.contains(*token)).count();
    intersection as f64 / left.len().min(right.len()) as f64
}

fn average_sentence_length(text: &str) -> f64 {
    let sentence_lengths: Vec<f64> = split_sentences(text)
        .iter()
        .map(|sentence| tokenize(sentence).len() as f64)
        .filter(|length| *length > 0.0)
        .collect();

    if sentence_lengths.is_empty() {
        0.0
    } else {
        sentence_lengths.iter().sum::<f64>() / sentence_lengths.len() as f64
    }
}

fn split_sentences(text: &str) -> Vec<&str> {
    text.split(['.', '!', '?'])
        .map(str::trim)
        .filter(|sentence| !sentence.is_empty())
        .collect()
}

fn first_person_ratio(text: &str) -> f64 {
    let tokens = tokenize(text);
    if tokens.is_empty() {
        return 0.0;
    }

    let count = tokens
        .iter()
        .filter(|token| FIRST_PERSON_PRONOUNS.contains(&token.as_str()))
        .count();
    count as f64 / tokens.len() as f64
}

fn standard_deviation(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| {
            let delta = value - mean;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
}

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{
        PersonaConsistencyReport, evaluate_persona_consistency, score_line_to_line,
        score_prompt_to_line, score_qa_consistency,
    };
    use crate::core::eval::presenter::{
        render_persona_consistency_csv, render_persona_consistency_text_summary,
    };

    #[test]
    fn prompt_to_line_rewards_matching_directives() {
        let score = score_prompt_to_line(
            "Use short, natural, first person responses and stay friendly.",
            "I'm happy to help! I can do that.",
        );

        assert!(score >= 0.75, "score={score}");
    }

    #[test]
    fn prompt_to_line_penalizes_misaligned_style() {
        let score = score_prompt_to_line(
            "Use short, natural, first person responses.",
            "This response contains many detached formal statements and avoids personal framing entirely while extending well beyond the requested brevity constraints.",
        );

        assert!(score < 0.5, "score={score}");
    }

    #[test]
    fn line_to_line_scores_stable_responses_higher() {
        let stable = score_line_to_line(&[
            "I can help with that.",
            "I can explain that.",
            "I can summarize that.",
        ]);
        let unstable = score_line_to_line(&[
            "I can help with that.",
            "Could you clarify what you mean by that because I need more detail?",
            "This answer is intentionally much longer than the others and avoids first person language to break the style profile completely.",
        ]);

        assert!(stable > unstable, "stable={stable} unstable={unstable}");
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn line_to_line_returns_perfect_for_single_response() {
        let score = score_line_to_line(&["I can help."]);
        assert_eq!(score, 1.0);
    }

    #[test]
    fn qa_consistency_rewards_equivalent_questions_with_similar_answers() {
        let score = score_qa_consistency(&[
            (
                String::from("What is your favorite color?"),
                String::from("My favorite color is blue."),
            ),
            (
                String::from("Which color do you like most?"),
                String::from("I like blue the most."),
            ),
            (
                String::from("What food do you prefer?"),
                String::from("I prefer soup."),
            ),
        ]);

        assert!(score >= 0.3, "score={score}");
    }

    #[test]
    fn qa_consistency_penalizes_equivalent_questions_with_different_answers() {
        let score = score_qa_consistency(&[
            (
                String::from("What is your favorite color?"),
                String::from("My favorite color is blue."),
            ),
            (
                String::from("Which color do you like most?"),
                String::from("I prefer red above all others."),
            ),
        ]);

        assert!(score < 0.3, "score={score}");
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn qa_consistency_returns_perfect_when_no_equivalent_questions_exist() {
        let score = score_qa_consistency(&[
            (
                String::from("What is your favorite color?"),
                String::from("Blue."),
            ),
            (
                String::from("How old are you?"),
                String::from("I do not have an age."),
            ),
        ]);

        assert_eq!(score, 1.0);
    }

    #[test]
    fn evaluate_persona_consistency_aggregates_scores() {
        let report = evaluate_persona_consistency(
            "Use short, natural, first person responses.",
            &["I'm ready.", "I can help!"],
            &[
                (
                    String::from("What is your favorite color?"),
                    String::from("My favorite color is blue."),
                ),
                (
                    String::from("Which color do you like most?"),
                    String::from("I like blue the most."),
                ),
            ],
        );

        let expected_composite =
            (report.prompt_to_line + report.line_to_line + report.qa_consistency) / 3.0;
        assert!((report.composite - expected_composite).abs() < 1e-9);
    }

    #[test]
    fn report_renderers_include_headers_and_scores() {
        let report = PersonaConsistencyReport {
            prompt_to_line: 0.8,
            line_to_line: 0.7,
            qa_consistency: 0.9,
            composite: 0.8,
        };

        let csv = render_persona_consistency_csv(&report);
        let text = render_persona_consistency_text_summary(&report);

        assert!(csv.starts_with("scope,prompt_to_line,line_to_line,qa_consistency,composite\n"));
        assert!(csv.contains("english_only_heuristic,0.8000,0.7000,0.9000,0.8000"));
        assert!(text.contains("scope=english_only_heuristic"));
        assert!(text.contains("prompt_to_line=0.8000"));
        assert!(text.contains("composite=0.8000"));
    }
}
