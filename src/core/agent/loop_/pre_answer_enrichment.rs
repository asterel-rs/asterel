//! Pre-answer enrichment: prepares the enriched prompt before inference.
//!
//! `build_pre_answer_enrichment` is called once per turn, before the
//! tool loop.  It assembles every piece of runtime context the LLM
//! needs to produce a well-grounded, personalized response:
//!
//! 1. **Context recall** — loads relevant memory slots and renders
//!    them as a `ContextContract` block prepended to the user message.
//! 2. **Skill hints** — if `skills.turn_hint_limit > 0`, relevant skill
//!    metadata is injected so the LLM knows which skills are available.
//! 3. **Link context** — detected URLs are surfaced as a `[Detected Links]`
//!    block (full content fetch is opt-in via `ASTEREL_ENABLE_LINK_FETCH`).
//! 4. **Response baseline** — injects response-mode guidance (e.g. prose
//!    vs. bullet) derived from the response-style classifier.
//! 5. **Decision core** — injects `JudgmentCore` blocks for ethical/safety
//!    anchoring on each turn.
//! 6. **Style guidance** — if a style profile exists, appends formality
//!    and verbosity guidance (skipped for pure conversational turns).
//! 7. **Persona context blocks** — narrative self-description, relationship
//!    state, and pending follow-up items from the persona layer.
//! 8. **Augmentation** — runs `DefaultAugmentor.pre_answer()` which may add
//!    situation-domain context, reasoning strategy, and affect overlays.
//!
//! The `system_prompt_addendum` (self-contract + self-model shadow) is
//! built separately and merged into the system prompt in
//! `session_posturn.rs`.

use std::sync::Arc;

use super::augment::{self, TurnAugmentor as _};
use super::context::{
    ContextRuntimeMetadata, build_context_contract_with_runtime_metadata, context_budget_for_model,
};
use super::types::RuntimeMemoryWriteContext;
use crate::config::Config;
use crate::contracts::observability::Observer;
use crate::core::agent::response_style::{
    ResponseMode, classify_response_mode, render_judgment_core_turn_block,
    render_response_style_block,
};
use crate::core::memory::Memory;
use crate::core::persona::embodied_state::{
    EmbodiedStateSnapshot, load_embodied_state_snapshot, reasoning_strategy_override_from_snapshot,
};
use crate::core::persona::follow_up_queue::{clear_follow_ups, load_pending_follow_ups};
use crate::core::persona::judgment_core::JudgmentCore;
use crate::core::persona::presenter::{
    render_follow_up_block, render_narrative_block, render_relationship_context_block,
    render_self_contract_block, render_self_model_shadow_block, render_style_guidance,
};
use crate::core::persona::self_contract::build_prompt_self_contract;
use crate::core::persona::self_model::{SelfModelShadow, build_self_model_shadow};
use crate::core::persona::style_profile::{StyleProfileState, load_style_profile};
use crate::core::providers::Provider;
use crate::core::subagents::SkillMetadataProvider;
use crate::security::SecurityPolicy;

pub(crate) struct PreAnswerSharedParams<'a> {
    pub(crate) person_id: &'a str,
    pub(crate) model_name: &'a str,
    pub(crate) skill_metadata_provider: &'a dyn SkillMetadataProvider,
    pub(crate) augmentor_provider: Option<Arc<dyn Provider>>,
    pub(crate) observer: Arc<dyn Observer>,
}

/// Aggregated pre-answer enrichment result containing the enriched
/// message and all auxiliary signals for the turn.
pub(super) struct PreAnswerEnrichment {
    /// User message enriched with context, style, augmentations.
    pub(super) enriched_message: String,
    /// System-prompt-level addendum: self-contract + self-model shadow.
    pub(super) system_prompt_addendum: String,
    /// Loaded style profile, if persona mode is active.
    pub(super) style_profile: Option<StyleProfileState>,
    /// Self-model shadow for metacognitive calibration.
    pub(super) self_model_shadow: Option<SelfModelShadow>,
    /// Embodied-state snapshot for policy modulation.
    pub(super) embodied_state: Option<EmbodiedStateSnapshot>,
    /// Temperature delta from the affect/taste style overlay.
    pub(super) style_overlay_temperature_delta: f64,
}

