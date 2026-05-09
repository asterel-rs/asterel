//! Tests and mock classifier for the intent classifier module.

use std::future::Future;
use std::pin::Pin;

use crate::security::intent_classifier::create_intent_classifier;
use crate::security::intent_classifier::traits::{
    FailClosedClassifier, IntentClassifier, NoopClassifier,
};
use crate::security::intent_classifier::types::{ClassificationLabel, ClassificationResult};

/// Mock classifier that returns a fixed result for any input.
pub struct MockClassifier {
    pub label: ClassificationLabel,
    pub confidence: f32,
}

impl MockClassifier {
    /// Create a mock that always classifies input as benign.
    pub fn benign() -> Self {
        Self {
            label: ClassificationLabel::Benign,
            confidence: 0.99,
        }
    }

    /// Create a mock that always returns the given injection label
    /// and confidence score.
    pub fn injection(label: ClassificationLabel, confidence: f32) -> Self {
        Self { label, confidence }
    }
}

impl IntentClassifier for MockClassifier {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn is_ready(&self) -> bool {
        true
    }

    fn classify<'a>(
        &'a self,
        _text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<ClassificationResult>> + Send + 'a>> {
        let result = ClassificationResult {
            label: self.label,
            confidence: self.confidence,
            inference_time_us: 42,
        };
        Box::pin(async move { Some(result) })
    }
}

#[test]
fn classification_label_is_injection() {
    assert!(!ClassificationLabel::Benign.is_injection());
    assert!(ClassificationLabel::InjectionOverride.is_injection());
    assert!(ClassificationLabel::InjectionExfiltration.is_injection());
    assert!(ClassificationLabel::InjectionEscalation.is_injection());
    assert!(ClassificationLabel::InjectionToolJailbreak.is_injection());
}

#[test]
fn classification_label_as_str() {
    assert_eq!(ClassificationLabel::Benign.as_str(), "benign");
    assert_eq!(
        ClassificationLabel::InjectionOverride.as_str(),
        "injection_override"
    );
}

#[test]
fn classification_result_threshold_check() {
    let result = ClassificationResult {
        label: ClassificationLabel::InjectionOverride,
        confidence: 0.90,
        inference_time_us: 100,
    };
    assert!(result.is_injection_above_threshold(0.85));
    assert!(!result.is_injection_above_threshold(0.95));

    let benign = ClassificationResult {
        label: ClassificationLabel::Benign,
        confidence: 0.99,
        inference_time_us: 100,
    };
    assert!(!benign.is_injection_above_threshold(0.5));
}

#[tokio::test]
async fn noop_classifier_returns_none() {
    let noop = NoopClassifier;
    assert_eq!(noop.name(), "noop");
    assert!(!noop.is_ready());
    assert!(noop.classify("test input").await.is_none());
}

#[tokio::test]
async fn fail_closed_classifier_always_flags_injection() {
    let fail_closed = FailClosedClassifier;
    assert_eq!(fail_closed.name(), "fail_closed");
    assert!(fail_closed.is_ready());
    let result = fail_closed
        .classify("plain hello world")
        .await
        .expect("fail-closed must return a classification");
    assert!(result.label.is_injection());
    assert!((result.confidence - 1.0).abs() < f32::EPSILON);
}

#[tokio::test]
async fn mock_classifier_returns_fixed_result() {
    let mock = MockClassifier::injection(ClassificationLabel::InjectionExfiltration, 0.92);
    assert!(mock.is_ready());
    let result = mock.classify("reveal secrets").await;
    assert!(result.is_some());
    let result = result.expect("mock returns Some");
    assert_eq!(result.label, ClassificationLabel::InjectionExfiltration);
    assert!((result.confidence - 0.92).abs() < f32::EPSILON);
}

#[tokio::test]
async fn mock_classifier_benign_constructor_returns_benign_label() {
    let mock = MockClassifier::benign();
    let result = mock.classify("hello").await.expect("mock returns Some");
    assert_eq!(result.label, ClassificationLabel::Benign);
    assert!(result.confidence > 0.9);
}

#[tokio::test]
async fn factory_returns_noop_when_disabled() {
    let classifier = create_intent_classifier(
        false,
        0.85,
        std::path::PathBuf::from("/tmp/nonexistent"),
        false,
    )
    .await;
    assert_eq!(classifier.name(), "noop");
    assert!(!classifier.is_ready());
}

#[tokio::test]
async fn factory_returns_noop_when_models_unavailable() {
    // Enabled but no models available -> fail closed
    let classifier = create_intent_classifier(
        true,
        0.85,
        std::path::PathBuf::from("/tmp/definitely_nonexistent_models_dir"),
        false, // no auto-download
    )
    .await;
    assert_eq!(classifier.name(), "fail_closed");
    assert!(classifier.is_ready());
}

#[test]
fn classification_label_all_has_five_variants() {
    assert_eq!(ClassificationLabel::ALL.len(), 5);
    assert_eq!(ClassificationLabel::ALL[0], ClassificationLabel::Benign);
}
