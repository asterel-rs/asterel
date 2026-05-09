//! Policy types controlling the agent's per-turn decision-making.
//!
//! Each turn follows a three-phase cycle:
//!
//! 1. **Feature extraction** — [`SituationFeatures`] is built from the
//!    incoming message: domain classification, complexity estimate, and the
//!    current affect reading.
//!
//! 2. **Policy selection** — [`PolicyDecision`] maps situation features to a
//!    [`ReasoningStrategy`] and a [`MemoryPolicy`]. The policy selector reads
//!    the features and produces the decision. A [`ReasonTrace`] records *why*
//!    each choice was made, including any risk flags that caused deviation
//!    from the default.
//!
//! 3. **Outcome recording** — [`TurnOutcome`] captures proxy quality signals
//!    once the turn completes (response length, tool use, success and effort
//!    scores). A [`TurnOutcomeRecord`] bundles all three phases together for
//!    retrospective analysis and experience distillation.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::contracts::affect::AffectLabel;
use crate::contracts::quality::TurnQualityVector;

/// Domain tag inferred from user message content.
///
/// Used to select domain-appropriate heuristics and to scope retrospective
/// outcome tracking
/// so that performance in one domain does not mask regression in another.
///
/// - `General` — default when no specific domain is recognized.
/// - `Technical` — coding, debugging, system configuration, and
///   infrastructure tasks.
/// - `Creative` — writing, brainstorming, art direction, and narrative work.
/// - `Personal` — emotional support, journaling, personal planning, and
///   relationship topics.
/// - `Administrative` — scheduling, email drafting, documentation, and
///   organizational tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DomainTag {
    #[default]
    General,
    Technical,
    Creative,
    Personal,
    Administrative,
}

impl DomainTag {
    /// Zero-allocation `snake_case` label matching the `serde` rename output.
    #[must_use]
    pub(crate) const fn as_snake_case(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Technical => "technical",
            Self::Creative => "creative",
            Self::Personal => "personal",
            Self::Administrative => "administrative",
        }
    }
}

/// Reasoning approach selected for the current turn.
///
/// The policy selector picks a strategy based on situation features and risk
/// flags. The strategy governs how the LLM prompt is structured and which
/// memory is foregrounded.
///
/// - `Standard` — the default path: direct answer generation with normal
///   memory retrieval. Selected when complexity is low and no risk flags are
///   active.
/// - `VerifyFirst` — before answering, the agent explicitly checks its
///   assumptions and any retrieved facts. Selected when
///   [`RiskFlag::LowDomainSuccess`] or [`RiskFlag::InsufficientData`] is
///   present, or when the domain historically has high error rates.
/// - `AskClarify` — the agent issues a clarifying question instead of
///   attempting a full answer. Selected when the message is ambiguous
///   ([`AffectLabel::Confused`] detected) or when confidence in the
///   domain classification is low.
/// - `Stepwise` — the agent explicitly breaks the problem into numbered steps
///   before answering. Selected when [`RiskFlag::HighComplexity`] is set or
///   when `affect_label` is [`AffectLabel::Overwhelmed`].
///
/// [`AffectLabel::Confused`]: crate::contracts::affect::AffectLabel::Confused
/// [`AffectLabel::Overwhelmed`]: crate::contracts::affect::AffectLabel::Overwhelmed
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReasoningStrategy {
    #[default]
    Standard,
    VerifyFirst,
    AskClarify,
    Stepwise,
}

impl ReasoningStrategy {
    /// Zero-allocation `snake_case` label matching the `serde` rename output.
    #[must_use]
    pub(crate) const fn as_snake_case(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::VerifyFirst => "verify_first",
            Self::AskClarify => "ask_clarify",
            Self::Stepwise => "stepwise",
        }
    }
}

/// Memory retrieval policy for the current turn.
///
/// These parameters are derived from companion-first retrieval heuristics and
/// applied to the retrieval query before the LLM call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MemoryPolicy {
    /// Maximum number of memory items to pass into the prompt context.
    /// Higher values give more context but increase token cost and latency.
    pub retrieve_top_k: usize,
    /// Minimum number of high-confidence "fact-tier" items that must be
    /// present. If retrieval returns fewer, the agent may ask for
    /// clarification rather than hallucinate. Set to `0` to disable the
    /// floor.
    pub min_facts: usize,
    /// Maximum number of low-confidence "noise-tier" items allowed into
    /// context. Noise items add diversity but can distract the LLM.
    pub noise_budget: usize,
}

