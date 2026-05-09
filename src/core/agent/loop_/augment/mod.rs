//! Turn augmentation module — the cognitive front-end of the agent loop.
//!
//! Every time the agent is about to answer a turn, `pre_answer` runs here and
//! gathers context from the companion-first feedback-loop-closing subsystems:
//!
//! | System | Purpose | Key module |
//! |--------|---------|------------|
//! | Grounding (System 2) | Recalled memory facts | `pipeline` → `memory_updates` |
//! | Experience (System 3) | Past-turn atoms & distilled principles | `distillation_trigger` |
//! | Affect (System 6) | Emotion detection, topology, governance | `pipeline` → `persona_updates` |
//! | Taste (System 1) | Aesthetic preference contract | `pipeline` |
//! | Policy selection | outcome-based heuristics | `policy_selector` |
//!
//! After the answer is returned, `post_answer` closes the remaining feedback loops
//! (memory writes, distillation triggers).
//!
//! ## Module map
//!
//! ```text
//! pipeline          ← DefaultAugmentor: pre_answer + post_answer entry points
//!   ├─ post_answer_capture  ← context assembly, policy setup, turn assessment
//!   │    ├─ memory_updates         ← experience atoms, outcome records, error taxonomy
//!   │    ├─ distillation_updates   ← experience-to-principle trigger
//!   │    └─ persona_updates        ← affect arc, Big Five, world model
//!   ├─ cognitive_budget     ← knapsack budget allocation for prompt blocks
//!   ├─ types                ← TurnAugmentations, TurnStyleOverlay
//!   ├─ policy / policy_types / policy_selector  ← governed strategy selection
//!   ├─ quality_vector       ← six-dimensional per-turn quality scoring
//!   ├─ outcome_record       ← persisted (situation, policy, outcome) tuples
//!   ├─ retrieval_quality    ← memory recall utilisation scoring + Self-RAG quality gate
//!   ├─ uncertainty          ← token log-prob uncertainty metrics + reward shaping
//!   ├─ distillation_trigger ← experience-to-principle pipeline scheduling
//!   ├─ distillation_updates ← distillation update orchestrator
//! ```

pub(crate) mod cognitive_budget;
mod distillation_trigger;
mod distillation_updates;
mod memory_updates;
pub(crate) mod outcome_record;
mod persona_updates;
mod pipeline;
pub(crate) mod policy;
pub(crate) mod policy_selector;
mod post_answer_capture;
pub(crate) mod quality_vector;
pub(crate) mod retrieval_quality;
mod types;

pub(crate) use pipeline::{DefaultAugmentor, TurnAugmentor};
pub(crate) use types::{TurnAugmentations, apply_augmentation_blocks_budgeted};
