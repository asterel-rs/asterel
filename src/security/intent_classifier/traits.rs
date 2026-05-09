//! Trait definition for intent classifiers and the noop fallback.

use std::future::Future;
use std::pin::Pin;

use super::types::{ClassificationLabel, ClassificationResult};

/// Async trait for intent classification of text inputs.
///
/// Implementations must be `Send + Sync` for use in concurrent contexts.
pub trait IntentClassifier: Send + Sync {
    /// Human-readable name for logging / diagnostics.
    fn name(&self) -> &'static str;

    /// Returns `true` when the classifier has loaded models and is ready for
    /// inference. Returns `false` if models are unavailable or download failed.
    fn is_ready(&self) -> bool;

    /// Classify the given text input.
    ///
    /// Returns `None` if the classifier is not ready or inference fails.
    fn classify<'a>(
        &'a self,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<ClassificationResult>> + Send + 'a>>;
}

/// No-op classifier that always returns `None`.
///
/// Used as the default fallback when the `intent-classifier` feature is
/// disabled or models are unavailable.
pub struct NoopClassifier;

impl IntentClassifier for NoopClassifier {
    fn name(&self) -> &'static str {
        "noop"
    }

    fn is_ready(&self) -> bool {
        false
    }

    fn classify<'a>(
        &'a self,
        _text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<ClassificationResult>> + Send + 'a>> {
        Box::pin(async { None })
    }
}

/// Fail-closed classifier used when ML classification is explicitly enabled
/// but model bootstrap fails.
///
/// This classifier always emits a high-confidence injection label so
/// downstream policy escalates external content to sanitization.
pub struct FailClosedClassifier;

impl IntentClassifier for FailClosedClassifier {
    fn name(&self) -> &'static str {
        "fail_closed"
    }

    fn is_ready(&self) -> bool {
        true
    }

    fn classify<'a>(
        &'a self,
        _text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<ClassificationResult>> + Send + 'a>> {
        Box::pin(async {
            Some(ClassificationResult {
                label: ClassificationLabel::InjectionToolJailbreak,
                confidence: 1.0,
                inference_time_us: 0,
            })
        })
    }
}
