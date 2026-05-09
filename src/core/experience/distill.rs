//! Experience distillation pipeline: clusters similar experience
//! atoms and compresses them into reusable `Principle` statements
//! persisted to memory for prompt injection.
#![allow(
    dead_code,
    clippy::cast_precision_loss,
    clippy::needless_pass_by_value,
    clippy::needless_borrow
)]

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use super::distill_types::{Principle, PrincipleCategory};
use crate::contracts::experience::{ExperienceAtom, ExperienceOutcome};
use crate::contracts::scores::Confidence;
use crate::contracts::strings::data_model::{
    PREFIX_PRINCIPLE_SLOT, SOURCE_EXPERIENCE_DISTILL_PRINCIPLE, SOURCE_REF_EXPERIENCE_DISTILL,
};
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryLayer, MemoryProvenance, MemorySource,
    PrivacyLevel, SourceKind,
};

// ── Trait ────────────────────────────────────────────────────────

/// Distills a collection of experience atoms into reusable principles.
pub(crate) trait Distiller: Send + Sync {
    /// Compress a set of experience atoms into reusable principles.
    fn distill<'a>(
        &'a self,
        experiences: &'a [ExperienceAtom],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Principle>>> + Send + 'a>>;
}

// ── Heuristic implementation ─────────────────────────────────────

/// Groups experiences by kind + outcome, then extracts principles from
/// clusters that show a consistent pattern (≥3 atoms, ≥67% agreement).
pub(crate) struct HeuristicDistiller;

impl Distiller for HeuristicDistiller {
    fn distill<'a>(
        &'a self,
        experiences: &'a [ExperienceAtom],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Principle>>> + Send + 'a>> {
        Box::pin(async move { Ok(distill_heuristic(experiences, &DistillationGate::default())) })
    }
}

/// Minimum cluster size before a principle is synthesised.
const MIN_CLUSTER_SIZE: usize = 3;

/// Minimum fraction of a cluster that must share the same outcome.
const MIN_AGREEMENT_RATIO: f64 = 0.67;

/// Quality gate configuration for distillation output.
#[derive(Debug, Clone)]
pub(crate) struct DistillationGate {
    /// Minimum novelty score (0.0-1.0) for a principle to pass.
    pub min_novelty: f64,
    /// Minimum cluster support count.
    pub min_support: usize,
    /// Maximum age in days for principles before they become stale.
    pub stale_max_age_days: u32,
}

impl Default for DistillationGate {
    fn default() -> Self {
        Self {
            min_novelty: 0.3,
            min_support: MIN_CLUSTER_SIZE,
            stale_max_age_days: 90,
        }
    }
}

impl DistillationGate {}

fn distill_heuristic(experiences: &[ExperienceAtom], gate: &DistillationGate) -> Vec<Principle> {
    // Group by kind only, so that agreement is computed across different outcomes.
    let mut clusters: HashMap<String, Vec<&ExperienceAtom>> = HashMap::new();
    for atom in experiences {
        clusters
            .entry(atom.kind.kind_str().to_string())
            .or_default()
            .push(atom);
    }

    let mut principles: Vec<Principle> = Vec::new();
    for (kind_str, group) in &clusters {
        if group.len() < gate.min_support {
            continue;
        }

        // Find the dominant outcome within the cluster.
        let mut outcome_counts: HashMap<ExperienceOutcome, usize> = HashMap::new();
        for a in group {
            *outcome_counts.entry(a.outcome).or_insert(0) += 1;
        }
        let (outcome, dominant_count) = outcome_counts
            .into_iter()
            .max_by_key(|&(_, count)| count)
            .unwrap_or((ExperienceOutcome::Unknown, 0));
        let agreement = dominant_count as f64 / group.len() as f64;
        if agreement < MIN_AGREEMENT_RATIO {
            continue;
        }

        let category = categorise_from_kind_outcome(kind_str, outcome);
        let statement = synthesise_statement(kind_str, outcome, group);
        let existing_statements: Vec<&str> =
            principles.iter().map(|p| p.statement.as_str()).collect();
        if novelty_score(&statement, &existing_statements) < gate.min_novelty {
            continue;
        }
        let avg_confidence =
            group.iter().map(|a| a.confidence.get()).sum::<f64>() / group.len() as f64;
        let source_ids = group.iter().map(|a| a.id.clone()).collect();

        principles.push(Principle {
            id: Uuid::new_v4().to_string(),
            category,
            statement,
            confidence: Confidence::new((avg_confidence * agreement).clamp(0.1, 0.95)),
            source_experience_ids: source_ids,
            validation_count: 0,
            created_at: Utc::now().to_rfc3339(),
            domain: None,
            q_value: 0.0,
            times_applied: 0,
        });
    }

    principles
}

