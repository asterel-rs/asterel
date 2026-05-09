//! Experience subsystem: atom ingestion, distillation, principle extraction, and RL signal.
//!
//! The experience subsystem records the outcomes of agent interactions and refines them into
//! durable, reusable memory. The pipeline proceeds in three stages:
//!
//! 1. **Ingest** ([`ingest`]) — Persists raw [`ExperienceAtom`] records from companion turns,
//!    tool outcomes, and self-task completions via [`persist_experience_atom`].
//! 2. **Distill** ([`distill`], [`distill_types`]) — Periodically condenses accumulated atoms
//!    into compact, domain-tagged principles using an LLM summarisation pass.
//! 3. **Retrieve** ([`retrieve`], [`principle_retrieve`]) — Surfaces relevant experiences and
//!    principles into context at inference time via semantic retrieval.
//!
//! The [`memory_rl`] module computes reinforcement-learning reward signals from experience
//! outcomes to adjust future behaviour. [`domain_tag`] provides normalised domain taxonomy
//! labels used during distillation and retrieval.

pub(crate) mod distill;
pub(crate) mod distill_types;
pub(crate) mod domain_tag;
mod ingest;
mod journal;
pub(crate) mod memory_rl;
pub(crate) mod presenter;
pub(crate) mod principle_retrieve;
mod retrieve;
mod types;

pub(crate) use crate::contracts::experience::{ExperienceAtom, ExperienceKind, ExperienceOutcome};
pub(crate) use ingest::persist_experience_atom;
pub(crate) use journal::{record_codespace_experience, record_self_task_experience};
pub(crate) use presenter::render_experience_block;
pub(crate) use retrieve::retrieve_relevant_experiences;
