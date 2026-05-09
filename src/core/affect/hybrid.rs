//! Hybrid affect detection: rule-based fast path with LLM disambiguation.
//!
//! # Design
//!
//! The rule-based detector (`detector.rs`) is fast and deterministic but
//! occasionally produces ambiguous or mixed-signal results — for example,
//! shouted imperatives ("DO IT NOW!!") that carry high arousal but no clear
//! emotional valence. In those cases a more capable model is worth the latency.
//!
//! [`hybrid_detect`] implements a two-stage strategy:
//!
//! 1. **Rule-based first** — always runs; if the result is unambiguous
//!    (high confidence, no mixed signal), it is returned immediately with no
//!    additional cost.
//!
//! 2. **LLM disambiguation** — runs when `needs_disambiguation()` is true
//!    *and* a detector is provided. The LLM reading wins if its confidence
//!    exceeds the rule-based confidence; otherwise the rule-based result is
//!    kept with a confidence dampening of ×0.85 to signal residual uncertainty.
//!
//! When no detector is provided (e.g., `enable_llm_affect = false`), the
//! function always falls through to the rule-based result, making LLM
//! involvement entirely optional and transparent to callers.

use super::{AffectDetector, AffectReading, RuleBasedDetector};
use crate::contracts::scores::Confidence;

/// The result of a hybrid detection pass, including provenance metadata.
#[derive(Debug, Clone)]
pub(crate) struct HybridAffectResult {
    /// The resolved affect reading (either rule-based or LLM-disambiguated).
    pub final_reading: AffectReading,
    /// Whether LLM disambiguation was attempted (regardless of which reading won).
    pub disambiguation_used: bool,
    /// Which detector contributed the winning reading, if disambiguation ran.
    /// `None` when the rule-based fast path was taken.
    pub disambiguation_source: Option<String>,
}

