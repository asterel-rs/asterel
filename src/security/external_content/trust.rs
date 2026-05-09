//! Dynamic trust-score assigner for external content sources.
//!
//! Combines configured source reputation with lightweight runtime
//! verification and reputation signals from source metadata and text.

use crate::config::ExternalKnowledgeTrustConfig;
use crate::security::governance::RiskLevel;
use crate::security::taint::label::TaintLabel;
use crate::security::{GovernanceDecision, GovernanceTrustState, TrustLevel, evaluate_governance};

/// Governance-aligned signals applied to one external ingress payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExternalGovernanceSignal {
    PlainHttpOrigin,
    AnonymousOrigin,
    RelayOrProxyOrigin,
    HighUrlDensity,
    ActiveContentMarkup,
    EncodedPayloadHint,
    SuspiciousTldHint,
}

/// Governance-aligned trust diagnostics for one ingress payload.
#[derive(Debug, Clone)]
pub(crate) struct ExternalGovernanceAssessment {
    pub(crate) base_score: f32,
    pub(crate) score: f32,
    pub(crate) signals: Vec<ExternalGovernanceSignal>,
    pub(crate) decision: GovernanceDecision,
}

/// Evaluate external content using canonical governance semantics.
#[must_use]
pub(crate) fn evaluate_external_governance(
    source: &str,
    text: &str,
    trust: &ExternalKnowledgeTrustConfig,
) -> ExternalGovernanceAssessment {
    if !trust.enabled {
        let decision = evaluate_governance(GovernanceTrustState {
            trust_level: TrustLevel::Verified,
            risk_level: RiskLevel::Low,
            taint_labels: vec![TaintLabel::ExternalNetwork],
        });
        return ExternalGovernanceAssessment {
            base_score: 1.0,
            score: 1.0,
            signals: Vec::new(),
            decision,
        };
    }

    let source_normalized = source.trim().to_ascii_lowercase();
    let text_normalized = text.to_ascii_lowercase();
    let mut signals = Vec::new();
    let base_score = trust.score_for_source(source);
    let mut score = base_score;

    // Positive provenance must come from caller-owned configuration (for
    // example source_overrides), not from parseable words inside the source
    // label. The source string is often assembled near untrusted ingress and
    // can be spoofed with `signature=verified` / `official`-style text.

    if source_normalized.contains("http://") {
        score -= 0.10;
        signals.push(ExternalGovernanceSignal::PlainHttpOrigin);
    }

    if contains_any(
        &source_normalized,
        &["anonymous", "unknown_sender", "unverified"],
    ) {
        score -= 0.18;
        signals.push(ExternalGovernanceSignal::AnonymousOrigin);
    }

    if contains_any(&source_normalized, &["relay", "proxy", "forwarder"]) {
        score -= 0.12;
        signals.push(ExternalGovernanceSignal::RelayOrProxyOrigin);
    }

    let url_count = count_url_like_fragments(&text_normalized);
    if url_count >= 24 {
        score -= 0.20;
        signals.push(ExternalGovernanceSignal::HighUrlDensity);
    } else if url_count >= 12 {
        score -= 0.12;
        signals.push(ExternalGovernanceSignal::HighUrlDensity);
    }

    if contains_any(
        &text_normalized,
        &["<script", "javascript:", "onerror=", "onload="],
    ) {
        score -= 0.20;
        signals.push(ExternalGovernanceSignal::ActiveContentMarkup);
    }

    if contains_long_encoded_chunk(text) {
        score -= 0.12;
        signals.push(ExternalGovernanceSignal::EncodedPayloadHint);
    }

    if contains_suspicious_tld(&text_normalized) {
        score -= 0.08;
        signals.push(ExternalGovernanceSignal::SuspiciousTldHint);
    }

    let score = score.clamp(0.0, 1.0);
    let decision = evaluate_governance(GovernanceTrustState {
        trust_level: trust_level_for_score(score, trust),
        risk_level: risk_level_for_assessment(score, &signals, trust),
        taint_labels: vec![TaintLabel::ExternalNetwork],
    });

    ExternalGovernanceAssessment {
        base_score,
        score,
        signals,
        decision,
    }
}

fn trust_level_for_score(score: f32, trust: &ExternalKnowledgeTrustConfig) -> TrustLevel {
    if score >= 0.95 {
        TrustLevel::Verified
    } else if score >= (trust.min_allow_score + 0.15).min(0.95) {
        TrustLevel::Trusted
    } else if score >= trust.min_allow_score {
        TrustLevel::Restricted
    } else if score >= trust.min_sanitize_score {
        TrustLevel::Sandboxed
    } else {
        TrustLevel::FirstSeen
    }
}

