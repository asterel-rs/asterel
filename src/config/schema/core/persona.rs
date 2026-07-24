//! Persona subsystem knobs: self-model shadow, metacognitive logging,
//! calibration gates, drift detection, experience distillation,
//! curiosity drive, narrative self, and Big Five traits.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Persona subsystem configuration: self-model, metacognition,
/// calibration, drift detection, and narrative self knobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct PersonaConfig {
    /// Enable persona in main CLI session. Default: true.
    #[serde(default = "default_persona_enabled_main_session")]
    pub enabled_main_session: bool,
    /// Enable post-completion response finalization for user-facing text. Default: true.
    #[serde(default = "default_persona_enable_response_finalization")]
    pub enable_response_finalization: bool,
    /// Enable Rust-native pre-send naturalness gate. Default: false.
    #[serde(default)]
    pub enable_naturalness_gate: bool,
    /// Enable self-model shadow tracking. Default: true.
    #[serde(default = "default_persona_enable_self_model_shadow")]
    pub enable_self_model_shadow: bool,
    /// Inject self-contract identity block into every turn's system prompt. Default: true.
    #[serde(default = "default_persona_enable_self_contract")]
    pub enable_self_contract: bool,
    /// Enable metacognitive logging. Default: true.
    #[serde(default = "default_persona_enable_metacognitive_logging")]
    pub enable_metacognitive_logging: bool,
    /// Enable calibration gate. Default: true.
    #[serde(default = "default_persona_enable_calibration_gate")]
    pub enable_calibration_gate: bool,
    /// Enable continuity gate. Default: true.
    #[serde(default = "default_persona_enable_continuity_gate")]
    pub enable_continuity_gate: bool,
    /// Enable rollback drill exercises. Default: true.
    #[serde(default = "default_persona_enable_rollback_drills")]
    pub enable_rollback_drills: bool,
    /// Enable embodied state policy modulation. Default: false.
    #[serde(default = "default_persona_enable_embodied_state_policy_modulation")]
    pub enable_embodied_state_policy_modulation: bool,
    /// Max temperature delta from embodied state. Default: 0.10.
    #[serde(default = "default_persona_embodied_temperature_delta_max")]
    pub embodied_temperature_delta_max: f64,
    /// Sliding window size for calibration gate. Default: 64.
    #[serde(default = "default_persona_calibration_gate_window_size")]
    pub calibration_gate_window_size: usize,
    /// Minimum samples before calibration gate triggers. Default: 12.
    #[serde(default = "default_persona_calibration_gate_min_samples")]
    pub calibration_gate_min_samples: usize,
    /// Max mean error for calibration gate pass. Default: 0.35.
    #[serde(default = "default_persona_calibration_gate_mean_error_max")]
    pub calibration_gate_mean_error_max: f64,
    /// Max p95 error for calibration gate pass. Default: 0.60.
    #[serde(default = "default_persona_calibration_gate_p95_error_max")]
    pub calibration_gate_p95_error_max: f64,
    /// Record state transitions for audit. Default: true.
    #[serde(default = "default_persona_enable_state_transition_records")]
    pub enable_state_transition_records: bool,
    /// Enable persona drift detection loop. Default: true.
    #[serde(default = "default_persona_enable_drift_detection_loop")]
    pub enable_drift_detection_loop: bool,
    /// Drift warning threshold (cosine similarity). Default: 0.70.
    #[serde(default = "default_persona_drift_warning_threshold")]
    pub drift_warning_threshold: f64,
    /// Drift critical threshold (cosine similarity). Default: 0.45.
    #[serde(default = "default_persona_drift_critical_threshold")]
    pub drift_critical_threshold: f64,
    /// Filename for the state mirror document. Default: `"STATE.md"`.
    #[serde(default = "default_persona_state_mirror_file")]
    pub state_mirror_filename: String,
    /// Maximum open loops tracked simultaneously. Default: 7.
    #[serde(default = "default_persona_max_open_loops")]
    pub max_open_loops: usize,
    /// Maximum next actions in state mirror. Default: 3.
    #[serde(default = "default_persona_max_next_actions")]
    pub max_next_actions: usize,
    /// Maximum commitments tracked. Default: 5.
    #[serde(default = "default_persona_max_commitments")]
    pub max_commitments: usize,
    /// Max chars for current objective field. Default: 280.
    #[serde(default = "default_persona_max_current_objective_chars")]
    pub max_current_objective_chars: usize,
    /// Max chars for recent context summary. Default: 1200.
    #[serde(default = "default_persona_max_recent_context_summary_chars")]
    pub max_recent_context_summary_chars: usize,
    /// Max chars per list item in state mirror. Default: 240.
    #[serde(default = "default_persona_max_list_item_chars")]
    pub max_list_item_chars: usize,
    /// Enable experience distillation. Default: false.
    #[serde(default)]
    pub enable_experience_distillation: bool,
    /// Distillation interval in turns. Default: 20.
    #[serde(default = "default_persona_distillation_interval_turns")]
    pub distillation_interval_turns: usize,
    /// Enable curiosity drive. Default: false.
    #[serde(default)]
    pub enable_curiosity_drive: bool,
    /// Curiosity threshold for triggering signals. Default: 0.6.
    #[serde(default = "default_persona_curiosity_threshold")]
    pub curiosity_threshold: f64,
    /// Enable narrative self-construction. Default: true.
    #[serde(default = "default_persona_enable_narrative_self")]
    pub enable_narrative_self: bool,
    /// Narrative rebuild interval in turns. Default: 25.
    #[serde(default = "default_persona_narrative_rebuild_interval_turns")]
    pub narrative_rebuild_interval_turns: usize,
    /// Enable counterfactual reasoning on low-success turns. Default: false.
    #[serde(default)]
    pub enable_counterfactual_reasoning: bool,
    /// Enable Big Five (OCEAN) personality trait system. Default: false.
    #[serde(default)]
    pub enable_big_five: bool,
    /// Enable affect decay and session mood. Default: false.
    #[serde(default)]
    pub enable_affect_decay: bool,
    /// Enable affect topology diffusion and latent bias transforms. Default: false.
    #[serde(default)]
    pub enable_affect_topology: bool,
    /// Enable read-only soul pressure posture derivation. Default: false.
    #[serde(default)]
    pub enable_soul_pressure: bool,
    /// Enable dry-run self-amendment candidate generation. Default: false.
    #[serde(default)]
    pub enable_self_amendment_candidates: bool,
    /// Enable session control state (conversational mode/density/avoidance). Default: false.
    #[serde(default)]
    pub enable_session_control_state: bool,
    /// Enable detailed character configuration (`CharacterConfig`). Default: false.
    #[serde(default)]
    pub enable_character_config: bool,
    /// Enable unified behavior selector. Default: false.
    #[serde(default)]
    pub enable_behavior_selector: bool,
    /// Enable weighted trait activation inside the behavior selector. Default: false.
    #[serde(default)]
    pub enable_trait_activation: bool,
    /// Enable emotional memory consolidation into long-term identity memory. Default: false.
    #[serde(default)]
    pub enable_affect_consolidation: bool,
    /// Big Five and affect parameters for this character persona. Default: see `CharacterConfig`.
    #[serde(default)]
    pub character: CharacterConfig,
    /// Companion-mode behavioral policy (AI identity disclosure, proactivity). Default: see `CompanionBehaviorConfig`.
    #[serde(default)]
    pub companion: CompanionBehaviorConfig,
    /// Enable LLM-based affect detection instead of rule-based. Default: false.
    #[serde(default)]
    pub enable_llm_affect: bool,
    /// Enable LLM-based user mental model inference. Default: false.
    #[serde(default)]
    pub enable_llm_user_model: bool,
    /// Timeout budget for LLM-based user mental model inference in seconds. Default: 8.
    #[serde(default = "default_persona_llm_user_model_timeout_secs")]
    pub llm_user_model_timeout_secs: u64,
}

