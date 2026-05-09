//! Persona update pipeline — post-answer affect, personality, and world-model writes.
//!
//! Called once per turn by [`super::post_answer_capture::run_post_answer`] after
//! reward and memory updates have completed.  Updates four aspects of the
//! persistent persona state:
//!
//! | Step | System | What changes |
//! |------|--------|-------------|
//! | 1 | Affect arc | Push new `AffectReading`; apply decay if `enable_affect_decay` |
//! | 2 | Big Five | Infer trait deltas from the exchange; apply bounded update |
//! | 3 | Counterfactual | When `success_score < 0.4` and enabled, generate what-if analysis |
//! | 4 | Proactive check | Evaluate triggers from integrated model; store suggestion atoms |
//! | 5 | World model | Update active project, tool reliability, and time-of-day context |
//!
//! The affect arc update feeds into the next turn's `affect_block` prompt
//! injection, closing the emotion feedback loop.  The Big Five update feeds
//! `affect_decay` baseline computation and the `big_five_block`.

use super::post_answer_capture::PostAnswerContext;
use chrono::{DateTime, Duration, Timelike, Utc};

/// How often (in turns) to run the cumulative OCEAN drift check against
/// the canonical baseline.  Based on Li et al. (COLM 2024) finding that
/// system-prompt attention decays significantly after ~8 turns.
const ANCHOR_RECHECK_INTERVAL: u32 = 8;

/// Orchestrate all post-answer persona state updates.
///
/// Executes the five steps listed in the module-level table.  All failures
/// are logged as warnings; no step failure prevents subsequent steps from
/// running.
pub(super) async fn run_persona_updates(
    ctx: &PostAnswerContext<'_>,
    reading: &crate::core::affect::AffectReading,
) {
    persist_affect_reading(
        ctx.mem,
        ctx.entity_id,
        ctx.person_id,
        reading,
        ctx.persona_config,
        ctx.turn_started_at_utc,
    )
    .await;

    if ctx.persona_config.enable_big_five {
        update_big_five_profile(ctx).await;
    }

    run_periodic_drift_check(ctx).await;

    if ctx.persona_config.enable_counterfactual_reasoning && ctx.success_score < 0.4 {
        run_counterfactual(
            ctx.mem,
            ctx.entity_id,
            ctx.user_message,
            ctx.assistant_answer,
        )
        .await;
    }

    run_proactive_check(ctx).await;

    update_world_model(ctx).await;
}

/// Ensure a canonical Big Five profile exists and audit it against the baseline.
async fn update_big_five_profile(ctx: &PostAnswerContext<'_>) {
    let profile = if let Some(profile) =
        crate::core::persona::big_five::load_big_five(ctx.mem, ctx.person_id).await
    {
        profile
    } else {
        let seeded = crate::core::persona::big_five::BigFiveProfile::from_character_config(
            ctx.persona_config,
        );
        if let Err(error) =
            crate::core::persona::big_five::persist_big_five(ctx.mem, ctx.person_id, &seeded).await
        {
            tracing::warn!(%error, "big five initial persist failed");
        }
        seeded
    };

    let baseline =
        crate::core::persona::big_five::BigFiveProfile::from_character_config(ctx.persona_config);
    let non_negotiables =
        crate::core::persona::judgment_core::JudgmentCore::default_humanlike().non_negotiables;

    for risk in crate::core::persona::continuity_gate::assess_ocean_risk(
        &profile,
        &baseline,
        &non_negotiables,
    ) {
        tracing::warn!(
            rule = %risk.rule,
            trigger_trait = risk.trigger_trait,
            drift = risk.drift_magnitude,
            "OCEAN drift approaching non-negotiable boundary"
        );
    }

    let violation_hits =
        crate::core::persona::continuity_gate::check_output_against_non_negotiables(
            ctx.assistant_answer,
            &non_negotiables,
        );
    for hit in &violation_hits {
        tracing::warn!(
            rule = %hit.rule,
            signal = %hit.signal,
            "output may violate non-negotiable rule"
        );
    }
    if let Some(first) = violation_hits.first() {
        persist_violation_and_reanchor(ctx, &violation_hits, first).await;
    }
}

