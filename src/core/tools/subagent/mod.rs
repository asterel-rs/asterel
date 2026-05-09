//! Sub-agent management tools.
//!
//! # Subagent lifecycle
//!
//! ```text
//! spawn  ──►  [running]  ──►  [completed]
//!                │
//!             cancel
//!                │
//!           [cancelled]
//! ```
//!
//! 1. `subagent_spawn` starts a new isolated run, either in the background
//!    (returns a `run_id` immediately) or inline (blocks until the run finishes
//!    and returns the output directly).
//! 2. `subagent_output` polls the run registry for the status and output of a
//!    background run identified by its `run_id`.
//! 3. `subagent_cancel` signals a running background run to stop.
//!
//! # Spawn limits
//!
//! The `ExecutionContext` carries two independent limits enforced by
//! `handoff::build_delegation_options` before a run is accepted:
//!
//! * **Delegation depth** — how many nested subagent layers are allowed
//!   (e.g., agent → subagent → sub-subagent). Spawn is rejected once
//!   `delegation_depth >= max_delegation_depth`.
//! * **Child quota** — how many child runs the current agent context may
//!   spawn in total. Each successful spawn atomically consumes one slot.
//!   Spawn is rejected once the quota reaches zero.
//!
//! Both limits are propagated to the child via `SubagentDelegationConfig`
//! so each nested layer inherits the same policy.
//!
//! # Security surface
//!
//! Spawn is blocked in `AutonomyLevel::ReadOnly` mode. The handoff envelope
//! (`objective`, `done_when`, `context`, `constraints`) is parsed and
//! validated before the run is launched; non-string array items in
//! `constraints` are rejected.

pub mod cancel;
mod handoff;
pub mod output;
pub mod spawn;

pub(crate) const SUBAGENT_OUTPUT_TAINT_LABEL: &str = "subagent_output";

#[must_use]
pub(crate) fn subagent_output_taint_labels() -> Vec<String> {
    vec![SUBAGENT_OUTPUT_TAINT_LABEL.to_string()]
}

pub use cancel::SubagentCancelTool;
pub(crate) use handoff::{build_delegation_options, parse_handoff_envelope};
pub use output::SubagentOutputTool;
pub use spawn::SubagentSpawnTool;
