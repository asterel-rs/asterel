//! Intent classifier for ML-based injection detection (L0 kernel layer).
//!
//! Provides an async `IntentClassifier` trait with two implementations:
//!
//! - `NoopClassifier` — always returns `None` (default when feature is off)
//! - `OrtIntentClassifier` — ONNX-based embedding + XGBoost pipeline
//!   (requires the `intent-classifier` feature gate)
//!
//! The classifier sits at L1.5 in the detection pipeline: pattern matching
//! runs first, and ML inference is only invoked when patterns return `Allow`.

mod traits;
mod types;

#[cfg(feature = "intent-classifier")]
mod classify;
#[cfg(feature = "intent-classifier")]
mod download;
#[cfg(feature = "intent-classifier")]
mod session;
#[cfg(feature = "intent-classifier")]
mod tokenizer;

#[cfg(test)]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use traits::{FailClosedClassifier, NoopClassifier};

pub use traits::IntentClassifier;

/// Factory function to create the appropriate intent classifier.
///
/// - When `enabled` is `false` or the `intent-classifier` feature is not
///   compiled in, returns a `NoopClassifier`.
/// - When enabled with the feature, attempts to load/download ONNX models
///   and returns an `OrtIntentClassifier`. Falls back to `NoopClassifier`
///   on any failure.
#[allow(clippy::unused_async)] // async required when intent-classifier feature is enabled
pub async fn create_intent_classifier(
    enabled: bool,
    _threshold: f32,
    models_dir: PathBuf,
    auto_download: bool,
) -> Arc<dyn IntentClassifier> {
    if !enabled {
        return Arc::new(NoopClassifier);
    }

    #[cfg(feature = "intent-classifier")]
    {
        match download::ensure_models(&models_dir, auto_download).await {
            Ok(true) => {
                let classifier = classify::OrtIntentClassifier::new(&models_dir);
                classifier.try_warmup();
                if classifier.is_ready() {
                    tracing::info!("intent classifier ready (ort)");
                    return Arc::new(classifier);
                }
                tracing::warn!("intent classifier warmup failed; enforcing fail-closed fallback");
            }
            Ok(false) => {
                tracing::warn!(
                    "intent classifier models not available; enforcing fail-closed fallback"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "intent classifier setup failed; enforcing fail-closed fallback"
                );
            }
        }
    }

    #[cfg(not(feature = "intent-classifier"))]
    {
        let _ = (models_dir, auto_download);
        tracing::warn!("intent-classifier feature not enabled; enforcing fail-closed fallback");
    }

    Arc::new(FailClosedClassifier)
}
