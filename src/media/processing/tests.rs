//! Unit tests for the media processing pipeline.

use std::future::Future;
use std::pin::Pin;

use anyhow::anyhow;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::stt::{estimate_audio_duration_secs, format_duration};
use super::tts::{normalize_tts_response_format, synthesized_audio_metadata};
use super::vision::IMAGE_DESCRIPTION_PROMPT;
use super::*;
use crate::core::providers::ProviderResult;
use crate::core::providers::response::{
    ContentBlock, ImageSource, MessageRole, ProviderMessage, ProviderResponse,
};
use crate::core::tools::traits::ToolSpec;
use crate::media::types::{MediaFile, MediaType};
use crate::utils::encoding::encode_base64;

#[derive(Debug, Clone, Copy)]
enum VisionMode {
    Success,
    EmptyText,
    Error,
}

type VisionCall = (Option<String>, String, f64, usize);

struct MockVisionProvider {
    supports_vision: bool,
    supported_models: Option<Vec<String>>,
    mode: VisionMode,
    calls: std::sync::Mutex<Vec<VisionCall>>,
}

impl MockVisionProvider {
    fn new(supports_vision: bool, mode: VisionMode) -> Self {
        Self {
            supports_vision,
            supported_models: None,
            mode,
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn with_supported_models(
        supports_vision: bool,
        mode: VisionMode,
        supported_models: Vec<&str>,
    ) -> Self {
        Self {
            supports_vision,
            supported_models: Some(
                supported_models
                    .into_iter()
                    .map(ToString::to_string)
                    .collect(),
            ),
            mode,
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    fn first_call(&self) -> VisionCall {
        self.calls.lock().unwrap()[0].clone()
    }
}

impl Provider for MockVisionProvider {
    fn chat_with_system<'a>(
        &'a self,
        _system_prompt: Option<&'a str>,
        _message: &'a str,
        _model: &'a str,
        _temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move { Ok("unused".to_string()) })
    }

    fn chat_with_tools<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push((
                system_prompt.map(ToString::to_string),
                model.to_string(),
                temperature,
                tools.len(),
            ));

            assert_eq!(messages.len(), 1);
            assert!(matches!(messages[0].role, MessageRole::User));
            assert!(matches!(&messages[0].content[0], ContentBlock::Text { .. }));
            assert!(matches!(
                &messages[0].content[1],
                ContentBlock::Image { .. }
            ));

            if let ContentBlock::Image { source } = &messages[0].content[1] {
                match source {
                    ImageSource::Base64 { media_type, data } => {
                        assert_eq!(media_type, "image/png");
                        assert_eq!(data, "AQID");
                    }
                    ImageSource::Url { .. } => panic!("expected base64 image source"),
                }
            }

            match self.mode {
                VisionMode::Success => Ok(ProviderResponse::text_only(
                    "A small test image with three bytes.".to_string(),
                )),
                VisionMode::EmptyText => Ok(ProviderResponse {
                    text: "   ".to_string(),
                    input_tokens: None,
                    output_tokens: None,
                    model: None,
                    content_blocks: vec![],
                    stop_reason: None,
                    logprobs: None,
                }),
                VisionMode::Error => Err(anyhow!("vision provider failed").into()),
            }
        })
    }

    fn supports_vision(&self) -> bool {
        self.supports_vision
    }

    fn supports_vision_model(&self, model: &str) -> bool {
        self.supported_models
            .as_ref()
            .map_or(self.supports_vision, |models| {
                models.iter().any(|candidate| candidate == model)
            })
    }
}