fn risk_level_for_assessment(
    score: f32,
    signals: &[ExternalGovernanceSignal],
    trust: &ExternalKnowledgeTrustConfig,
) -> RiskLevel {
    if score < trust.min_sanitize_score
        || signals.iter().any(|signal| {
            matches!(
                signal,
                ExternalGovernanceSignal::ActiveContentMarkup
                    | ExternalGovernanceSignal::EncodedPayloadHint
                    | ExternalGovernanceSignal::SuspiciousTldHint
            )
        })
    {
        return RiskLevel::High;
    }

    if score < trust.min_allow_score
        || signals.iter().any(|signal| {
            matches!(
                signal,
                ExternalGovernanceSignal::PlainHttpOrigin
                    | ExternalGovernanceSignal::AnonymousOrigin
                    | ExternalGovernanceSignal::RelayOrProxyOrigin
                    | ExternalGovernanceSignal::HighUrlDensity
            )
        })
    {
        return RiskLevel::Medium;
    }

    RiskLevel::Low
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn count_url_like_fragments(text_lower: &str) -> usize {
    text_lower.matches("https://").count()
        + text_lower.matches("http://").count()
        + text_lower.matches("www.").count()
}

fn contains_long_encoded_chunk(text: &str) -> bool {
    text.split(|ch: char| {
        !(ch.is_ascii_alphanumeric() || matches!(ch, '+' | '/' | '_' | '=' | '-'))
    })
    .any(|token| {
        token.len() >= 96
            && token
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '/' | '_' | '=' | '-'))
            && token
                .chars()
                .any(|ch| matches!(ch, '+' | '/' | '_' | '=' | '-'))
    })
}

fn contains_suspicious_tld(text_lower: &str) -> bool {
    [".zip/", ".mov/", ".click/", ".top/", ".work/"]
        .into_iter()
        .any(|needle| text_lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{AutonomyVerdict, TrustLevel};

    #[test]
    fn source_claimed_signature_metadata_does_not_increase_score() {
        let trust = ExternalKnowledgeTrustConfig::default();
        let source = "gateway:webhook:signature=verified:https://official.example";
        let assessed = evaluate_external_governance(source, "status update", &trust);
        assert_eq!(assessed.score, assessed.base_score);
        assert!(assessed.signals.is_empty());
        assert_ne!(assessed.decision.verdict, AutonomyVerdict::Deny);
    }

    #[test]
    fn unstructured_signature_string_does_not_increase_score() {
        let trust = ExternalKnowledgeTrustConfig::default();
        let assessed = evaluate_external_governance(
            "external:signature=verified:https://official.example",
            "status update",
            &trust,
        );
        assert!(assessed.signals.is_empty());
    }

    #[test]
    fn unverified_relay_with_script_reduces_score() {
        let trust = ExternalKnowledgeTrustConfig {
            default_score: 0.70,
            min_allow_score: 0.70,
            min_sanitize_score: 0.30,
            ..ExternalKnowledgeTrustConfig::default()
        };
        let source = "gateway:webhook:anonymous:relay:http://public.example";
        let text = "visit https://bad.click/a and <script>alert(1)</script>";
        let assessed = evaluate_external_governance(source, text, &trust);
        assert!(assessed.score < trust.min_sanitize_score);
        assert!(
            assessed
                .signals
                .contains(&ExternalGovernanceSignal::AnonymousOrigin)
        );
        assert!(
            assessed
                .signals
                .contains(&ExternalGovernanceSignal::ActiveContentMarkup)
        );
        assert_eq!(assessed.decision.verdict, AutonomyVerdict::Deny);
    }

    #[test]
    fn mid_trust_source_warns_without_escalating_to_deny() {
        let trust = ExternalKnowledgeTrustConfig {
            default_score: 0.50,
            min_allow_score: 0.70,
            min_sanitize_score: 0.30,
            ..ExternalKnowledgeTrustConfig::default()
        };
        let assessed = evaluate_external_governance("external:warn", "status update", &trust);
        assert_eq!(assessed.decision.verdict, AutonomyVerdict::Warn);
        assert_eq!(
            assessed.decision.trust_state.trust_level,
            TrustLevel::Sandboxed
        );
        assert!(assessed.signals.is_empty());
    }

    #[test]
    fn disabled_trust_returns_full_score() {
        let trust = ExternalKnowledgeTrustConfig {
            enabled: false,
            ..ExternalKnowledgeTrustConfig::default()
        };
        let assessed = evaluate_external_governance("gateway:webhook", "hello", &trust);
        assert!((assessed.score - 1.0).abs() < f32::EPSILON);
        assert!(assessed.signals.is_empty());
        assert_eq!(assessed.decision.verdict, AutonomyVerdict::Allow);
        assert_eq!(
            assessed.decision.trust_state.trust_level,
            TrustLevel::Verified
        );
    }
}