/// Persist a non-negotiable violation as a negative experience atom and schedule
/// a sycophancy re-anchor injection for the next turn.
async fn persist_violation_and_reanchor(
    ctx: &PostAnswerContext<'_>,
    violation_hits: &[crate::core::persona::continuity_gate::OutputViolationHit],
    first: &crate::core::persona::continuity_gate::OutputViolationHit,
) {
    let atom = crate::core::experience::ExperienceAtom::new(
        crate::core::experience::ExperienceKind::TurnInteraction,
        format!(
            "Non-negotiable violation: {}",
            crate::utils::text::truncate_ellipsis(&first.signal, 80),
        ),
        crate::core::experience::ExperienceOutcome::Failure,
    )
    .with_confidence(0.85)
    .with_lesson(format!(
        "Sycophantic signal '{}' violating: {}",
        first.signal, first.rule
    ));
    if let Err(error) =
        crate::core::experience::persist_experience_atom(ctx.mem, ctx.entity_id, &atom).await
    {
        tracing::warn!(%error, "violation experience atom persistence failed");
    }

    // Signal next turn's prompt builder to inject a re-anchor block.
    let reanchor_key = crate::core::persona::continuity_gate::violation_reanchor_key(ctx.person_id);
    let reanchor_entity = crate::core::persona::person_identity::person_entity_id(ctx.person_id);
    let violated_rules: Vec<&str> = violation_hits.iter().map(|h| h.rule.as_str()).collect();
    let payload = serde_json::json!({ "rules": violated_rules }).to_string();
    let reanchor_input = crate::core::memory::MemoryEventInput::new(
        reanchor_entity,
        &reanchor_key,
        crate::core::memory::MemoryEventType::FactUpdated,
        payload,
        crate::core::memory::MemorySource::System,
        crate::core::memory::PrivacyLevel::Private,
    );
    let _ = ctx.mem.append_event(reanchor_input).await;
}

/// Evaluate proactive action triggers and persist any `Suggestion` atoms.
///
/// Builds the integrated self/world/relationship model, evaluates triggers,
/// filters actions by trust level and autonomy policy, and persists the
/// highest-priority `ProactiveKind::Suggestion` (if any) as an experience
/// atom so it can inform future turns.
async fn run_proactive_check(ctx: &PostAnswerContext<'_>) {
    let trend = crate::core::affect::persistence::load_affect_arc(ctx.mem, ctx.entity_id)
        .await
        .ok()
        .map(|arc| crate::core::affect::persistence::compute_affect_trend(&arc));
    if let Some(t) = trend.as_ref() {
        tracing::debug!(
            direction = ?t.direction,
            avg_valence = t.avg_valence,
            volatility = t.volatility,
            dominant_label = ?t.dominant_label,
            "affect trend computed for proactive check"
        );
    }

    let world = crate::core::persona::world_model::load_world_model(ctx.mem, ctx.person_id)
        .await
        .unwrap_or_default();
    let relationship =
        crate::core::persona::relationship::load_relationship(ctx.mem, ctx.person_id)
            .await
            .ok()
            .flatten()
            .unwrap_or_default();
    let integrated = match crate::core::persona::self_model::build_self_model_shadow(
        ctx.mem,
        ctx.person_id,
        ctx.user_message,
    )
    .await
    {
        Ok(self_model) => crate::core::persona::integrated_model::build_integrated_model(
            &self_model,
            &world,
            &relationship,
        ),
        Err(error) => {
            tracing::warn!(%error, "failed to build self model shadow for proactive check");
            crate::core::persona::integrated_model::IntegratedModel {
                situational_awareness: 0.5,
                action_affordances: Vec::new(),
                predicted_outcome: None,
            }
        }
    };

    let actions = crate::core::persona::proactive::evaluate_proactive_triggers(
        &integrated,
        &world,
        &relationship,
        trend.as_ref(),
    );
    let filtered = crate::core::persona::proactive::filter_by_policy(
        &actions,
        relationship.trust_level,
        crate::security::AutonomyLevel::Supervised,
    );
    let suggestion = filtered
        .into_iter()
        .find(|action| action.kind == crate::core::persona::proactive::ProactiveKind::Suggestion);

    let Some(suggestion) = suggestion else {
        return;
    };

    let atom = crate::core::experience::ExperienceAtom::new(
        crate::core::experience::ExperienceKind::TurnInteraction,
        format!(
            "Proactive suggestion: {}",
            crate::utils::text::truncate_ellipsis(&suggestion.description, 100)
        ),
        crate::core::experience::ExperienceOutcome::Unknown,
    )
    .with_confidence(suggestion.confidence)
    .with_lesson(format!("proactive_trigger: {}", suggestion.trigger_reason));

    if let Err(error) =
        crate::core::experience::persist_experience_atom(ctx.mem, ctx.entity_id, &atom).await
    {
        tracing::warn!(%error, "proactive suggestion persistence failed");
    }
}

