//! Affect subsystem: the companion's emotional processing pipeline.
//!
//! # Architecture
//!
//! The pipeline transforms raw user text into observable expression in five stages:
//!
//! ```text
//! user text
//!     │
//!     ▼
//! [detection]     detector.rs / llm_detector.rs / hybrid.rs
//!     │  VAD + label (valence, arousal, dominance)
//!     ▼
//! [appraisal]     appraisal.rs
//!     │  multi-dimensional meaning (reward, loss_risk, norm_violation …)
//!     ▼
//! [topology]      topology.rs
//!     │  base activation → graph diffusion → latent bias → surfaced
//!     ▼
//! [style overlay] style_overlay.rs
//!     │  VAD → formality / verbosity / temperature deltas
//!     ▼
//! [surface]       presenter.rs
//!     │  topology snapshot + desire + mood → prompt guidance blocks
//!     ▼
//! LLM turn
//! ```
//!
//! # Key design choices
//!
//! ## Topology over labels
//! Rather than mapping the detected label directly to a behaviour, the pipeline
//! routes emotion through a character-specific graph (`topology.rs`). Different
//! characters transform the same event through different internal routes — this
//! is the primary anti-thinness mechanism. The same joy signal produces relief in
//! one character and irony in another, depending on edge weights.
//!
//! ## Latent bias
//! Each character carries a `LatentBiasProfile` that applies suppression,
//! amplification, or rerouting *after* diffusion. Shame sensitivity suppresses
//! pride; abandonment fear routes attachment energy into anxiety; ironic deflection
//! activates the irony node under high emotional intensity. This is what makes
//! emotional responses feel *characterful* rather than uniform.
//!
//! ## Anxiety floor
//! Anxiety is never fully suppressed (floor = 0.05) because `Anthropic`'s
//! [EMOTION-CONCEPTS-LLM] research shows that reducing anxiety increases harmful
//! behaviour. The floor is a safety signal that persists regardless of other bias
//! transforms.
//!
//! ## Desire-driven objectives
//! Beyond style modulation, affect reshapes *what the agent prioritises* via
//! `desire.rs`. An anxious user triggers a Safety desire; a curious user triggers
//! Exploration. This follows the [DESIRE-DRIVEN] cognitive modeling framework.
//!
//! ## Session mood
//! `mood.rs` maintains a PAD-space mood that accumulates across turns using
//! `ALMA`'s two-force model: emotions push mood, a homeostatic spring pulls it
//! back toward the personality baseline.
//!
//! # Module map
//!
//! | Module | Role |
//! |--------|------|
//! | `appraisal` | Event → meaning dimensions |
//! | `cause` | Attribute the cause behind a user's emotional state |
//! | `consolidation` | Bayesian affect memory consolidation across sessions |
//! | `desire` | Affect → objective modulation |
//! | `detector` | Rule-based VAD-first affect detection |
//! | `hybrid` | Rule-based + LLM disambiguation pipeline |
//! | `llm_detector` | LLM-based affect detection with fallback |
//! | `mood` | Session-level PAD mood accumulation |
//! | `persistence` | Affect arc serialization and trend analysis |
//! | `presenter` | Topology + desire + mood → prompt guidance blocks |
//! | `style_overlay` | VAD → formality / verbosity / temperature deltas |
//! | `topology` | Affect topology graph: diffusion and latent bias |
//! | `types` | Core value types: `AffectArc`, `ActiveEmotions` |

pub(crate) mod appraisal;
pub(crate) mod cause;
pub(crate) mod consolidation;
pub(crate) mod decay;
pub(crate) mod desire;
mod detector;
pub(crate) mod hybrid;
pub(crate) mod llm_detector;
pub(crate) mod mood;
pub(crate) mod persistence;
pub(crate) mod presenter;
mod style_overlay;
pub(crate) mod topology;
mod types;

pub use crate::contracts::affect::{AffectLabel, AffectReading};
pub(crate) use consolidation::{EmotionalMemory, compute_session_sentiment, consolidate_session};
pub(crate) use detector::RuleBasedDetector;
pub(crate) use hybrid::hybrid_detect;
pub(crate) use llm_detector::{AffectDetector, build_affect_detector};
pub(crate) use mood::{SessionMood, select_mood};
pub(crate) use persistence::{
    AffectTrend, TrendDirection, compute_affect_trend, load_emotional_memories, load_session_mood,
    persist_emotional_memories, persist_promoted_emotional_memories, persist_session_mood,
};
pub(crate) use presenter::{render_affect_block, render_session_mood_block, render_topology_block};
pub(crate) use style_overlay::{affect_to_style_delta, affect_vad_to_style_delta};
pub(crate) use types::AffectArc;