/// Assemble all pre-answer enrichment: context recall, style
/// guidance, self-model shadow, augmentation blocks, and
/// narrative/relationship injection.
pub(super) async fn build_pre_answer_enrichment(
    config: &Config,
    security: &SecurityPolicy,
    mem: &dyn Memory,
    params: PreAnswerSharedParams<'_>,
    write_context: &RuntimeMemoryWriteContext,
    user_message: &str,
    ephemeral: bool,
) -> PreAnswerEnrichment {
    let enriched = if ephemeral {
        user_message.to_string()
    } else {
        build_enriched_message(
            mem,
            write_context,
            user_message,
            params.model_name,
            &config.workspace_dir,
            ephemeral,
        )
        .await
    };
    let enriched = inject_relevant_skills_block(
        config,
        security,
        params.skill_metadata_provider,
        user_message,
        enriched,
    );
    let enriched = enrich_user_message_with_link_context(&enriched, user_message).await;
    let enriched = enrich_user_message_with_response_baseline(&enriched, user_message);
    let enriched = enrich_user_message_with_decision_core(
        &enriched,
        user_message,
        &JudgmentCore::from_workspace(&config.workspace_dir),
    );
    let (style_profile, self_model_shadow, embodied_state) = tokio::join!(
        load_turn_style_profile(config, mem, params.person_id),
        load_turn_self_model_shadow(config, mem, params.person_id, user_message),
        load_turn_embodied_state(config, mem, params.person_id, ephemeral)
    );
    let enriched =
        enrich_user_message_with_style_guidance(&enriched, style_profile.as_ref(), user_message);

    let system_prompt_addendum =
        build_system_prompt_addendum(config, mem, params.person_id, self_model_shadow.as_ref())
            .await;

    let enriched = inject_persona_context_blocks(
        config,
        mem,
        write_context.entity_id.as_str(),
        params.person_id,
        user_message,
        enriched,
    )
    .await;

    let augmentor = augment::DefaultAugmentor::new(
        config.workspace_dir.clone(),
        config.persona.clone(),
        params.augmentor_provider.clone(),
        params.model_name.to_string(),
        Some(crate::core::tools::middleware::global_trust_tracker()),
        Some(params.observer.clone()),
    );
    let mut augmentations = augmentor
        .pre_answer(
            mem,
            write_context.entity_id.as_str(),
            params.person_id,
            user_message,
        )
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "turn augmentation pre-answer failed, using defaults");
            augment::TurnAugmentations::default()
        });
    if let Some(ref override_strategy) =
        reasoning_strategy_override_from_snapshot(&config.persona, embodied_state.as_ref())
    {
        let mut rb = String::with_capacity(14 + override_strategy.len() + 41);
        rb.push_str("[Reasoning: ");
        rb.push_str(override_strategy);
        rb.push_str("]\nVerify your reasoning before responding.\n");
        augmentations.reasoning_block = rb;
    }

    tracing::debug!(
        domain = ?augmentations.situation.domain,
        reasoning = ?augmentations.policy.reasoning,
        has_content = augmentations.has_content(),
        style_overlay_zero = augmentations.style_overlay.is_zero(),
        "pre-answer augmentations prepared"
    );

    let augmentation_budget =
        augment::cognitive_budget::AugmentationBudget::for_model(params.model_name);
    let enriched = augment::apply_augmentation_blocks_budgeted(
        &enriched,
        &augmentations,
        &augmentation_budget,
    );
    pre_answer_enrichment_result(
        enriched,
        system_prompt_addendum,
        style_profile,
        self_model_shadow,
        embodied_state,
        &augmentations,
    )
}

