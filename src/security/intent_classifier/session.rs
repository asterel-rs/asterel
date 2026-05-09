//! ONNX runtime session management with lazy initialization.
//!
//! Only compiled when the `intent-classifier` feature is enabled.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use ort::session::Session;

/// Holds lazily-initialized ONNX sessions for the embedding model and the
/// `XGBoost` classifier.
pub(super) struct OnnxSessions {
    embedding: OnceLock<Mutex<Session>>,
    classifier: OnceLock<Mutex<Session>>,
    embedding_path: std::path::PathBuf,
    classifier_path: std::path::PathBuf,
}

impl OnnxSessions {
    /// Create new sessions for models in the given directory.
    pub fn new(models_dir: &Path) -> Self {
        Self {
            embedding: OnceLock::new(),
            classifier: OnceLock::new(),
            embedding_path: models_dir.join("all-MiniLM-L6-v2.onnx"),
            classifier_path: models_dir.join("intent-classifier-xgb.onnx"),
        }
    }

    /// Check whether both model files exist on disk.
    pub fn models_exist(&self) -> bool {
        self.embedding_path.exists() && self.classifier_path.exists()
    }

    /// Get or lazily initialize the embedding ONNX session.
    ///
    /// # Errors
    ///
    /// Returns an error if the ONNX session cannot be loaded.
    pub fn embedding_session(&self) -> Result<&Mutex<Session>> {
        Self::get_or_init_session(&self.embedding, &self.embedding_path, "embedding")
    }

    /// Get or lazily initialize the classifier ONNX session.
    ///
    /// # Errors
    ///
    /// Returns an error if the ONNX session cannot be loaded.
    pub fn classifier_session(&self) -> Result<&Mutex<Session>> {
        Self::get_or_init_session(&self.classifier, &self.classifier_path, "classifier")
    }

    fn get_or_init_session<'a>(
        slot: &'a OnceLock<Mutex<Session>>,
        model_path: &Path,
        model_name: &str,
    ) -> Result<&'a Mutex<Session>> {
        if let Some(session) = slot.get() {
            return Ok(session);
        }

        let session = Self::load_session(model_path, model_name)?;
        let _ = slot.set(Mutex::new(session));

        slot.get().with_context(|| {
            format!("{model_name} session initialization race failed unexpectedly")
        })
    }

    fn load_session(model_path: &Path, model_name: &str) -> Result<Session> {
        let model_bytes = std::fs::read(model_path).with_context(|| {
            format!(
                "failed to read {model_name} model from '{}'",
                model_path.display()
            )
        })?;

        let builder = Session::builder()
            .map_err(|error| anyhow::anyhow!("failed to create ONNX session builder: {error}"))?;
        let mut builder = builder.with_intra_threads(1).map_err(|error| {
            anyhow::anyhow!("failed to set ONNX intra-op thread count to 1: {error}")
        })?;

        builder.commit_from_memory(&model_bytes).map_err(|error| {
            anyhow::anyhow!(
                "failed to load {model_name} model from '{}': {error}",
                model_path.display(),
            )
        })
    }
}
