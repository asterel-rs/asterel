//! Turn augmentation pipeline: defines the `TurnAugmentor` trait
//! and the default implementation that wires grounding, experience,
//! affect, taste, and post-answer outcome capture.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use super::policy::{extract_situation, modulate_policy_for_affect};
use super::types::{TurnAugmentations, TurnStyleOverlay};
use crate::config::PersonaConfig;
use crate::contracts::observability::Observer;
use crate::core::affect::AffectDetector;
use crate::core::memory::MemoryRecallEntry;
use crate::core::memory::influence::CompanionGroundingAugmentation;
use crate::core::providers::Provider;

/// Trait for the turn augmentation pipeline.
///
/// Called at two points in the agent loop:
/// - **pre-answer**: produces augmentation blocks + style overlay
///   that influence the LLM prompt and rendering parameters.
/// - **post-answer**: performs post-turn bookkeeping (contradiction
///   detection, experience recording, etc.).
///
/// Implementations must be `Send + Sync` for use in async contexts
/// behind `Arc`.
pub(crate) trait TurnAugmentor: Send + Sync {
    /// Produce augmentation blocks before the LLM answer is generated.
    ///
    /// Each subsystem contributes a text block and/or style overlay.
    /// Empty blocks are silently omitted from the enriched message.
    fn pre_answer<'a>(
        &'a self,
        mem: &'a dyn crate::core::memory::Memory,
        entity_id: &'a str,
        person_id: &'a str,
        user_message: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<TurnAugmentations>> + Send + 'a>>;

    /// Run post-answer bookkeeping after the LLM response is saved.
    ///
    /// This is where contradiction detection, experience ingestion,
    /// and other post-turn operations happen. When `logprobs` are
    /// available from the provider, uncertainty-aware reward shaping
    /// is applied.
    fn post_answer<'a>(
        &'a self,
        mem: &'a dyn crate::core::memory::Memory,
        entity_id: &'a str,
        person_id: &'a str,
        user_message: &'a str,
        assistant_answer: &'a str,
        logprobs: Option<Vec<crate::core::providers::response::TokenLogprob>>,
    ) -> Pin<
        Box<dyn Future<Output = Result<Option<crate::contracts::policy::ReasonTrace>>> + Send + 'a>,
    >;
}

/// Default augmentation pipeline wiring the companion-first feedback-loop
/// subsystems (grounding, experience, affect, taste, policy).
pub(crate) struct DefaultAugmentor {
    workspace_dir: std::path::PathBuf,
    persona_config: PersonaConfig,
    /// Provider for LLM-based augmentation subsystems (affect
    /// detection and user modelling). `None`
    /// disables LLM-based features; rule-based fallbacks are used.
    provider: Option<Arc<dyn Provider>>,
    /// Model identifier for auxiliary LLM calls.
    model_name: String,
    /// LLM-based affect detector, built when `enable_llm_affect` is true.
    affect_detector: Option<Arc<dyn AffectDetector>>,
    /// Session-scoped control state (mode, density, avoidance). Updated each turn.
    session_control: std::sync::Mutex<crate::core::agent::session_control::SessionControlState>,
    /// Affect → governance bridge: tracks distress and demotes autonomy when sustained.
    affect_governance: std::sync::Mutex<crate::security::affect_governance::AffectGovernanceBridge>,
    /// Process-wide trust tracker for the affect-governance bridge.
    trust_tracker: Option<Arc<crate::security::domain_trust::DomainTrustTracker>>,
    /// Runtime observer for augmentation degradation signals.
    observer: Option<Arc<dyn Observer>>,
}

impl DefaultAugmentor {
    /// Create a new augmentor with the given workspace and persona config.
    pub(crate) fn new(
        workspace_dir: std::path::PathBuf,
        persona_config: PersonaConfig,
        provider: Option<Arc<dyn Provider>>,
        model_name: String,
        trust_tracker: Option<Arc<crate::security::domain_trust::DomainTrustTracker>>,
        observer: Option<Arc<dyn Observer>>,
    ) -> Self {
        let affect_detector = if persona_config.enable_llm_affect {
            Some(crate::core::affect::build_affect_detector(
                true,
                provider.clone(),
                model_name.clone(),
            ))
        } else {
            None
        };
        Self {
            workspace_dir,
            persona_config,
            provider,
            model_name,
            affect_detector,
            session_control: std::sync::Mutex::new(
                crate::core::agent::session_control::SessionControlState::default(),
            ),
            affect_governance: std::sync::Mutex::new(
                crate::security::affect_governance::AffectGovernanceBridge::default(),
            ),
            trust_tracker,
            observer,
        }
    }
}