fn test_media_file(media_type: MediaType, mime_type: &str, size_bytes: u64) -> MediaFile {
    MediaFile {
        id: "id-1".to_string(),
        mime_type: mime_type.to_string(),
        media_type,
        filename: Some("asset.bin".to_string()),
        size_bytes,
        storage_path: "media/asset.bin".to_string(),
        created_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

#[tokio::test]
async fn describe_image_without_provider_uses_metadata() {
    let processor = MediaProcessor::new();
    let file = test_media_file(MediaType::Image, "image/png", 3);

    let description = processor.describe(&file, &[1, 2, 3]).await.unwrap();

    assert_eq!(description, "[Image: asset.bin (image/png, 3 bytes)]");
}

#[tokio::test]
async fn describe_audio_without_provider_uses_metadata_with_duration() {
    let processor = MediaProcessor::new();
    let file = test_media_file(MediaType::Audio, "audio/mpeg", 16_000);

    let description = processor.describe(&file, &[]).await.unwrap();

    assert_eq!(
        description,
        "[Audio: asset.bin (audio/mpeg, 16000 bytes, ~1s estimated)]"
    );
}

#[tokio::test]
async fn describe_audio_with_stt_config_uses_transcription() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "text": "hello from voice input"
        })))
        .mount(&server)
        .await;

    let processor = MediaProcessor::new().with_stt_config(
        SttConfig::groq("sk-test").with_endpoint(format!("{}/audio/transcriptions", server.uri())),
    );
    let file = test_media_file(MediaType::Audio, "audio/ogg", 8);

    let description = processor.describe(&file, b"FAKEAUDIO").await.unwrap();
    assert_eq!(
        description,
        "[Audio transcript: asset.bin] hello from voice input"
    );
}

#[tokio::test]
async fn describe_audio_with_stt_language_and_prompt_sends_optional_fields() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/audio/transcriptions"))
        .and(body_string_contains("name=\"language\""))
        .and(body_string_contains("ja"))
        .and(body_string_contains("name=\"prompt\""))
        .and(body_string_contains("domain glossary"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "text": "language tuned transcript"
        })))
        .mount(&server)
        .await;

    let processor = MediaProcessor::new().with_stt_config(
        SttConfig::openai("sk-test")
            .with_endpoint(format!("{}/audio/transcriptions", server.uri()))
            .with_language("ja")
            .with_prompt("domain glossary"),
    );
    let file = test_media_file(MediaType::Audio, "audio/ogg", 16);

    let description = processor.describe(&file, b"FAKEAUDIO").await.unwrap();
    assert_eq!(
        description,
        "[Audio transcript: asset.bin] language tuned transcript"
    );
}

#[tokio::test]
async fn describe_audio_stt_failure_falls_back_to_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let processor = MediaProcessor::new().with_stt_config(
        SttConfig::openai("sk-test")
            .with_endpoint(format!("{}/audio/transcriptions", server.uri())),
    );
    let file = test_media_file(MediaType::Audio, "audio/mpeg", 16_000);

    let description = processor.describe(&file, b"FAKEAUDIO").await.unwrap();
    assert_eq!(
        description,
        "[Audio: asset.bin (audio/mpeg, 16000 bytes, ~1s estimated)]"
    );
}

#[tokio::test]
async fn describe_image_with_provider_uses_vision_output() {
    let provider = Arc::new(MockVisionProvider::new(true, VisionMode::Success));
    let provider_for_assert = Arc::clone(&provider);
    let processor = MediaProcessor::with_provider(provider, "vision-model".to_string());
    let file = test_media_file(MediaType::Image, "image/png", 3);

    let description = processor.describe(&file, &[1, 2, 3]).await.unwrap();

    assert_eq!(description, "A small test image with three bytes.");
    assert_eq!(provider_for_assert.call_count(), 1);
    assert_eq!(
        provider_for_assert.first_call(),
        (
            Some(IMAGE_DESCRIPTION_PROMPT.to_string()),
            "vision-model".to_string(),
            0.2,
            0,
        )
    );
}

#[tokio::test]
async fn describe_image_skips_provider_when_vision_not_supported() {
    let provider = Arc::new(MockVisionProvider::new(false, VisionMode::Success));
    let provider_for_assert = Arc::clone(&provider);
    let processor = MediaProcessor::with_provider(provider, "vision-model".to_string());
    let file = test_media_file(MediaType::Image, "image/png", 3);

    let description = processor.describe(&file, &[1, 2, 3]).await.unwrap();

    assert_eq!(description, "[Image: asset.bin (image/png, 3 bytes)]");
    assert_eq!(provider_for_assert.call_count(), 0);
}

