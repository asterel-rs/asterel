//! Memory update pipeline — post-answer episodic and knowledge storage.
//!
//! Called once per turn by [`super::post_answer_capture::run_post_answer`] after
//! the assistant's response has been generated.  Writes four categories of
//! memory artifacts:
//!
//! | Step | What is stored | Where |
//! |------|----------------|-------|
//! | 1 | Experience atom (turn outcome + lesson) | Episodic memory |
//! | 2 | Turn outcome record (situation + policy + outcome) | Outcome record store |
//! | 3 | Error taxonomy classification | `persona.error_taxonomy.*` slots |
//! | 4 | Error taxonomy pattern log | `tracing::info!` events |
//!
//! Additionally, two live models are updated each turn:
//!
//! - **Value signals** (taste feature; gated on `feature = "taste"`): extracts
//!   preference cues from the exchange and updates the user's value profile.
//! - **User knowledge graph**: updates the triplet-based model of what the
//!   user knows, keyed by `person_id` (not `entity_id`).
//!
//! Memory writes are deterministic on the companion-first mainline path.
use super::post_answer_capture::PostAnswerContext;

const ERROR_TAXONOMY_SLOT_PREFIX: &str = "persona.error_taxonomy.";

#[derive(serde::Serialize)]
struct ErrorTaxonomyPayload<'a> {
    category: crate::core::persona::error_taxonomy::ErrorCategory,
    module: crate::core::persona::error_taxonomy::ErrorModule,
    learnable: bool,
    confidence: crate::contracts::scores::Confidence,
    reasoning: &'a str,
    factors: &'a [String],
}

#[derive(serde::Serialize)]
struct LessonPayload<'a> {
    outcome: &'a super::policy::TurnOutcome,
    situation: &'a super::policy::SituationFeatures,
    policy: &'a super::policy::PolicyDecision,
    error_taxonomy: Option<ErrorTaxonomyPayload<'a>>,
}

/// Orchestrate all post-answer memory writes for the current turn.
///
/// Executes steps 1–4 from the module-level table, then updates the value
/// profile and user knowledge graph. All failures are logged as warnings and
/// do not abort subsequent steps.
pub(super) async fn run_memory_updates(ctx: &PostAnswerContext<'_>) {
    persist_turn_experience_atom(ctx).await;
    persist_turn_outcome_record(ctx).await;
    persist_error_taxonomy(ctx).await;
    emit_error_taxonomy_patterns(ctx).await;

    extract_and_persist_value_signals(
        ctx.mem,
        ctx.entity_id,
        ctx.user_message,
        ctx.assistant_answer,
    )
    .await;
    update_user_knowledge(
        ctx.mem,
        ctx.person_id,
        ctx.user_message,
        ctx.assistant_answer,
        ctx.success_score,
    )
    .await;
}

/// Build and persist a `TurnInteraction` experience atom for the current turn.
///
/// The atom carries the turn's success score and, when a classified error is
/// present, the full error taxonomy as a JSON lesson payload.  Confidence is
/// set to 0.6 to reflect that turn-level signals are noisier than multi-turn
/// distilled principles.
async fn persist_turn_experience_atom(ctx: &PostAnswerContext<'_>) {
    let error_taxonomy_payload = ctx
        .classified_error
        .as_ref()
        .map(|error| ErrorTaxonomyPayload {
            category: error.category,
            module: error.category.module(),
            learnable: error.category.is_learnable(),
            confidence: error.confidence,
            reasoning: &error.reasoning,
            factors: &error.factors,
        });
    let lesson_payload = LessonPayload {
        outcome: &ctx.outcome,
        situation: &ctx.situation,
        policy: &ctx.policy,
        error_taxonomy: error_taxonomy_payload,
    };
    let lesson_json = match serde_json::to_string(&lesson_payload) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(%error, "failed to serialize lesson payload");
            None
        }
    };

    let mut atom = crate::core::experience::ExperienceAtom::new(
        crate::core::experience::ExperienceKind::TurnInteraction,
        format!(
            "Turn outcome: success={:.2}, len={}",
            ctx.success_score, ctx.outcome.response_length
        ),
        experience_outcome(ctx.success_score),
    )
    .with_confidence(0.6);
    if let Some(lesson) = lesson_json {
        atom = atom.with_lesson(lesson);
    }

    if let Err(error) =
        crate::core::experience::persist_experience_atom(ctx.mem, ctx.entity_id, &atom).await
    {
        tracing::warn!(%error, "post-answer outcome persistence failed");
    }
}

