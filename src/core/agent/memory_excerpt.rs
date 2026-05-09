use crate::security::scrub::scrub_secrets;
use crate::utils::text::{
    sanitize_prompt_line, strip_internal_prompt_blocks, strip_reasoning, truncate_ellipsis,
};

/// Build a bounded, single-line memory writeback excerpt.
///
/// This is deliberately stricter than ordinary display truncation: post-turn
/// memory summaries are durable, so they must not retain hidden reasoning tags,
/// internal prompt blocks, multiline prompt structure, or secret-like tokens.
#[must_use]
pub(crate) fn safe_memory_excerpt(value: &str, max_chars: usize) -> String {
    let without_reasoning = strip_reasoning(value);
    let without_internal_blocks = strip_internal_prompt_blocks(&without_reasoning);
    let single_line = sanitize_prompt_line(&without_internal_blocks);
    let scrubbed = scrub_secrets(&single_line);
    truncate_ellipsis(scrubbed.as_ref(), max_chars)
}

#[cfg(test)]
mod tests {
    use super::safe_memory_excerpt;

    #[test]
    fn safe_memory_excerpt_strips_reasoning_blocks_and_secrets() {
        let excerpt = safe_memory_excerpt(
            "<think>internal plan sk-12345678901234567890</think>answer\nAuthorization: Bearer sk-abcdefghijklmnopqrstuvwxyz",
            200,
        );

        assert!(!excerpt.contains("internal plan"));
        assert!(!excerpt.contains("sk-abcdefghijklmnopqrstuvwxyz"));
        assert!(excerpt.contains("answer"));
        assert!(excerpt.contains("[REDACTED]"));
        assert!(!excerpt.contains('\n'));
    }
}