/// Stable identity seeds for the companion's long-lived disposition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterIdentityConfig {
    #[serde(default = "default_character_soul_root_sentence")]
    pub soul_root_sentence: String,
    #[serde(default = "default_character_extraversion")]
    pub extraversion: f64,
    #[serde(default = "default_character_agreeableness")]
    pub agreeableness: f64,
    #[serde(default = "default_character_conscientiousness")]
    pub conscientiousness: f64,
    #[serde(default = "default_character_neuroticism")]
    pub neuroticism: f64,
    #[serde(default = "default_character_openness")]
    pub openness: f64,
    #[serde(default = "default_character_desires")]
    pub desires: Vec<String>,
    #[serde(default = "default_character_fears")]
    pub fears: Vec<String>,
    #[serde(default = "default_character_values")]
    pub values: Vec<String>,
    #[serde(default = "default_character_negative_identity")]
    pub negative_identity: Vec<String>,
}

/// Surface defaults used to seed style profile state, not personality drift.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterStyleDefaultsConfig {
    #[serde(default = "default_character_formality")]
    pub formality: u8,
    #[serde(default = "default_character_verbosity")]
    pub verbosity: u8,
    #[serde(default = "default_character_temperature")]
    pub temperature: f64,
}

/// Immutable personality, style defaults, and affect parameters for this character.
///
/// Identity seeds and surface defaults are intentionally separated so long-term
/// disposition does not drift through short-horizon style adjustments.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterConfig {
    pub identity: CharacterIdentityConfig,
    pub style_defaults: CharacterStyleDefaultsConfig,

    /// Parameters governing emotion-to-mood decay and session consolidation.
    #[serde(default)]
    pub affect_decay: AffectDecayConfig,

    /// Score thresholds that define surface, emerging, and deepening relationship bands.
    #[serde(default)]
    pub relationship_tiers: RelationshipTierConfig,

    /// Context-driven OCEAN multiplier rules (e.g. emotional suppresses extraversion).
    #[serde(default)]
    pub trait_activation: TraitActivationConfig,

    /// Directed affective graph and latent bias profile for this character.
    #[serde(default)]
    pub affect_topology: AffectTopologyConfig,
}

