//! Tokenizer handle for the sentence-transformer embedding model.
//!
//! Only compiled when the `intent-classifier` feature is enabled.

use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow};

const MAX_TOKENS: usize = 128;

/// Thread-safe handle wrapping a `HuggingFace` `tokenizers::Tokenizer`.
pub(super) struct TokenizerHandle {
    inner: OnceLock<tokenizers::Tokenizer>,
    tokenizer_path: std::path::PathBuf,
}

impl TokenizerHandle {
    /// Create a handle pointing to the tokenizer file in the models dir.
    pub fn new(models_dir: &Path) -> Self {
        Self {
            inner: OnceLock::new(),
            tokenizer_path: models_dir.join("tokenizer.json"),
        }
    }

    /// Check whether the tokenizer file exists on disk.
    pub fn exists(&self) -> bool {
        self.tokenizer_path.exists()
    }

    fn get_or_load(&self) -> Result<&tokenizers::Tokenizer> {
        if let Some(tokenizer) = self.inner.get() {
            return Ok(tokenizer);
        }

        let tokenizer = Self::load_tokenizer(&self.tokenizer_path)?;
        let _ = self.inner.set(tokenizer);

        self.inner
            .get()
            .context("tokenizer initialization race failed unexpectedly")
    }

    fn load_tokenizer(path: &Path) -> Result<tokenizers::Tokenizer> {
        let mut tokenizer = tokenizers::Tokenizer::from_file(path)
            .map_err(|e| anyhow!("failed to load tokenizer from '{}': {e}", path.display()))?;

        tokenizer
            .with_truncation(Some(tokenizers::TruncationParams {
                max_length: MAX_TOKENS,
                ..Default::default()
            }))
            .map_err(|e| anyhow!("failed to set truncation: {e}"))?;
        tokenizer.with_padding(Some(tokenizers::PaddingParams {
            strategy: tokenizers::PaddingStrategy::Fixed(MAX_TOKENS),
            ..Default::default()
        }));

        Ok(tokenizer)
    }

    /// Encode text for embedding inference.
    ///
    /// Returns `(input_ids, attention_mask)` as `Vec<i64>`.
    pub fn encode_for_embedding(&self, text: &str) -> Result<(Vec<i64>, Vec<i64>)> {
        let tokenizer = self.get_or_load()?;
        let encoding = tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("tokenization failed: {e}"))?;

        let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| i64::from(id)).collect();
        let attention_mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|&m| i64::from(m))
            .collect();

        Ok((input_ids, attention_mask))
    }
}
