//! Speech-to-text transcription via `OpenAI` Whisper or Groq.
//!
//! Configures provider-specific endpoints, models, and optional
//! language/prompt hints, then sends audio for transcription.

use anyhow::{Context, Result};
use serde::Deserialize;

use super::MediaProcessor;
use crate::media::types::MediaFile;
use crate::security::scrub::sanitize_api_error;

const OPENAI_TRANSCRIPTION_URL: &str = "https://api.openai.com/v1/audio/transcriptions";
const GROQ_TRANSCRIPTION_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const OPENAI_STT_MODEL: &str = "whisper-1";
const GROQ_STT_MODEL: &str = "whisper-large-v3-turbo";

/// Supported speech-to-text API providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SttProvider {
    /// `OpenAI` Whisper API.
    OpenAi,
    /// Groq-hosted Whisper endpoint.
    Groq,
    /// Any OpenAI-compatible transcription endpoint.
    OpenAiCompatible,
}

/// Configuration for a speech-to-text transcription backend.
#[derive(Debug, Clone)]
pub(crate) struct SttConfig {
    /// Which STT provider to use.
    pub(super) provider: SttProvider,
    /// Bearer token for the STT API.
    pub(super) api_key: String,
    /// Transcription model identifier.
    pub(super) model: String,
    /// API endpoint URL.
    pub(super) endpoint: String,
    /// Optional ISO 639-1 language hint.
    pub(super) language: Option<String>,
    /// Optional domain vocabulary prompt hint.
    pub(super) prompt: Option<String>,
}

impl SttConfig {
    /// Create config for the `OpenAI` Whisper transcription API.
    #[must_use]
    pub(crate) fn openai(api_key: impl Into<String>) -> Self {
        Self {
            provider: SttProvider::OpenAi,
            api_key: api_key.into(),
            model: OPENAI_STT_MODEL.to_string(),
            endpoint: OPENAI_TRANSCRIPTION_URL.to_string(),
            language: None,
            prompt: None,
        }
    }

    /// Create config for the Groq Whisper transcription API.
    #[must_use]
    pub(crate) fn groq(api_key: impl Into<String>) -> Self {
        Self {
            provider: SttProvider::Groq,
            api_key: api_key.into(),
            model: GROQ_STT_MODEL.to_string(),
            endpoint: GROQ_TRANSCRIPTION_URL.to_string(),
            language: None,
            prompt: None,
        }
    }

    /// Create config for an OpenAI-compatible transcription
    /// endpoint.
    #[must_use]
    pub(crate) fn openai_compatible(
        api_key: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> Self {
        Self {
            provider: SttProvider::OpenAiCompatible,
            api_key: api_key.into(),
            model: OPENAI_STT_MODEL.to_string(),
            endpoint: endpoint.into(),
            language: None,
            prompt: None,
        }
    }

    /// Override the transcription model.
    #[must_use]
    pub(crate) fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Override the transcription API endpoint.
    #[must_use]
    pub(crate) fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Set an ISO 639-1 language hint for transcription.
    #[must_use]
    pub(crate) fn with_language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Set a domain vocabulary prompt hint for the model.
    #[must_use]
    pub(crate) fn with_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }
}

#[derive(Debug, Deserialize)]
/// JSON response from a Whisper-compatible transcription API.
pub(super) struct TranscriptionResponse {
    /// The transcribed text.
    pub(super) text: String,
}

