//! Media processor construction for channel sessions: resolves STT/TTS
//! provider configs from the runtime configuration and model selection.
use std::sync::Arc;

use super::super::startup::ChannelRuntime;
use super::super::traits::ChannelMessage;
use crate::config::Config;
use crate::contracts::providers::normalize_provider_alias;
use crate::media::{MediaProcessor, SttConfig, TtsConfig};

/// Builds a `MediaProcessor` configured with STT/TTS settings from the
/// channel runtime.
pub(super) fn media_processor_for_runtime(rt: &ChannelRuntime) -> MediaProcessor {
    let mut processor = MediaProcessor::with_provider(Arc::clone(&rt.provider), rt.model.clone());
    let selection = rt.config.resolve_model(None, None);
    let provider = selection.provider.to_ascii_lowercase();
    let api_base = normalized_owned(selection.api_base.as_deref());
    let api_key = selection.api_key.or_else(|| rt.config.api_key.clone());

    if let Some(stt_config) = resolve_stt_runtime_config(
        &rt.config,
        &provider,
        api_key.as_deref(),
        api_base.as_deref(),
    ) {
        processor = processor.with_stt_config(stt_config);
    }
    if let Some(tts_config) = resolve_tts_runtime_config(
        &rt.config,
        &provider,
        api_key.as_deref(),
        api_base.as_deref(),
    ) {
        processor = processor.with_tts_config(tts_config);
    }

    processor
}

/// Resolves the speech-to-text configuration from user config, falling
/// back to the default provider and API key when not explicitly set.
pub(super) fn resolve_stt_runtime_config(
    config: &Config,
    default_provider: &str,
    default_api_key: Option<&str>,
    default_api_base: Option<&str>,
) -> Option<SttConfig> {
    let default_provider = canonical_media_provider(default_provider);
    let enabled_by_default_provider = matches!(default_provider.as_str(), "openai" | "groq")
        || is_openai_compatible_provider(&default_provider);
    let enabled = config
        .media
        .stt
        .enabled
        .unwrap_or(enabled_by_default_provider);
    if !enabled {
        return None;
    }

    let provider = normalized_or_default(config.media.stt.provider.as_deref(), &default_provider);
    let api_key = normalized_owned(config.media.stt.api_key.as_deref())
        .or_else(|| default_api_key.map(ToOwned::to_owned))?;
    let endpoint_override = normalized_owned(config.media.stt.endpoint.as_deref());
    let custom_endpoint = endpoint_override.clone().or_else(|| {
        custom_endpoint_from_provider(&provider).map(|endpoint| {
            openai_audio_endpoint(
                &endpoint,
                OPENAI_COMPAT_STT_PATH,
                OPENAI_COMPAT_STT_FALLBACK,
            )
        })
    });
    let inferred_endpoint = custom_endpoint.or_else(|| {
        default_api_base.map(|base| {
            openai_audio_endpoint(base, OPENAI_COMPAT_STT_PATH, OPENAI_COMPAT_STT_FALLBACK)
        })
    });

    let mut stt = match provider.as_str() {
        "openai" => SttConfig::openai(api_key),
        "groq" => SttConfig::groq(api_key),
        token if is_openai_compatible_provider(token) => {
            let Some(endpoint) = inferred_endpoint else {
                tracing::warn!(
                    provider = provider.as_str(),
                    "STT custom/openai-compatible provider requires endpoint; disabling STT"
                );
                return None;
            };
            SttConfig::openai_compatible(api_key, endpoint)
        }
        _ => {
            tracing::warn!(
                provider = provider.as_str(),
                "unsupported STT provider configured; disabling STT"
            );
            return None;
        }
    };

    if let Some(model) = normalized(config.media.stt.model.as_deref()) {
        stt = stt.with_model(model);
    }
    if let Some(endpoint) = endpoint_override.as_deref() {
        stt = stt.with_endpoint(endpoint);
    }
    if let Some(language) = normalized(config.media.stt.language.as_deref()) {
        stt = stt.with_language(language);
    }
    if let Some(prompt) = normalized(config.media.stt.prompt.as_deref()) {
        stt = stt.with_prompt(prompt);
    }

    Some(stt)
}