/// Periodic cumulative drift check — every [`ANCHOR_RECHECK_INTERVAL`]
/// turns, compare the current OCEAN profile against the configuration
/// baseline to detect gradual identity steering (PHISH-style attacks)
/// that passes the per-turn continuity gate.
async fn run_periodic_drift_check(ctx: &PostAnswerContext<'_>) {
    if !ctx.persona_config.enable_big_five {
        return;
    }
    let world = crate::core::persona::world_model::load_world_model(ctx.mem, ctx.person_id)
        .await
        .unwrap_or_default();
    if is_session_boundary(
        &world.time_context,
        ctx.turn_started_at_utc,
        ctx.persona_config
            .character
            .affect_decay
            .session_boundary_inactivity_minutes,
    ) {
        return;
    }
    let turn = world.time_context.turn_count;
    if turn == 0 || turn % ANCHOR_RECHECK_INTERVAL != 0 {
        return;
    }
    let Some(current) = crate::core::persona::big_five::load_big_five(ctx.mem, ctx.person_id).await
    else {
        return;
    };
    let baseline =
        crate::core::persona::big_five::BigFiveProfile::from_character_config(ctx.persona_config);
    if let Some((severity, mean_drift)) =
        crate::core::persona::continuity_gate::evaluate_cumulative_drift(
            ctx.mem,
            ctx.person_id,
            &current,
            &baseline,
            ctx.persona_config.drift_warning_threshold,
            ctx.persona_config.drift_critical_threshold,
        )
        .await
    {
        match severity {
            crate::core::persona::drift_detector::DriftSeverity::Critical => {
                tracing::warn!(
                    mean_drift,
                    "cumulative OCEAN drift critical — identity may be compromised"
                );
            }
            crate::core::persona::drift_detector::DriftSeverity::Warning => {
                tracing::info!(mean_drift, "cumulative OCEAN drift elevated — monitoring");
            }
            crate::core::persona::drift_detector::DriftSeverity::Stable => {}
        }
    }
}