#[tokio::test]
async fn describe_image_skips_provider_when_model_policy_rejects_vision() {
    let provider = Arc::new(MockVisionProvider::with_supported_models(
        true,
        VisionMode::Success,
        vec!["vision-model"],
    ));
    let provider_for_assert = Arc::clone(&provider);
    let processor = MediaProcessor::with_provider(provider, "text-only-model".to_string());
    let file = test_media_file(MediaType::Image, "image/png", 3);

    let description = processor.describe(&file, &[1, 2, 3]).await.unwrap();

    assert_eq!(description, "[Image: asset.bin (image/png, 3 bytes)]");
    assert_eq!(provider_for_assert.call_count(), 0);
}

#[tokio::test]
async fn describe_image_falls_back_when_provider_errors() {
    let provider = Arc::new(MockVisionProvider::new(true, VisionMode::Error));
    let processor = MediaProcessor::with_provider(provider, "vision-model".to_string());
    let file = test_media_file(MediaType::Image, "image/png", 3);

    let description = processor.describe(&file, &[1, 2, 3]).await.unwrap();

    assert_eq!(description, "[Image: asset.bin (image/png, 3 bytes)]");
}

#[tokio::test]
async fn describe_image_falls_back_when_provider_returns_empty_text() {
    let provider = Arc::new(MockVisionProvider::new(true, VisionMode::EmptyText));
    let processor = MediaProcessor::with_provider(provider, "vision-model".to_string());
    let file = test_media_file(MediaType::Image, "image/png", 3);

    let description = processor.describe(&file, &[1, 2, 3]).await.unwrap();

    assert_eq!(description, "[Image: asset.bin (image/png, 3 bytes)]");
}

#[tokio::test]
async fn describe_document_text_includes_preview() {
    let processor = MediaProcessor::new();
    let file = test_media_file(MediaType::Document, "text/plain", 11);

    let description = processor.describe(&file, b"hello world").await.unwrap();

    assert_eq!(
        description,
        "[Document: asset.bin (text/plain, 11 bytes)] Preview: hello world"
    );
}

#[tokio::test]
async fn describe_document_text_truncates_preview_to_500_chars() {
    let processor = MediaProcessor::new();
    let long_text = "a".repeat(600);
    let file = test_media_file(MediaType::Document, "text/plain", 600);

    let description = processor
        .describe(&file, long_text.as_bytes())
        .await
        .unwrap();

    assert!(description.ends_with(&"a".repeat(500)));
    assert!(!description.ends_with(&"a".repeat(501)));
}

#[tokio::test]
async fn describe_document_pdf_returns_pdf_metadata() {
    let processor = MediaProcessor::new();
    let file = test_media_file(MediaType::Document, "application/pdf", 42);

    let description = processor.describe(&file, b"%PDF-1.7").await.unwrap();

    assert_eq!(description, "[PDF document: asset.bin (42 bytes)]");
}

#[tokio::test]
async fn describe_document_other_type_returns_generic_metadata() {
    let processor = MediaProcessor::new();
    let file = test_media_file(MediaType::Document, "application/msword", 84);

    let description = processor.describe(&file, &[0, 1, 2]).await.unwrap();

    assert_eq!(
        description,
        "[Document: asset.bin (application/msword, 84 bytes)]"
    );
}

#[tokio::test]
async fn describe_document_invalid_utf8_falls_back_to_metadata() {
    let processor = MediaProcessor::new();
    let file = test_media_file(MediaType::Document, "text/plain", 3);

    let description = processor
        .describe(&file, &[0xFF, 0xFE, 0xFD])
        .await
        .unwrap();

    assert_eq!(description, "[Document: asset.bin (text/plain, 3 bytes)]");
}

#[tokio::test]
async fn describe_unknown_type_still_reports_unknown() {
    let processor = MediaProcessor::new();
    let file = test_media_file(MediaType::Unknown, "application/octet-stream", 8);

    let description = processor.describe(&file, &[0; 8]).await.unwrap();

    assert_eq!(description, "[Unknown media type]");
}

#[tokio::test]
async fn synthesize_speech_with_tts_config_returns_audio_payload() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/audio/speech"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "audio/ogg")
                .set_body_bytes(vec![0x4f, 0x67, 0x67]),
        )
        .mount(&server)
        .await;

    let processor = MediaProcessor::new().with_tts_config(
        TtsConfig::openai("sk-test").with_endpoint(format!("{}/audio/speech", server.uri())),
    );

    let speech = processor
        .synthesize_speech("hello world")
        .await
        .unwrap()
        .expect("tts should return synthesized payload");
    assert_eq!(speech.mime_type, "audio/ogg");
    assert_eq!(speech.filename, "voice.ogg");
    assert_eq!(speech.bytes, vec![0x4f, 0x67, 0x67]);
}

