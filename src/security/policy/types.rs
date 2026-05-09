//! Policy type definitions: autonomy levels, external action
//! execution modes, and action verdicts (allow/deny with reason).

use serde::{Deserialize, Serialize};

pub use crate::contracts::security::{AutonomyLevel, ExternalActionExecution};

/// Result of a policy check: allowed or denied with a reason string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionPolicyVerdict {
    /// Whether the action was permitted.
    pub allowed: bool,
    /// Human-readable explanation for the decision.
    pub reason: String,
}

impl ActionPolicyVerdict {
    /// Create an "allowed" verdict with the given reason.
    pub fn allow(reason: impl Into<String>) -> Self {
        Self {
            allowed: true,
            reason: reason.into(),
        }
    }

    /// Create a "denied" verdict with the given reason.
    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            allowed: false,
            reason: reason.into(),
        }
    }
}
