//! Verify-repair escalation: resilience layer for transient provider failures.
//!
//! # Why it exists
//!
//! `Asterel` relies on external LLM providers (`OpenAI`,
//! `Anthropic`, and others) whose APIs are subject to transient
//! errors: network timeouts, momentary 503s, and short-lived 429
//! rate-limit bursts.  Without a retry layer every such blip would
//! surface as a hard failure to the user.
//!
//! The verify-repair loop in `session.rs` calls into this module to:
//! 1. **Classify** the error as retryable (transient) or non-retryable
//!    (quota exhaustion, 4xx client errors, policy limits).
//! 2. **Decide** whether to retry, escalate, or bail based on the
//!    configured caps and current attempt/repair-depth counters.
//! 3. **Record** an escalation event to memory when the loop gives up,
//!    giving the operator a persistent audit trail.
//!
//! # Escalation conditions
//!
//! Escalation is triggered by the first of:
//! - `attempts >= max_attempts` — total retry budget exhausted.
//! - `repair_depth >= max_repair_depth` — nested repair calls would
//!   risk cascading side-effects.
//! - `!analysis.retryable` — the error is classified as permanent
//!   (e.g. quota exhausted, unknown model, policy limit).
//!
//! Both caps come from `config.autonomy` and are bounded by a
//! hard-coded ceiling of 10 in `session.rs` as a last-resort safety net.

use anyhow::Result;
use serde_json::{Value, json};

use crate::config::Config;
use crate::core::memory::{
    Memory, MemoryEventInput, MemoryEventType, MemoryProvenance, MemorySource, PrivacyLevel,
    SourceKind,
};
use crate::security::writeback_guard::enforce_verify_repair_write_policy;

/// Configuration caps for verify-repair escalation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct VerifyRepairCaps {
    /// Maximum number of total retry attempts.
    pub(super) max_attempts: u32,
    /// Maximum cumulative repair depth before escalation.
    pub(super) max_repair_depth: u32,
}

impl VerifyRepairCaps {
    /// Load caps from the autonomy configuration.
    pub(super) fn from_config(config: &Config) -> Self {
        Self {
            max_attempts: config.autonomy.verify_repair_max_attempts,
            max_repair_depth: config.autonomy.verify_repair_max_repair_depth,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VerifyRepairEscalationReason {
    MaxAttemptsReached,
    MaxRepairDepthReached,
    NonRetryableFailure,
}

impl VerifyRepairEscalationReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::MaxAttemptsReached => "max_attempts_reached",
            Self::MaxRepairDepthReached => "max_repair_depth_reached",
            Self::NonRetryableFailure => "non_retryable_failure",
        }
    }
}

/// Classification of a verify-repair failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct VerifyFailureAnalysis {
    /// Descriptive class of the failure (e.g. "`transient_failure`").
    pub(super) failure_class: &'static str,
    /// Whether the failure is eligible for retry.
    pub(super) retryable: bool,
}

/// Record of an escalation event when verify-repair gives up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VerifyRepairEscalation {
    reason: VerifyRepairEscalationReason,
    /// Number of attempts made before escalation.
    pub(super) attempts: u32,
    /// Current repair depth at escalation time.
    pub(super) repair_depth: u32,
    /// Configured maximum attempts.
    pub(super) max_attempts: u32,
    /// Configured maximum repair depth.
    pub(super) max_repair_depth: u32,
    /// Failure classification string.
    pub(super) failure_class: &'static str,
    /// Text of the last error that triggered escalation.
    pub(super) last_error: String,
}

impl VerifyRepairEscalation {
    /// Format a human-readable escalation message for error reporting.
    pub(super) fn contract_message(&self) -> String {
        format!(
            "verify/repair escalated: reason={} attempts={} repair_depth={} max_attempts={} max_repair_depth={} failure_class={} last_error={}",
            self.reason.as_str(),
            self.attempts,
            self.repair_depth,
            self.max_attempts,
            self.max_repair_depth,
            self.failure_class,
            self.last_error
        )
    }

    fn event_payload(&self) -> Value {
        json!({
            "reason": self.reason.as_str(),
            "attempts": self.attempts,
            "repair_depth": self.repair_depth,
            "max_attempts": self.max_attempts,
            "max_repair_depth": self.max_repair_depth,
            "failure_class": self.failure_class,
            "last_error": self.last_error,
        })
    }
}

/// Memory slot key for verify-repair escalation events.
pub(super) use crate::contracts::strings::data_model::SLOT_VERIFY_REPAIR_ESCALATION as VERIFY_REPAIR_ESCALATION_SLOT_KEY;

