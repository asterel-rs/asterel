use std::fmt::Write as _;

use crate::contracts::experience::{ExperienceAtom, ExperienceOutcome};

use super::distill_types::{Principle, PrincipleCategory};

const MAX_PRINCIPLES_IN_BLOCK: usize = 5;

#[must_use]
pub(crate) fn render_experience_block(experiences: &[ExperienceAtom]) -> String {
    if experiences.is_empty() {
        return String::new();
    }

    let mut block = String::with_capacity(256);
    block.push_str("[Past Experiences]\n");
    for atom in experiences.iter().take(5) {
        let outcome_str = match atom.outcome {
            ExperienceOutcome::Success => "✓",
            ExperienceOutcome::Failure => "✗",
            ExperienceOutcome::Partial => "~",
            ExperienceOutcome::Unknown => "?",
        };
        let summary = sanitize_prompt_line(&atom.summary);
        let _ = write!(&mut block, "- [{outcome_str}] {summary}");
        if !atom.lesson.is_empty() {
            let lesson = sanitize_prompt_line(&atom.lesson);
            let _ = write!(&mut block, " → Lesson: {lesson}");
        }
        block.push('\n');
    }
    block
}

#[must_use]
pub(crate) fn render_principle_block(principles: &[Principle]) -> String {
    if principles.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(256);
    out.push_str("[Distilled Principles]\n");
    for p in principles.iter().take(MAX_PRINCIPLES_IN_BLOCK) {
        let cat = match p.category {
            PrincipleCategory::Strategy => "strategy",
            PrincipleCategory::Constraint => "constraint",
            PrincipleCategory::Heuristic => "heuristic",
        };
        let _ = writeln!(
            out,
            "- [{cat}, conf={:.2}, q={:+.2}, used={}x] {}",
            p.confidence,
            p.q_value,
            p.times_applied,
            sanitize_prompt_line(&p.statement)
        );
    }
    out
}

fn sanitize_prompt_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::experience::ExperienceKind;

    #[test]
    fn render_experience_block_collapses_multiline_fields() {
        let atom = ExperienceAtom::new(
            ExperienceKind::TurnInteraction,
            "first line\nsecond line",
            ExperienceOutcome::Success,
        )
        .with_lesson("lesson one\nlesson two");

        let rendered = render_experience_block(&[atom]);
        assert!(rendered.contains("first line second line"));
        assert!(rendered.contains("lesson one lesson two"));
        assert!(!rendered.contains("first line\nsecond line"));
    }
}
