//! Pre- and post-turn enrichment hooks.
//!
//! **Pre-turn** (`enrich_pre_turn`): builds the enriched system prompt for
//! each turn by layering affect detection, persona context (relationship
//! state, user profile, recall, tone guidance), and response style blocks
//! on top of the base prompt. Also adjusts the sampling temperature via the
//! affect-to-style delta.
//!
//! **Post-turn** (`run_post_turn_hooks`): fires after the tool loop completes.
//! Updates the relationship model and saves compact user/assistant turn summaries
//! to working memory when the turn contract allows those writeback slots. Working
//! memory views accumulated during execution are flushed separately via
//! `flush_working_memory`. These hooks run in a background task and do not block
//! the caller.

use std::{collections::HashSet, path::Path};

use num_traits::ToPrimitive;

use crate::config::PersonaConfig;
use crate::contracts::ids::SessionId;
use crate::core::affect::appraisal::{appraise_event, topic_is_personal_text_cue};
use crate::core::affect::topology::{
    TopologyGraph, TopologySnapshot, activate_from_appraisal, apply_latent_bias, build_snapshot,
    diffuse_on_topology,
};
use crate::core::affect::{
    AffectLabel, AffectReading, RuleBasedDetector, SessionMood, affect_to_style_delta,
    render_session_mood_block, render_topology_block,
};
use crate::core::agent::presenter::render_tone_guidance;
use crate::core::agent::response_audit::ExposurePlanContract;
use crate::core::agent::response_style::{
    render_judgment_core_turn_block, render_response_style_block,
};
use crate::core::agent::turn_contract::CompanionTurnContract;
use crate::core::memory::influence::build_companion_grounding_augmentation_with_privacy;
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryRecallEntry, MemorySource, PrivacyLevel,
    RecallQuery, WorkingMemoryView,
};
use crate::core::persona::judgment_core::JudgmentCore;
use crate::core::persona::person_identity::person_entity_id;
use crate::core::persona::presenter::{
    render_behavior_selection_block, render_relationship_context_block, render_self_contract_block,
    render_style_guidance, render_user_model_block, render_user_profile_block,
};
use crate::core::persona::relationship::load_relationship_for_entity;
use crate::core::persona::self_contract::build_prompt_self_contract;
use crate::core::persona::soul_core::{
    SoulIdentityCues, SoulPressureInput, SoulRecallExposure, SoulSurfaceExposure, SoulTopologyCues,
    derive_soul_pressure, derive_soul_pressure_with_topology, render_soul_pressure_block,
};
use crate::core::persona::style_profile::{StyleProfileState, load_style_profile};
use crate::core::persona::user_facts::load_user_profile_for_entity;
use crate::core::persona::user_model::infer_user_model;
use crate::security::policy::TenantPolicyContext;
use crate::utils::text::truncate_ellipsis;

mod contract;
mod post_turn;
mod working_memory;

pub use contract::{compile_turn_contract, render_system_prompt_from_contract};
pub use post_turn::{PostTurnInput, run_post_turn_hooks};
pub use working_memory::{flush_working_memory, materialize_working_memory};

const DEFAULT_RECALL_LIMIT: usize = 12;
const DEFAULT_RECALL_MIN_CONFIDENCE: f64 = 0.3;
#[cfg(test)]
const RECALL_VALUE_MAX_CHARS: usize = 240;

/// All inputs required to enrich a system prompt before a turn.
pub struct PreTurnInput<'a> {
    pub mem: &'a dyn Memory,
    pub workspace_dir: &'a Path,
    pub base_prompt: &'a str,
    pub user_message: &'a str,
    pub entity_id: &'a str,
    pub person_id: &'a str,
    pub base_temperature: f64,
    pub policy_context: &'a TenantPolicyContext,
    pub recall_min_confidence: Option<f64>,
    pub persona_config: Option<&'a PersonaConfig>,
    /// Optional session manager for storing/retrieving thin companion session state.
    pub session_manager: Option<&'a crate::core::sessions::SessionOrchestrator>,
    /// Surface name used to resolve the current session when session state is enabled.
    pub session_surface: Option<&'a str>,
    /// Whether this turn is addressed directly to the companion rather than ambient room context.
    pub is_direct_address: bool,
    /// Owner scope used to resolve the current session when session state is enabled.
    pub session_owner_scope: Option<&'a str>,
    /// Canonical session id when the ingress surface already resolved one.
    pub session_id: Option<&'a str>,
    /// Pre-rendered policy section (heading + content blocks).
    /// Built by the caller via `assemble_policy_blocks` + `render_policy_section`.
    /// Pass `""` if no policy blocks are needed for this turn.
    pub policy_section: &'a str,
    /// Surface exposure plan selected by the companion policy owner.
    pub exposure_plan: Option<ExposurePlanContract>,
    /// Optional materialized working-memory view for the current session/turn.
    pub working_memory: Option<&'a WorkingMemoryView>,
}

/// Result of pre-turn enrichment: the assembled system prompt, adjusted
/// temperature, and the detected affect reading for this turn.
pub struct PreTurnEnrichment {
    /// Structured contract compiled for this turn.
    pub contract: CompanionTurnContract,
    /// System prompt with all enrichment blocks injected.
    pub system_prompt: String,
    /// Sampling temperature after applying the affect-to-style delta.
    pub temperature: f64,
    /// Affect reading used to drive tone guidance and temperature adjustment.
    pub affect: AffectReading,
}

struct PersonaContextInput<'a> {
    mem: &'a dyn Memory,
    entity_id: &'a str,
    person_id: &'a str,
    user_message: &'a str,
    affect: &'a AffectReading,
    policy_context: &'a TenantPolicyContext,
    recall_min_confidence: Option<f64>,
    persona_config: Option<&'a PersonaConfig>,
    session_manager: Option<&'a crate::core::sessions::SessionOrchestrator>,
    session_surface: Option<&'a str>,
    is_direct_address: bool,
    session_owner_scope: Option<&'a str>,
    session_id: Option<&'a str>,
    working_memory: Option<&'a WorkingMemoryView>,
    exposure_plan: Option<ExposurePlanContract>,
}

mod turn_enrichment_io;
mod turn_enrichment_pipeline;

pub use turn_enrichment_pipeline::{affect_intensity, enrich_pre_turn};

#[cfg(test)]
mod tests;