/// Resolves the text-to-speech configuration from user config, falling
/// back to the default provider and API key when not explicitly set.
pub(super) fn resolve_tts_runtime_config(
    config: &Config,
    default_provider: &str,
    default_api_key: Option<&str>,
    default_api_base: Option<&str>,
) -> Option<TtsConfig> {
    let default_provider = canonical_media_provider(default_provider);
    let enabled_by_default_provider =
        default_provider == "openai" || is_openai_compatible_provider(&default_provider);
    let enabled = config
        .media
        .tts
        .enabled
        .unwrap_or(enabled_by_default_provider);
    if !enabled {
        return None;
    }

    let provider_fallback =
        if default_provider == "openai" || is_openai_compatible_provider(&default_provider) {
            default_provider.as_str()
        } else {
            "openai"
        };
    let provider = normalized_or_default(config.media.tts.provider.as_deref(), provider_fallback);
    let api_key = normalized_owned(config.media.tts.api_key.as_deref())
        .or_else(|| default_api_key.map(ToOwned::to_owned))?;
    let endpoint_override = normalized_owned(config.media.tts.endpoint.as_deref());
    let custom_endpoint = endpoint_override.clone().or_else(|| {
        custom_endpoint_from_provider(&provider).map(|endpoint| {
            openai_audio_endpoint(
                &endpoint,
                OPENAI_COMPAT_TTS_PATH,
                OPENAI_COMPAT_TTS_FALLBACK,
            )
        })
    });
    let inferred_endpoint = custom_endpoint.or_else(|| {
        default_api_base.map(|base| {
            openai_audio_endpoint(base, OPENAI_COMPAT_TTS_PATH, OPENAI_COMPAT_TTS_FALLBACK)
        })
    });

    let mut tts = match provider.as_str() {
        "openai" => TtsConfig::openai(api_key),
        token if is_openai_compatible_provider(token) => {
            let Some(endpoint) = inferred_endpoint else {
                tracing::warn!(
                    provider = provider.as_str(),
                    "TTS custom/openai-compatible provider requires endpoint; disabling TTS"
                );
                return None;
            };
            TtsConfig::openai(api_key).with_endpoint(endpoint)
        }
        _ => {
            tracing::warn!(
                provider = provider.as_str(),
                "unsupported TTS provider configured; disabling TTS"
            );
            return None;
        }
    };

    if let Some(model) = normalized(config.media.tts.model.as_deref()) {
        tts = tts.with_model(model);
    }
    if let Some(voice) = normalized(config.media.tts.voice.as_deref()) {
        tts = tts.with_voice(voice);
    }
    if let Some(response_format) = normalized(config.media.tts.response_format.as_deref()) {
        tts = tts.with_response_format(response_format);
    }
    if let Some(endpoint) = endpoint_override.as_deref() {
        tts = tts.with_endpoint(endpoint);
    }

    Some(tts)
}

