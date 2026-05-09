//! Media subsystem configuration contracts shared between `media` and `config`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SttRuntimeConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TtsRuntimeConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub response_format: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    pub enabled: bool,
    pub storage_dir: Option<String>,
    pub max_file_size_mb: u64,
    #[serde(default)]
    pub stt: SttRuntimeConfig,
    #[serde(default)]
    pub tts: TtsRuntimeConfig,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            storage_dir: None,
            max_file_size_mb: 25,
            stt: SttRuntimeConfig::default(),
            tts: TtsRuntimeConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_config_default_matches_expected_values() {
        let cfg = MediaConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.storage_dir, None);
        assert_eq!(cfg.max_file_size_mb, 25);
    }

    #[test]
    fn stt_runtime_config_default_all_fields_none() {
        let stt = SttRuntimeConfig::default();
        assert_eq!(stt.enabled, None);
        assert_eq!(stt.provider, None);
        assert_eq!(stt.model, None);
        assert_eq!(stt.endpoint, None);
        assert_eq!(stt.language, None);
        assert_eq!(stt.prompt, None);
        assert_eq!(stt.api_key, None);
    }

    #[test]
    fn tts_runtime_config_default_all_fields_none() {
        let tts = TtsRuntimeConfig::default();
        assert_eq!(tts.enabled, None);
        assert_eq!(tts.provider, None);
        assert_eq!(tts.model, None);
        assert_eq!(tts.voice, None);
        assert_eq!(tts.response_format, None);
        assert_eq!(tts.endpoint, None);
        assert_eq!(tts.api_key, None);
    }

    #[test]
    fn media_config_serde_roundtrip() {
        let cfg = MediaConfig {
            enabled: true,
            storage_dir: Some("/tmp/media".to_string()),
            max_file_size_mb: 64,
            stt: SttRuntimeConfig {
                enabled: Some(true),
                provider: Some("openai".to_string()),
                model: Some("gpt-4o-mini-transcribe".to_string()),
                endpoint: Some("https://stt.example.com".to_string()),
                language: Some("en".to_string()),
                prompt: Some("transcribe clearly".to_string()),
                api_key: Some("secret".to_string()),
            },
            tts: TtsRuntimeConfig {
                enabled: Some(true),
                provider: Some("openai".to_string()),
                model: Some("gpt-4o-mini-tts".to_string()),
                voice: Some("alloy".to_string()),
                response_format: Some("mp3".to_string()),
                endpoint: Some("https://tts.example.com".to_string()),
                api_key: Some("secret".to_string()),
            },
        };

        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: MediaConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.enabled, cfg.enabled);
        assert_eq!(parsed.storage_dir, cfg.storage_dir);
        assert_eq!(parsed.max_file_size_mb, cfg.max_file_size_mb);
        assert_eq!(parsed.stt.provider, cfg.stt.provider);
        assert_eq!(parsed.tts.voice, cfg.tts.voice);
    }
}