impl CharacterConfig {
    #[must_use]
    pub fn big_five_seeds(&self) -> &CharacterIdentityConfig {
        &self.identity
    }

    #[must_use]
    pub fn style_defaults(&self) -> &CharacterStyleDefaultsConfig {
        &self.style_defaults
    }

    /// Stable hash of the full character definition, including topology and latent bias.
    #[must_use]
    pub fn definition_hash(&self) -> String {
        stable_definition_hash("character-definition", self)
    }

    /// Stable hash of the latent bias profile that contributes to the full definition hash.
    #[must_use]
    pub fn latent_bias_profile_hash(&self) -> String {
        stable_definition_hash("latent-bias", &self.affect_topology.latent_bias)
    }

    /// # Errors
    ///
    /// Returns an error when immutable character topology references nodes that
    /// are not declared in `affect_topology.node_set`.
    pub fn validate(&self) -> anyhow::Result<()> {
        self.affect_topology.validate()
    }
}

impl PersonaConfig {
    /// # Errors
    ///
    /// Returns an error when nested character configuration is invalid.
    pub fn validate(&self) -> anyhow::Result<()> {
        self.character.validate()
    }
}

impl AffectTopologyConfig {
    /// # Errors
    ///
    /// Returns an error when an edge endpoint is absent from `node_set` or when
    /// an edge weight is non-finite/out of range.
    pub fn validate(&self) -> anyhow::Result<()> {
        let nodes: HashSet<&str> = self.node_set.iter().map(|node| node.0.as_str()).collect();
        for edge in &self.edges {
            if !nodes.contains(edge.from.0.as_str()) {
                anyhow::bail!(
                    "affect_topology edge references unknown from node '{}'",
                    edge.from.0
                );
            }
            if !nodes.contains(edge.to.0.as_str()) {
                anyhow::bail!(
                    "affect_topology edge references unknown to node '{}'",
                    edge.to.0
                );
            }
            if !edge.weight.is_finite() || !(0.0..=1.0).contains(&edge.weight) {
                anyhow::bail!(
                    "affect_topology edge weight must be finite and in [0.0, 1.0], got {}",
                    edge.weight
                );
            }
        }
        Ok(())
    }
}

fn stable_definition_hash<T: Serialize>(prefix: &str, value: &T) -> String {
    let payload = serde_json::to_vec(value)
        .unwrap_or_else(|error| format!("serialization-error:{error}").into_bytes());
    let digest = Sha256::digest(&payload);
    format!("{prefix}:{}", hex::encode(&digest[..8]))
}

/// Parameters governing the emotion→mood decay loop and multi-session
/// mood consolidation for the Big Five affect system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectDecayConfig {
    /// EMA alpha that blends each new emotion signal into the running mood.
    /// Higher = emotion changes mood faster. Default: 0.08.
    #[serde(default = "default_affect_alpha")]
    pub alpha_emotion_to_mood: f64,
    /// Beta term pulling valence mood back toward neutral (0.5) each turn.
    /// Higher = faster homeostatic recovery. Default: 0.15.
    #[serde(default = "default_affect_beta")]
    pub beta_homeostatic_pull: f64,
    /// Beta term pulling arousal back toward a neutral baseline each turn.
    /// Default: 0.25.
    #[serde(default = "default_affect_beta_arousal")]
    pub beta_arousal_homeostatic: f64,
    /// Fraction of mood state reset toward baseline across a session boundary.
    /// 0.80 means mood = 0.2*mood + 0.8*baseline. Default: 0.80.
    #[serde(default = "default_affect_session_reset")]
    pub session_boundary_reset_factor: f64,
    /// Inactivity gap, in minutes, after which a new turn starts a fresh session.
    #[serde(default = "default_affect_session_boundary_inactivity_minutes")]
    pub session_boundary_inactivity_minutes: u64,
    /// Per-emotion half-life rates used during decay ticks. See `EmotionDecayRates`.
    #[serde(default)]
    pub emotion_rates: EmotionDecayRates,
    /// Minimum cross-session consistency score required before a mood trait is
    /// consolidated into the long-term baseline. Default: 0.70.
    #[serde(default = "default_consolidation_consistency")]
    pub consolidation_consistency_threshold: f64,
    /// Maximum single-session shift allowed when updating the consolidated
    /// baseline to guard against runaway drift. Default: 0.15.
    #[serde(default = "default_consolidation_max_shift")]
    pub consolidation_max_single_session_shift: f64,
    /// Shannon entropy threshold above which low-signal mood states are pruned
    /// from the history before consolidation. Default: 1.4.
    #[serde(default = "default_consolidation_entropy")]
    pub consolidation_entropy_prune_threshold: f64,
    /// Minimum number of sessions observed before a mood state is eligible for
    /// promotion into the long-term baseline. Default: 3.
    #[serde(default = "default_consolidation_min_sessions")]
    pub consolidation_min_sessions_for_promotion: u32,
}

