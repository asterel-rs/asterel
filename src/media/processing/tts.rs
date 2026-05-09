//! Text-to-speech synthesis via `OpenAI` TTS API.
//!
//! Sends text to the configured TTS endpoint and returns the
//! synthesized audio bytes with metadata (MIME type, duration).

use anyhow::{Context, Result};

use super::MediaProcessor;
use crate::security::scrub::sanitize_api_error;

const OPENAI_TTS_URL: &str = "https://api.openai.com/v1/audio/speech";
const OPENAI_TTS_MODEL: &str = "gpt-4o-mini-tts";
const OPENAI_TTS_VOICE: &str = "alloy";
const OPENAI_TTS_RESPONSE_FORMAT: &str = "opus";

/// Configuration for a text-to-speech synthesis backend.
#[derive(Debug, Clone)]
pub(crate) struct TtsConfig {
    /// Bearer token for the TTS API.
    pub(super) api_key: String,
    /// TTS model identifier.
    pub(super) model: String,
    /// Voice preset name.
    pub(super) voice: String,
    /// Audio output format (e.g. `opus`, `mp3`).
    pub(super) response_format: String,
    /// API endpoint URL.
    pub(super) endpoint: String,
}

impl TtsConfig {
    /// Create config for the `OpenAI` TTS API with default voice
    /// and format.
    #[must_use]
    pub(crate) fn openai(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: OPENAI_TTS_MODEL.to_string(),
            voice: OPENAI_TTS_VOICE.to_string(),
            response_format: OPENAI_TTS_RESPONSE_FORMAT.to_string(),
            endpoint: OPENAI_TTS_URL.to_string(),
        }
    }

    /// Override the TTS model.
    #[must_use]
    pub(crate) fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Override the voice preset.
    #[must_use]
    pub(crate) fn with_voice(mut self, voice: impl Into<String>) -> Self {
        self.voice = voice.into();
        self
    }

    /// Override the audio output format (e.g. `mp3`, `wav`).
    #[must_use]
    pub(crate) fn with_response_format(mut self, response_format: impl Into<String>) -> Self {
        self.response_format = response_format.into();
        self
    }

    /// Override the TTS API endpoint URL.
    #[must_use]
    pub(crate) fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }
}

/// Result of a text-to-speech synthesis request.
#[derive(Debug, Clone)]
pub(crate) struct SynthesizedSpeech {
    /// Raw audio bytes.
    pub bytes: Vec<u8>,
    /// MIME type of the audio (e.g. `audio/ogg`).
    pub mime_type: String,
    /// Suggested filename (e.g. `voice.ogg`).
    pub filename: String,
}

impl MediaProcessor {
    /// # Errors
    ///
    /// Returns an error when the TTS request cannot be sent, when the provider
    /// returns a non-success HTTP status, or when the audio response cannot be
    /// read.
    pub(crate) async fn synthesize_speech(&self, text: &str) -> Result<Option<SynthesizedSpeech>> {
        let Some(config) = &self.tts else {
            return Ok(None);
        };

        if text.trim().is_empty() {
            return Ok(None);
        }

        let response_format = normalize_tts_response_format(&config.response_format);
        let payload = serde_json::json!({
            "model": config.model,
            "input": text,
            "voice": config.voice,
            "response_format": response_format,
        });
        let client = crate::utils::http::try_build_http_client()
            .context("failed to build HTTP client for TTS")?;
        let response = client
            .post(&config.endpoint)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await
            .context("send TTS request")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let sanitized = sanitize_api_error(&body);
            anyhow::bail!("TTS request failed ({status}): {sanitized}");
        }

        let content_type_header = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(ToString::to_string);
        let (mime_type, filename) =
            synthesized_audio_metadata(content_type_header.as_deref(), &response_format);

        let bytes = response
            .bytes()
            .await
            .context("read TTS audio response")?
            .to_vec();
        if bytes.is_empty() {
            return Ok(None);
        }

        Ok(Some(SynthesizedSpeech {
            bytes,
            mime_type: mime_type.to_string(),
            filename: filename.to_string(),
        }))
    }
}

/// Normalize a TTS response format string to its canonical form.
pub(super) fn normalize_tts_response_format(response_format: &str) -> String {
    let normalized = response_format.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "opus" | "ogg" => "opus".to_string(),
        "mp3" | "mpeg" => "mp3".to_string(),
        "wav" | "wave" => "wav".to_string(),
        "flac" => "flac".to_string(),
        "aac" => "aac".to_string(),
        _ => OPENAI_TTS_RESPONSE_FORMAT.to_string(),
    }
}

/// Determine MIME type and filename for synthesized audio from
/// the HTTP content-type header or the requested response format.
pub(super) fn synthesized_audio_metadata(
    content_type_header: Option<&str>,
    response_format: &str,
) -> (&'static str, &'static str) {
    if let Some(content_type) = content_type_header
        && let Some((mime_type, filename)) = audio_metadata_from_content_type(content_type)
    {
        return (mime_type, filename);
    }

    match response_format {
        "mp3" => ("audio/mpeg", "voice.mp3"),
        "wav" => ("audio/wav", "voice.wav"),
        "flac" => ("audio/flac", "voice.flac"),
        "aac" => ("audio/aac", "voice.aac"),
        _ => ("audio/ogg", "voice.ogg"),
    }
}

fn audio_metadata_from_content_type(content_type: &str) -> Option<(&'static str, &'static str)> {
    let mime = content_type
        .split(';')
        .next()
        .map_or("", str::trim)
        .to_ascii_lowercase();
    match mime.as_str() {
        "audio/ogg" | "audio/opus" => Some(("audio/ogg", "voice.ogg")),
        "audio/mpeg" | "audio/mp3" => Some(("audio/mpeg", "voice.mp3")),
        "audio/wav" | "audio/x-wav" => Some(("audio/wav", "voice.wav")),
        "audio/flac" => Some(("audio/flac", "voice.flac")),
        "audio/aac" => Some(("audio/aac", "voice.aac")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    #[tokio::test]
    async fn tts_provider_error_body_is_sanitized() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/speech"))
            .respond_with(
                ResponseTemplate::new(500)
                    .set_body_string("provider rejected api_key=sk-leaked-tts-token"),
            )
            .mount(&server)
            .await;

        let processor = MediaProcessor::new().with_tts_config(
            TtsConfig::openai("sk-client-token")
                .with_endpoint(format!("{}/audio/speech", server.uri())),
        );

        let error = processor
            .synthesize_speech("hello")
            .await
            .expect_err("TTS provider failure should return an error")
            .to_string();

        assert!(error.contains("TTS request failed"));
        assert!(error.contains("[REDACTED]"));
        assert!(!error.contains("sk-leaked-tts-token"));
    }
}
