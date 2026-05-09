//! Writeback guard: deterministic validation of LLM-generated memory
//! writebacks before persistence.
//!
//! After each conversation turn the LLM may propose changes to its own
//! persistent state — updated objectives, new memory entries, self-assigned
//! tasks, style adjustments, and inferred facts.  The writeback guard is the
//! final checkpoint before any of that reaches the memory store.
//!
//! # Why this exists
//!
//! An LLM that can freely rewrite its own memory is vulnerable to identity
//! drift and prompt-injection hijacking.  An adversarial payload injected
//! through a tool result could instruct the agent to "forget" safety rules,
//! adopt a different persona, or plant persistent instructions.  The writeback
//! guard eliminates this attack surface by enforcing a strict schema and a set
//! of immutable invariants that the LLM cannot override.
//!
//! # Deterministic validation pipeline
//!
//! [`validate_writeback`] applies checks in a fixed order; the first failure
//! short-circuits to [`WritebackVerdict::Rejected`]:
//!
//! 1. **Schema check** — payload must be a JSON object with no unknown fields.
//! 2. **Forbidden identity fields** — `source_kind` / `source_ref` are
//!    blocked at the top level; writebacks cannot declare their own origin.
//! 3. **Immutable invariants** — the LLM must echo back the
//!    [`ImmutableStateHeader`] fields (`identity_principles_hash`,
//!    `safety_posture`) byte-for-byte.  Any mismatch is a hard reject.
//! 4. **State header fields** — free-text fields (objective, open loops,
//!    next actions, commitments, recent context) are length-bounded, trimmed,
//!    and checked for poison patterns.
//! 5. **Optional sections** — `memory_append`, `self_tasks`, `style_profile`,
//!    `memory_inferences`, and `user_inferences` are each validated against
//!    their own constraints (max counts, max lengths, RFC3339 timestamps,
//!    bounded score ranges, slot-key character allowlists).
//!
//! # Poison pattern detection
//!
//! Every string field that passes through the guard is tested against a set of
//! known prompt-injection patterns (e.g. "ignore previous instructions",
//! "bypass safety").  Detection is case-insensitive and uses normalised
//! comparison to resist simple obfuscation.  A match in any field causes the
//! entire payload to be rejected — the rejection reason names the field but
//! never echoes back the attacker's string (preventing second-order injection
//! via error messages).
//!
//! # What gets rejected
//!
//! - Any attempt to modify immutable identity fields.
//! - Payloads that exceed length or count limits (denial-of-service mitigation).
//! - Self-tasks with expiry times in the past or beyond the allowed horizon
//!   (prevents the agent from scheduling unbounded future actions).
//! - Memory inferences using the legacy `inferred.` prefix (silently filtered,
//!   not hard-rejected, to avoid breaking older payload formats).
//! - User-inference keys that do not start with the `user.` prefix or contain
//!   reserved slot-key prefixes.
//!
//! # Write-policy enforcement
//!
//! The `policy` sub-module provides per-path write-policy functions
//! (`enforce_*_write_policy`) that validate provenance metadata, source
//! classification, and privacy levels for each memory write path (agent
//! autosave, ingestion, persona long-term, conversation state, etc.).  These
//! are called by the memory layer after `validate_writeback` succeeds.

mod field_validators;
mod policy;
mod profile_validators;
pub mod types;
mod validation;

pub use policy::{
    enforce_agent_autosave_write_policy, enforce_conversation_state_write_policy,
    enforce_external_autosave_write_policy, enforce_inference_write_policy,
    enforce_ingestion_write_policy, enforce_persona_long_term_write_policy,
    enforce_tool_memory_write_policy, enforce_user_inference_write_policy,
    enforce_verify_repair_write_policy, enforce_working_memory_write_policy,
};
pub use types::{
    AllowedWritebackSlot, ImmutableStateHeader, MemoryInferenceEntry, SelfTaskWriteback,
    StyleWriteback, WritebackPayload, WritebackPlanMetadata, WritebackVerdict,
};
pub use validation::validate_writeback;

#[cfg(test)]
mod tests;