impl Default for MemoryPolicy {
    fn default() -> Self {
        Self {
            retrieve_top_k: 10,
            min_facts: 0,
            noise_budget: 3,
        }
    }
}

/// Situation features extracted from the current turn context.
///
/// These features are the input to the policy engine. All policy decisions
/// — strategy selection, memory tuning, affect modulation — are conditioned
/// on this struct. It is also stored on [`TurnOutcomeRecord`] so that
/// retrospective analysis can correlate features with outcomes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SituationFeatures {
    /// Inferred task domain (used for outcome scoping and strategy weighting).
    pub domain: DomainTag,
    /// Estimated task complexity in `[0.0, 1.0]`. Derived from message length,
    /// vocabulary density, and presence of multi-step indicators. Values above
    /// `0.7` tend to trigger the `Stepwise` strategy.
    pub complexity: f32,
    /// Discrete affect label from the most recent affect reading.
    pub affect_label: AffectLabel,
    /// Intensity of the detected affect in `[0.0, 1.0]`, derived from the
    /// magnitude of the VAD vector relative to its prototype. High intensity
    /// amplifies affect-driven strategy modulation.
    pub affect_intensity: f32,
}

impl Default for SituationFeatures {
    fn default() -> Self {
        Self {
            domain: DomainTag::General,
            complexity: 0.0,
            affect_label: AffectLabel::Neutral,
            affect_intensity: 0.0,
        }
    }
}

/// Policy decision governing the current turn's behavior.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct PolicyDecision {
    pub reasoning: ReasoningStrategy,
    pub memory: MemoryPolicy,
}

/// Bounded quality score in the range `[0.0, 1.0]`.
///
/// `0.0` represents the worst possible outcome for that dimension;
/// `1.0` represents the best. The default is `0.5` (neutral / unknown).
///
/// Use this type rather than raw `f32` when recording turn quality so that
/// the clamping invariant is always enforced. Both `TurnOutcome::success`
/// and `TurnOutcome::user_effort` are expressed as `OutcomeScore`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct OutcomeScore(f32);

impl OutcomeScore {
    #[must_use]
    pub(crate) fn new(value: f32) -> Self {
        Self(value.clamp(0.0, 1.0))
    }

    #[must_use]
    pub(crate) fn value(self) -> f32 {
        self.0
    }
}

impl Default for OutcomeScore {
    fn default() -> Self {
        Self(0.5)
    }
}

/// Quantitative outcome signals captured after a turn completes.
///
/// These signals are proxy metrics — they approximate quality without
/// requiring explicit user ratings. They feed outcome recording and the
/// principle distillation pipeline.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct TurnOutcome {
    /// Proxy success score in `[0.0, 1.0]` based on response quality
    /// heuristics (e.g. absence of hedging language, structural completeness,
    /// no apparent contradiction with retrieved facts). Not a user rating.
    pub success: OutcomeScore,
    /// Proxy user-effort score in `[0.0, 1.0]`. Lower scores indicate more
    /// friction: the user had to send corrections, repeat themselves, or
    /// express frustration. Derived from follow-up message patterns and
    /// external fitness signals. A score of `1.0` means the turn completed
    /// without observable user effort.
    pub user_effort: OutcomeScore,
    /// Character length of the assistant response. Tracked to detect
    /// verbosity drift over repeated turns.
    pub response_length: usize,
    /// Whether the turn invoked any tools. Tool-using turns are evaluated
    /// separately because their success proxy differs from pure text turns.
    pub had_tool_calls: bool,
}