fn pre_answer_enrichment_result(
    enriched_message: String,
    system_prompt_addendum: String,
    style_profile: Option<StyleProfileState>,
    self_model_shadow: Option<SelfModelShadow>,
    embodied_state: Option<EmbodiedStateSnapshot>,
    augmentations: &augment::TurnAugmentations,
) -> PreAnswerEnrichment {
    let style_overlay_temperature_delta = augmentations.style_overlay.temperature_delta;
    PreAnswerEnrichment {
        enriched_message,
        system_prompt_addendum,
        style_profile,
        self_model_shadow,
        embodied_state,
        style_overlay_temperature_delta,
    }
}

/// Prepend a `[Relevant Skills]` hint block when skills are configured
/// and the metadata provider finds matches for `user_message`.
///
/// Only metadata (id, description, tags) is loaded at this stage —
/// full skill prompt bodies are not fetched to avoid inflating context.
/// Returns the original `enriched` string unchanged when `turn_hint_limit`
/// is zero or no skills match.
fn inject_relevant_skills_block(
    config: &Config,
    security: &SecurityPolicy,
    skill_metadata_provider: &dyn SkillMetadataProvider,
    user_message: &str,
    enriched: String,
) -> String {
    if config.skills.turn_hint_limit == 0 {
        return enriched;
    }

    let skill_snapshot = skill_metadata_provider
        .load_skill_metadata_snapshot_with_policy_and_config(
            &config.workspace_dir,
            security,
            &config.skills,
        );
    let block = skill_snapshot.render_relevant_block(
        user_message,
        config.skills.prompt_description_chars,
        config.skills.turn_hint_limit,
    );
    if block.is_empty() {
        enriched
    } else {
        block + &enriched
    }
}

/// Inject all persona-specific context blocks into the enriched message:
/// narrative self-description, relationship state, and pending
/// follow-up questions queued from the previous turn.
///
/// All three are prepended in order so the user message appears last,
/// closest to the inference call.
async fn inject_persona_context_blocks(
    config: &Config,
    mem: &dyn Memory,
    entity_id: &str,
    person_id: &str,
    user_message: &str,
    enriched: String,
) -> String {
    let enriched = inject_narrative_context_block(config, mem, person_id, enriched).await;
    let enriched = inject_relationship_context_block(config, mem, person_id, enriched).await;
    inject_pending_follow_ups_block(config, mem, entity_id, person_id, user_message, enriched).await
}

/// Prepend any pending follow-up questions to the enriched message
/// and clear them from memory so they are not repeated on future turns.
///
/// Follow-ups are enqueued by prior turns when the persona decides to ask
/// clarifying questions. They are consumed exactly once per turn.
async fn inject_pending_follow_ups_block(
    config: &Config,
    mem: &dyn Memory,
    entity_id: &str,
    person_id: &str,
    user_message: &str,
    enriched: String,
) -> String {
    if !config.persona.enabled_main_session {
        return enriched;
    }
    if !matches!(classify_response_mode(user_message), ResponseMode::Task) {
        return enriched;
    }

    let pending = load_pending_follow_ups(mem, entity_id).await;
    if pending.is_empty() {
        return enriched;
    }

    let block = render_follow_up_block(&pending);
    clear_follow_ups(mem, entity_id, person_id).await;
    block + &enriched
}

async fn inject_narrative_context_block(
    config: &Config,
    mem: &dyn Memory,
    person_id: &str,
    enriched: String,
) -> String {
    if !config.persona.enabled_main_session || !config.persona.enable_narrative_self {
        return enriched;
    }

    match crate::core::persona::narrative::load_narrative(mem, person_id).await {
        Ok(Some(narrative)) => {
            let block = render_narrative_block(&narrative);
            if block.is_empty() {
                enriched
            } else {
                block + &enriched
            }
        }
        Ok(None) => enriched,
        Err(error) => {
            tracing::warn!(%error, "narrative load failed; continuing without narrative block");
            enriched
        }
    }
}