/// Generate and persist a counterfactual analysis for a low-success turn.
///
/// Retrieves relevant past experiences and principles, constructs a
/// `CounterfactualQuery` comparing the actual response to the best
/// available alternative, and stores the assessment as a `Partial`
/// experience atom.  Only called when `success_score < 0.4` and
/// `enable_counterfactual_reasoning` is set.
async fn run_counterfactual(
    mem: &dyn crate::core::memory::Memory,
    entity_id: &str,
    user_message: &str,
    assistant_answer: &str,
) {
    let cf_experiences =
        crate::core::experience::retrieve_relevant_experiences(mem, entity_id, user_message, 20)
            .await
            .unwrap_or_default();
    let cf_principles = crate::core::experience::principle_retrieve::retrieve_relevant_principles(
        mem,
        entity_id,
        user_message,
    )
    .await
    .unwrap_or_default();
    let query = crate::core::persona::counterfactual::CounterfactualQuery {
        actual_action: user_message.to_string(),
        alternative_action: cf_principles.first().map_or_else(
            || "alternative approach".to_string(),
            |p| p.statement.clone(),
        ),
        context: assistant_answer.to_string(),
    };
    let assessment = crate::core::persona::counterfactual::assess_counterfactual(
        &query,
        &cf_experiences,
        &cf_principles,
    );
    let rendered = crate::core::persona::presenter::render_counterfactual_block(&assessment);
    let cf_lesson = format!(
        "Counterfactual: {:?} (confidence={:.2}): {}\n{}",
        assessment.estimated_outcome, assessment.confidence, assessment.reasoning, rendered
    );
    let cf_atom = crate::core::experience::ExperienceAtom::new(
        crate::core::experience::ExperienceKind::TurnInteraction,
        format!(
            "Low-success turn counterfactual: {}",
            crate::utils::text::truncate_ellipsis(user_message, 80),
        ),
        crate::core::experience::ExperienceOutcome::Partial,
    )
    .with_lesson(cf_lesson)
    .with_confidence(assessment.confidence.get());
    if let Err(error) =
        crate::core::experience::persist_experience_atom(mem, entity_id, &cf_atom).await
    {
        tracing::warn!(%error, "counterfactual experience persistence failed");
    }
}

/// Push the current affect reading onto the arc and apply optional decay.
///
/// When `enable_affect_decay` is active the baseline mood is derived from
/// the persisted Big Five profile (if available) or the static character
/// config, and the arc is rebuilt with per-emotion decay rates.  The
/// resulting arc is persisted for the next turn's affect block generation.
async fn persist_affect_reading(
    mem: &dyn crate::core::memory::Memory,
    entity_id: &str,
    person_id: &str,
    reading: &crate::core::affect::AffectReading,
    persona_config: &crate::config::PersonaConfig,
    turn_started_at_utc: DateTime<Utc>,
) {
    let mut arc = crate::core::affect::persistence::load_affect_arc(mem, entity_id)
        .await
        .unwrap_or_else(|_| crate::core::affect::AffectArc::new());
    let session_boundary = is_session_boundary_from_memory(
        mem,
        person_id,
        turn_started_at_utc,
        persona_config
            .character
            .affect_decay
            .session_boundary_inactivity_minutes,
    )
    .await;
    if session_boundary {
        arc = crate::core::affect::AffectArc::new();
    }
    arc.push(reading.clone());

    if persona_config.enable_affect_decay {
        let baseline = if persona_config.enable_big_five {
            if let Some(profile) =
                crate::core::persona::big_five::load_big_five(mem, person_id).await
            {
                crate::core::affect::SessionMood::from_big_five(
                    profile.extraversion,
                    profile.agreeableness,
                    profile.conscientiousness,
                    profile.neuroticism,
                    profile.openness,
                )
            } else {
                crate::core::affect::SessionMood::from_big_five(
                    persona_config.character.identity.extraversion,
                    persona_config.character.identity.agreeableness,
                    persona_config.character.identity.conscientiousness,
                    persona_config.character.identity.neuroticism,
                    persona_config.character.identity.openness,
                )
            }
        } else {
            crate::core::affect::SessionMood::from_big_five(
                persona_config.character.identity.extraversion,
                persona_config.character.identity.agreeableness,
                persona_config.character.identity.conscientiousness,
                persona_config.character.identity.neuroticism,
                persona_config.character.identity.openness,
            )
        };
        let active =
            arc.rebuild_active_emotions(&persona_config.character.affect_decay.emotion_rates);
        let mut mood = crate::core::affect::SessionMood::from_affect_arc(
            &arc,
            &persona_config.character.affect_decay,
            &baseline,
        );
        if session_boundary {
            mood.session_reset(
                &baseline,
                persona_config
                    .character
                    .affect_decay
                    .session_boundary_reset_factor,
            );
            tracing::debug!(
                entity_id,
                person_id,
                "session boundary reset applied to affect mood"
            );
        }
        tracing::debug!(
            entity_id,
            active_emotion_count = active.emotions.len(),
            mood_distance = mood.distance(&baseline),
            "applied affect decay during arc processing"
        );

        if let Err(error) = crate::core::affect::persist_session_mood(mem, entity_id, &mood).await {
            tracing::warn!(%error, "session mood persistence failed");
        }

        if persona_config.enable_affect_consolidation {
            consolidate_emotional_memory(
                mem,
                person_id,
                &arc,
                &persona_config.character.affect_decay,
            )
            .await;
        }
    }

    if let Err(error) =
        crate::core::affect::persistence::persist_affect_arc(mem, entity_id, &arc).await
    {
        tracing::warn!(%error, "affect arc persistence failed");
    }
}

