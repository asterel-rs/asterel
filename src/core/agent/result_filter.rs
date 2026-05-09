//! Tool result self-filtering: lets the model compress oversized
//! tool outputs before they consume context window tokens.
//!
//! Based on the pattern from Anthropic's "Harnessing Claude's Intelligence"
//! (2026): giving the model the ability to filter its own tool outputs
//! brought `BrowseComp` accuracy from 45.3% to 61.6%.
//!
//! When a tool result exceeds `FILTER_THRESHOLD_CHARS`, a lightweight
//! LLM call extracts only the task-relevant information, replacing the
//! full output with a compact summary.

use crate::core::providers::traits::Provider;

/// Character threshold above which a tool result triggers self-filtering.
/// Below this, the result passes through unchanged.
pub(crate) const FILTER_THRESHOLD_CHARS: usize = 6_000;

/// Maximum characters to send to the filter model. Content beyond this
/// is mechanically truncated before the LLM call to bound cost.
const FILTER_INPUT_CAP_CHARS: usize = 32_000;

/// System prompt for the filter model — short, focused, cacheable.
const FILTER_SYSTEM_PROMPT: &str = "\
You are a tool output filter. Given a tool name, the user's task context, \
and a large tool output, extract ONLY the information relevant to the task. \
Be concise. Preserve exact values, paths, error messages, and key data. \
Omit boilerplate, repetition, and irrelevant sections. \
If the output is an error, preserve the full error message. \
Reply with the filtered content only — no commentary.";

/// Result of a filter attempt.
pub(crate) enum FilterOutcome {
    /// Output was below threshold; passed through unchanged.
    BelowThreshold,
    /// Output was filtered by the model.
    Filtered {
        original_chars: usize,
        filtered_chars: usize,
    },
    /// Filtering failed; original output is preserved.
    Failed(String),
}

/// Find the last byte index that is on a char boundary at or before `max`.
fn floor_char_boundary(s: &str, max: usize) -> usize {
    if max >= s.len() {
        return s.len();
    }
    let mut i = max;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Attempt to filter a tool result using a lightweight LLM call.
///
/// Returns the (possibly filtered) content and the outcome.
pub(crate) async fn filter_tool_result(
    provider: &dyn Provider,
    model: &str,
    tool_name: &str,
    task_context: &str,
    content: &str,
) -> (String, FilterOutcome) {
    let original_chars = content.len();

    if original_chars <= FILTER_THRESHOLD_CHARS {
        return (content.to_string(), FilterOutcome::BelowThreshold);
    }

    // Cap the input sent to the filter model.
    let capped = if content.len() > FILTER_INPUT_CAP_CHARS {
        &content[..floor_char_boundary(content, FILTER_INPUT_CAP_CHARS)]
    } else {
        content
    };

    let user_message =
        format!("Tool: {tool_name}\nTask context: {task_context}\n\n---\n\n{capped}");

    match provider
        .chat_with_system(Some(FILTER_SYSTEM_PROMPT), &user_message, model, 0.0)
        .await
    {
        Ok(filtered) => {
            let filtered_chars = filtered.len();
            // Only use filtered version if it's actually shorter.
            if filtered_chars < original_chars {
                #[allow(clippy::cast_precision_loss)]
                let orig_f64 = original_chars as f64;
                #[allow(clippy::cast_precision_loss)]
                let filt_f64 = filtered_chars as f64;
                let reduction = (1.0 - filt_f64 / orig_f64) * 100.0;
                tracing::info!(
                    tool_name,
                    original_chars,
                    filtered_chars,
                    reduction_pct = format!("{reduction:.0}%"),
                    "tool result self-filtered"
                );
                let annotated = format!(
                    "{filtered}\n\n[filtered from {original_chars} to {filtered_chars} chars]"
                );
                (
                    annotated,
                    FilterOutcome::Filtered {
                        original_chars,
                        filtered_chars,
                    },
                )
            } else {
                // Filter didn't reduce size — keep original.
                (content.to_string(), FilterOutcome::BelowThreshold)
            }
        }
        Err(error) => {
            tracing::debug!(
                tool_name,
                error = %error,
                "tool result self-filter failed; keeping original"
            );
            (
                content.to_string(),
                FilterOutcome::Failed(error.to_string()),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_char_boundary_respects_utf8() {
        let s = "héllo"; // 'é' is 2 bytes
        let boundary = floor_char_boundary(s, 2);
        assert!(s.is_char_boundary(boundary));
        assert!(boundary <= 2);
    }

    #[test]
    fn floor_char_boundary_at_end() {
        let s = "abc";
        assert_eq!(floor_char_boundary(s, 100), 3);
    }

    #[test]
    fn filter_input_cap_truncates_before_llm_call() {
        let huge = "x".repeat(FILTER_INPUT_CAP_CHARS * 2);
        let capped = &huge[..floor_char_boundary(&huge, FILTER_INPUT_CAP_CHARS)];
        assert!(capped.len() <= FILTER_INPUT_CAP_CHARS);
    }
}