/// Async wrapper that reads taste ratings from `PostgreSQL`.
#[cfg(all(feature = "taste", feature = "postgres"))]
async fn load_taste_ratings_async(
    workspace_dir: &std::path::Path,
) -> anyhow::Result<Vec<(String, f64, u32)>> {
    use sqlx_core::pool::PoolOptions;
    use sqlx_core::query::query;
    use sqlx_core::row::Row;
    use sqlx_postgres::Postgres;

    let Some(database_url) =
        crate::utils::postgres::resolve_postgres_url(None, Some(workspace_dir))
    else {
        return Ok(Vec::new());
    };

    let pool = PoolOptions::<Postgres>::new()
        .max_connections(3)
        .connect(&database_url)
        .await
        .map_err(|error| anyhow::anyhow!("connect postgres for taste ratings: {error}"))?;

    let rows = query(
        "SELECT item_id, rating, n_comparisons
         FROM taste_ratings
         WHERE domain = $1",
    )
    .bind("text")
    .fetch_all(&pool)
    .await
    .map_err(|error| anyhow::anyhow!("query taste ratings: {error}"))?;

    rows.into_iter()
        .map(|row| {
            let item_id: String = row.get("item_id");
            let rating: f64 = row.get("rating");
            let n_comparisons_raw: i64 = row.get("n_comparisons");
            let n_comparisons = u32::try_from(n_comparisons_raw)
                .map_err(|error| anyhow::anyhow!("taste n_comparisons overflow: {error}"))?;
            Ok((item_id, rating, n_comparisons))
        })
        .collect()
}

#[cfg(all(feature = "taste", not(feature = "postgres")))]
async fn load_taste_ratings_async(
    _workspace_dir: &std::path::Path,
) -> anyhow::Result<Vec<(String, f64, u32)>> {
    Ok(Vec::new())
}

/// Run hybrid affect detection and derive style overlay + prompt blocks.
///
/// Uses the optional LLM-backed `detector` for disambiguation; falls back
/// to rule-based detection when `detector` is `None`.  Returns the affect
/// reading, clamped style overlay, rendered `affect_block` string, and
/// `cause_guidance_block` (empty for Neutral affect).
///
/// Style deltas are derived from VAD (Valence-Arousal-Dominance) when
/// confidence ≥ 0.65, otherwise from the coarser label-to-delta mapping.
async fn detect_affect_and_style(
    user_message: &str,
    detector: Option<&dyn AffectDetector>,
) -> (
    crate::core::affect::AffectReading,
    TurnStyleOverlay,
    String,
    String,
) {
    let hybrid = crate::core::affect::hybrid_detect(user_message, detector).await;
    let reading = hybrid.final_reading;
    tracing::debug!(
        disambiguation_used = hybrid.disambiguation_used,
        disambiguation_source = ?hybrid.disambiguation_source,
        affect_label = ?reading.label,
        affect_confidence = reading.confidence.get(),
        "hybrid affect detection completed"
    );
    let delta = if reading.confidence.get() >= 0.65 {
        crate::core::affect::affect_vad_to_style_delta(&reading)
    } else {
        crate::core::affect::affect_to_style_delta(reading.label)
    };
    let affect_block =
        crate::core::affect::render_affect_block(reading.label, reading.confidence.get());

    let cause = crate::core::affect::cause::attribute_cause_vad(
        user_message,
        reading.label,
        reading.valence,
        reading.dominance,
        reading.arousal,
    );
    let cause_guidance_block = if reading.label == crate::core::affect::AffectLabel::Neutral {
        String::new()
    } else {
        let guidance = crate::core::affect::cause::cause_to_guidance(cause);
        let mut s = String::with_capacity(16 + guidance.len());
        s.push_str("[Affect Cause]\n");
        s.push_str(guidance);
        s.push('\n');
        s
    };

    let style_overlay = TurnStyleOverlay {
        formality_delta: delta.formality_delta,
        verbosity_delta: delta.verbosity_delta,
        temperature_delta: delta.temperature_delta,
    }
    .clamped();

    (reading, style_overlay, affect_block, cause_guidance_block)
}

/// Build the session mood prompt block for the current turn.
///
/// Loads the affect arc from memory, pushes the current reading, applies
/// decay if `enable_affect_decay` is set, and renders the mood block.
/// Returns an empty string when decay is disabled.
async fn build_session_mood_block(
    mem: &dyn crate::core::memory::Memory,
    entity_id: &str,
    person_id: &str,
    reading: &crate::core::affect::AffectReading,
    persona_config: &PersonaConfig,
) -> String {
    if !persona_config.enable_affect_decay {
        return String::new();
    }

    let mut arc = match crate::core::affect::persistence::load_affect_arc(mem, entity_id).await {
        Ok(arc) => arc,
        Err(error) => {
            tracing::debug!(%error, entity_id, "failed to load affect arc for session mood");
            crate::core::affect::AffectArc::new()
        }
    };
    let session_boundary = super::persona_updates::is_session_boundary_from_memory(
        mem,
        person_id,
        chrono::Utc::now(),
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

    let baseline = if persona_config.enable_big_five {
        if let Some(profile) = crate::core::persona::big_five::load_big_five(mem, person_id).await {
            crate::core::affect::SessionMood::from_big_five(
                profile.extraversion,
                profile.agreeableness,
                profile.conscientiousness,
                profile.neuroticism,
                profile.openness,
            )
        } else {
            let identity = &persona_config.character.identity;
            crate::core::affect::SessionMood::from_big_five(
                identity.extraversion,
                identity.agreeableness,
                identity.conscientiousness,
                identity.neuroticism,
                identity.openness,
            )
        }
    } else {
        let identity = &persona_config.character.identity;
        crate::core::affect::SessionMood::from_big_five(
            identity.extraversion,
            identity.agreeableness,
            identity.conscientiousness,
            identity.neuroticism,
            identity.openness,
        )
    };

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
    }
    let behavioral_mood = crate::core::affect::select_mood(reading);
    tracing::debug!(
        entity_id,
        behavior_mood = behavioral_mood.label(),
        behavior_nudge = behavioral_mood.nudge(),
        mood_distance = mood.distance(&baseline),
        "derived session mood for prompt injection"
    );
    crate::core::affect::render_session_mood_block(&mood)
}