impl MediaProcessor {
    /// Transcribe audio data, falling back to metadata on failure.
    pub(super) async fn transcribe_audio(&self, file: &MediaFile, data: &[u8]) -> String {
        if let Some(config) = &self.stt {
            match self.transcribe_audio_with_config(file, data, config).await {
                Ok(text) => {
                    let transcript = text.trim();
                    if !transcript.is_empty() {
                        let filename = file.filename.as_deref().unwrap_or("unnamed");
                        return format!("[Audio transcript: {filename}] {transcript}");
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        filename = ?file.filename,
                        mime_type = %file.mime_type,
                        provider = ?config.provider,
                        error = %error,
                        "audio transcription failed; using audio metadata fallback"
                    );
                }
            }
        }

        let duration_secs = estimate_audio_duration_secs(file.size_bytes, &file.mime_type);
        let duration = format_duration(duration_secs);
        format!(
            "[Audio: {} ({}, {} bytes, ~{} estimated)]",
            file.filename.as_deref().unwrap_or("unnamed"),
            file.mime_type,
            file.size_bytes,
            duration,
        )
    }

    async fn transcribe_audio_with_config(
        &self,
        file: &MediaFile,
        data: &[u8],
        config: &SttConfig,
    ) -> Result<String> {
        if data.is_empty() {
            anyhow::bail!("audio payload is empty");
        }

        let file_name = file
            .filename
            .as_deref()
            .unwrap_or(match config.provider {
                SttProvider::OpenAi => "audio.wav",
                SttProvider::Groq | SttProvider::OpenAiCompatible => "audio.ogg",
            })
            .to_string();
        let part = reqwest::multipart::Part::bytes(data.to_vec())
            .file_name(file_name)
            .mime_str(&file.mime_type)
            .with_context(|| format!("invalid MIME type '{}'", file.mime_type))?;
        let mut form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("model", config.model.clone())
            .text("response_format", "json");
        if let Some(language) = config
            .language
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            form = form.text("language", language.to_string());
        }
        if let Some(prompt) = config
            .prompt
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            form = form.text("prompt", prompt.to_string());
        }
        let client = crate::utils::http::try_build_http_client()
            .context("failed to build HTTP client for STT")?;
        let response = client
            .post(&config.endpoint)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .multipart(form)
            .send()
            .await
            .context("send STT request")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let sanitized = sanitize_api_error(&body);
            anyhow::bail!("STT request failed ({status}): {sanitized}");
        }

        let parsed: TranscriptionResponse = response
            .json()
            .await
            .context("parse STT response payload")?;
        Ok(parsed.text)
    }
}

/// Estimate audio duration from file size using per-format
/// byte-rate heuristics.
pub(super) fn estimate_audio_duration_secs(size_bytes: u64, mime_type: &str) -> u64 {
    let bytes_per_second = match mime_type {
        "audio/wav" | "audio/x-wav" | "audio/wave" => 176_400,
        _ => 16_000,
    };

    size_bytes / bytes_per_second
}

/// Format a duration in seconds as a human-readable string
/// (e.g. `45s` or `2m 30s`).
pub(super) fn format_duration(duration_secs: u64) -> String {
    if duration_secs < 60 {
        return format!("{duration_secs}s");
    }

    let minutes = duration_secs / 60;
    let seconds = duration_secs % 60;
    format!("{minutes}m {seconds}s")
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::media::types::{MediaFile, MediaType};

    fn audio_file() -> MediaFile {
        MediaFile {
            id: "id-1".to_string(),
            mime_type: "audio/ogg".to_string(),
            media_type: MediaType::Audio,
            filename: Some("voice.ogg".to_string()),
            size_bytes: 8,
            storage_path: "media/voice.ogg".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[tokio::test]
    async fn stt_provider_error_body_is_sanitized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(
                ResponseTemplate::new(500)
                    .set_body_string("provider rejected api_key=sk-leaked-stt-token"),
            )
            .mount(&server)
            .await;

        let config = SttConfig::openai("sk-client-token")
            .with_endpoint(format!("{}/audio/transcriptions", server.uri()));
        let processor = MediaProcessor::new();

        let error = processor
            .transcribe_audio_with_config(&audio_file(), b"FAKEAUDIO", &config)
            .await
            .expect_err("STT provider failure should return an error")
            .to_string();

        assert!(error.contains("STT request failed"));
        assert!(error.contains("[REDACTED]"));
        assert!(!error.contains("sk-leaked-stt-token"));
    }
}