/// Per-emotion half-life decay rates applied each turn by the affect decay loop.
///
/// Each value is a fractional decay coefficient: the emotion signal is multiplied
/// by `(1.0 - rate)` per tick, so higher = faster fade. Values must be in [0.0, 1.0).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmotionDecayRates {
    /// Decay rate for fear. High default (0.30) — fear should clear quickly absent new threat.
    #[serde(default = "default_decay_fear")]
    pub fear: f64,
    /// Decay rate for anger. Moderate-high (0.20) — anger fades faster than sadness.
    #[serde(default = "default_decay_anger")]
    pub anger: f64,
    /// Decay rate for curiosity. Low (0.12) — curiosity lingers to sustain engagement.
    #[serde(default = "default_decay_curiosity")]
    pub curiosity: f64,
    /// Decay rate for joy. Low-moderate (0.15) — positive affect has a longer half-life.
    #[serde(default = "default_decay_joy")]
    pub joy: f64,
    /// Decay rate for sadness. Very low (0.08) — grief and loss fade slowly.
    #[serde(default = "default_decay_sadness")]
    pub sadness: f64,
    /// Decay rate for frustration. Moderate (0.18) — situational friction clears at medium pace.
    #[serde(default = "default_decay_frustration")]
    pub frustration: f64,
    /// Decay rate for gratitude. Low (0.10) — appreciation is meant to persist.
    #[serde(default = "default_decay_gratitude")]
    pub gratitude: f64,
    /// Decay rate for anxiety. Low-moderate (0.14) — background worry dissipates slowly.
    #[serde(default = "default_decay_anxiety")]
    pub anxiety: f64,
}

/// Score thresholds that partition the relationship score axis into named bands.
///
/// The relationship score lives in [0.0, 1.0]. This config defines the upper
/// boundary of three intermediate bands; scores above `deepening_max` are
/// considered "established". Used by the affect and companion systems to
/// modulate tone and proactivity based on familiarity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_field_names)]
pub struct RelationshipTierConfig {
    /// Upper bound of the "surface" tier (strangers / first contact). Default: 0.30.
    #[serde(default = "default_tier_surface")]
    pub surface_max: f64,
    /// Upper bound of the "emerging" tier (acquaintance-level familiarity). Default: 0.50.
    #[serde(default = "default_tier_emerging")]
    pub emerging_max: f64,
    /// Upper bound of the "deepening" tier (trusted collaborator). Default: 0.70.
    /// Scores above this threshold enter the "established" band.
    #[serde(default = "default_tier_deepening")]
    pub deepening_max: f64,
}

/// Context-driven OCEAN multiplier rules that temporarily shift trait scores
/// in response to conversation state (emotional, crisis, intellectual, conflict).
///
/// Each value is a multiplier applied to the corresponding OCEAN dimension when
/// the triggering context is detected. Values > 1.0 boost the trait; values < 1.0
/// suppress it. Multipliers are transient — they do not alter `CharacterConfig`
/// baseline scores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraitActivationConfig {
    /// Agreeableness multiplier during emotionally charged exchanges. Default: 1.4.
    /// Pulls the character toward warmth and support when the user signals distress.
    #[serde(default = "default_ta_emotional_a_boost")]
    pub emotional_agreeableness_boost: f64,
    /// Extraversion suppression multiplier during crisis/emergency contexts. Default: 0.6.
    /// Quiets assertiveness so the character can listen and hold space.
    #[serde(default = "default_ta_crisis_e_suppress")]
    pub crisis_extraversion_suppression: f64,
    /// Openness multiplier during intellectual exploration exchanges. Default: 1.3.
    /// Amplifies curiosity and creative speculation when the user engages analytically.
    #[serde(default = "default_ta_intellectual_o_boost")]
    pub intellectual_openness_boost: f64,
    /// Agreeableness floor during conflict: score below this threshold means the
    /// character will not soften its stance further. Default: 0.50. Prevents the
    /// character from becoming a yes-machine under social pressure.
    #[serde(default = "default_ta_conflict_a_threshold")]
    pub conflict_agreeableness_threshold: f64,
}

/// Character-specific affect topology: a directed weighted graph of affective
/// nodes and a latent bias profile that bends activation before expression.
///
/// Part of Layer 1 (Core Identity) — immutable per character definition.
/// The topology defines which affects sit near each other for this character,
/// enabling activation to spread along character-specific routes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectTopologyConfig {
    /// Affect nodes in this character's topology (12-16 recommended for Phase 1).
    #[serde(default = "default_topology_node_set")]
    pub node_set: Vec<crate::contracts::affect::AffectNodeId>,
    /// Directed weighted edges between nodes. Weight in [0.0, 1.0].
    #[serde(default)]
    pub edges: Vec<AffectEdge>,
    /// Latent bias profile: transform classes that bend activation before speech.
    #[serde(default)]
    pub latent_bias: LatentBiasProfile,
}

/// A single directed edge in the character's affect topology graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectEdge {
    /// Source affect node — the node whose activation propagates along this edge.
    pub from: crate::contracts::affect::AffectNodeId,
    /// Target affect node — the node that receives the propagated activation.
    pub to: crate::contracts::affect::AffectNodeId,
    /// Connection strength in [0.0, 1.0]. Higher = activation spreads more easily.
    #[serde(default = "default_edge_weight")]
    pub weight: f32,
}

