//! Experience retrieval: queries memory for past experience atoms
//! matching a context string and renders them into a prompt block.

use std::future::Future;
use std::pin::Pin;

use crate::contracts::experience::ExperienceAtom;
use crate::contracts::strings::data_model::PREFIX_EXPERIENCE_SLOT;
use crate::core::memory::{Memory, RecallQuery};

/// Retrieve past experience atoms matching a context query.
///
/// Queries memory for items under the `experience.` slot prefix,
/// deserialises matching records, and returns up to `limit` atoms.
///
/// # Errors
///
/// Returns an error if the memory recall query fails.
pub(crate) fn retrieve_relevant_experiences<'a>(
    mem: &'a dyn Memory,
    entity_id: &'a str,
    query_context: &'a str,
    limit: usize,
) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<ExperienceAtom>>> + Send + 'a>> {
    Box::pin(async move {
        let query = RecallQuery::new(entity_id, query_context, limit);
        let items = mem.recall_scoped(query).await?;

        let mut experiences = Vec::new();
        for item in items {
            if !item.slot_key.as_str().starts_with(PREFIX_EXPERIENCE_SLOT) {
                continue;
            }
            if let Ok(atom) = serde_json::from_str::<ExperienceAtom>(&item.value) {
                experiences.push(atom);
            }
        }

        Ok(experiences)
    })
}

#[cfg(test)]
mod tests {
    use crate::core::experience::presenter::render_experience_block;
    use crate::core::experience::{ExperienceAtom, ExperienceKind, ExperienceOutcome};

    #[test]
    fn render_experience_block_empty_list_returns_empty_string() {
        let rendered = render_experience_block(&[]);
        assert_eq!(rendered, "");
    }

    #[test]
    fn render_experience_block_with_items_formats_output() {
        let first = ExperienceAtom::new(
            ExperienceKind::SelfTask,
            "Self-task: Improve prompt shaping",
            ExperienceOutcome::Success,
        )
        .with_lesson("Use tighter output constraints");
        let second = ExperienceAtom::new(
            ExperienceKind::TurnInteraction,
            "Companion turn: recover after a failed tool attempt",
            ExperienceOutcome::Failure,
        );

        let rendered = render_experience_block(&[first, second]);

        assert!(rendered.starts_with("[Past Experiences]\n"));
        assert!(rendered.contains(
            "- [✓] Self-task: Improve prompt shaping → Lesson: Use tighter output constraints\n"
        ));
        assert!(rendered.contains("- [✗] Companion turn: recover after a failed tool attempt\n"));
    }
}
