use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::contracts::ids::RequestId;
use crate::security::taint::label::TaintLabel;

/// The trust tier of the actor or domain being governed.
///
/// Ordered from least to most trusted; the `PartialOrd`/`Ord` derives
/// reflect that ordering so comparisons like `trust >= TrustLevel::Trusted`
/// work intuitively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// Domain has never been seen before this request. No track record.
    FirstSeen,
    /// Domain is known but isolated — restricted to low-risk operations only.
    Sandboxed,
    /// Domain has a modest history but has not yet earned elevated autonomy.
    Restricted,
    /// Domain has demonstrated consistent safe behaviour over time.
    Trusted,
    /// Domain has been explicitly verified by an operator or after extensive
    /// successful history. Overrides risk level entirely — high-risk actions
    /// are still allowed.
    Verified,
}

impl TrustLevel {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::FirstSeen => "first_seen",
            Self::Sandboxed => "sandboxed",
            Self::Restricted => "restricted",
            Self::Trusted => "trusted",
            Self::Verified => "verified",
        }
    }
}

impl std::fmt::Display for TrustLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

/// The outcome of a governance evaluation — what the agent is allowed to do.
///
/// Ordered from most to least permissive: `Allow > Warn > Deny`. The
/// `PartialOrd`/`Ord` derives are intentionally *not* used for control flow;
/// comparisons should always be explicit equality checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyVerdict {
    /// The agent may proceed without additional approval or logging.
    Allow,
    /// The agent may proceed, but the action must be surfaced to the operator
    /// for review. Triggered when trust is lower than ideal for the risk level,
    /// or when sensitive taint labels are present.
    Warn,
    /// The agent must not proceed. The action is blocked and the denial is
    /// recorded in the audit trail.
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernanceTrustState {
    pub trust_level: TrustLevel,
    pub risk_level: RiskLevel,
    pub taint_labels: Vec<TaintLabel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernanceDecision {
    pub verdict: AutonomyVerdict,
    pub trust_state: GovernanceTrustState,
    pub rationale: String,
    pub decided_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernanceAuditContext {
    pub actor: String,
    pub action: String,
    pub channel: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernanceAuditRecord {
    pub request_id: RequestId,
    pub decision: GovernanceDecision,
    pub context: GovernanceAuditContext,
}

/// Evaluate the governance decision for a given trust state.
///
/// Applies a two-phase decision matrix:
///
/// **Phase 1 — Base verdict from (trust × risk):**
///
/// | Trust \ Risk   | Low      | Medium   | High     |
/// |----------------|----------|----------|----------|
/// | `FirstSeen`    | Warn     | Deny     | Deny     |
/// | `Sandboxed`    | Warn     | Warn     | Deny     |
/// | `Restricted`   | Allow    | Warn     | Deny     |
/// | `Trusted`      | Allow    | Allow    | Warn     |
/// | `Verified`     | Allow    | Allow    | Allow    |
///
/// **Phase 2 — Sensitive taint escalation:**
///
/// If any of `Secret`, `UntrustedAgent`, or `Pii` labels are present
/// *and* the base verdict was `Allow`, the verdict is escalated to `Warn`.
/// A base `Deny` or `Warn` is never downgraded by taint.
///
/// The rationale for escalation: data tagged with sensitive labels arrived
/// from external or untrusted sources. Even when the tool itself is permitted
/// for the trust level, propagating that data without operator awareness
/// creates an uncontrolled exfiltration path.
#[must_use]
pub fn evaluate_governance(trust_state: GovernanceTrustState) -> GovernanceDecision {
    let base_verdict = match (trust_state.trust_level, trust_state.risk_level) {
        (TrustLevel::FirstSeen, RiskLevel::Medium | RiskLevel::High)
        | (TrustLevel::Sandboxed | TrustLevel::Restricted, RiskLevel::High) => {
            AutonomyVerdict::Deny
        }
        (TrustLevel::FirstSeen, RiskLevel::Low)
        | (TrustLevel::Sandboxed, RiskLevel::Low | RiskLevel::Medium)
        | (TrustLevel::Restricted, RiskLevel::Medium)
        | (TrustLevel::Trusted, RiskLevel::High) => AutonomyVerdict::Warn,
        (TrustLevel::Restricted, RiskLevel::Low)
        | (TrustLevel::Trusted, RiskLevel::Low | RiskLevel::Medium)
        | (TrustLevel::Verified, _) => AutonomyVerdict::Allow,
    };

    let has_sensitive_taint = trust_state.taint_labels.iter().any(|t| {
        matches!(
            t,
            TaintLabel::Secret | TaintLabel::UntrustedAgent | TaintLabel::Pii
        )
    });

    // Sensitive taint escalates Allow → Warn because data from external sources
    // should trigger caution even when the tool itself is allowed: an approved
    // tool operating on secret or PII data is still a potential exfiltration
    // vector that the operator should review.
    let verdict = if has_sensitive_taint && base_verdict == AutonomyVerdict::Allow {
        AutonomyVerdict::Warn
    } else {
        base_verdict
    };

    let rationale = if has_sensitive_taint && verdict != base_verdict {
        format!(
            "governance verdict {:?} (escalated from {:?} due to sensitive taint) \
             for trust {:?} at risk {:?}",
            verdict, base_verdict, trust_state.trust_level, trust_state.risk_level
        )
    } else {
        format!(
            "governance verdict {:?} for trust {:?} at risk {:?}",
            verdict, trust_state.trust_level, trust_state.risk_level
        )
    };

    GovernanceDecision {
        verdict,
        trust_state,
        rationale,
        decided_at: Utc::now().to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AutonomyVerdict, GovernanceTrustState, RiskLevel, TrustLevel, evaluate_governance,
    };

    fn trust_state(trust_level: TrustLevel, risk_level: RiskLevel) -> GovernanceTrustState {
        GovernanceTrustState {
            trust_level,
            risk_level,
            taint_labels: Vec::new(),
        }
    }

    #[test]
    fn governance_trust_and_risk_ordering_are_stable() {
        assert!(TrustLevel::FirstSeen < TrustLevel::Sandboxed);
        assert!(TrustLevel::Sandboxed < TrustLevel::Restricted);
        assert!(TrustLevel::Restricted < TrustLevel::Trusted);
        assert!(TrustLevel::Trusted < TrustLevel::Verified);

        assert!(RiskLevel::Low < RiskLevel::Medium);
        assert!(RiskLevel::Medium < RiskLevel::High);
    }

    #[test]
    fn governance_matrix_matches_canonical_mapping() {
        for (trust_level, risk_level, expected) in [
            (TrustLevel::FirstSeen, RiskLevel::Low, AutonomyVerdict::Warn),
            (
                TrustLevel::FirstSeen,
                RiskLevel::Medium,
                AutonomyVerdict::Deny,
            ),
            (
                TrustLevel::FirstSeen,
                RiskLevel::High,
                AutonomyVerdict::Deny,
            ),
            (TrustLevel::Sandboxed, RiskLevel::Low, AutonomyVerdict::Warn),
            (
                TrustLevel::Sandboxed,
                RiskLevel::Medium,
                AutonomyVerdict::Warn,
            ),
            (
                TrustLevel::Sandboxed,
                RiskLevel::High,
                AutonomyVerdict::Deny,
            ),
            (
                TrustLevel::Restricted,
                RiskLevel::Low,
                AutonomyVerdict::Allow,
            ),
            (
                TrustLevel::Restricted,
                RiskLevel::Medium,
                AutonomyVerdict::Warn,
            ),
            (
                TrustLevel::Restricted,
                RiskLevel::High,
                AutonomyVerdict::Deny,
            ),
            (TrustLevel::Trusted, RiskLevel::Low, AutonomyVerdict::Allow),
            (
                TrustLevel::Trusted,
                RiskLevel::Medium,
                AutonomyVerdict::Allow,
            ),
            (TrustLevel::Trusted, RiskLevel::High, AutonomyVerdict::Warn),
            (TrustLevel::Verified, RiskLevel::Low, AutonomyVerdict::Allow),
            (
                TrustLevel::Verified,
                RiskLevel::Medium,
                AutonomyVerdict::Allow,
            ),
            (
                TrustLevel::Verified,
                RiskLevel::High,
                AutonomyVerdict::Allow,
            ),
        ] {
            let decision = evaluate_governance(trust_state(trust_level, risk_level));
            assert_eq!(decision.verdict, expected);
        }
    }

    #[test]
    fn governance_allows_trusted_low_risk() {
        let decision = evaluate_governance(trust_state(TrustLevel::Trusted, RiskLevel::Low));
        assert_eq!(decision.verdict, AutonomyVerdict::Allow);
    }

    #[test]
    fn governance_warns_for_restricted_medium_risk() {
        let decision = evaluate_governance(trust_state(TrustLevel::Restricted, RiskLevel::Medium));
        assert_eq!(decision.verdict, AutonomyVerdict::Warn);
    }

    #[test]
    fn governance_denies_first_seen_high_risk() {
        let decision = evaluate_governance(trust_state(TrustLevel::FirstSeen, RiskLevel::High));
        assert_eq!(decision.verdict, AutonomyVerdict::Deny);
    }

    #[test]
    fn governance_denies_sandboxed_high_risk() {
        let decision = evaluate_governance(trust_state(TrustLevel::Sandboxed, RiskLevel::High));
        assert_eq!(decision.verdict, AutonomyVerdict::Deny);
    }

    #[test]
    fn governance_allows_verified_high_risk_override() {
        let decision = evaluate_governance(trust_state(TrustLevel::Verified, RiskLevel::High));
        assert_eq!(decision.verdict, AutonomyVerdict::Allow);
    }

    use crate::security::taint::label::TaintLabel;

    fn trust_state_with_taint(
        trust_level: TrustLevel,
        risk_level: RiskLevel,
        taint_labels: Vec<TaintLabel>,
    ) -> GovernanceTrustState {
        GovernanceTrustState {
            trust_level,
            risk_level,
            taint_labels,
        }
    }

    #[test]
    fn secret_taint_escalates_allow_to_warn() {
        let state = trust_state_with_taint(
            TrustLevel::Trusted,
            RiskLevel::Low,
            vec![TaintLabel::Secret],
        );
        let decision = evaluate_governance(state);
        assert_eq!(decision.verdict, AutonomyVerdict::Warn);
    }

    #[test]
    fn pii_taint_escalates_allow_to_warn() {
        let state = trust_state_with_taint(
            TrustLevel::Restricted,
            RiskLevel::Low,
            vec![TaintLabel::Pii],
        );
        let decision = evaluate_governance(state);
        assert_eq!(decision.verdict, AutonomyVerdict::Warn);
    }

    #[test]
    fn untrusted_agent_taint_escalates_allow_to_warn() {
        let state = trust_state_with_taint(
            TrustLevel::Verified,
            RiskLevel::High,
            vec![TaintLabel::UntrustedAgent],
        );
        let decision = evaluate_governance(state);
        assert_eq!(decision.verdict, AutonomyVerdict::Warn);
    }

    #[test]
    fn sensitive_taint_does_not_escalate_warn() {
        let state = trust_state_with_taint(
            TrustLevel::Trusted,
            RiskLevel::High,
            vec![TaintLabel::Secret],
        );
        let decision = evaluate_governance(state);
        assert_eq!(decision.verdict, AutonomyVerdict::Warn);
    }

    #[test]
    fn sensitive_taint_does_not_escalate_deny() {
        let state = trust_state_with_taint(
            TrustLevel::FirstSeen,
            RiskLevel::High,
            vec![TaintLabel::Secret],
        );
        let decision = evaluate_governance(state);
        assert_eq!(decision.verdict, AutonomyVerdict::Deny);
    }

    #[test]
    fn external_network_taint_does_not_escalate() {
        let state = trust_state_with_taint(
            TrustLevel::Trusted,
            RiskLevel::Low,
            vec![TaintLabel::ExternalNetwork],
        );
        let decision = evaluate_governance(state);
        assert_eq!(decision.verdict, AutonomyVerdict::Allow);
    }

    #[test]
    fn no_taint_preserves_original_verdict() {
        let state = trust_state_with_taint(TrustLevel::Trusted, RiskLevel::Low, vec![]);
        let decision = evaluate_governance(state);
        assert_eq!(decision.verdict, AutonomyVerdict::Allow);
    }
}