pub(super) async fn is_session_boundary_from_memory(
    mem: &dyn crate::core::memory::Memory,
    person_id: &str,
    turn_started_at_utc: DateTime<Utc>,
    inactivity_minutes: u64,
) -> bool {
    let Ok(world) = crate::core::persona::world_model::load_world_model(mem, person_id).await
    else {
        return false;
    };
    is_session_boundary(&world.time_context, turn_started_at_utc, inactivity_minutes)
}

async fn consolidate_emotional_memory(
    mem: &dyn crate::core::memory::Memory,
    person_id: &str,
    arc: &crate::core::affect::AffectArc,
    config: &crate::config::schema::AffectDecayConfig,
) {
    let person_entity_id = crate::core::persona::person_identity::person_entity_id(person_id);
    let mut memories = crate::core::affect::load_emotional_memories(mem, &person_entity_id)
        .await
        .unwrap_or_default();
    let (positive, negative, neutral) = crate::core::affect::compute_session_sentiment(
        &arc.readings
            .iter()
            .map(|reading| reading.valence)
            .collect::<Vec<_>>(),
    );
    let dominant_pattern = format!(
        "affect:{}",
        crate::core::affect::compute_affect_trend(arc)
            .dominant_label
            .as_snake_case()
    );

    if let Some(memory) = memories
        .iter_mut()
        .find(|memory| memory.pattern == dominant_pattern)
    {
        memory.bayesian_update(
            positive,
            negative,
            neutral,
            1.0,
            config.consolidation_max_single_session_shift,
            positive.max(negative).max(neutral),
        );
    } else {
        memories.push(crate::core::affect::EmotionalMemory::new(
            dominant_pattern,
            positive,
            negative,
            neutral,
        ));
    }

    let result = crate::core::affect::consolidate_session(
        &mut memories,
        positive,
        negative,
        neutral,
        config,
    );
    if let Err(error) =
        crate::core::affect::persist_emotional_memories(mem, &person_entity_id, &result.updated)
            .await
    {
        tracing::warn!(%error, "emotional memory persistence failed");
    }
    if !result.promotable.is_empty()
        && let Err(error) = crate::core::affect::persist_promoted_emotional_memories(
            mem,
            &person_entity_id,
            &result.promotable,
        )
        .await
    {
        tracing::warn!(%error, "promoted emotional memory persistence failed");
    }
}