/// Run the affect topology pipeline: appraisal → diffusion → latent bias → render.
///
/// Returns both the rendered prompt block and the raw `TopologySnapshot`
/// so the caller can feed it to the affect-governance bridge.
async fn build_topology_block(
    reading: &crate::contracts::affect::AffectReading,
    persona_config: &PersonaConfig,
    _session_mood_block: &str,
    mem: &dyn crate::core::memory::Memory,
    entity_id: &str,
    person_id: &str,
    user_message: &str,
) -> (
    String,
    Option<crate::core::affect::topology::TopologySnapshot>,
) {
    use crate::core::affect::appraisal::{appraise_event, topic_is_personal_text_cue};
    use crate::core::affect::topology::{
        TopologyGraph, activate_from_appraisal, apply_latent_bias, build_snapshot,
        diffuse_on_topology,
    };

    let topo_config = &persona_config.character.affect_topology;
    if topo_config.node_set.is_empty() {
        return (String::new(), None);
    }

    let graph = TopologyGraph::from_config(topo_config);

    // Determine contextual signals for appraisal
    let is_direct_address = true; // conservative default: companion is always addressed
    let topic_is_personal = topic_is_personal_text_cue(user_message);

    let appraisal = appraise_event(reading, is_direct_address, topic_is_personal);
    let base = activate_from_appraisal(&appraisal, &graph);
    let diffused = diffuse_on_topology(&base, &graph);

    // Load relationship depth for expression gating
    let relationship_depth = load_relationship_depth(mem, entity_id, person_id).await;

    // Load or reconstruct session mood for bias computation
    let session_mood = if persona_config.enable_affect_decay {
        load_session_mood_for_topology(mem, entity_id, persona_config, person_id).await
    } else {
        crate::core::affect::SessionMood::default()
    };

    let (surfaced, suppressed) = apply_latent_bias(
        &diffused,
        &topo_config.latent_bias,
        &graph,
        relationship_depth,
        &session_mood,
    );

    let snapshot = build_snapshot(&graph, &base, &diffused, &surfaced, &suppressed);
    let block = crate::core::affect::render_topology_block(&snapshot);
    (block, Some(snapshot))
}

/// Load relationship trust/rapport as a single depth scalar for topology gating.
async fn load_relationship_depth(
    mem: &dyn crate::core::memory::Memory,
    entity_id: &str,
    person_id: &str,
) -> f32 {
    match crate::core::persona::relationship::load_relationship_for_entity(
        mem, entity_id, person_id,
    )
    .await
    {
        Ok(Some(rel)) => {
            let depth = f32::midpoint(rel.trust_level, rel.rapport);
            depth.clamp(0.0, 1.0)
        }
        _ => 0.3_f32, // conservative default for unknown relationships
    }
}

/// Load the current session mood for topology bias computation.
async fn load_session_mood_for_topology(
    mem: &dyn crate::core::memory::Memory,
    entity_id: &str,
    persona_config: &PersonaConfig,
    person_id: &str,
) -> crate::core::affect::SessionMood {
    let Ok(mut arc) = crate::core::affect::persistence::load_affect_arc(mem, entity_id).await
    else {
        return crate::core::affect::SessionMood::default();
    };
    let character = &persona_config.character;
    let baseline = if persona_config.enable_big_five {
        let identity = &character.identity;
        crate::core::affect::SessionMood::from_big_five(
            identity.extraversion,
            identity.agreeableness,
            identity.conscientiousness,
            identity.neuroticism,
            identity.openness,
        )
    } else {
        crate::core::affect::SessionMood::default()
    };
    let session_boundary = super::persona_updates::is_session_boundary_from_memory(
        mem,
        person_id,
        chrono::Utc::now(),
        character.affect_decay.session_boundary_inactivity_minutes,
    )
    .await;
    if session_boundary {
        arc = crate::core::affect::AffectArc::new();
    }
    crate::core::affect::SessionMood::from_affect_arc(&arc, &character.affect_decay, &baseline)
}

#[cfg(feature = "taste")]
async fn build_taste_block(workspace_dir: &std::path::Path) -> String {
    match load_taste_ratings_async(workspace_dir).await {
        Ok(ratings) => {
            let guidance = crate::core::taste::influence::build_taste_guidance(&ratings);
            crate::core::taste::presenter::render_taste_contract(&guidance)
        }
        Err(error) => {
            tracing::warn!(%error, "failed to load taste ratings for augmentation");
            String::new()
        }
    }
}

#[cfg(not(feature = "taste"))]
async fn build_taste_block(_: &std::path::Path) -> String {
    String::new()
}