#[tokio::test]
async fn synthesize_speech_uses_requested_response_format_and_mime() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/audio/speech"))
        .and(body_string_contains("\"response_format\":\"mp3\""))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "audio/mpeg")
                .set_body_bytes(vec![0x49, 0x44, 0x33]),
        )
        .mount(&server)
        .await;

    let processor = MediaProcessor::new().with_tts_config(
        TtsConfig::openai("sk-test")
            .with_endpoint(format!("{}/audio/speech", server.uri()))
            .with_response_format("mp3"),
    );

    let speech = processor
        .synthesize_speech("hello mp3")
        .await
        .unwrap()
        .expect("tts should return synthesized payload");
    assert_eq!(speech.mime_type, "audio/mpeg");
    assert_eq!(speech.filename, "voice.mp3");
    assert_eq!(speech.bytes, vec![0x49, 0x44, 0x33]);
}

#[tokio::test]
async fn synthesize_speech_falls_back_to_response_format_when_header_unknown() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/audio/speech"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/octet-stream")
                .set_body_bytes(vec![0x52, 0x49, 0x46, 0x46]),
        )
        .mount(&server)
        .await;

    let processor = MediaProcessor::new().with_tts_config(
        TtsConfig::openai("sk-test")
            .with_endpoint(format!("{}/audio/speech", server.uri()))
            .with_response_format("wav"),
    );

    let speech = processor
        .synthesize_speech("hello wav")
        .await
        .unwrap()
        .expect("tts should return synthesized payload");
    assert_eq!(speech.mime_type, "audio/wav");
    assert_eq!(speech.filename, "voice.wav");
    assert_eq!(speech.bytes, vec![0x52, 0x49, 0x46, 0x46]);
}

#[tokio::test]
async fn synthesize_speech_without_tts_config_returns_none() {
    let processor = MediaProcessor::new();
    let speech = processor.synthesize_speech("hello world").await.unwrap();
    assert!(speech.is_none());
}

#[test]
fn estimate_audio_duration_uses_mp3_heuristic() {
    assert_eq!(estimate_audio_duration_secs(32_000, "audio/mpeg"), 2);
}

#[test]
fn estimate_audio_duration_uses_wav_heuristic() {
    assert_eq!(estimate_audio_duration_secs(352_800, "audio/wav"), 2);
}

#[test]
fn estimate_audio_duration_uses_ogg_heuristic() {
    assert_eq!(estimate_audio_duration_secs(48_000, "audio/ogg"), 3);
}

#[test]
fn estimate_audio_duration_uses_default_heuristic() {
    assert_eq!(estimate_audio_duration_secs(16_000, "audio/flac"), 1);
}

#[test]
fn format_duration_seconds_only() {
    assert_eq!(format_duration(45), "45s");
}

#[test]
fn format_duration_minutes_and_seconds() {
    assert_eq!(format_duration(150), "2m 30s");
}

#[test]
fn normalize_tts_response_format_maps_aliases_and_defaults() {
    assert_eq!(normalize_tts_response_format("opus"), "opus");
    assert_eq!(normalize_tts_response_format("ogg"), "opus");
    assert_eq!(normalize_tts_response_format("mp3"), "mp3");
    assert_eq!(normalize_tts_response_format("wave"), "wav");
    assert_eq!(normalize_tts_response_format("unknown"), "opus");
}

#[test]
fn synthesized_audio_metadata_prefers_content_type_and_then_format() {
    assert_eq!(
        synthesized_audio_metadata(Some("audio/mpeg; charset=binary"), "opus"),
        ("audio/mpeg", "voice.mp3")
    );
    assert_eq!(
        synthesized_audio_metadata(Some("application/octet-stream"), "flac"),
        ("audio/flac", "voice.flac")
    );
}

#[test]
fn encode_base64_matches_expected_output() {
    assert_eq!(encode_base64(&[1, 2, 3]), "AQID");
}
