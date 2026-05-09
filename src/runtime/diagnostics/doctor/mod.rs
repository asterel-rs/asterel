//! Doctor diagnostic command: validates setup, daemon health, and
//! governance/rollout state.

mod checks;
mod report;
mod setup;

pub use checks::run;
#[cfg(test)]
use report::{
    autonomy_governance_lines, memory_rollout_lines, memory_signal_stats_lines, parse_rfc3339,
    persona_calibration_lines, persona_continuity_gate_lines, persona_drift_lines,
    persona_embodied_state_lines,
};

#[cfg(test)]
mod tests;
