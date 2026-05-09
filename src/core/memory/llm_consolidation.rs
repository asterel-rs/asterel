//! LLM-driven memory consolidation: fact extraction, merge, and
//! compression.
//!
//! Inspired by the Nanobot MEMORY.md + HISTORY.md pattern: recent
//! conversation turns are distilled into structured facts by an LLM,
//! then merged with existing long-term memory to produce a compact,
//! non-redundant knowledge base.
//!
//! Gated by `MemoryConfig::enable_llm_consolidation`; when disabled
//! the caller falls back to the rule-based consolidation path.

use std::fmt::Write as FmtWrite;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::core::memory::traits::MemoryLayer;
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryProvenance, MemorySource, PrivacyLevel,
    RecallQuery,
};
use crate::core::providers::Provider;
use crate::utils::text::{sanitize_prompt_line, truncate_ellipsis};

// ── Constants ──────────────────────────────────────────────────

/// Slot key for LLM-consolidated fact entries.
pub const LLM_CONSOLIDATION_SLOT_KEY: &str =
    crate::contracts::strings::data_model::SLOT_CONSOLIDATION_LLM_FACTS;

const LLM_CONSOLIDATION_PROVENANCE_REF: &str = "memory.consolidation.llm_facts";

/// Maximum number of existing semantic memories to retrieve for
/// merge context.
const EXISTING_MEMORY_RECALL_LIMIT: usize = 20;

/// Maximum characters of user/assistant text sent to the LLM.
const MAX_INPUT_CHARS: usize = 1200;

/// System prompt for fact extraction.
const FACT_EXTRACTION_SYSTEM_PROMPT: &str = "\
You are a memory consolidation module. Given conversation memory entries, \
extract the most important facts and merge them with existing long-term \
knowledge. Output ONLY a JSON object with these fields:
- \"facts\": array of strings, each a concise factual statement
- \"merged_summary\": string summarizing the consolidated knowledge
- \"dropped\": array of strings, facts that were redundant or superseded
Output ONLY the JSON object, no other text.";

// ── Types ──────────────────────────────────────────────────────

/// Structured output from the LLM fact extraction call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LlmConsolidationResult {
    /// Extracted factual statements.
    pub facts: Vec<String>,
    /// Merged summary of all consolidated knowledge.
    pub merged_summary: String,
    /// Facts that were dropped as redundant or superseded.
    #[serde(default)]
    pub dropped: Vec<String>,
}

/// Disposition of an LLM consolidation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmConsolidationDisposition {
    /// LLM consolidation succeeded and facts were persisted.
    Consolidated,
    /// LLM call failed or timed out; caller should fall back.
    Fallback,
}

// ── Core logic ─────────────────────────────────────────────────

/// Run a single LLM consolidation pass: extract facts from recent
/// memory entries, merge with existing long-term knowledge, and
/// persist the result.
///
/// Returns `Fallback` on any LLM/parse failure so the caller can
/// use the rule-based path instead.
///
/// # Errors
///
/// Returns an error only for memory persistence failures (not LLM
/// call failures, which are handled as graceful fallback).
pub(crate) async fn run_llm_consolidation(
    memory: &dyn Memory,
    provider: &dyn Provider,
    model: &str,
    entity_id: &str,
    user_message: &str,
    assistant_response: &str,
    timeout: Duration,
) -> Result<LlmConsolidationDisposition> {
    // ── 1. Retrieve existing semantic memories for context ──────
    let existing = recall_existing_facts(memory, entity_id).await;

    // ── 2. Build the LLM prompt ────────────────────────────────
    let prompt = build_consolidation_prompt(
        user_message,
        assistant_response,
        &existing,
    );

    // ── 3. Call the LLM with timeout ───────────────────────────
    let result = tokio::time::timeout(
        timeout,
        provider.chat_with_system(
            Some(FACT_EXTRACTION_SYSTEM_PROMPT),
            &prompt,
            model,
            0.1,
        ),
    )
    .await;

    let parsed = match result {
        Ok(Ok(response)) => match parse_llm_response(&response) {
            Some(p) => p,
            None => {
                tracing::debug!("LLM consolidation response parse failed, falling back");
                return Ok(LlmConsolidationDisposition::Fallback);
            }
        },
        Ok(Err(error)) => {
            tracing::debug!(%error, "LLM consolidation call failed, falling back");
            return Ok(LlmConsolidationDisposition::Fallback);
        }
        Err(_) => {
            tracing::debug!("LLM consolidation call timed out, falling back");
            return Ok(LlmConsolidationDisposition::Fallback);
        }
    };

    // ── 4. Persist the consolidated result ─────────────────────
    persist_consolidated_facts(memory, entity_id, &parsed)
        .await
        .context("persist LLM-consolidated facts")?;

    if !parsed.dropped.is_empty() {
        tracing::debug!(
            dropped_count = parsed.dropped.len(),
            "LLM consolidation dropped redundant facts"
        );
    }

    Ok(LlmConsolidationDisposition::Consolidated)
}

// ── Helpers ────────────────────────────────────────────────────

/// Retrieve existing semantic-layer memories for merge context.
async fn recall_existing_facts(memory: &dyn Memory, entity_id: &str) -> Vec<String> {
    let query = RecallQuery::new(
        entity_id,
        "consolidated facts long-term knowledge",
        EXISTING_MEMORY_RECALL_LIMIT,
    );
    match memory.recall_scoped(query).await {
        Ok(items) => items
            .into_iter()
            .map(|item| truncate_ellipsis(&item.value, 200))
            .collect(),
        Err(error) => {
            tracing::debug!(%error, "failed to recall existing facts for LLM consolidation");
            Vec::new()
        }
    }
}