/// Load recalled memory items and relevant principles for the current turn.
///
/// Uses a fixed `top_k` of 10 for companion-first mainline recall. Both recall
/// and principle retrieval failures are logged as warnings and return empty
/// collections rather than propagating errors.
async fn load_recall_and_principles(
    mem: &dyn crate::core::memory::Memory,
    entity_id: &str,
    user_message: &str,
    reading: &crate::core::affect::AffectReading,
) -> (
    Vec<MemoryRecallEntry>,
    Vec<crate::core::experience::distill_types::Principle>,
    super::retrieval_quality::SelfRagConfig,
    usize,
) {
    let _quick_domain =
        super::policy::extract_situation(user_message, reading.label, reading.confidence.get())
            .domain;
    let meta_top_k = 10;
    let self_rag_config = super::retrieval_quality::SelfRagConfig::default();

    let recall_items = {
        let query = crate::core::memory::RecallQuery::new(entity_id, user_message, meta_top_k);
        mem.recall_scoped(query).await.unwrap_or_else(|error| {
            tracing::warn!(%error, "grounding recall failed in augmentor");
            Vec::new()
        })
    };
    let principles = crate::core::experience::principle_retrieve::retrieve_relevant_principles(
        mem,
        entity_id,
        user_message,
    )
    .await
    .unwrap_or_else(|error| {
        tracing::warn!(%error, "principle retrieval failed in augmentor");
        Vec::new()
    });

    (recall_items, principles, self_rag_config, meta_top_k)
}

/// Build the grounding contract block from sanitised recall items.
///
/// Sanitises items for external content safety, builds a context bundle
/// (deduplication + contradiction detection), and renders the grounding
/// contract string.
#[cfg(test)]
fn build_grounding_block(user_message: &str, recall_items: &[MemoryRecallEntry]) -> String {
    build_grounding_augmentation(user_message, recall_items).block
}

fn build_grounding_augmentation(
    user_message: &str,
    recall_items: &[MemoryRecallEntry],
) -> CompanionGroundingAugmentation {
    let augmentation = crate::core::memory::influence::build_companion_grounding_augmentation(
        user_message,
        recall_items,
        0.3,
    );
    if !augmentation.block.is_empty() {
        let companion_grounding = crate::core::memory::graphrag::build_companion_memory_grounding(
            user_message,
            recall_items,
        );
        tracing::debug!(
            companion_user_focus = companion_grounding.user_focus.len(),
            companion_topics = companion_grounding.active_topics.len(),
            companion_continuity = companion_grounding.continuity_cues.len(),
            "grounding block built"
        );
    }
    if augmentation.exposure.has_suppression() {
        tracing::debug!(
            public_visible = augmentation.exposure.public_visible,
            private_internal = augmentation.exposure.private_internal,
            secret_suppressed = augmentation.exposure.secret_suppressed,
            "grounding exposure rail suppressed secret recall"
        );
    }
    augmentation
}

/// Build the experience hint block from the top-5 relevant past experience atoms.
async fn build_experience_block(
    mem: &dyn crate::core::memory::Memory,
    entity_id: &str,
    user_message: &str,
) -> String {
    match crate::core::experience::retrieve_relevant_experiences(mem, entity_id, user_message, 5)
        .await
    {
        Ok(experiences) => crate::core::experience::render_experience_block(&experiences),
        Err(error) => {
            tracing::warn!(%error, "experience retrieval failed in augmentor");
            String::new()
        }
    }
}

/// Select the turn policy.
///
/// Extracts situation features, selects a base policy via stored outcomes and
/// distilled principles, then modulates for affect.
async fn build_policy(
    mem: &dyn crate::core::memory::Memory,
    user_message: &str,
    reading: &crate::core::affect::AffectReading,
    principles: &[crate::core::experience::distill_types::Principle],
    entity_id: &str,
) -> (
    super::policy::SituationFeatures,
    super::policy::PolicyDecision,
) {
    let situation = extract_situation(user_message, reading.label, reading.confidence.get());
    let outcomes = super::outcome_record::retrieve_recent_outcomes(mem, entity_id, 50)
        .await
        .unwrap_or_default();
    let base_policy = super::policy_selector::select_policy(&situation, &outcomes, principles);
    let policy =
        modulate_policy_for_affect(&base_policy, reading.label, situation.affect_intensity);
    (situation, policy)
}