/// Classify a verification failure as retryable or non-retryable
/// based on error message heuristics.
pub(super) fn analyze_verify_failure(error: &anyhow::Error) -> VerifyFailureAnalysis {
    let message = error.to_string();
    let lower = message.to_ascii_lowercase();
    if lower.contains("action limit exceeded") || lower.contains("daily cost limit exceeded") {
        return VerifyFailureAnalysis {
            failure_class: "policy_limit",
            retryable: false,
        };
    }

    if lower.contains("insufficient_quota")
        || lower.contains("exceeded your current quota")
        || (lower.contains("429") && lower.contains("billing"))
    {
        return VerifyFailureAnalysis {
            failure_class: "quota_exhausted",
            retryable: false,
        };
    }

    for word in lower.split(|c: char| !c.is_ascii_digit()) {
        if let Ok(code) = word.parse::<u16>()
            && (400..500).contains(&code)
            && code != 408
            && code != 429
        {
            return VerifyFailureAnalysis {
                failure_class: "non_retryable_provider_error",
                retryable: false,
            };
        }
    }

    VerifyFailureAnalysis {
        failure_class: "transient_failure",
        retryable: true,
    }
}

/// Decide whether to escalate based on caps, attempt count, repair
/// depth, and failure retryability. Returns `None` if retry is
/// allowed.
pub(super) fn decide_verify_repair_escalation(
    caps: VerifyRepairCaps,
    attempts: u32,
    repair_depth: u32,
    analysis: VerifyFailureAnalysis,
    last_error: &anyhow::Error,
) -> Option<VerifyRepairEscalation> {
    let reason = if attempts >= caps.max_attempts {
        Some(VerifyRepairEscalationReason::MaxAttemptsReached)
    } else if repair_depth >= caps.max_repair_depth {
        Some(VerifyRepairEscalationReason::MaxRepairDepthReached)
    } else if !analysis.retryable {
        Some(VerifyRepairEscalationReason::NonRetryableFailure)
    } else {
        None
    }?;

    Some(VerifyRepairEscalation {
        reason,
        attempts,
        repair_depth,
        max_attempts: caps.max_attempts,
        max_repair_depth: caps.max_repair_depth,
        failure_class: analysis.failure_class,
        last_error: last_error.to_string(),
    })
}

/// Persist an escalation event to memory for auditing.
///
/// # Errors
///
/// Returns an error if write-policy enforcement or memory
/// persistence fails.
pub(super) async fn emit_verify_repair_escalation_event(
    mem: &dyn Memory,
    entity_id: &str,
    escalation: &VerifyRepairEscalation,
) -> Result<()> {
    let event = MemoryEventInput::new(
        entity_id,
        VERIFY_REPAIR_ESCALATION_SLOT_KEY,
        MemoryEventType::SummaryCompacted,
        escalation.event_payload().to_string(),
        MemorySource::System,
        PrivacyLevel::Private,
    )
    .with_confidence(1.0)
    .with_importance(0.9)
    .with_source_kind(SourceKind::Manual)
    .with_source_ref("verify-repair.escalation")
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::System,
        "verify-repair.escalation",
    ));
    enforce_verify_repair_write_policy(&event)?;
    mem.append_event(event).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::analyze_verify_failure;

    #[test]
    fn analyze_verify_failure_marks_quota_as_non_retryable() {
        let err = anyhow::anyhow!(
            "{}",
            "OpenAI API error (429 Too Many Requests): {\"error\":{\"message\":\"You exceeded your current quota\",\"type\":\"insufficient_quota\"}}"
        );
        let analysis = analyze_verify_failure(&err);
        assert_eq!(analysis.failure_class, "quota_exhausted");
        assert!(!analysis.retryable);
    }

    #[test]
    fn analyze_verify_failure_keeps_transient_errors_retryable() {
        let err = anyhow::anyhow!("transport timeout while calling provider");
        let analysis = analyze_verify_failure(&err);
        assert_eq!(analysis.failure_class, "transient_failure");
        assert!(analysis.retryable);
    }

    #[test]
    fn analyze_verify_failure_marks_404_as_non_retryable() {
        let err = anyhow::anyhow!("OpenAI API error (404 Not Found): model not found");
        let analysis = analyze_verify_failure(&err);
        assert_eq!(analysis.failure_class, "non_retryable_provider_error");
        assert!(!analysis.retryable);
    }
}