async fn inject_relationship_context_block(
    config: &Config,
    mem: &dyn Memory,
    person_id: &str,
    enriched: String,
) -> String {
    if !config.persona.enabled_main_session {
        return enriched;
    }

    match crate::core::persona::relationship::load_relationship(mem, person_id).await {
        Ok(Some(rel_state)) => {
            let block = render_relationship_context_block(&rel_state);
            block + &enriched
        }
        _ => enriched,
    }
}

async fn enrich_user_message_with_link_context(enriched: &str, user_message: &str) -> String {
    let config = crate::utils::links::types::LinkConfig::default();
    if !config.enabled {
        return enriched.to_string();
    }

    let urls = crate::utils::links::detector::detect_urls(user_message);
    if urls.is_empty() {
        return enriched.to_string();
    }
    let enriched = {
        let mut block = String::with_capacity(128);
        block.push_str("[Detected Links]\n");
        for url in urls.into_iter().take(config.max_links_per_message) {
            block.push_str("- ");
            block.push_str(url.as_str());
            block.push('\n');
        }
        block.push_str(enriched);
        block
    };

    #[cfg(feature = "link-extraction")]
    {
        if std::env::var("ASTEREL_ENABLE_LINK_FETCH")
            .ok()
            .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        {
            return crate::utils::links::extractor::enrich_message_with_links(&enriched, &config)
                .await;
        }
    }

    enriched
}

/// Prepend `context` to `user_message`, or return `user_message`
/// unchanged when `context` is empty.  The raw concatenation is
/// intentional — callers are responsible for formatting each block
/// with trailing newlines before passing it as `context`.
fn enrich_user_message(context: &str, user_message: &str) -> String {
    if context.is_empty() {
        user_message.to_string()
    } else {
        let mut out = String::with_capacity(context.len() + user_message.len());
        out.push_str(context);
        out.push_str(user_message);
        out
    }
}

fn enrich_user_message_with_style_guidance(
    enriched_user_message: &str,
    style_profile: Option<&StyleProfileState>,
    user_message: &str,
) -> String {
    if classify_response_mode(user_message) == ResponseMode::Conversation {
        return enriched_user_message.to_string();
    }

    match style_profile {
        Some(profile) => inject_block_before_live_user_message(
            enriched_user_message,
            user_message,
            &render_style_guidance(profile),
        ),
        None => enriched_user_message.to_string(),
    }
}

fn enrich_user_message_with_response_baseline(
    enriched_user_message: &str,
    user_message: &str,
) -> String {
    inject_block_before_live_user_message(
        enriched_user_message,
        user_message,
        &render_response_style_block(user_message),
    )
}

fn enrich_user_message_with_decision_core(
    enriched_user_message: &str,
    user_message: &str,
    judgment_core: &JudgmentCore,
) -> String {
    inject_block_before_live_user_message(
        enriched_user_message,
        user_message,
        &render_judgment_core_turn_block(judgment_core, user_message),
    )
}

/// Insert `block` immediately before the live `user_message` portion
/// of the already-enriched string.
///
/// When `enriched_user_message` ends with the original `user_message`,
/// the block is spliced in between the prefix context and the raw
/// user text.  This keeps all injected guidance adjacent to the
/// message it annotates rather than at the very top of the prompt.
/// Falls back to simple prepend when the suffix check fails.
fn inject_block_before_live_user_message(
    enriched_user_message: &str,
    user_message: &str,
    block: &str,
) -> String {
    if user_message.is_empty() || !enriched_user_message.ends_with(user_message) {
        let mut out = String::with_capacity(block.len() + enriched_user_message.len());
        out.push_str(block);
        out.push_str(enriched_user_message);
        return out;
    }

    let split_at = enriched_user_message.len() - user_message.len();
    let (prefix, suffix) = enriched_user_message.split_at(split_at);
    let mut out = String::with_capacity(prefix.len() + block.len() + suffix.len());
    out.push_str(prefix);
    out.push_str(block);
    out.push_str(suffix);
    out
}