/// Build the reasoning strategy, principle, attention, and curiosity blocks.
///
/// Returns `(reasoning_block, principle_block, attention_block, curiosity_block)`.
/// The curiosity block is empty when `enable_curiosity_drive` is `false` or
/// the computed curiosity signal is below `curiosity_threshold`.
fn build_reasoning_principle_attention_curiosity(
    policy: &super::policy::PolicyDecision,
    principles: &[crate::core::experience::distill_types::Principle],
    recall_items: &[MemoryRecallEntry],
    user_message: &str,
    reading: &crate::core::affect::AffectReading,
    persona_config: &PersonaConfig,
    situation: &super::policy::SituationFeatures,
) -> (String, String, String, String) {
    let reasoning_block =
        crate::core::agent::presenter::render_reasoning_strategy_block(policy.reasoning);
    let principle_block = crate::core::experience::presenter::render_principle_block(principles);

    let (attention_block, attention_schema) = {
        let schema = crate::core::persona::attention::AttentionSchema::compute(
            user_message,
            recall_items,
            principles,
            reading.confidence.get(),
            0.5,
        );
        let block = crate::core::persona::presenter::render_attention_block(&schema);
        (block, schema)
    };

    let curiosity_block = if persona_config.enable_curiosity_drive {
        let signal = crate::core::persona::curiosity::compute_curiosity(
            &attention_schema,
            f64::from(situation.complexity),
            reading.label,
            reading.confidence.get(),
            persona_config.curiosity_threshold,
        );
        crate::core::persona::presenter::render_curiosity_block(signal.as_ref())
    } else {
        String::new()
    };

    (
        reasoning_block,
        principle_block,
        attention_block,
        curiosity_block,
    )
}

/// Build user model, Big Five personality guidance, and value profile blocks.
///
/// Uses LLM-enhanced inference when `enable_llm_user_model` is set; falls
/// back to rule-based inference otherwise.  Merges user model and user
/// knowledge blocks when both are non-empty.
///
/// Returns `(user_model_block, big_five_block, value_block)`.
/// `value_block` is always empty when the `taste` feature is disabled.
async fn build_user_model_big_five_value_blocks(
    augmentor: &DefaultAugmentor,
    mem: &dyn crate::core::memory::Memory,
    _entity_id: &str,
    person_id: &str,
    user_message: &str,
    reading: &crate::core::affect::AffectReading,
    recall_items: &[MemoryRecallEntry],
) -> (String, String, String) {
    let user_model_block_base = if augmentor.persona_config.enable_llm_user_model {
        let enhanced = crate::core::persona::llm_user_model::infer_enhanced_user_model(
            augmentor.provider.as_ref(),
            augmentor.observer.as_deref(),
            &augmentor.model_name,
            Duration::from_secs(augmentor.persona_config.llm_user_model_timeout_secs.max(1)),
            user_message,
            reading,
            recall_items,
        )
        .await;
        crate::core::persona::presenter::render_enhanced_user_model_block(&enhanced)
    } else {
        let model =
            crate::core::persona::user_model::infer_user_model(user_message, reading, recall_items);
        crate::core::persona::presenter::render_user_model_block(&model)
    };
    let user_knowledge_block =
        match crate::core::persona::user_knowledge::load_user_knowledge(mem, person_id).await {
            Ok(kg) => crate::core::persona::presenter::render_knowledge_block(&kg, user_message),
            Err(error) => {
                tracing::warn!(%error, "user knowledge load failed in pre-answer augmentor");
                String::new()
            }
        };
    let user_model_block = if user_knowledge_block.is_empty() {
        user_model_block_base
    } else if user_model_block_base.is_empty() {
        user_knowledge_block
    } else {
        let mut block = user_model_block_base;
        block.reserve(2 + user_knowledge_block.len());
        block.push_str("\n\n");
        block.push_str(&user_knowledge_block);
        block
    };

    let big_five_block = if augmentor.persona_config.enable_big_five {
        match crate::core::persona::big_five::load_big_five(mem, person_id).await {
            Some(profile) => crate::core::persona::presenter::render_guidance_block(&profile),
            None => crate::core::persona::presenter::render_guidance_block(
                &crate::core::persona::big_five::BigFiveProfile::from_character_config(
                    &augmentor.persona_config,
                ),
            ),
        }
    } else {
        String::new()
    };

    #[cfg(feature = "taste")]
    let value_block = {
        match crate::core::taste::value_profile::load_value_profile(mem, person_id).await {
            Ok(Some(profile)) => crate::core::taste::presenter::render_value_guidance(&profile),
            Ok(None) => String::new(),
            Err(error) => {
                tracing::warn!(%error, "value profile load failed in augmentor");
                String::new()
            }
        }
    };
    #[cfg(not(feature = "taste"))]
    let value_block = {
        let _ = person_id;
        String::new()
    };

    (user_model_block, big_five_block, value_block)
}

async fn build_behavior_block(
    mem: &dyn crate::core::memory::Memory,
    person_id: &str,
    user_message: &str,
    reading: &crate::core::affect::AffectReading,
    recall_items: &[MemoryRecallEntry],
    persona_config: &PersonaConfig,
) -> String {
    if !persona_config.enable_behavior_selector {
        return String::new();
    }

    let relationship = crate::core::persona::relationship::load_relationship(mem, person_id)
        .await
        .ok()
        .flatten();
    let style_profile = crate::core::persona::style_profile::load_style_profile(mem, person_id)
        .await
        .unwrap_or(None)
        .or_else(|| {
            persona_config.enable_character_config.then(|| {
                crate::core::persona::style_profile::StyleProfileState::from_character_config(
                    &persona_config.character,
                    "config-seed",
                )
            })
        });
    let big_five = crate::core::persona::big_five::load_big_five(mem, person_id)
        .await
        .unwrap_or_else(|| {
            crate::core::persona::big_five::BigFiveProfile::from_character_config(persona_config)
        });
    let user_model =
        crate::core::persona::user_model::infer_user_model(user_message, reading, recall_items);
    let selection = crate::core::persona::select_behavior(
        reading,
        relationship.as_ref(),
        crate::core::persona::continuity_v2::classify_dialogue_act(user_message),
        &big_five,
        style_profile.as_ref(),
        &user_model,
        &persona_config.character.relationship_tiers,
        &persona_config.character.trait_activation,
        persona_config.enable_trait_activation,
        None,
    );
    crate::core::persona::presenter::render_behavior_selection_block(&selection)
}

