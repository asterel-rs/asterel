#[cfg(test)]
mod tests {
    use crate::contracts::experience::{ExperienceAtom, ExperienceKind, ExperienceOutcome};

    #[test]
    fn experience_atom_construction_sets_defaults() {
        let atom = ExperienceAtom::new(
            ExperienceKind::SelfTask,
            "Investigate failing runtime test",
            ExperienceOutcome::Unknown,
        );

        assert!(!atom.id.is_empty());
        assert_eq!(atom.kind, ExperienceKind::SelfTask);
        assert_eq!(atom.summary, "Investigate failing runtime test");
        assert_eq!(atom.outcome, ExperienceOutcome::Unknown);
        assert_eq!(atom.lesson, "");
        assert!(!atom.occurred_at.is_empty());
        assert_eq!(
            atom.confidence,
            crate::contracts::scores::Confidence::new(0.7)
        );
    }

    #[test]
    fn experience_atom_builder_methods_override_lesson_and_confidence() {
        let atom = ExperienceAtom::new(
            ExperienceKind::TurnInteraction,
            "Recover after a failed tool attempt",
            ExperienceOutcome::Partial,
        )
        .with_lesson("Retry in smaller chunks")
        .with_confidence(0.85);

        assert_eq!(atom.lesson, "Retry in smaller chunks");
        assert_eq!(
            atom.confidence,
            crate::contracts::scores::Confidence::new(0.85)
        );
    }

    #[test]
    fn experience_kind_kind_str_values_match_expected_tokens() {
        assert_eq!(ExperienceKind::SelfTask.kind_str(), "self_task");
        assert_eq!(
            ExperienceKind::PersonaWriteback.kind_str(),
            "persona_writeback"
        );
    }

    #[test]
    fn experience_kind_turn_interaction_kind_str() {
        assert_eq!(
            ExperienceKind::TurnInteraction.kind_str(),
            "turn_interaction"
        );
    }

    #[test]
    fn experience_kind_reads_legacy_evolution_change_token() {
        let parsed: ExperienceKind = serde_json::from_str("\"evolution_change\"")
            .expect("legacy experience kind token should deserialize");
        assert_eq!(parsed, ExperienceKind::PersonaWriteback);
    }

    #[test]
    fn experience_kind_reads_legacy_plan_execution_token_as_turn_interaction() {
        let parsed: ExperienceKind = serde_json::from_str("\"plan_execution\"")
            .expect("legacy experience kind token should deserialize");
        assert_eq!(parsed, ExperienceKind::TurnInteraction);
        assert_eq!(parsed.kind_str(), "turn_interaction");
    }
}