/// Build and persist a structured `TurnOutcomeRecord` for this turn.
///
/// The record bundles the situation features, policy decision, and outcome
/// metrics as training data for the Phase 2B policy learners. Optional
/// quality-vector detail is attached when available.
async fn persist_turn_outcome_record(ctx: &PostAnswerContext<'_>) {
    let outcome_record = {
        let record = super::outcome_record::TurnOutcomeRecord::new(
            ctx.situation.clone(),
            ctx.policy.clone(),
            ctx.outcome.clone(),
        );
        match ctx.quality_vector.clone() {
            Some(qv) => record.with_quality_vector(qv),
            None => record,
        }
    };
    if let Err(error) =
        super::outcome_record::persist_outcome_record(ctx.mem, ctx.entity_id, &outcome_record).await
    {
        tracing::warn!(%error, "turn outcome record persistence failed");
    }
}

fn experience_outcome(success_score: f32) -> crate::core::experience::ExperienceOutcome {
    if success_score >= 0.5 {
        crate::core::experience::ExperienceOutcome::Success
    } else {
        crate::core::experience::ExperienceOutcome::Partial
    }
}

#[cfg(feature = "taste")]
async fn extract_and_persist_value_signals(
    mem: &dyn crate::core::memory::Memory,
    person_id: &str,
    user_message: &str,
    assistant_answer: &str,
) {
    let signals = crate::core::taste::value_signals::extract_value_signals(
        user_message,
        assistant_answer,
        false,
    );
    if signals.is_empty() {
        return;
    }

    let mut profile =
        match crate::core::taste::value_profile::load_value_profile(mem, person_id).await {
            Ok(Some(profile)) => profile,
            Ok(None) => crate::core::taste::value_profile::ValueProfile::default(),
            Err(error) => {
                tracing::warn!(%error, "value profile load failed");
                crate::core::taste::value_profile::ValueProfile::default()
            }
        };
    profile.update(&signals);
    if let Err(error) =
        crate::core::taste::value_profile::persist_value_profile(mem, person_id, &profile).await
    {
        tracing::warn!(%error, "value profile persistence failed");
    }
}

#[cfg(not(feature = "taste"))]
async fn extract_and_persist_value_signals(
    _mem: &dyn crate::core::memory::Memory,
    _person_id: &str,
    _user_message: &str,
    _assistant_answer: &str,
) {
}

async fn update_user_knowledge(
    mem: &dyn crate::core::memory::Memory,
    person_id: &str,
    user_message: &str,
    assistant_answer: &str,
    success_score: f32,
) {
    let mut kg = crate::core::persona::user_knowledge::load_user_knowledge(mem, person_id)
        .await
        .unwrap_or_default();
    kg.update_from_turn(user_message, assistant_answer, f64::from(success_score));
    if let Err(error) =
        crate::core::persona::user_knowledge::persist_user_knowledge(mem, person_id, &kg).await
    {
        tracing::warn!(%error, "user knowledge graph persistence failed");
    }
}

/// Persist the current turn's error taxonomy classification to memory.
///
/// Written to a unique `persona.error_taxonomy.<uuid>` slot so that
/// `emit_error_taxonomy_patterns` can recall a sliding window of recent
/// classifications and detect recurring error patterns.
async fn persist_error_taxonomy(ctx: &PostAnswerContext<'_>) {
    let Some(classified) = &ctx.classified_error else {
        return;
    };

    let slot_key = format!("{ERROR_TAXONOMY_SLOT_PREFIX}{}", uuid::Uuid::new_v4());
    let value = match serde_json::to_string(classified) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(%error, "failed to serialize error taxonomy classification");
            return;
        }
    };

    let input = crate::core::memory::MemoryEventInput::new(
        ctx.entity_id,
        &slot_key,
        crate::core::memory::MemoryEventType::FactAdded,
        value,
        crate::core::memory::MemorySource::System,
        crate::core::memory::PrivacyLevel::Private,
    )
    .with_confidence(classified.confidence.get().clamp(0.1, 0.99))
    .with_importance(0.5)
    .with_source_ref("persona.error_taxonomy.classification");

    if let Err(error) = ctx.mem.append_event(input).await {
        tracing::warn!(%error, "error taxonomy persistence failed");
    }
}