/// Build the integrated self/world/relationship model block.
///
/// Loads the self-model shadow, world model, and relationship state, merges
/// them via `build_integrated_model`, and renders both the integrated and
/// world model blocks (concatenated when both are non-empty).
async fn build_integrated_model_block(
    mem: &dyn crate::core::memory::Memory,
    person_id: &str,
    user_message: &str,
) -> String {
    let domain = super::policy::extract_situation(
        user_message,
        crate::core::affect::AffectLabel::Neutral,
        0.0,
    )
    .domain;
    if !matches!(
        domain,
        super::policy::DomainTag::Technical | super::policy::DomainTag::Administrative
    ) {
        return String::new();
    }

    let self_model = match crate::core::persona::self_model::build_self_model_shadow(
        mem,
        person_id,
        user_message,
    )
    .await
    {
        Ok(model) => model,
        Err(error) => {
            tracing::warn!(%error, "self model build failed in pre-answer augmentor");
            return String::new();
        }
    };
    let world = match crate::core::persona::world_model::load_world_model(mem, person_id).await {
        Ok(model) => model,
        Err(error) => {
            tracing::warn!(%error, "world model load failed in pre-answer augmentor");
            return String::new();
        }
    };
    let relationship =
        match crate::core::persona::relationship::load_relationship(mem, person_id).await {
            Ok(state) => state.unwrap_or_default(),
            Err(error) => {
                tracing::warn!(%error, "relationship load failed in pre-answer augmentor");
                return String::new();
            }
        };

    let integrated = crate::core::persona::integrated_model::build_integrated_model(
        &self_model,
        &world,
        &relationship,
    );
    let integrated_block =
        crate::core::persona::presenter::render_integrated_model_block(&integrated);
    let world_block = crate::core::persona::presenter::render_world_model_block(&world);
    if integrated_block.is_empty() {
        world_block
    } else if world_block.is_empty() {
        integrated_block
    } else {
        let mut block = integrated_block;
        block.reserve(2 + world_block.len());
        block.push_str("\n\n");
        block.push_str(&world_block);
        block
    }
}

/// Build the desire drive and cognitive scaffolding blocks.
///
/// The desire block is derived from the current VAD affect reading.
/// The scaffolding block is constructed from relationship state, style
/// profile, situation domain/complexity, and the active reasoning strategy.
///
/// Returns `(desire_block, scaffolding_block)`.
async fn build_desire_and_scaffolding(
    mem: &dyn crate::core::memory::Memory,
    person_id: &str,
    reading: &crate::core::affect::AffectReading,
    situation: &super::policy::SituationFeatures,
    policy: &super::policy::PolicyDecision,
) -> (String, String) {
    let desire_state = crate::core::affect::desire::derive_desire(
        reading.label,
        reading.valence,
        reading.arousal,
        reading.confidence.get(),
    );
    let desire_block = crate::core::affect::presenter::render_desire_block(&desire_state);

    let relationship_state = crate::core::persona::relationship::load_relationship(mem, person_id)
        .await
        .unwrap_or(None)
        .unwrap_or_default();
    let style_profile_for_scaffolding =
        crate::core::persona::style_profile::load_style_profile(mem, person_id)
            .await
            .unwrap_or(None)
            .unwrap_or_else(|| crate::core::persona::style_profile::StyleProfileState {
                formality: 40,
                verbosity: 50,
                temperature: 0.7,
                updated_at: String::new(),
            });
    let scaffolding_state = crate::core::persona::scaffolding::build_scaffolding_state(
        reading.label,
        reading.confidence.get(),
        &relationship_state,
        situation.domain,
        f64::from(situation.complexity),
        policy.reasoning,
        "natural",
        &style_profile_for_scaffolding,
    );
    let scaffolding_block =
        crate::core::persona::presenter::render_scaffolding_block(&scaffolding_state);

    (desire_block, scaffolding_block)
}

impl DefaultAugmentor {
    /// Evaluate the affect-governance bridge against a topology snapshot.
    ///
    /// When the trust tracker singleton is available and distress is sustained
    /// across consecutive turns, soft violations are recorded to reduce
    /// tool-execution autonomy.
    fn evaluate_affect_governance(
        &self,
        snapshot: &crate::core::affect::topology::TopologySnapshot,
    ) {
        let Some(tracker) = &self.trust_tracker else {
            return;
        };
        let result = self
            .affect_governance
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .evaluate(snapshot, tracker);
        if result.violation_recorded {
            tracing::warn!(
                distress_score = result.distress_score,
                consecutive_turns = result.consecutive_turns,
                "affect governance bridge recorded trust violation due to sustained distress"
            );
        } else if result.distress_score >= 0.3 {
            tracing::debug!(
                distress_score = result.distress_score,
                consecutive_turns = result.consecutive_turns,
                "affect governance bridge: elevated distress (no violation yet)"
            );
        }
    }
}