/// Build the system-prompt addendum that is appended to the base
/// system prompt for every main-session turn.
///
/// Contains up to two blocks (in order):
/// 1. **Self-contract** — the persona's core commitments and identity
///    principles, rendered as a prompt directive.
/// 2. **Self-model shadow** — a summary of metacognitive calibration:
///    predicted success rate, continuity score, and coherence level.
///
/// Both are conditional on their respective feature flags.  An empty
/// string is returned when neither is enabled.
async fn build_system_prompt_addendum(
    config: &Config,
    mem: &dyn Memory,
    person_id: &str,
    self_model_shadow: Option<&SelfModelShadow>,
) -> String {
    let mut out = String::new();

    if config.persona.enabled_main_session && config.persona.enable_self_contract {
        let contract = build_prompt_self_contract(mem, person_id, Some(&config.persona)).await;
        out.push_str(&render_self_contract_block(&contract));
    }

    if let Some(model) = self_model_shadow {
        out.push_str(&render_self_model_shadow_block(model));
    }

    out
}

/// Perform memory recall and render the context contract into the
/// enriched message string.
///
/// The context budget is sized to the model's context window via
/// `context_budget_for_model`, preventing over-recall from overflowing
/// the provider's token limit.  On any recall failure the function
/// falls back to the raw `user_message` with a warning rather than
/// aborting the turn.
async fn build_enriched_message(
    mem: &dyn Memory,
    write_context: &RuntimeMemoryWriteContext,
    user_message: &str,
    model_name: &str,
    workspace_dir: &std::path::Path,
    ephemeral: bool,
) -> String {
    let runtime_metadata = ContextRuntimeMetadata::from_entity_scope(
        write_context.entity_id.as_str(),
        &write_context.policy_context,
    )
    .with_model_name(model_name)
    .with_workspace_dir(workspace_dir)
    .with_ephemeral(ephemeral);
    let context_contract = match build_context_contract_with_runtime_metadata(
        mem,
        write_context.entity_id.as_str(),
        user_message,
        write_context.policy_context.clone(),
        context_budget_for_model(model_name),
        Some(&runtime_metadata),
    )
    .await
    {
        Ok(ctx) => ctx,
        Err(error) => {
            tracing::warn!(%error, "memory recall failed; proceeding with empty context");
            return user_message.to_string();
        }
    };
    if context_contract.has_sanitized_untrusted_content() {
        tracing::debug!(
            fragment_count = context_contract.fragments.len(),
            "pre-answer context includes sanitized untrusted fragments"
        );
    }
    enrich_user_message(&context_contract.render(), user_message)
}

/// Load the persona's learned style profile for this turn.
///
/// Returns `None` (disabling style adaptation) when persona mode is
/// off or when the load fails — the turn proceeds without style
/// guidance rather than hard-failing.
async fn load_turn_style_profile(
    config: &Config,
    mem: &dyn Memory,
    person_id: &str,
) -> Option<StyleProfileState> {
    if !config.persona.enabled_main_session {
        return None;
    }

    match load_style_profile(mem, person_id).await {
        Ok(Some(profile)) => Some(profile),
        Ok(None) if config.persona.enable_character_config => Some(
            StyleProfileState::from_character_config(&config.persona.character, "config-seed"),
        ),
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "style profile load failed; continuing without style adaptation"
            );
            config.persona.enable_character_config.then(|| {
                StyleProfileState::from_character_config(&config.persona.character, "config-seed")
            })
        }
    }
}

/// Load the embodied-state snapshot for temperature/top-p/max-tokens
/// modulation this turn.
///
/// Skipped in ephemeral mode (single-shot `-m` turns) and when the
/// feature flag `enable_embodied_state_policy_modulation` is off.
/// Load failures are logged and treated as absent rather than fatal.
async fn load_turn_embodied_state(
    config: &Config,
    mem: &dyn Memory,
    person_id: &str,
    ephemeral: bool,
) -> Option<EmbodiedStateSnapshot> {
    if ephemeral
        || !config.persona.enabled_main_session
        || !config.persona.enable_embodied_state_policy_modulation
    {
        return None;
    }

    match load_embodied_state_snapshot(mem, person_id).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "embodied-state load failed; continuing without policy modulation"
            );
            None
        }
    }
}

