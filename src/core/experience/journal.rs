//! Journal helpers for constructing `ExperienceAtom` records from
//! companion turns, self-tasks, and codespace activities.

use crate::contracts::experience::{ExperienceAtom, ExperienceKind, ExperienceOutcome};

/// Build an `ExperienceAtom` for a self-initiated task.
pub(crate) fn record_self_task_experience(
    title: &str,
    instructions: &str,
    outcome: ExperienceOutcome,
) -> ExperienceAtom {
    let summary = format!("Self-task: {title} — {instructions}");
    ExperienceAtom::new(ExperienceKind::SelfTask, summary, outcome)
}

/// Build an `ExperienceAtom` for a codespace project activity.
pub(crate) fn record_codespace_experience(
    project_name: &str,
    action: &str,
    outcome: ExperienceOutcome,
    lesson: &str,
) -> ExperienceAtom {
    let summary = format!("Codespace [{project_name}]: {action}");
    ExperienceAtom::new(ExperienceKind::CodespaceActivity, summary, outcome)
        .with_lesson(lesson.to_string())
}

#[cfg(test)]
mod tests {
    use super::record_self_task_experience;
    use crate::core::experience::{ExperienceKind, ExperienceOutcome};

    #[test]
    fn record_self_task_experience_creates_expected_atom() {
        let atom = record_self_task_experience(
            "Refine reflection",
            "Tighten memory summary constraints",
            ExperienceOutcome::Success,
        );

        assert_eq!(atom.kind, ExperienceKind::SelfTask);
        assert_eq!(
            atom.summary,
            "Self-task: Refine reflection — Tighten memory summary constraints"
        );
        assert_eq!(atom.outcome, ExperienceOutcome::Success);
    }
}