/// Recall recent error taxonomy slots and log any recurring patterns.
///
/// Looks back over the last 20 classifications.  Detected patterns are
/// emitted as `tracing::info!` events; the agent loop can monitor these
/// logs to surface learning opportunities (e.g., "dominant module: Tool").
async fn emit_error_taxonomy_patterns(ctx: &PostAnswerContext<'_>) {
    let recent: Vec<crate::core::persona::error_taxonomy::ClassifiedError> =
        match crate::core::memory::recall_helpers::recall_typed(
            ctx.mem,
            ctx.entity_id,
            ERROR_TAXONOMY_SLOT_PREFIX,
            20,
        )
        .await
        {
            Ok(items) => items,
            Err(error) => {
                tracing::warn!(%error, "error taxonomy recall failed");
                return;
            }
        };

    let patterns = crate::core::persona::error_taxonomy::identify_error_patterns(&recent);
    for pattern in patterns {
        let pattern_kind = match pattern.pattern_type {
            crate::core::persona::error_taxonomy::PatternType::DominantModule(module) => {
                format!("dominant_module:{module:?}")
            }
            crate::core::persona::error_taxonomy::PatternType::RecurringCategory(category) => {
                format!("recurring_category:{category:?}")
            }
        };
        tracing::info!(
            pattern = %pattern_kind,
            frequency = pattern.frequency,
            recommendation = %pattern.recommendation,
            "error taxonomy pattern detected"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PersonaConfig;
    use crate::contracts::memory_traits::MemoryReader;
    use crate::core::agent::loop_::augment::policy;
    use crate::core::memory::{MarkdownMemory, Memory};

    fn test_context<'a>(
        mem: &'a dyn Memory,
        persona_config: &'a PersonaConfig,
        entity_id: &'a str,
        person_id: &'a str,
        user_message: &'a str,
    ) -> PostAnswerContext<'a> {
        PostAnswerContext {
            mem,
            entity_id,
            person_id,
            user_message,
            assistant_answer: "Acknowledged.",
            persona_config,
            outcome: policy::TurnOutcome::default(),
            situation: policy::SituationFeatures::default(),
            policy: policy::PolicyDecision::default(),
            success_score: 0.8,
            quality_vector: None,
            classified_error: None,
            turn_started_at_utc: chrono::DateTime::parse_from_rfc3339("2026-04-24T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        }
    }

    #[tokio::test]
    async fn run_memory_updates_routes_user_knowledge_by_person_id_not_entity_id() {
        let temp = tempfile::TempDir::new().unwrap();
        let mem = MarkdownMemory::new(temp.path());
        let persona_config = PersonaConfig::default();
        let ctx = test_context(
            &mem,
            &persona_config,
            "person:entity-scope",
            "local-default",
            "I prefer concise answers while working on tokio debugging.",
        );

        run_memory_updates(&ctx).await;

        let slot = mem
            .resolve_slot(
                "person:local-default",
                "persona/local-default/user_knowledge/v1",
            )
            .await
            .expect("resolve succeeds")
            .expect("user knowledge slot exists");

        assert!(slot.value.contains("\"triplets\""));
    }

    #[cfg(feature = "postgres")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn run_memory_updates_persists_user_knowledge_in_postgres_memory() {
        let _db_guard = crate::utils::test_env::acquire_test_db().await;
        use std::sync::Arc;
        use std::time::Duration;

        use crate::core::memory::embeddings::{EmbeddingFuture, EmbeddingProvider};
        use crate::core::memory::postgres::{PostgresConnectOptions, PostgresMemory};
        use crate::utils::test_env::EnvVarGuard;

        struct FailingEmbedding;

        impl EmbeddingProvider for FailingEmbedding {
            fn name(&self) -> &'static str {
                "failing_test"
            }

            fn dimensions(&self) -> usize {
                3
            }

            fn embed<'a>(&'a self, _texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
                Box::pin(async move { anyhow::bail!("synthetic embedding failure") })
            }
        }

        let _env_guard = EnvVarGuard::require_postgres_url();
        let database_url = std::env::var("ASTEREL_POSTGRES_URL").expect("postgres url");
        let mem = PostgresMemory::connect_with_options(
            &database_url,
            Arc::new(FailingEmbedding),
            PostgresConnectOptions {
                cache_max: 16,
                graph_retrieval_fusion_enabled: false,
                graph_retrieval_weight: 0.0,
                max_connections: 4,
                min_connections: 1,
                connect_timeout: Duration::from_secs(5),
                idle_timeout: Duration::from_secs(30),
                vector_weight: 0.7,
                keyword_weight: 0.3,
                max_lifetime: Duration::from_secs(60),
                hnsw_ef_search: 0,
            },
        )
        .await
        .expect("connect postgres memory");

        let persona_config = PersonaConfig::default();
        let person_id = format!("postgres-memory-updates-{}", uuid::Uuid::new_v4().simple());
        let entity_id = format!("person:{person_id}");
        let ctx = test_context(
            &mem,
            &persona_config,
            &entity_id,
            &person_id,
            "I prefer concise Rust tokio debugging help.",
        );

        run_memory_updates(&ctx).await;

        let slot = mem
            .resolve_slot(
                &entity_id,
                &format!("persona/{person_id}/user_knowledge/v1"),
            )
            .await
            .expect("resolve succeeds")
            .expect("user knowledge slot exists");

        assert!(slot.value.contains("\"triplets\""));
    }
}
