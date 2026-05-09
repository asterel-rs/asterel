//! Media processing pipeline (STT, TTS, vision).
//!
//! Provides [`MediaProcessor`] which delegates to speech-to-text,
//! text-to-speech, and vision description backends.

mod prompt_media;
mod stt;
#[cfg(test)]
mod tests;
mod tts;
mod vision;

use std::sync::Arc;

use anyhow::Result;
pub(crate) use prompt_media::describe_media_for_prompt;
pub(crate) use stt::SttConfig;
pub(crate) use tts::TtsConfig;

use crate::core::providers::Provider;
use crate::media::types::{MediaFile, MediaType};

/// Delegates media description to STT, TTS, or vision backends.
#[derive(Clone)]
pub(crate) struct MediaProcessor {
    provider: Option<Arc<dyn Provider>>,
    model: Option<String>,
    stt: Option<SttConfig>,
    tts: Option<TtsConfig>,
}

impl MediaProcessor {
    /// Create a processor with no provider or speech config.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self {
            provider: None,
            model: None,
            stt: None,
            tts: None,
        }
    }

    /// Create a processor with a vision-capable LLM provider.
    #[must_use]
    pub(crate) fn with_provider(provider: Arc<dyn Provider>, model: String) -> Self {
        Self {
            provider: Some(provider),
            model: Some(model),
            stt: None,
            tts: None,
        }
    }

    /// Attach speech-to-text configuration (builder pattern).
    #[must_use]
    pub(crate) fn with_stt_config(mut self, config: SttConfig) -> Self {
        self.stt = Some(config);
        self
    }

    /// Attach text-to-speech configuration (builder pattern).
    #[must_use]
    pub(crate) fn with_tts_config(mut self, config: TtsConfig) -> Self {
        self.tts = Some(config);
        self
    }

    /// Produce a human-readable description of a media file.
    ///
    /// Dispatches to vision, STT, or metadata-based fallback
    /// depending on the file's [`MediaType`].
    ///
    /// # Errors
    ///
    /// Currently infallible; always returns a best-effort
    /// description.
    pub(crate) async fn describe(&self, file: &MediaFile, data: &[u8]) -> Result<String> {
        let description = match file.media_type {
            MediaType::Image => self.describe_image(file, data).await,
            MediaType::Audio => self.transcribe_audio(file, data).await,
            MediaType::Video => "[Video content - processing not yet supported]".into(),
            MediaType::Document => Self::describe_document(file, data),
            MediaType::Unknown => "[Unknown media type]".into(),
        };
        Ok(description)
    }
}

impl Default for MediaProcessor {
    fn default() -> Self {
        Self::new()
    }
}