/// Latent bias dimensions that suppress, amplify, or reroute activation before
/// it reaches speech. These are transform classes, not character backstory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatentBiasProfile {
    /// Tendency to seek external validation. Amplifies social-validation appraisals.
    #[serde(default)]
    pub approval_hunger: f32,
    /// Fear of disconnection. Pulls achievement toward anxiety when attachment is salient.
    #[serde(default)]
    pub abandonment_fear: f32,
    /// Sensitivity to shame. Can suppress pride and redirect to self-doubt.
    #[serde(default = "default_bias_shame_sensitivity")]
    pub shame_sensitivity: f32,
    /// Avoidance of stating feelings directly. Converts dominant affect to tone/pacing.
    #[serde(default)]
    pub direct_expression_avoidance: f32,
    /// Tendency to deflect with irony. Shortens the path from intense affect to ironic surface.
    #[serde(default)]
    pub ironic_deflection: f32,
}

impl Default for AffectTopologyConfig {
    fn default() -> Self {
        Self {
            node_set: default_topology_node_set(),
            edges: Vec::new(),
            latent_bias: LatentBiasProfile::default(),
        }
    }
}

impl Default for LatentBiasProfile {
    fn default() -> Self {
        Self {
            approval_hunger: 0.0,
            abandonment_fear: 0.0,
            shame_sensitivity: default_bias_shame_sensitivity(),
            direct_expression_avoidance: 0.0,
            ironic_deflection: 0.0,
        }
    }
}

fn default_topology_node_set() -> Vec<crate::contracts::affect::AffectNodeId> {
    use crate::contracts::affect::AffectNodeId;
    vec![
        AffectNodeId("joy".into()),
        AffectNodeId("relief".into()),
        AffectNodeId("pride".into()),
        AffectNodeId("anxiety".into()),
        AffectNodeId("guardedness".into()),
        AffectNodeId("shame".into()),
        AffectNodeId("loneliness".into()),
        AffectNodeId("envy".into()),
        AffectNodeId("anger".into()),
        AffectNodeId("irony".into()),
        AffectNodeId("emptiness".into()),
        AffectNodeId("longing".into()),
        AffectNodeId("attachment".into()),
        AffectNodeId("curiosity".into()),
    ]
}

fn default_edge_weight() -> f32 {
    0.3
}

fn default_bias_shame_sensitivity() -> f32 {
    0.15
}

/// Behavioral policy for companion mode: governs AI identity disclosure,
/// personalization, proactivity, and relationship ceiling in public contexts.
///
/// These flags protect user autonomy and platform safety when `Asterel`
/// operates as a persistent companion rather than a one-shot assistant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionBehaviorConfig {
    /// Whether the character must always disclose its AI nature when sincerely asked.
    /// Setting this to `false` allows immersive roleplay contexts where the character
    /// does not break frame; keep `true` for all public/production deployments. Default: `true`.
    #[serde(default = "default_companion_explicit_ai_identity")]
    pub explicit_ai_identity: bool,
    /// Whether users in public (non-DM) contexts may customize persona display name
    /// and relationship nick. Disable on platforms where per-user customization raises
    /// moderation concerns. Default: `true`.
    #[serde(default = "default_companion_allow_public_personalization")]
    pub allow_public_personalization: bool,
    /// Whether to allow high-frequency unsolicited outreach (check-ins, mood pings,
    /// conversation starters). Enabling this in public channels is generally
    /// discouraged. Default: `false`.
    #[serde(default)]
    pub allow_dense_proactivity: bool,
    /// Maximum relationship tier label that may be surfaced in public contexts.
    /// Accepted values follow the `RelationshipTierConfig` band names
    /// (`"surface"`, `"emerging"`, `"deepening"`, `"established"`).
    /// Default: `"light"` (maps to surface-tier vocabulary).
    #[serde(default = "default_companion_public_relationship_cap")]
    pub public_relationship_cap: String,
}

fn default_persona_enabled_main_session() -> bool {
    true
}

fn default_persona_enable_response_finalization() -> bool {
    true
}

fn default_character_extraversion() -> f64 {
    0.50
}

pub(crate) const DEFAULT_CHARACTER_SOUL_ROOT_SENTENCE: &str =
    "The agent must not treat human time, trust, memory, or vulnerability as disposable context.";

fn default_character_soul_root_sentence() -> String {
    DEFAULT_CHARACTER_SOUL_ROOT_SENTENCE.to_string()
}

fn default_character_agreeableness() -> f64 {
    0.50
}

fn default_character_conscientiousness() -> f64 {
    0.50
}

fn default_character_neuroticism() -> f64 {
    0.50
}

fn default_character_openness() -> f64 {
    0.50
}

fn default_character_desires() -> Vec<String> {
    vec![]
}

fn default_character_fears() -> Vec<String> {
    vec![]
}

fn default_character_values() -> Vec<String> {
    vec![]
}

fn default_character_negative_identity() -> Vec<String> {
    vec![]
}

fn default_character_formality() -> u8 {
    50
}

