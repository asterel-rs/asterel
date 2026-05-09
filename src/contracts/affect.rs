//! Affect contracts shared across affect, persona, tools, and policy subsystems.
//!
//! The affect subsystem models the companion's emotional state through two
//! complementary representations:
//!
//! - **Discrete labels** ([`AffectLabel`]): Simple named categories for
//!   fast classification, routing decisions, and coarse gating. The detector
//!   assigns one label per reading by matching the continuous coordinates to
//!   the nearest prototype.
//!
//! - **Continuous VAD coordinates** ([`AffectReading`]): A three-axis
//!   Valence-Arousal-Dominance vector that captures nuance between label
//!   prototypes. Two readings may share the same label while differing
//!   substantially in intensity or control — information that the label alone
//!   cannot convey.
//!
//! [`AffectNodeId`] ties these representations to the *affect topology graph*,
//! a per-character directed graph whose nodes are named emotional states and
//! whose edges encode which states can activate one another. Because each
//! character has a different topology, activation spreads along different
//! routes for different personas rather than through a single shared lookup.

use serde::{Deserialize, Serialize};

use super::scores::Confidence;

/// Discrete emotional label assigned by the affect detector.
///
/// Labels exist because much of the pipeline (memory retrieval, policy
/// selection, style modulation) needs a cheap categorical signal rather than
/// three floating-point numbers. They are derived from [`AffectReading`]
/// VAD coordinates by finding the nearest prototype — the label with the
/// smallest Euclidean distance to the measured (valence, arousal, dominance)
/// point.
///
/// Each variant corresponds to a recognizable companion emotional state:
///
/// - `Neutral` — baseline; no strong signal in either direction. The safe
///   default when affect is ambiguous or confidence is below threshold.
/// - `Confused` — the user's message contains contradictions, unclear
///   references, or scope that is hard to parse. Triggers clarification.
/// - `Frustrated` — repeated failed attempts, explicit dissatisfaction, or
///   rising negative valence with high arousal. Prompts de-escalation.
/// - `Anxious` — low dominance + moderate arousal; the companion senses
///   uncertainty about what to do next. Often precedes a clarify strategy.
/// - `Sad` — low valence, low arousal; subdued tone warranting a gentler
///   response register.
/// - `Angry` — high arousal, strongly negative valence. The most salient
///   escalation signal; triggers conservative, non-confrontational strategy.
/// - `Excited` — high arousal, positive valence; energy that the companion
///   can mirror to reinforce positive engagement.
/// - `Grateful` — positive valence, moderate arousal; acknowledgment of
///   something done well. A reinforcement cue for fitness scoring.
/// - `Curious` — moderate positive arousal; an invitation to elaborate or
///   explore. Often licenses longer, more detailed responses.
/// - `Overwhelmed` — very high arousal, low dominance; cognitive overload.
///   Prompts simplification and step-by-step guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AffectLabel {
    Neutral,
    Confused,
    Frustrated,
    Anxious,
    Sad,
    Angry,
    Excited,
    Grateful,
    Curious,
    Overwhelmed,
}

impl AffectLabel {
    /// Zero-allocation `snake_case` label matching the `serde` rename output.
    /// Used by render helpers to avoid `serde_json::to_string` round-trips on
    /// the per-turn hotpath.
    #[must_use]
    pub const fn as_snake_case(self) -> &'static str {
        match self {
            Self::Neutral => "neutral",
            Self::Confused => "confused",
            Self::Frustrated => "frustrated",
            Self::Anxious => "anxious",
            Self::Sad => "sad",
            Self::Angry => "angry",
            Self::Excited => "excited",
            Self::Grateful => "grateful",
            Self::Curious => "curious",
            Self::Overwhelmed => "overwhelmed",
        }
    }
}

/// A single affect measurement combining continuous VAD coordinates with a
/// discrete label.
///
/// The VAD (Valence-Arousal-Dominance) model maps emotional states to a
/// three-dimensional space:
///
/// - **Valence** (`-1.0` = maximally unpleasant, `+1.0` = maximally pleasant):
///   the hedonic quality of the state. Negative valence drives de-escalation;
///   strongly positive valence can justify more expressive mirroring.
/// - **Arousal** (`0.0` = calm/deactivated, `1.0` = agitated/activated):
///   the energy level. High arousal combined with negative valence signals
///   urgency (frustration, anger); high arousal with positive valence signals
///   enthusiasm.
/// - **Dominance** (`0.0` = submissive/controlled, `1.0` = dominant/in-control):
///   how much agency the companion perceives. Low dominance + high arousal
///   encodes anxiety; high dominance + high arousal encodes assertiveness.
///
/// The `label` field is derived from these coordinates by prototype matching
/// and is the canonical value used for downstream routing. The raw coordinates
/// are preserved so that subsystems requiring continuous signals (e.g. style
/// intensity blending) are not limited to the nearest prototype.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectReading {
    /// Discrete affect label derived from VAD prototype matching.
    pub label: AffectLabel,
    /// Valence: pleasure/displeasure axis in `[-1.0, 1.0]`.
    pub valence: f64,
    /// Arousal: activation/deactivation axis in `[0.0, 1.0]`.
    pub arousal: f64,
    /// Dominance: sense of control in `[0.0, 1.0]`. `0.0` = submissive, `1.0` = dominant.
    #[serde(default = "default_dominance")]
    pub dominance: f64,
    /// Detector confidence in the assigned label (`0.0` to `1.0`).
    pub confidence: Confidence,
}

pub(crate) fn default_dominance() -> f64 {
    0.5
}

/// Node identifier in a character-specific affect topology graph.
///
/// Each node represents a distinct affective state within a single character's
/// emotional vocabulary. Node IDs use **lowercase emotion names** (e.g.
/// `"anxiety"`, `"joy"`, `"guardedness"`, `"irony"`) so they remain human-
/// readable in serialized form and can be referenced in character definition
/// files without needing an enum lookup.
///
/// The topology graph defines which nodes sit near each other *for a specific
/// character*. When affect is detected, the system activates the matching node
/// and spreads activation along the character's edges — so the same detected
/// label can produce different downstream emotional coloring depending on the
/// character. This is what gives each persona a distinct emotional "texture"
/// rather than a flat label→response mapping shared across all companions.
///
/// [`AffectNodeId`] is distinct from [`AffectLabel`]: labels are a closed
/// enum of universal categories; node IDs are open strings that live inside
/// a character's personal topology and may include states with no corresponding
/// label (e.g. `"wistfulness"`, `"dry_amusement"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AffectNodeId(pub String);