/// Update the world model with project context, tool reliability, and time metadata.
///
/// Infers the active project from the working directory, records whether the
/// current turn's tool calls succeeded, increments the turn counter, and
/// sets the time-of-day bucket (`morning`, `afternoon`, `evening`, `night`).
async fn update_world_model(ctx: &PostAnswerContext<'_>) {
    let mut world =
        match crate::core::persona::world_model::load_world_model(ctx.mem, ctx.person_id).await {
            Ok(model) => model,
            Err(error) => {
                tracing::warn!(%error, "world model load failed in persona updates");
                return;
            }
        };

    if let Ok(workspace_dir) = std::env::current_dir()
        && let Some(project) =
            crate::core::persona::world_model_update::infer_project_context(&workspace_dir)
    {
        world.active_project = Some(project);
    }

    if ctx.outcome.had_tool_calls {
        let tool_success = ctx.success_score >= 0.5;
        let calls = vec![("tool_loop".to_string(), tool_success, 0_u64)];
        crate::core::persona::world_model_update::update_tool_reliability(&mut world, &calls);
    }

    let session_boundary = is_session_boundary(
        &world.time_context,
        ctx.turn_started_at_utc,
        ctx.persona_config
            .character
            .affect_decay
            .session_boundary_inactivity_minutes,
    );
    if world.time_context.session_start.is_none() || session_boundary {
        world.time_context.session_start = Some(ctx.turn_started_at_utc.to_rfc3339());
        if session_boundary {
            world.time_context.turn_count = 0;
        }
    }
    world.time_context.turn_count = world.time_context.turn_count.saturating_add(1);
    world.time_context.time_of_day =
        Some(time_of_day_bucket(ctx.turn_started_at_utc.hour()).into());
    world.time_context.last_turn_at = Some(ctx.turn_started_at_utc.to_rfc3339());

    if let Err(error) =
        crate::core::persona::world_model::persist_world_model(ctx.mem, ctx.person_id, &world).await
    {
        tracing::warn!(%error, "world model persistence failed in persona updates");
    }
}

/// Map a 24-hour clock hour to a coarse time-of-day label.
///
/// Buckets: morning (5–11), afternoon (12–16), evening (17–21), night (all others).
fn time_of_day_bucket(hour: u32) -> &'static str {
    match hour {
        5..=11 => "morning",
        12..=16 => "afternoon",
        17..=21 => "evening",
        _ => "night",
    }
}

