//! Tool execution policy engine: loadable rules with explicit precedence.
//!
//! Replaces hardcoded risk classification with a configurable rule set
//! that evaluates in a defined order:
//!
//! 1. explicit-deny rules (hard block)
//! 2. explicit-ask rules (force approval prompt)
//! 3. explicit-allow rules (bypass approval)
//! 4. capability-mode fallback (autonomy level check)
//! 5. default → deny
//!
//! Hook decisions (from WP-G2) will slot between deny and ask in the
//! precedence chain when implemented.
//!
//! Design source: ecosystem survey 2026-04-03 (claw-code-parity, OpenClaw, Codex CLI).
//! Per §6.4.H: principles adopted, surface expressions redesigned for Asterel.

mod engine;
mod rules;

pub use engine::{
    PolicyDecisionKind, PolicyEngine, PolicyEvaluation, SUPERVISED_FALLBACK_APPROVAL_REASON,
    is_read_only_tool,
};
pub use rules::{PolicyRule, PolicyRuleSet, ToolPattern};
