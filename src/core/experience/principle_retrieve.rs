//! Principle retrieval and ranking: loads persisted principles,
//! applies persisted-Q-value scoring, and renders a top-N
//! block for prompt injection.
#![allow(clippy::cast_precision_loss)]

use anyhow::Result;

use super::distill_types::Principle;
use super::memory_rl::MemoryRL;
use crate::core::memory::Memory;

const MAX_PRINCIPLES_IN_BLOCK: usize = 5;
const MEMORY_RL_LAMBDA: f64 = 0.4;

/// Retrieve principles relevant to the current user message.
///
/// Uses Q-value-boosted ranking: principles are scored by a composite of
/// keyword/domain similarity and already-persisted utility (`q_value`). This
/// read path does not update Q-values.
///
/// Falls back to pure keyword scoring when `MemRL` is disabled.
pub(crate) async fn retrieve_relevant_principles(
    mem: &dyn Memory,
    entity_id: &str,
    user_message: &str,
) -> Result<Vec<Principle>> {
    let memory_rl = MemoryRL::new(MEMORY_RL_LAMBDA);
    super::memory_rl::retrieve_principles_with_q(
        mem,
        entity_id,
        user_message,
        &memory_rl,
        MAX_PRINCIPLES_IN_BLOCK,
    )
    .await
}

#[cfg(test)]
mod tests {
    use crate::core::experience::distill_types::{Principle, PrincipleCategory};
    use crate::core::experience::presenter::render_principle_block;
    use crate::utils::text::keyword_overlap_score;

    fn test_principle(statement: &str, confidence: f64) -> Principle {
        Principle {
            id: uuid::Uuid::new_v4().to_string(),
            category: PrincipleCategory::Heuristic,
            statement: statement.to_string(),
            confidence: confidence.into(),
            source_experience_ids: vec![],
            validation_count: 2,
            created_at: String::new(),
            domain: None,
            q_value: 0.0,
            times_applied: 0,
        }
    }

    fn test_principle_with_q(statement: &str, confidence: f64, q_value: f64) -> Principle {
        Principle {
            q_value,
            ..test_principle(statement, confidence)
        }
    }

    #[test]
    fn render_principle_block_empty_on_no_principles() {
        assert!(render_principle_block(&[]).is_empty());
    }

    #[test]
    fn render_principle_block_includes_header_and_principles() {
        let principles = vec![
            test_principle("prefer stepwise reasoning for math", 0.85),
            test_principle("avoid auto-approving file deletions", 0.72),
        ];
        let block = render_principle_block(&principles);
        assert!(block.contains("[Distilled Principles]"));
        assert!(block.contains("prefer stepwise"));
        assert!(block.contains("avoid auto-approving"));
        assert!(block.contains("heuristic"));
    }

    #[test]
    fn render_principle_block_shows_q_value() {
        let principles = vec![test_principle_with_q("test principle", 0.7, 0.45)];
        let block = render_principle_block(&principles);
        assert!(block.contains("q=+0.45"));
    }

    #[test]
    fn keyword_overlap_detects_shared_words() {
        let words = vec!["stepwise", "reasoning", "for", "math"];
        let overlap =
            keyword_overlap_score(&words, "Use stepwise reasoning when solving equations");
        assert!(overlap > 0.0);
    }

    #[test]
    fn keyword_overlap_ignores_short_words() {
        let words = vec!["if", "do", "a"];
        let overlap = keyword_overlap_score(&words, "if do a thing");
        assert!((overlap - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn keyword_overlap_empty_message() {
        let overlap = keyword_overlap_score(&[], "anything");
        assert!((overlap - 0.0).abs() < f64::EPSILON);
    }
}