/// Compute novelty score for a principle statement against existing ones.
/// Returns 1.0 for fully novel, 0.0 for fully duplicated.
fn novelty_score(statement: &str, existing_statements: &[&str]) -> f64 {
    if existing_statements.is_empty() {
        return 1.0;
    }

    let words: HashSet<&str> = statement
        .split_whitespace()
        .filter(|word| word.len() > 3)
        .collect();
    if words.is_empty() {
        return 0.0;
    }

    let mut max_overlap = 0.0_f64;
    for existing in existing_statements {
        let existing_words: HashSet<&str> = existing
            .split_whitespace()
            .filter(|word| word.len() > 3)
            .collect();
        if existing_words.is_empty() {
            continue;
        }
        let intersection = words.intersection(&existing_words).count();
        let union = words.union(&existing_words).count();
        if union > 0 {
            // Cast safety: token-set intersection and union sizes are bounded by statement length.
            #[allow(clippy::cast_precision_loss)]
            let jaccard = intersection as f64 / union as f64;
            max_overlap = max_overlap.max(jaccard);
        }
    }

    1.0 - max_overlap
}

/// Identify principle IDs that are older than the configured stale threshold
/// and have low q-value.
///
/// Production does not currently have a wired validation-count feedback path,
/// so stale-GC must not depend on `validation_count` as proof that a principle
/// has or has not been reinforced.
pub(crate) fn identify_stale_principles(
    principles: &[Principle],
    gate: &DistillationGate,
) -> Vec<String> {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(i64::from(gate.stale_max_age_days));
    let cutoff_str = cutoff.to_rfc3339();

    principles
        .iter()
        .filter(|principle| principle.created_at < cutoff_str && principle.q_value < 0.1)
        .map(|principle| principle.id.clone())
        .collect()
}

fn categorise_from_kind_outcome(kind: &str, outcome: ExperienceOutcome) -> PrincipleCategory {
    match (kind, outcome) {
        (_, ExperienceOutcome::Failure) => PrincipleCategory::Constraint,
        ("turn_interaction", ExperienceOutcome::Success) => PrincipleCategory::Strategy,
        _ => PrincipleCategory::Heuristic,
    }
}

fn experience_context_label(kind: &str) -> &'static str {
    match kind {
        "turn_interaction" => "companion-turn",
        "self_task" => "self-task",
        "codespace_activity" => "codespace-activity",
        _ => "experience",
    }
}

fn synthesise_statement(
    kind: &str,
    outcome: ExperienceOutcome,
    group: &[&ExperienceAtom],
) -> String {
    let outcome_label = match outcome {
        ExperienceOutcome::Success => "succeeded",
        ExperienceOutcome::Failure => "failed",
        ExperienceOutcome::Partial => "partially succeeded",
        ExperienceOutcome::Unknown => "had unknown outcome",
    };

    // Extract most common lesson keywords (crude but effective).
    let lessons: Vec<&str> = group
        .iter()
        .filter(|a| !a.lesson.is_empty())
        .map(|a| a.lesson.as_str())
        .collect();

    let lesson_hint = if lessons.is_empty() {
        String::new()
    } else {
        format!(" Lesson pattern: {}", truncate(&lessons[0], 120))
    };

    format!(
        "In {context} scenarios, actions {outcome_label} {}/{} times.{lesson_hint}",
        group.iter().filter(|a| a.outcome == outcome).count(),
        group.len(),
        context = experience_context_label(kind),
    )
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let end = s.char_indices().nth(max).map_or(s.len(), |(idx, _)| idx);
        &s[..end]
    }
}

// ── Persistence helpers ──────────────────────────────────────────

/// Persist a principle to memory as a `principle.{id}` slot.
pub(crate) async fn persist_principle(
    mem: &dyn Memory,
    entity_id: &str,
    principle: &Principle,
) -> Result<()> {
    let slot_key = format!("{PREFIX_PRINCIPLE_SLOT}{}", principle.id);
    let payload = serde_json::to_string(principle)?;

    let input = MemoryEventInput::new(
        entity_id,
        slot_key,
        MemoryEventType::FactAdded,
        payload,
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_confidence(principle.confidence.get())
    .with_importance(0.8)
    .with_layer(MemoryLayer::Procedural)
    .with_source_kind(SourceKind::Manual)
    .with_source_ref(SOURCE_REF_EXPERIENCE_DISTILL)
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::System,
        SOURCE_EXPERIENCE_DISTILL_PRINCIPLE,
    ));

    mem.append_event(input).await?;
    Ok(())
}