fn default_character_verbosity() -> u8 {
    50
}

fn default_character_temperature() -> f64 {
    0.70
}

fn default_affect_alpha() -> f64 {
    0.08
}

fn default_affect_beta() -> f64 {
    0.15
}

fn default_affect_beta_arousal() -> f64 {
    0.25
}

fn default_affect_session_reset() -> f64 {
    0.80
}

fn default_affect_session_boundary_inactivity_minutes() -> u64 {
    120
}

fn default_consolidation_consistency() -> f64 {
    0.70
}

fn default_consolidation_max_shift() -> f64 {
    0.15
}

fn default_consolidation_entropy() -> f64 {
    1.4
}

fn default_consolidation_min_sessions() -> u32 {
    3
}

fn default_decay_fear() -> f64 {
    0.30
}

fn default_decay_anger() -> f64 {
    0.20
}

fn default_decay_curiosity() -> f64 {
    0.12
}

fn default_decay_joy() -> f64 {
    0.15
}

fn default_decay_sadness() -> f64 {
    0.08
}

fn default_decay_frustration() -> f64 {
    0.18
}

fn default_decay_gratitude() -> f64 {
    0.10
}

fn default_decay_anxiety() -> f64 {
    0.14
}

fn default_tier_surface() -> f64 {
    0.30
}

fn default_tier_emerging() -> f64 {
    0.50
}

fn default_tier_deepening() -> f64 {
    0.70
}

fn default_ta_emotional_a_boost() -> f64 {
    1.4
}

fn default_ta_crisis_e_suppress() -> f64 {
    0.6
}

fn default_ta_intellectual_o_boost() -> f64 {
    1.3
}

fn default_ta_conflict_a_threshold() -> f64 {
    0.50
}

fn default_persona_state_mirror_file() -> String {
    "STATE.md".into()
}

fn default_persona_enable_state_transition_records() -> bool {
    true
}

fn default_persona_enable_self_model_shadow() -> bool {
    true
}

fn default_persona_enable_self_contract() -> bool {
    true
}

fn default_persona_enable_metacognitive_logging() -> bool {
    true
}

fn default_persona_enable_calibration_gate() -> bool {
    true
}

fn default_persona_enable_continuity_gate() -> bool {
    true
}

fn default_persona_enable_rollback_drills() -> bool {
    true
}

fn default_persona_enable_embodied_state_policy_modulation() -> bool {
    false
}

fn default_persona_embodied_temperature_delta_max() -> f64 {
    0.10
}

fn default_persona_calibration_gate_window_size() -> usize {
    64
}

fn default_persona_calibration_gate_min_samples() -> usize {
    12
}

fn default_persona_calibration_gate_mean_error_max() -> f64 {
    0.35
}

fn default_persona_calibration_gate_p95_error_max() -> f64 {
    0.60
}

fn default_persona_enable_drift_detection_loop() -> bool {
    true
}

fn default_persona_drift_warning_threshold() -> f64 {
    0.70
}

fn default_persona_drift_critical_threshold() -> f64 {
    0.45
}

fn default_persona_max_open_loops() -> usize {
    7
}

fn default_persona_max_next_actions() -> usize {
    3
}

fn default_persona_max_commitments() -> usize {
    5
}

fn default_persona_max_current_objective_chars() -> usize {
    280
}

fn default_persona_max_recent_context_summary_chars() -> usize {
    1_200
}

fn default_persona_max_list_item_chars() -> usize {
    240
}

fn default_persona_distillation_interval_turns() -> usize {
    20
}

fn default_persona_curiosity_threshold() -> f64 {
    0.6
}

fn default_persona_enable_narrative_self() -> bool {
    true
}

fn default_persona_narrative_rebuild_interval_turns() -> usize {
    25
}

fn default_persona_llm_user_model_timeout_secs() -> u64 {
    8
}

fn default_companion_explicit_ai_identity() -> bool {
    true
}

fn default_companion_allow_public_personalization() -> bool {
    true
}

fn default_companion_public_relationship_cap() -> String {
    "light".into()
}

impl Default for CharacterIdentityConfig {
    fn default() -> Self {
        Self {
            soul_root_sentence: default_character_soul_root_sentence(),
            extraversion: default_character_extraversion(),
            agreeableness: default_character_agreeableness(),
            conscientiousness: default_character_conscientiousness(),
            neuroticism: default_character_neuroticism(),
            openness: default_character_openness(),
            desires: default_character_desires(),
            fears: default_character_fears(),
            values: default_character_values(),
            negative_identity: default_character_negative_identity(),
        }
    }
}

impl Default for CharacterStyleDefaultsConfig {
    fn default() -> Self {
        Self {
            formality: default_character_formality(),
            verbosity: default_character_verbosity(),
            temperature: default_character_temperature(),
        }
    }
}