/// Build the user prompt for the LLM consolidation call.
fn build_consolidation_prompt(
    user_message: &str,
    assistant_response: &str,
    existing_facts: &[String],
) -> String {
    let user_truncated = sanitize_prompt_line(&truncate_ellipsis(user_message, MAX_INPUT_CHARS));
    let assistant_truncated =
        sanitize_prompt_line(&truncate_ellipsis(assistant_response, MAX_INPUT_CHARS));

    let mut prompt = format!(
        "## Recent conversation turn\n\
         User: {user_truncated}\n\
         Assistant: {assistant_truncated}\n\n"
    );

    if !existing_facts.is_empty() {
        prompt.push_str("## Existing long-term knowledge\n");
        for (i, fact) in existing_facts.iter().enumerate() {
            let fact = sanitize_prompt_line(fact);
            let _ = write!(prompt, "{}. {fact}\n", i + 1);
        }
        prompt.push('\n');
    }

    prompt.push_str(
        "Extract important facts from the conversation, merge with \
         existing knowledge, remove redundancies, and return the \
         consolidated result as JSON.",
    );

    prompt
}

/// Parse the LLM JSON response, handling code-fenced output.
fn parse_llm_response(response: &str) -> Option<LlmConsolidationResult> {
    let stripped = response.trim();
    let json_str = stripped
        .strip_prefix("```json")
        .map(str::trim)
        .and_then(|s| s.strip_suffix("```"))
        .map_or(stripped, str::trim);

    match serde_json::from_str::<LlmConsolidationResult>(json_str) {
        Ok(parsed) if !parsed.facts.is_empty() || !parsed.merged_summary.is_empty() => {
            Some(parsed)
        }
        Ok(_) => {
            tracing::debug!("LLM consolidation returned empty facts and summary");
            None
        }
        Err(error) => {
            tracing::debug!(%error, "failed to parse LLM consolidation response");
            None
        }
    }
}

/// Persist the consolidated facts as a semantic memory event.
async fn persist_consolidated_facts(
    memory: &dyn Memory,
    entity_id: &str,
    result: &LlmConsolidationResult,
) -> Result<()> {
    let payload = serde_json::to_string(result)
        .context("serialize LLM consolidation result")?;

    memory
        .append_event(
            MemoryEventInput::new(
                entity_id,
                LLM_CONSOLIDATION_SLOT_KEY,
                MemoryEventType::SummaryCompacted,
                payload,
                MemorySource::System,
                PrivacyLevel::Private,
            )
            .with_layer(MemoryLayer::Semantic)
            .with_confidence(0.90)
            .with_importance(0.75)
            .with_provenance(MemoryProvenance::source_reference(
                MemorySource::System,
                LLM_CONSOLIDATION_PROVENANCE_REF,
            )),
        )
        .await?;

    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_consolidation_response() {
        let json = r#"{
            "facts": ["User prefers Rust", "Project uses async/await"],
            "merged_summary": "User works on a Rust async project.",
            "dropped": ["User uses Rust (duplicate)"]
        }"#;
        let result = parse_llm_response(json).unwrap();
        assert_eq!(result.facts.len(), 2);
        assert_eq!(result.facts[0], "User prefers Rust");
        assert_eq!(result.merged_summary, "User works on a Rust async project.");
        assert_eq!(result.dropped.len(), 1);
    }

    #[test]
    fn parse_code_fenced_response() {
        let response = "```json\n{\"facts\":[\"fact1\"],\"merged_summary\":\"summary\",\"dropped\":[]}\n```";
        let result = parse_llm_response(response).unwrap();
        assert_eq!(result.facts, vec!["fact1"]);
    }

    #[test]
    fn parse_empty_facts_returns_none() {
        let json = r#"{"facts":[],"merged_summary":"","dropped":[]}"#;
        assert!(parse_llm_response(json).is_none());
    }

    #[test]
    fn parse_invalid_json_returns_none() {
        assert!(parse_llm_response("not json at all").is_none());
    }

    #[test]
    fn parse_missing_dropped_field_defaults_empty() {
        let json = r#"{"facts":["a fact"],"merged_summary":"summary"}"#;
        let result = parse_llm_response(json).unwrap();
        assert!(result.dropped.is_empty());
    }

    #[test]
    fn build_prompt_includes_conversation() {
        let prompt = build_consolidation_prompt("hello", "world", &[]);
        assert!(prompt.contains("User: hello"));
        assert!(prompt.contains("Assistant: world"));
        assert!(!prompt.contains("Existing long-term knowledge"));
    }

    #[test]
    fn build_prompt_includes_existing_facts() {
        let facts = vec!["fact one".to_string(), "fact two".to_string()];
        let prompt = build_consolidation_prompt("hi", "there", &facts);
        assert!(prompt.contains("Existing long-term knowledge"));
        assert!(prompt.contains("1. fact one"));
        assert!(prompt.contains("2. fact two"));
    }

    #[test]
    fn slot_key_matches_contract() {
        assert_eq!(
            LLM_CONSOLIDATION_SLOT_KEY,
            "consolidation.llm_facts.latest"
        );
    }
}