// Cast safety: affect confidences are normalized to [0.0, 1.0] before f32 conversions in this impl.
#[allow(clippy::cast_possible_truncation, clippy::too_many_lines)]
impl TurnAugmentor for DefaultAugmentor {
    fn pre_answer<'a>(
        &'a self,
        mem: &'a dyn crate::core::memory::Memory,
        entity_id: &'a str,
        person_id: &'a str,
        user_message: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<TurnAugmentations>> + Send + 'a>> {
        Box::pin(async move {
            let (reading, style_overlay, affect_block, cause_guidance_block) =
                detect_affect_and_style(user_message, self.affect_detector.as_deref()).await;
            let mut affect_block = affect_block;
            let session_mood_block =
                build_session_mood_block(mem, entity_id, person_id, &reading, &self.persona_config)
                    .await;
            if !session_mood_block.is_empty() {
                affect_block.push_str(&session_mood_block);
                affect_block.push('\n');
            }

            // Affect topology: route emotions through character-specific graph
            let (topology_block, topology_snapshot) = if self.persona_config.enable_affect_topology
            {
                build_topology_block(
                    &reading,
                    &self.persona_config,
                    &session_mood_block,
                    mem,
                    entity_id,
                    person_id,
                    user_message,
                )
                .await
            } else {
                (String::new(), None)
            };

            // Affect → governance bridge: evaluate distress and demote trust if sustained
            let topology_diagnostics = if let Some(snapshot) = &topology_snapshot {
                self.evaluate_affect_governance(snapshot);
                let diagnostics = snapshot.diffusion_diagnostics();
                if let Some(top) = diagnostics.first() {
                    let character_definition_hash = self.persona_config.character.definition_hash();
                    tracing::debug!(
                        diagnostics = diagnostics.len(),
                        character_definition_hash = %character_definition_hash,
                        node = %top.node.0,
                        base = top.base_intensity,
                        diffused = top.diffused_intensity,
                        diffusion_delta = top.diffusion_delta,
                        surfaced = top.surfaced_intensity,
                        surface_delta = top.surface_delta,
                        suppressed = top.suppressed,
                        "affect topology diffusion diagnostics prepared"
                    );
                }
                diagnostics
            } else {
                Vec::new()
            };

            // Session control state: update mode/density/avoidance from current turn
            let session_control_block = if self.persona_config.enable_session_control_state {
                let mut state = self
                    .session_control
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                crate::core::agent::session_control::update_control_state(
                    &mut state,
                    user_message,
                    &reading,
                );
                crate::core::agent::session_control::render_session_control_block(&state)
            } else {
                String::new()
            };

            let taste_block = build_taste_block(&self.workspace_dir).await;

            let (recall_items, principles, self_rag_config, top_k) =
                load_recall_and_principles(mem, entity_id, user_message, &reading).await;

            let (recall_items, requeried) = super::retrieval_quality::self_rag_recall(
                mem,
                entity_id,
                user_message,
                recall_items,
                &self_rag_config,
                top_k,
            )
            .await;
            if requeried {
                tracing::info!(
                    items = recall_items.len(),
                    "self-RAG re-query produced merged result set"
                );
            }

            let grounding_augmentation = build_grounding_augmentation(user_message, &recall_items);
            let grounding_exposure = grounding_augmentation.exposure;
            let grounding_block = grounding_augmentation.block;
            let experience_block = build_experience_block(mem, entity_id, user_message).await;

            let (situation, policy) =
                build_policy(mem, user_message, &reading, &principles, entity_id).await;

            let (reasoning_block, principle_block, attention_block, curiosity_block) =
                build_reasoning_principle_attention_curiosity(
                    &policy,
                    &principles,
                    &recall_items,
                    user_message,
                    &reading,
                    &self.persona_config,
                    &situation,
                );

            let (user_model_block, big_five_block, value_block) =
                build_user_model_big_five_value_blocks(
                    self,
                    mem,
                    entity_id,
                    person_id,
                    user_message,
                    &reading,
                    &recall_items,
                )
                .await;
            let behavior_block = build_behavior_block(
                mem,
                person_id,
                user_message,
                &reading,
                &recall_items,
                &self.persona_config,
            )
            .await;
            let integrated_model_block =
                build_integrated_model_block(mem, person_id, user_message).await;

            let (desire_block, scaffolding_block) =
                build_desire_and_scaffolding(mem, person_id, &reading, &situation, &policy).await;

            Ok(TurnAugmentations {
                grounding_block,
                grounding_exposure,
                taste_block,
                affect_block,
                experience_block,
                principle_block,
                attention_block,
                curiosity_block,
                value_block,
                user_model_block,
                integrated_model_block,
                cause_guidance_block,
                big_five_block,
                behavior_block,
                style_overlay,
                reasoning_block,
                scaffolding_block,
                desire_block,
                topology_block,
                topology_diagnostics,
                session_control_block,
                situation,
                policy,
            })
        })
    }

    // ── Post-answer outcome capture ────────────────────────────────
    fn post_answer<'a>(
        &'a self,
        mem: &'a dyn crate::core::memory::Memory,
        entity_id: &'a str,
        person_id: &'a str,
        user_message: &'a str,
        assistant_answer: &'a str,
        logprobs: Option<Vec<crate::core::providers::response::TokenLogprob>>,
    ) -> Pin<
        Box<dyn Future<Output = Result<Option<crate::contracts::policy::ReasonTrace>>> + Send + 'a>,
    > {
        Box::pin(async move {
            super::post_answer_capture::run_post_answer(
                mem,
                entity_id,
                person_id,
                user_message,
                assistant_answer,
                &self.persona_config,
                logprobs.as_deref(),
            )
            .await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{build_grounding_augmentation, build_grounding_block, build_session_mood_block};
    use crate::config::PersonaConfig;
    use crate::core::memory::{MarkdownMemory, MemoryRecallEntry, MemorySource, PrivacyLevel};

    fn recall_item_with_privacy(
        slot_key: &str,
        value: &str,
        privacy_level: PrivacyLevel,
    ) -> MemoryRecallEntry {
        MemoryRecallEntry {
            entity_id: "default".into(),
            slot_key: slot_key.into(),
            value: value.to_string(),
            source: MemorySource::ExplicitUser,
            confidence: crate::contracts::scores::Confidence::new(0.9),
            importance: crate::contracts::scores::Importance::new(0.6),
            privacy_level,
            score: 0.8,
            occurred_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn recall_item(slot_key: &str, value: &str) -> MemoryRecallEntry {
        recall_item_with_privacy(slot_key, value, PrivacyLevel::Private)
    }

    #[test]
    fn build_grounding_block_appends_companion_memory_graph_summary() {
        let items = vec![
            recall_item("profile.name", "Haru prefers quiet replies"),
            recall_item(
                "channel.context",
                "Writer Lounge is a shared worldbuilding room",
            ),
            recall_item(
                "continuity.thread",
                "Follow up from last week's noir planning thread",
            ),
        ];

        let rendered = build_grounding_block("continue our noir thread", &items);

        assert!(rendered.contains("<grounding>"));
        assert!(rendered.contains("[Companion Memory Graph]"));
        assert!(rendered.contains("User focus:"));
        assert!(rendered.contains("Continuity:"));
    }

    #[test]
    fn build_grounding_augmentation_reports_exposure_counts() {
        let items = vec![
            recall_item_with_privacy("profile.name", "Haru", PrivacyLevel::Public),
            recall_item_with_privacy("profile.note", "quiet replies", PrivacyLevel::Private),
            recall_item_with_privacy(
                "profile.secret",
                "SECRET_VALUE_DO_NOT_RENDER",
                PrivacyLevel::Secret,
            ),
        ];

        let augmentation = build_grounding_augmentation("profile", &items);

        assert_eq!(augmentation.exposure.public_visible, 1);
        assert_eq!(augmentation.exposure.private_internal, 1);
        assert_eq!(augmentation.exposure.secret_suppressed, 1);
        assert!(augmentation.exposure.has_suppression());
        assert!(!augmentation.block.contains("profile.secret"));
        assert!(!augmentation.block.contains("SECRET_VALUE_DO_NOT_RENDER"));
    }

    #[test]
    fn reasoning_strategy_blocks() {
        use crate::core::agent::loop_::augment::policy::ReasoningStrategy;
        assert!(
            crate::core::agent::presenter::render_reasoning_strategy_block(
                ReasoningStrategy::Standard,
            )
            .is_empty()
        );
        let stepwise = crate::core::agent::presenter::render_reasoning_strategy_block(
            ReasoningStrategy::Stepwise,
        );
        assert!(stepwise.contains("[Reasoning: Stepwise]") && stepwise.contains("step-by-step"));
        assert!(
            crate::core::agent::presenter::render_reasoning_strategy_block(
                ReasoningStrategy::VerifyFirst,
            )
            .contains("[Reasoning: VerifyFirst]")
        );
        assert!(
            crate::core::agent::presenter::render_reasoning_strategy_block(
                ReasoningStrategy::AskClarify,
            )
            .contains("[Reasoning: AskClarify]")
        );
    }

    #[tokio::test]
    async fn session_mood_block_ignores_previous_arc_after_inactivity_boundary() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let mem = MarkdownMemory::new(temp.path());
        let persona_config = PersonaConfig::default();
        let previous = crate::core::persona::world_model::WorldModel {
            time_context: crate::core::persona::world_model::TimeContext {
                session_start: Some("2000-01-01T00:00:00Z".to_string()),
                last_turn_at: Some("2000-01-01T00:05:00Z".to_string()),
                turn_count: 8,
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
            dominance: 0.1,
            confidence: crate::contracts::scores::Confidence::new(1.0),
        });
        crate::core::affect::persistence::persist_affect_arc(&mem, "person:clock-test", &old_arc)
            .await
            .unwrap();

        let block = build_session_mood_block(
            &mem,
            "person:clock-test",
            "clock-test",
            &crate::core::affect::AffectReading::neutral(),
            &persona_config,
        )
        .await;

        assert!(block.is_empty());
    }
}