/// Run the full distillation pipeline: retrieve recent experiences,
/// distill into principles, persist new principles.
pub(crate) async fn run_distillation(
    mem: &dyn Memory,
    entity_id: &str,
    experiences: &[ExperienceAtom],
) -> Result<Vec<Principle>> {
    let distiller = HeuristicDistiller;
    let principles = distiller.distill(experiences).await?;

    for principle in &principles {
        persist_principle(mem, entity_id, principle).await?;
    }

    Ok(principles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::experience::{ExperienceAtom, ExperienceKind, ExperienceOutcome};
    use crate::contracts::memory_domain::{
        BeliefSlot, MemoryEvent, MemoryRecallEntry, RecallQuery,
    };
    use crate::contracts::memory_error::{MemoryError, MemoryResult};
    use crate::contracts::memory_forget::{ForgetMode, ForgetOutcome};
    use crate::contracts::memory_traits::{MemoryGovernance, MemoryReader, MemoryWriter};
    use std::future::Future;
    use std::pin::Pin;

    struct FailingWriteMemory;

    impl MemoryWriter for FailingWriteMemory {
        fn append_event(
            &self,
            _input: MemoryEventInput,
        ) -> Pin<Box<dyn Future<Output = MemoryResult<MemoryEvent>> + Send + '_>> {
            Box::pin(async {
                Err(MemoryError::write(
                    "forced distillation persistence failure",
                ))
            })
        }
    }

    impl MemoryReader for FailingWriteMemory {
        fn recall_scoped(
            &self,
            _query: RecallQuery,
        ) -> Pin<Box<dyn Future<Output = MemoryResult<Vec<MemoryRecallEntry>>> + Send + '_>>
        {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn resolve_slot<'a>(
            &'a self,
            _entity_id: &'a str,
            _slot_key: &'a str,
        ) -> Pin<Box<dyn Future<Output = MemoryResult<Option<BeliefSlot>>> + Send + 'a>> {
            Box::pin(async { Ok(None) })
        }
    }

    impl MemoryGovernance for FailingWriteMemory {
        fn name(&self) -> &str {
            "failing-write-memory"
        }

        fn health_check(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
            Box::pin(async { true })
        }

        fn forget_slot<'a>(
            &'a self,
            _entity_id: &'a str,
            _slot_key: &'a str,
            _mode: ForgetMode,
            _reason: &'a str,
        ) -> Pin<Box<dyn Future<Output = MemoryResult<ForgetOutcome>> + Send + 'a>> {
            Box::pin(async { Err(MemoryError::unsupported("not needed for distillation test")) })
        }

        fn count_events<'a>(
            &'a self,
            _entity_id: Option<&'a str>,
        ) -> Pin<Box<dyn Future<Output = MemoryResult<usize>> + Send + 'a>> {
            Box::pin(async { Ok(0) })
        }
    }

    fn make_atoms(
        kind: ExperienceKind,
        outcome: ExperienceOutcome,
        count: usize,
    ) -> Vec<ExperienceAtom> {
        (0..count)
            .map(|i| {
                ExperienceAtom::new(kind.clone(), format!("test atom {i}"), outcome)
                    .with_lesson(format!("lesson {i}"))
                    .with_confidence(0.7)
            })
            .collect()
    }

    #[test]
    fn heuristic_distiller_requires_minimum_cluster_size() {
        let atoms = make_atoms(
            ExperienceKind::TurnInteraction,
            ExperienceOutcome::Success,
            2,
        );
        let principles = distill_heuristic(&atoms, &DistillationGate::default());
        assert!(principles.is_empty(), "should not distill from <3 atoms");
    }

    #[test]
    fn heuristic_distiller_produces_principle_from_cluster() {
        let atoms = make_atoms(
            ExperienceKind::TurnInteraction,
            ExperienceOutcome::Success,
            5,
        );
        let principles = distill_heuristic(&atoms, &DistillationGate::default());
        assert_eq!(principles.len(), 1);
        assert_eq!(principles[0].source_experience_ids.len(), 5);
        assert!(principles[0].confidence > crate::contracts::scores::Confidence::new(0.0));
    }

    #[test]
    fn failure_cluster_produces_constraint_category() {
        let atoms = make_atoms(ExperienceKind::SelfTask, ExperienceOutcome::Failure, 4);
        let principles = distill_heuristic(&atoms, &DistillationGate::default());
        assert_eq!(principles.len(), 1);
        assert_eq!(principles[0].category, PrincipleCategory::Constraint);
    }

    #[test]
    fn turn_success_cluster_produces_strategy_category() {
        let atoms = make_atoms(
            ExperienceKind::TurnInteraction,
            ExperienceOutcome::Success,
            3,
        );
        let principles = distill_heuristic(&atoms, &DistillationGate::default());
        assert_eq!(principles.len(), 1);
        assert_eq!(principles[0].category, PrincipleCategory::Strategy);
        assert!(principles[0].statement.contains("companion-turn"));
    }

    #[test]
    fn mixed_outcomes_below_agreement_threshold_produce_nothing() {
        let mut atoms = make_atoms(ExperienceKind::SelfTask, ExperienceOutcome::Success, 2);
        atoms.extend(make_atoms(
            ExperienceKind::SelfTask,
            ExperienceOutcome::Failure,
            2,
        ));
        // 2 success + 2 failure grouped separately — each cluster has 2, below MIN_CLUSTER_SIZE
        let principles = distill_heuristic(&atoms, &DistillationGate::default());
        // Both clusters have only 2 atoms
        assert!(principles.is_empty());
    }

    #[test]
    fn novelty_score_identical_statement_is_zero() {
        let score = novelty_score("alpha beta gamma delta", &["alpha beta gamma delta"]);
        assert!((score - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn novelty_score_completely_different_is_one() {
        let score = novelty_score("alpha beta gamma delta", &["omega sigma theta lambda"]);
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn novelty_score_partial_overlap_between_zero_and_one() {
        let score = novelty_score("alpha beta gamma delta", &["alpha beta omega sigma"]);
        assert!(score > 0.0);
        assert!(score < 1.0);
    }

    #[test]
    fn distill_heuristic_novelty_gate_filters_duplicates() {
        let mut atoms = make_atoms(
            ExperienceKind::TurnInteraction,
            ExperienceOutcome::Success,
            3,
        );
        atoms.extend(make_atoms(
            ExperienceKind::SelfTask,
            ExperienceOutcome::Success,
            3,
        ));
        for atom in &mut atoms {
            atom.lesson = "repeatable lesson phrase for overlap".to_string();
        }

        let gate = DistillationGate {
            min_novelty: 0.9,
            ..DistillationGate::default()
        };
        let principles = distill_heuristic(&atoms, &gate);
        assert_eq!(principles.len(), 1);
    }

    #[test]
    fn identify_stale_principles_finds_old_low_value_records() {
        let gate = DistillationGate {
            stale_max_age_days: 30,
            ..DistillationGate::default()
        };
        let stale = Principle {
            id: "stale-1".to_string(),
            category: PrincipleCategory::Heuristic,
            statement: "old stale principle".to_string(),
            confidence: Confidence::new(0.2),
            source_experience_ids: vec!["e1".to_string()],
            validation_count: 1,
            created_at: (Utc::now() - chrono::Duration::days(60)).to_rfc3339(),
            domain: None,
            q_value: 0.05,
            times_applied: 0,
        };
        let fresh = Principle {
            id: "fresh-1".to_string(),
            category: PrincipleCategory::Heuristic,
            statement: "fresh principle".to_string(),
            confidence: Confidence::new(0.7),
            source_experience_ids: vec!["e2".to_string()],
            validation_count: 4,
            created_at: Utc::now().to_rfc3339(),
            domain: None,
            q_value: 0.5,
            times_applied: 0,
        };

        let stale_ids = identify_stale_principles(&[stale, fresh], &gate);
        assert_eq!(stale_ids, vec!["stale-1".to_string()]);
    }

    #[tokio::test]
    async fn run_distillation_round_trip() {
        let temp = tempfile::TempDir::new().unwrap();
        let mem = crate::core::memory::MarkdownMemory::new(temp.path());
        let atoms = make_atoms(
            ExperienceKind::TurnInteraction,
            ExperienceOutcome::Success,
            4,
        );

        let principles = run_distillation(&mem, "test-entity", &atoms).await.unwrap();
        assert!(!principles.is_empty());
    }

    #[tokio::test]
    async fn run_distillation_surfaces_persistence_failure() {
        let atoms = make_atoms(
            ExperienceKind::TurnInteraction,
            ExperienceOutcome::Success,
            4,
        );

        let error = run_distillation(&FailingWriteMemory, "test-entity", &atoms)
            .await
            .expect_err("persistence failure should be visible to caller");

        assert!(
            error
                .to_string()
                .contains("forced distillation persistence failure")
        );
    }
}