/// Run hybrid affect detection: rule-based first, then LLM-based disambiguation
/// when the rule-based result is ambiguous and a detector is provided.
///
/// When `detector` is `None`, behaves identically to the pure rule-based path.
/// When `detector` is `Some` and the rule-based confidence is below 0.8,
/// the LLM result is merged (LLM label wins if its confidence exceeds
/// the rule-based confidence).
pub(crate) async fn hybrid_detect(
    user_message: &str,
    detector: Option<&dyn AffectDetector>,
) -> HybridAffectResult {
    let rule_based = RuleBasedDetector::new().detect(user_message);
    if !rule_based.needs_disambiguation() {
        return HybridAffectResult {
            final_reading: rule_based,
            disambiguation_used: false,
            disambiguation_source: None,
        };
    }

    let Some(detector) = detector else {
        return HybridAffectResult {
            final_reading: rule_based,
            disambiguation_used: false,
            disambiguation_source: None,
        };
    };

    // High-confidence rule-based: apply confidence dampening only.
    if rule_based.confidence.get() >= 0.8 {
        let mut disambiguated = rule_based;
        disambiguated.confidence = Confidence::new(disambiguated.confidence.get() * 0.85);
        return HybridAffectResult {
            final_reading: disambiguated,
            disambiguation_used: true,
            disambiguation_source: Some("hybrid_disambiguation".to_string()),
        };
    }

    // Low-confidence rule-based: call LLM detector for disambiguation.
    let llm_reading = detector.detect(user_message).await;
    let final_reading = if llm_reading.confidence > rule_based.confidence {
        llm_reading
    } else {
        let mut dampened = rule_based;
        dampened.confidence = Confidence::new(dampened.confidence.get() * 0.85);
        dampened
    };

    HybridAffectResult {
        final_reading,
        disambiguation_used: true,
        disambiguation_source: Some("llm_affect_detector".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use super::{HybridAffectResult, hybrid_detect};
    use crate::core::affect::{AffectDetector, AffectLabel, AffectReading, RuleBasedDetector};

    struct MockDetector {
        reading: AffectReading,
    }

    impl AffectDetector for MockDetector {
        fn detect<'a>(
            &'a self,
            _user_message: &'a str,
        ) -> Pin<Box<dyn Future<Output = AffectReading> + Send + 'a>> {
            Box::pin(async move { self.reading.clone() })
        }
    }

    #[tokio::test]
    async fn rule_based_confident_passes_through() {
        let message = "I am furious and angry about this terrible bug";
        let rule = RuleBasedDetector::new().detect(message);
        assert!(!rule.needs_disambiguation());

        let result = hybrid_detect(message, None).await;
        assert!(!result.disambiguation_used);
        assert!(result.disambiguation_source.is_none());
        assert_eq!(result.final_reading.label, rule.label);
    }

    #[test]
    fn ambiguous_reading_flagged_for_disambiguation() {
        let reading = AffectReading {
            label: AffectLabel::Confused,
            valence: -0.1,
            arousal: 0.3,
            dominance: 0.2,
            confidence: crate::contracts::scores::Confidence::new(0.4),
        };
        assert!(reading.needs_disambiguation());
    }

    #[test]
    fn mixed_signal_flagged() {
        let reading = AffectReading {
            label: AffectLabel::Neutral,
            valence: 0.05,
            arousal: 0.9,
            dominance: 0.5,
            confidence: crate::contracts::scores::Confidence::new(0.8),
        };
        assert!(reading.is_mixed_signal());
    }

    #[tokio::test]
    async fn hybrid_detect_without_detector_uses_rules() {
        let message = "THIS IS URGENT!!";
        let rule = RuleBasedDetector::new().detect(message);
        let result: HybridAffectResult = hybrid_detect(message, None).await;

        assert!(!result.disambiguation_used);
        assert!(result.disambiguation_source.is_none());
        assert_eq!(result.final_reading.label, rule.label);
        assert!(
            (result.final_reading.confidence.get() - rule.confidence.get()).abs() < f64::EPSILON
        );
    }

    #[tokio::test]
    async fn llm_enhanced_merges_high_confidence_result() {
        // "DO IT NOW!! MAKE IT WORK!!" produces:
        //   - High arousal (caps + punct + urgency) with neutral valence → mixed signal
        //   - Nearest prototype is Neutral at distance ~0.53 → confidence ~0.73 (< 0.8)
        let message = "DO IT NOW!! MAKE IT WORK!!";
        let rule = RuleBasedDetector::new().detect(message);
        assert!(
            rule.needs_disambiguation(),
            "test precondition: message must trigger disambiguation"
        );
        assert!(
            rule.confidence.get() < 0.8,
            "test precondition: rule confidence ({}) must be < 0.8",
            rule.confidence
        );

        let mock = MockDetector {
            reading: AffectReading {
                label: AffectLabel::Frustrated,
                valence: -0.6,
                arousal: 0.7,
                dominance: 0.3,
                confidence: crate::contracts::scores::Confidence::new(0.95),
            },
        };

        let result = hybrid_detect(message, Some(&mock)).await;
        assert!(result.disambiguation_used);
        assert_eq!(
            result.disambiguation_source.as_deref(),
            Some("llm_affect_detector")
        );
        // LLM confidence (0.95) > rule confidence, so LLM label wins
        assert_eq!(result.final_reading.label, AffectLabel::Frustrated);
        assert!((result.final_reading.confidence.get() - 0.95).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn backward_compat_none_detector_returns_rule_based() {
        let message = "maybe this could work somehow";
        let rule = RuleBasedDetector::new().detect(message);
        let result = hybrid_detect(message, None).await;

        // With None detector, should never use disambiguation
        assert!(!result.disambiguation_used);
        assert!(result.disambiguation_source.is_none());
        assert_eq!(result.final_reading.label, rule.label);
        assert!(
            (result.final_reading.confidence.get() - rule.confidence.get()).abs() < f64::EPSILON
        );
    }

    #[tokio::test]
    async fn fallback_when_detector_returns_low_confidence() {
        // Same precondition message as llm_enhanced test
        let message = "DO IT NOW!! MAKE IT WORK!!";
        let rule = RuleBasedDetector::new().detect(message);
        assert!(
            rule.needs_disambiguation(),
            "test precondition: message must trigger disambiguation"
        );
        assert!(
            rule.confidence.get() < 0.8,
            "test precondition: rule confidence ({}) must be < 0.8",
            rule.confidence
        );

        // Mock detector returns much lower confidence than rule-based
        let mock = MockDetector {
            reading: AffectReading {
                label: AffectLabel::Anxious,
                valence: -0.3,
                arousal: 0.4,
                dominance: 0.3,
                confidence: crate::contracts::scores::Confidence::new(rule.confidence.get() * 0.1),
            },
        };

        let result = hybrid_detect(message, Some(&mock)).await;
        assert!(result.disambiguation_used);
        // Rule-based wins since LLM confidence is lower
        assert_eq!(result.final_reading.label, rule.label);
        // Confidence should be dampened
        let expected = crate::contracts::scores::Confidence::new(rule.confidence.get() * 0.85);
        assert!(
            (result.final_reading.confidence.get() - expected.get()).abs() < f64::EPSILON,
            "expected dampened confidence {}, got {}",
            expected.get(),
            result.final_reading.confidence.get()
        );
    }
}