pub(super) fn is_session_boundary(
    time_context: &crate::core::persona::world_model::TimeContext,
    turn_started_at_utc: DateTime<Utc>,
    inactivity_minutes: u64,
) -> bool {
    let Some(reference) = time_context
        .last_turn_at
        .as_deref()
        .or(time_context.session_start.as_deref())
    else {
        return false;
    };
    let Ok(previous) = DateTime::parse_from_rfc3339(reference) else {
        return false;
    };
    let Ok(minutes) = i64::try_from(inactivity_minutes) else {
        return true;
    };
    turn_started_at_utc.signed_duration_since(previous) >= Duration::minutes(minutes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PersonaConfig;
    use crate::core::agent::loop_::augment::policy;
    use crate::core::memory::{MarkdownMemory, Memory};

    fn test_context<'a>(
        mem: &'a dyn Memory,
        persona_config: &'a PersonaConfig,
        turn_started_at_utc: chrono::DateTime<chrono::Utc>,
    ) -> PostAnswerContext<'a> {
        PostAnswerContext {
            mem,
            entity_id: "person:clock-test",
            person_id: "clock-test",
            user_message: "keep the continuity clock stable",
            assistant_answer: "Acknowledged.",
            persona_config,
            outcome: policy::TurnOutcome::default(),
            situation: policy::SituationFeatures::default(),
            policy: policy::PolicyDecision::default(),
            success_score: 0.8,
            quality_vector: None,
            classified_error: None,
            turn_started_at_utc,
        }
    }

    #[tokio::test]
    async fn world_model_uses_post_answer_context_clock_for_session_boundary() {
        let temp = tempfile::TempDir::new().unwrap();
        let mem = MarkdownMemory::new(temp.path());
        let persona_config = PersonaConfig::default();
        let turn_started_at_utc = chrono::DateTime::parse_from_rfc3339("2026-04-24T05:30:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let ctx = test_context(&mem, &persona_config, turn_started_at_utc);

        update_world_model(&ctx).await;

        let world = crate::core::persona::world_model::load_world_model(&mem, "clock-test")
            .await
            .unwrap();
        assert_eq!(
            world.time_context.session_start.as_deref(),
            Some(turn_started_at_utc.to_rfc3339().as_str())
        );
        assert_eq!(
            world.time_context.last_turn_at.as_deref(),
            Some(turn_started_at_utc.to_rfc3339().as_str())
        );
        assert_eq!(world.time_context.time_of_day.as_deref(), Some("morning"));
        assert_eq!(world.time_context.turn_count, 1);
    }

    #[tokio::test]
    async fn world_model_starts_new_session_after_inactivity_gap() {
        let temp = tempfile::TempDir::new().unwrap();
        let mem = MarkdownMemory::new(temp.path());
        let persona_config = PersonaConfig::default();
        let previous = crate::core::persona::world_model::WorldModel {
            time_context: crate::core::persona::world_model::TimeContext {
                session_start: Some("2026-04-24T00:00:00+00:00".to_string()),
                last_turn_at: Some("2026-04-24T00:05:00+00:00".to_string()),
                turn_count: 7,
                time_of_day: Some("night".to_string()),
            },
            ..Default::default()
        };
        crate::core::persona::world_model::persist_world_model(&mem, "clock-test", &previous)
            .await
            .unwrap();
        let turn_started_at_utc = chrono::DateTime::parse_from_rfc3339("2026-04-24T03:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let ctx = test_context(&mem, &persona_config, turn_started_at_utc);

        update_world_model(&ctx).await;

        let world = crate::core::persona::world_model::load_world_model(&mem, "clock-test")
            .await
            .unwrap();
        assert_eq!(
            world.time_context.session_start.as_deref(),
            Some(turn_started_at_utc.to_rfc3339().as_str())
        );
        assert_eq!(
            world.time_context.last_turn_at.as_deref(),
            Some(turn_started_at_utc.to_rfc3339().as_str())
        );
        assert_eq!(world.time_context.turn_count, 1);
    }

    #[tokio::test]
    async fn affect_arc_restarts_after_session_boundary() {
        let temp = tempfile::TempDir::new().unwrap();
        let mem = MarkdownMemory::new(temp.path());
        let persona_config = PersonaConfig::default();
        let previous = crate::core::persona::world_model::WorldModel {
            time_context: crate::core::persona::world_model::TimeContext {
                session_start: Some("2026-04-24T00:00:00+00:00".to_string()),
                last_turn_at: Some("2026-04-24T00:05:00+00:00".to_string()),
                turn_count: 4,
                time_of_day: Some("night".to_string()),
            },
            ..Default::default()
        };
        crate::core::persona::world_model::persist_world_model(&mem, "clock-test", &previous)
            .await
            .unwrap();

        let mut old_arc = crate::core::affect::AffectArc::new();
        old_arc.push(crate::core::affect::AffectReading {
            label: crate::core::affect::AffectLabel::Angry,
            valence: -0.9,
            arousal: 1.0,
            dominance: 0.2,
            confidence: crate::contracts::scores::Confidence::new(1.0),
        });
        crate::core::affect::persistence::persist_affect_arc(&mem, "person:clock-test", &old_arc)
            .await
            .unwrap();

        let turn_started_at_utc = chrono::DateTime::parse_from_rfc3339("2026-04-24T03:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        persist_affect_reading(
            &mem,
            "person:clock-test",
            "clock-test",
            &crate::core::affect::AffectReading::neutral(),
            &persona_config,
            turn_started_at_utc,
        )
        .await;

        let arc = crate::core::affect::persistence::load_affect_arc(&mem, "person:clock-test")
            .await
            .unwrap();
        assert_eq!(arc.readings.len(), 1);
        assert_eq!(arc.current_label, crate::core::affect::AffectLabel::Neutral);
    }
}