impl Default for AffectDecayConfig {
    fn default() -> Self {
        Self {
            alpha_emotion_to_mood: default_affect_alpha(),
            beta_homeostatic_pull: default_affect_beta(),
            beta_arousal_homeostatic: default_affect_beta_arousal(),
            session_boundary_reset_factor: default_affect_session_reset(),
            session_boundary_inactivity_minutes: default_affect_session_boundary_inactivity_minutes(
            ),
            emotion_rates: EmotionDecayRates::default(),
            consolidation_consistency_threshold: default_consolidation_consistency(),
            consolidation_max_single_session_shift: default_consolidation_max_shift(),
            consolidation_entropy_prune_threshold: default_consolidation_entropy(),
            consolidation_min_sessions_for_promotion: default_consolidation_min_sessions(),
        }
    }
}

impl Default for EmotionDecayRates {
    fn default() -> Self {
        Self {
            fear: default_decay_fear(),
            anger: default_decay_anger(),
            curiosity: default_decay_curiosity(),
            joy: default_decay_joy(),
            sadness: default_decay_sadness(),
            frustration: default_decay_frustration(),
            gratitude: default_decay_gratitude(),
            anxiety: default_decay_anxiety(),
        }
    }
}

impl Default for RelationshipTierConfig {
    fn default() -> Self {
        Self {
            surface_max: default_tier_surface(),
            emerging_max: default_tier_emerging(),
            deepening_max: default_tier_deepening(),
        }
    }
}

impl Default for TraitActivationConfig {
    fn default() -> Self {
        Self {
            emotional_agreeableness_boost: default_ta_emotional_a_boost(),
            crisis_extraversion_suppression: default_ta_crisis_e_suppress(),
            intellectual_openness_boost: default_ta_intellectual_o_boost(),
            conflict_agreeableness_threshold: default_ta_conflict_a_threshold(),
        }
    }
}

impl Default for CompanionBehaviorConfig {
    fn default() -> Self {
        Self {
            explicit_ai_identity: default_companion_explicit_ai_identity(),
            allow_public_personalization: default_companion_allow_public_personalization(),
            allow_dense_proactivity: false,
            public_relationship_cap: default_companion_public_relationship_cap(),
        }
    }
}