fn normalized(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn normalized_owned(value: Option<&str>) -> Option<String> {
    normalized(value).map(ToString::to_string)
}

fn normalized_or_default(value: Option<&str>, fallback: &str) -> String {
    normalized(value).unwrap_or(fallback).to_ascii_lowercase()
}

fn canonical_media_provider(provider: &str) -> String {
    normalize_provider_alias(provider).to_ascii_lowercase()
}

const OPENAI_COMPAT_STT_PATH: &str = "audio/transcriptions";
const OPENAI_COMPAT_TTS_PATH: &str = "audio/speech";
const OPENAI_COMPAT_STT_FALLBACK: &str = "https://api.openai.com/v1/audio/transcriptions";
const OPENAI_COMPAT_TTS_FALLBACK: &str = "https://api.openai.com/v1/audio/speech";

/// Builds an OpenAI-compatible audio endpoint URL, appending the path
/// segment when the base does not already contain `/audio/`.
pub(super) fn openai_audio_endpoint(base_or_endpoint: &str, path: &str, fallback: &str) -> String {
    let trimmed = base_or_endpoint.trim();
    if trimmed.is_empty() {
        return fallback.to_string();
    }

    let normalized = trimmed.trim_end_matches('/');
    if normalized.contains("/audio/") {
        return normalized.to_string();
    }

    format!("{normalized}/{path}")
}

/// Returns `true` if the provider string denotes an OpenAI-compatible
/// or custom endpoint.
pub(super) fn is_openai_compatible_provider(provider: &str) -> bool {
    provider.starts_with("custom:")
}

fn custom_endpoint_from_provider(provider: &str) -> Option<String> {
    provider
        .strip_prefix("custom:")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

/// Returns `true` if the message contains any audio attachments.
pub(super) fn has_audio_input(msg: &ChannelMessage) -> bool {
    msg.attachments
        .iter()
        .any(|attachment| attachment.mime_type.starts_with("audio/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::channels::traits::{MediaAttachment, MediaContent};

    fn config_with_provider_and_key(provider: &str, api_key: &str) -> crate::config::Config {
        crate::config::Config {
            default_provider: Some(provider.to_string()),
            api_key: Some(api_key.to_string()),
            ..crate::config::Config::default()
        }
    }

    fn config_with_key(api_key: &str) -> crate::config::Config {
        crate::config::Config {
            api_key: Some(api_key.to_string()),
            ..crate::config::Config::default()
        }
    }

    fn discord_message_with_attachments() -> ChannelMessage {
        ChannelMessage {
            id: "msg-1".to_string(),
            sender: "user-42".to_string(),
            content: "hello from discord".to_string(),
            channel: "discord".to_string(),
            context_hint: None,
            conversation_id: Some("channel-77".to_string()),
            thread_id: Some("thread-9".to_string()),
            reply_to: None,
            message_id: Some("discord-msg-abc".to_string()),
            timestamp: 1_716_171_717,
            attachments: vec![
                MediaAttachment {
                    mime_type: "image/png".to_string(),
                    data: MediaContent::Url("https://cdn.discord.test/img.png".to_string()),
                    filename: Some("img.png".to_string()),
                },
                MediaAttachment {
                    mime_type: "application/pdf".to_string(),
                    data: MediaContent::Url("https://cdn.discord.test/doc.pdf".to_string()),
                    filename: Some("doc.pdf".to_string()),
                },
            ],
        }
    }

    #[test]
    fn has_audio_input_detects_audio_attachments() {
        let mut msg = discord_message_with_attachments();
        assert!(!has_audio_input(&msg));

        msg.attachments.push(MediaAttachment {
            mime_type: "audio/ogg".to_string(),
            data: MediaContent::Bytes(vec![0x4f, 0x67, 0x67, 0x53]),
            filename: Some("voice.ogg".to_string()),
        });
        assert!(has_audio_input(&msg));
    }

    #[test]
    fn stt_runtime_config_uses_default_openai_when_enabled_by_fallback() {
        let config = config_with_provider_and_key("openai", "sk-default");

        let stt = resolve_stt_runtime_config(&config, "openai", config.api_key.as_deref(), None);
        assert!(stt.is_some());
    }

    #[test]
    fn stt_runtime_config_treats_openai_codex_as_openai_backend() {
        let config = config_with_provider_and_key("openai-codex", "sk-default");

        let stt =
            resolve_stt_runtime_config(&config, "openai-codex", config.api_key.as_deref(), None);
        assert!(stt.is_some());
    }

    #[test]
    fn stt_runtime_config_can_be_explicitly_disabled() {
        let mut config = config_with_provider_and_key("openai", "sk-default");
        config.media.stt.enabled = Some(false);

        let stt = resolve_stt_runtime_config(&config, "openai", config.api_key.as_deref(), None);
        assert!(stt.is_none());
    }

    #[test]
    fn stt_runtime_config_supports_custom_provider_with_endpoint() {
        let mut config = config_with_key("sk-default");
        config.media.stt.enabled = Some(true);
        config.media.stt.provider = Some("custom:https://voice.example".to_string());
        config.media.stt.endpoint = Some("https://voice.example/v1/transcriptions".to_string());
        config.media.stt.language = Some("ja".to_string());
        config.media.stt.prompt = Some("domain terms".to_string());

        let stt = resolve_stt_runtime_config(&config, "openai", config.api_key.as_deref(), None);
        assert!(stt.is_some());
    }

    #[test]
    fn stt_runtime_config_rejects_custom_provider_without_url() {
        let mut config = config_with_key("sk-default");
        config.media.stt.enabled = Some(true);
        config.media.stt.provider = Some("custom:".to_string());

        let stt = resolve_stt_runtime_config(&config, "openai", config.api_key.as_deref(), None);
        assert!(stt.is_none());
    }

    #[test]
    fn stt_runtime_config_supports_openai_compatible_default_provider_from_api_base() {
        let config = config_with_key("sk-default");

        let stt = resolve_stt_runtime_config(
            &config,
            "custom:https://proxy.example.com/v1",
            config.api_key.as_deref(),
            Some("https://proxy.example.com/v1"),
        );
        assert!(stt.is_some());
    }

    #[test]
    fn tts_runtime_config_defaults_to_none_for_non_openai_provider() {
        let config = config_with_provider_and_key("groq", "gsk-default");

        let tts = resolve_tts_runtime_config(&config, "groq", config.api_key.as_deref(), None);
        assert!(tts.is_none());
    }

    #[test]
    fn tts_runtime_config_treats_openai_codex_as_openai_backend() {
        let config = config_with_provider_and_key("openai-codex", "sk-default");

        let tts =
            resolve_tts_runtime_config(&config, "openai-codex", config.api_key.as_deref(), None);
        assert!(tts.is_some());
    }

    #[test]
    fn tts_runtime_config_can_use_explicit_openai_key_with_non_openai_default() {
        let mut config = config_with_provider_and_key("groq", "gsk-default");
        config.media.tts.enabled = Some(true);
        config.media.tts.provider = Some("openai".to_string());
        config.media.tts.api_key = Some("sk-openai".to_string());

        let tts = resolve_tts_runtime_config(&config, "groq", config.api_key.as_deref(), None);
        assert!(tts.is_some());
    }

    #[test]
    fn tts_runtime_config_supports_custom_provider_with_endpoint() {
        let mut config = config_with_key("sk-default");
        config.media.tts.enabled = Some(true);
        config.media.tts.provider = Some("custom:https://voice.example".to_string());
        config.media.tts.endpoint = Some("https://voice.example/v1/speech".to_string());
        config.media.tts.response_format = Some("mp3".to_string());

        let tts = resolve_tts_runtime_config(&config, "openai", config.api_key.as_deref(), None);
        assert!(tts.is_some());
    }

    #[test]
    fn tts_runtime_config_rejects_custom_provider_without_url() {
        let mut config = config_with_key("sk-default");
        config.media.tts.enabled = Some(true);
        config.media.tts.provider = Some("custom:".to_string());

        let tts = resolve_tts_runtime_config(&config, "openai", config.api_key.as_deref(), None);
        assert!(tts.is_none());
    }

    #[test]
    fn tts_runtime_config_rejects_unknown_provider() {
        let mut config = config_with_provider_and_key("openai", "sk-default");
        config.media.tts.enabled = Some(true);
        config.media.tts.provider = Some("mystery".to_string());

        let tts = resolve_tts_runtime_config(&config, "openai", config.api_key.as_deref(), None);
        assert!(tts.is_none());
    }

    #[test]
    fn tts_runtime_config_supports_openai_compatible_default_provider_from_api_base() {
        let config = config_with_key("sk-default");

        let tts = resolve_tts_runtime_config(
            &config,
            "custom:https://proxy.example.com/v1",
            config.api_key.as_deref(),
            Some("https://proxy.example.com/v1"),
        );
        assert!(tts.is_some());
    }

    #[test]
    fn openai_audio_endpoint_appends_path_for_api_base() {
        assert_eq!(
            openai_audio_endpoint(
                "https://proxy.example.com/v1",
                "audio/transcriptions",
                "https://api.openai.com/v1/audio/transcriptions"
            ),
            "https://proxy.example.com/v1/audio/transcriptions"
        );
    }

    #[test]
    fn openai_audio_endpoint_keeps_full_audio_endpoint() {
        assert_eq!(
            openai_audio_endpoint(
                "https://proxy.example.com/v1/audio/speech",
                "audio/speech",
                "https://api.openai.com/v1/audio/speech"
            ),
            "https://proxy.example.com/v1/audio/speech"
        );
    }
}