/// Build the self-model shadow for this turn.
///
/// The shadow is a lightweight metacognitive snapshot — predicted
/// success rate, continuity score, coherence level — derived from
/// past calibration data.  It is injected into the system prompt
/// addendum and used by the metacognitive logging handler to compare
/// prediction vs. observed outcome.
///
/// Returns `None` when the feature is disabled or generation fails.
async fn load_turn_self_model_shadow(
    config: &Config,
    mem: &dyn Memory,
    person_id: &str,
    user_message: &str,
) -> Option<SelfModelShadow> {
    if !config.persona.enabled_main_session || !config.persona.enable_self_model_shadow {
        return None;
    }

    match build_self_model_shadow(mem, person_id, user_message).await {
        Ok(shadow) => Some(shadow),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "self model shadow generation failed; continuing without self model block"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::inject_relevant_skills_block;
    use crate::config::{Config, SkillsRuntimeConfig};
    use crate::core::subagents::NoopSkillMetadataProvider;
    use crate::security::SecurityPolicy;

    #[test]
    fn inject_relevant_skills_block_skips_loading_when_turn_hints_disabled() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let skill_dir = workspace.path().join("skills").join("rust-review");
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        std::fs::write(
            skill_dir.join("extension.toml"),
            r#"
[extension]
id = "rust-review"
kind = "skill"
description = "Review Rust crates and investigate failing cargo test runs"
tags = ["rust", "review", "cargo"]

[skill]
prompt_bodies = ["SKILL.md"]
"#,
        )
        .expect("write skill manifest");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "# Rust Review\nFocus on cargo failures.\n",
        )
        .expect("write skill prompt");

        let config = Config {
            workspace_dir: workspace.path().to_path_buf(),
            skills: SkillsRuntimeConfig {
                turn_hint_limit: 0,
                ..SkillsRuntimeConfig::default()
            },
            ..Config::default()
        };

        let original = "review failing Rust tests".to_string();
        let skill_metadata_provider = NoopSkillMetadataProvider::new();
        let enriched = inject_relevant_skills_block(
            &config,
            &SecurityPolicy::default(),
            &skill_metadata_provider,
            &original,
            original.clone(),
        );

        assert_eq!(enriched, original);
    }

    #[test]
    fn inject_relevant_skills_block_uses_metadata_only_loading() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let skill_dir = workspace.path().join("skills").join("rust-review");
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        std::fs::write(
            skill_dir.join("extension.toml"),
            r#"
[extension]
id = "rust-review"
kind = "skill"
description = "Review Rust crates and investigate failing cargo test runs"
tags = ["rust", "review", "cargo"]

[skill]
prompt_bodies = ["SKILL.md"]
"#,
        )
        .expect("write skill manifest");
        std::fs::write(skill_dir.join("SKILL.md"), [0xff_u8, 0xfe_u8])
            .expect("write invalid utf8 prompt body");

        let config = Config {
            workspace_dir: workspace.path().to_path_buf(),
            skills: SkillsRuntimeConfig {
                turn_hint_limit: 2,
                ..SkillsRuntimeConfig::default()
            },
            ..Config::default()
        };

        let original = "review failing Rust tests".to_string();
        let skill_metadata_provider = crate::runtime::services::runtime_skill_metadata_provider();
        let enriched = inject_relevant_skills_block(
            &config,
            &SecurityPolicy::default(),
            skill_metadata_provider.as_ref(),
            &original,
            original.clone(),
        );

        assert!(enriched.starts_with("[Relevant Skills]"));
        assert!(enriched.contains("rust-review"));
        assert!(enriched.ends_with(&original));
    }
}