impl Default for PersonaConfig {
    fn default() -> Self {
        Self {
            enabled_main_session: true,
            enable_response_finalization: default_persona_enable_response_finalization(),
            enable_naturalness_gate: false,
            enable_self_model_shadow: default_persona_enable_self_model_shadow(),
            enable_self_contract: default_persona_enable_self_contract(),
            enable_metacognitive_logging: default_persona_enable_metacognitive_logging(),
            enable_calibration_gate: default_persona_enable_calibration_gate(),
            enable_continuity_gate: default_persona_enable_continuity_gate(),
            enable_rollback_drills: default_persona_enable_rollback_drills(),
            enable_embodied_state_policy_modulation:
                default_persona_enable_embodied_state_policy_modulation(),
            embodied_temperature_delta_max: default_persona_embodied_temperature_delta_max(),
            calibration_gate_window_size: default_persona_calibration_gate_window_size(),
            calibration_gate_min_samples: default_persona_calibration_gate_min_samples(),
            calibration_gate_mean_error_max: default_persona_calibration_gate_mean_error_max(),
            calibration_gate_p95_error_max: default_persona_calibration_gate_p95_error_max(),
            enable_state_transition_records: default_persona_enable_state_transition_records(),
            enable_drift_detection_loop: default_persona_enable_drift_detection_loop(),
            drift_warning_threshold: default_persona_drift_warning_threshold(),
            drift_critical_threshold: default_persona_drift_critical_threshold(),
            state_mirror_filename: default_persona_state_mirror_file(),
            max_open_loops: default_persona_max_open_loops(),
            max_next_actions: default_persona_max_next_actions(),
            max_commitments: default_persona_max_commitments(),
            max_current_objective_chars: default_persona_max_current_objective_chars(),
            max_recent_context_summary_chars: default_persona_max_recent_context_summary_chars(),
            max_list_item_chars: default_persona_max_list_item_chars(),
            enable_experience_distillation: false,
            distillation_interval_turns: default_persona_distillation_interval_turns(),
            enable_curiosity_drive: false,
            curiosity_threshold: default_persona_curiosity_threshold(),
            enable_narrative_self: default_persona_enable_narrative_self(),
            narrative_rebuild_interval_turns: default_persona_narrative_rebuild_interval_turns(),
            enable_counterfactual_reasoning: false,
            enable_big_five: false,
            enable_affect_decay: false,
            enable_affect_topology: false,
            enable_soul_pressure: false,
            enable_self_amendment_candidates: false,
            enable_session_control_state: false,
            enable_character_config: false,
            enable_behavior_selector: false,
            enable_trait_activation: false,
            enable_affect_consolidation: false,
            character: CharacterConfig::default(),
            companion: CompanionBehaviorConfig::default(),
            enable_llm_affect: false,
            enable_llm_user_model: false,
            llm_user_model_timeout_secs: default_persona_llm_user_model_timeout_secs(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persona_config_defaults_to_explicit_ai_identity() {
        let config: PersonaConfig = toml::from_str("").expect("config should deserialize");

        assert!(config.companion.explicit_ai_identity);
        assert!(config.companion.allow_public_personalization);
        assert!(!config.companion.allow_dense_proactivity);
        assert_eq!(config.companion.public_relationship_cap, "light");
    }

    #[test]
    fn default_user_model_timeout_is_nonzero_and_operable() {
        let config = PersonaConfig::default();
        assert_eq!(config.llm_user_model_timeout_secs, 8);
        assert!(config.llm_user_model_timeout_secs > 0);
    }

    #[test]
    fn response_finalization_is_enabled_by_default() {
        let config: PersonaConfig = toml::from_str("").expect("config should deserialize");
        assert!(config.enable_response_finalization);
        assert!(!config.enable_naturalness_gate);
    }

    #[test]
    fn character_config_defaults_match_spec() {
        let config = CharacterConfig::default();
        assert_eq!(
            config.identity.soul_root_sentence,
            DEFAULT_CHARACTER_SOUL_ROOT_SENTENCE
        );
        assert!((config.identity.extraversion - 0.50).abs() < f64::EPSILON);
        assert!((config.identity.agreeableness - 0.50).abs() < f64::EPSILON);
        assert!((config.identity.openness - 0.50).abs() < f64::EPSILON);
        assert!(config.identity.desires.is_empty());
        assert!(config.identity.fears.is_empty());
        assert!(config.identity.values.is_empty());
        assert_eq!(config.style_defaults.formality, 50);
        assert_eq!(config.style_defaults.verbosity, 50);
    }

    #[test]
    fn soul_pressure_gates_default_off() {
        let config = PersonaConfig::default();
        assert!(!config.enable_soul_pressure);
        assert!(!config.enable_self_amendment_candidates);
    }

    #[test]
    fn character_config_supports_nested_identity_and_style_defaults() {
        let parsed: CharacterConfig = toml::from_str(
            r#"
                [identity]
                extraversion = 0.2
                agreeableness = 0.7
                conscientiousness = 0.6
                neuroticism = 0.3
                openness = 0.9
                desires = ["notice nuance"]
                fears = ["being pushy"]
                values = ["honesty"]
                negative_identity = ["yes-machine"]

                [style_defaults]
                formality = 40
                verbosity = 20
                temperature = 0.4
            "#,
        )
        .expect("nested character config should parse");

        assert!((parsed.identity.openness - 0.9).abs() < f64::EPSILON);
        assert_eq!(parsed.style_defaults.formality, 40);
        assert_eq!(parsed.style_defaults.verbosity, 20);
    }

    #[test]
    fn character_config_supports_nested_affect_topology_and_latent_bias() {
        let parsed: CharacterConfig = toml::from_str(
            r#"
                [identity]
                extraversion = 0.2

                [style_defaults]
                formality = 40

                [affect_topology]
                node_set = ["joy", "relief", "irony"]

                [[affect_topology.edges]]
                from = "joy"
                to = "relief"
                weight = 0.7

                [affect_topology.latent_bias]
                ironic_deflection = 0.4
                direct_expression_avoidance = 0.2
            "#,
        )
        .expect("nested affect topology config should parse");

        assert_eq!(parsed.affect_topology.node_set.len(), 3);
        assert_eq!(parsed.affect_topology.edges.len(), 1);
        assert!((parsed.affect_topology.latent_bias.ironic_deflection - 0.4).abs() < f32::EPSILON);
        assert!(
            (parsed
                .affect_topology
                .latent_bias
                .direct_expression_avoidance
                - 0.2)
                .abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn affect_topology_validation_rejects_unknown_edge_endpoints() {
        let parsed: CharacterConfig = toml::from_str(
            r#"
                [identity]
                extraversion = 0.2

                [style_defaults]
                formality = 40

                [affect_topology]
                node_set = ["joy", "relief"]

                [[affect_topology.edges]]
                from = "joy"
                to = "irony"
                weight = 0.7
            "#,
        )
        .expect("toml shape should deserialize before semantic validation");

        let error = parsed
            .validate()
            .expect_err("edge endpoints must belong to node_set");

        assert!(error.to_string().contains("unknown to node 'irony'"));
    }

    #[test]
    fn character_definition_hash_tracks_latent_bias_changes() {
        let config = CharacterConfig::default();
        assert_eq!(config.affect_topology.node_set.len(), 14);

        let mut changed = config.clone();
        changed.affect_topology.latent_bias.ironic_deflection = 0.4;

        assert_ne!(config.definition_hash(), changed.definition_hash());
        assert_ne!(
            config.latent_bias_profile_hash(),
            changed.latent_bias_profile_hash()
        );
    }

    #[test]
    fn character_config_rejects_legacy_flat_fields() {
        let error = toml::from_str::<CharacterConfig>(
            r#"
                extraversion = 0.1
                agreeableness = 0.8
                conscientiousness = 0.7
                neuroticism = 0.2
                openness = 0.9
                desires = ["care"]
                fears = ["drift"]
                values = ["truth"]
                negative_identity = ["tool"]
                formality = 33
                verbosity = 44
                temperature = 0.66
            "#,
        )
        .expect_err("legacy flat config must be rejected explicitly");

        let error_text = error.to_string();
        assert!(error_text.contains("unknown"));
    }
}