/// Explanation of why the policy engine made its decisions for a turn.
///
/// Stored alongside the [`TurnOutcomeRecord`] so that developers and operators
/// can audit policy choices, understand affect-driven deviations, and improve
/// the strategy selection logic without re-running turns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ReasonTrace {
    /// The strategy that was actually executed (may differ from `base_strategy`
    /// if affect modulation overrode the initial selection).
    pub strategy_selected: ReasoningStrategy,
    /// Identifiers of the memory slots that most influenced the decision.
    /// Useful for tracing which retrieved facts led to a particular strategy.
    pub key_evidence_slots: Vec<String>,
    /// Policy selector confidence in `strategy_selected` (`0.0`–`1.0`).
    /// Low confidence may indicate heuristic uncertainty or that risk flags
    /// significantly constrained the choice set.
    pub confidence: f32,
    /// Active risk flags at decision time. An empty vec means the turn was
    /// considered low-risk and the default strategy applied.
    pub risk_flags: Vec<RiskFlag>,
    /// Whether the affect subsystem overrode or adjusted the base strategy.
    pub was_affect_modulated: bool,
    /// The strategy the policy engine would have selected without affect
    /// modulation. `None` if `was_affect_modulated` is `false`.
    pub base_strategy: Option<ReasoningStrategy>,
    /// Human-readable explanation of any affect-driven empathy adjustments
    /// (e.g. why a `Stepwise` strategy was chosen after detecting `Overwhelmed`).
    ///
    /// Stored as `Option<Cow<'static, str>>` so the construction path — a
    /// fixed template literal from `select_empathy_response_style` — stays
    /// allocation-free. Deserialisation always produces `Cow::Owned`, which
    /// is fine because the record is only deserialised during replay, not
    /// on the per-turn hotpath.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empathy_rationale: Option<std::borrow::Cow<'static, str>>,
    /// Human-readable explanation of style register changes (e.g. why tone
    /// was softened or formality reduced).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_rationale: Option<std::borrow::Cow<'static, str>>,
}

/// Conditions detected during policy evaluation that elevate risk and may
/// override the default strategy selection.
///
/// Multiple flags can be active simultaneously; the policy engine applies
/// a priority ordering (safety flags beat exploration flags) when resolving
/// conflicts.
///
/// - `LowDomainSuccess` — the current domain's recent outcome baseline is
///   below a configured threshold, suggesting the default strategy is
///   struggling here. Biases selection toward `VerifyFirst`.
/// - `HighComplexity` — `SituationFeatures::complexity` exceeded the
///   high-complexity threshold. Biases selection toward `Stepwise`.
/// - `NegativeUserFeedback` — recent turns in this session show elevated
///   user-effort scores or detected correction patterns. Triggers a
///   conservative strategy and may prompt a clarify turn.
/// - `InsufficientData` — memory retrieval returned fewer fact-tier items
///   than `MemoryPolicy::min_facts` requires. Biases selection toward
///   `VerifyFirst` or `AskClarify`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RiskFlag {
    LowDomainSuccess,
    HighComplexity,
    NegativeUserFeedback,
    InsufficientData,
}

/// A structured record correlating situation features, policy decision, and
/// observed outcome for a single turn.
///
/// This is the primary per-turn decision/outcome record. Each turn produces
/// one record; records accumulate in the store and are periodically consumed
/// by offline diagnostics and the experience distiller.
///
/// The optional fields `quality_vector` and `reason_trace` are populated by
/// separate subsystems after the turn completes and may be absent for turns
/// that pre-date those subsystems or where those subsystems were disabled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TurnOutcomeRecord {
    pub id: String,
    pub situation: SituationFeatures,
    pub policy: PolicyDecision,
    pub outcome: TurnOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_vector: Option<TurnQualityVector>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_trace: Option<ReasonTrace>,
    pub occurred_at: String,
}

impl TurnOutcomeRecord {
    #[must_use]
    pub(crate) fn new(
        situation: SituationFeatures,
        policy: PolicyDecision,
        outcome: TurnOutcome,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            situation,
            policy,
            outcome,
            quality_vector: None,
            reason_trace: None,
            occurred_at: Utc::now().to_rfc3339(),
        }
    }

    #[must_use]
    pub(crate) fn with_quality_vector(mut self, qv: TurnQualityVector) -> Self {
        self.quality_vector = Some(qv);
        self
    }
}
