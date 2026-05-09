//! Media domain types: file metadata, media categories, and config.

use serde::{Deserialize, Serialize};

pub use crate::contracts::media::{MediaConfig, SttRuntimeConfig, TtsRuntimeConfig};

fn _speech_config_marker(
    stt: Option<SttRuntimeConfig>,
    tts: Option<TtsRuntimeConfig>,
) -> Option<(SttRuntimeConfig, TtsRuntimeConfig)> {
    stt.zip(tts)
}

/// High-level category of a media file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MediaType {
    /// Raster or vector image (PNG, JPEG, GIF, WebP, etc.).
    Image,
    /// Audio recording or music (MP3, WAV, OGG, etc.).
    Audio,
    /// Video clip or stream (MP4, `WebM`, etc.).
    Video,
    /// Text or PDF document.
    Document,
    /// Unrecognized media category.
    Unknown,
}

impl MediaType {
    /// Classify a MIME type string into a [`MediaType`] variant.
    #[must_use]
    pub(crate) fn from_mime(mime: &str) -> Self {
        if mime.starts_with("image/") {
            Self::Image
        } else if mime.starts_with("audio/") {
            Self::Audio
        } else if mime.starts_with("video/") {
            Self::Video
        } else if mime.starts_with("application/pdf") || mime.starts_with("text/") {
            Self::Document
        } else {
            Self::Unknown
        }
    }

    /// Return the lowercase string representation of this variant.
    #[must_use]
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Document => "document",
            Self::Unknown => "unknown",
        }
    }

    /// Parse a kind string (e.g. from a database row) into a
    /// [`MediaType`].
    #[must_use]
    pub(crate) fn from_kind(kind: &str) -> Self {
        match kind {
            "image" => Self::Image,
            "audio" => Self::Audio,
            "video" => Self::Video,
            "document" => Self::Document,
            _ => Self::Unknown,
        }
    }
}

/// Metadata for a stored media file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MediaFile {
    /// Unique identifier (UUID).
    pub id: String,
    /// MIME type (e.g. `image/png`).
    pub mime_type: String,
    /// High-level media category.
    pub media_type: MediaType,
    /// Original filename, if known.
    pub filename: Option<String>,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Absolute path on disk where the file is stored.
    pub storage_path: String,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
}

/// A media file together with its raw byte content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredMedia {
    /// File metadata.
    pub file: MediaFile,
    /// Raw file bytes.
    pub data: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::{MediaConfig, MediaType, SttRuntimeConfig, TtsRuntimeConfig};

    #[test]
    fn media_config_defaults_match_expected_values() {
        let config = MediaConfig::default();
        assert!(!config.enabled);
        assert!(config.storage_dir.is_none());
        assert_eq!(config.max_file_size_mb, 25);
        assert_eq!(config.stt.enabled, None);
        assert_eq!(config.tts.enabled, None);
    }

    #[test]
    fn media_config_speech_blocks_round_trip_in_toml() {
        let config = MediaConfig {
            enabled: true,
            storage_dir: Some("/tmp/media".to_string()),
            max_file_size_mb: 32,
            stt: SttRuntimeConfig {
                enabled: Some(true),
                provider: Some("openai".to_string()),
                model: Some("whisper-1".to_string()),
                endpoint: Some("https://api.openai.com/v1/audio/transcriptions".to_string()),
                language: Some("ja".to_string()),
                prompt: Some("domain vocabulary".to_string()),
                api_key: Some("stt-key".to_string()),
            },
            tts: TtsRuntimeConfig {
                enabled: Some(true),
                provider: Some("openai".to_string()),
                model: Some("gpt-4o-mini-tts".to_string()),
                voice: Some("alloy".to_string()),
                response_format: Some("opus".to_string()),
                endpoint: Some("https://api.openai.com/v1/audio/speech".to_string()),
                api_key: Some("tts-key".to_string()),
            },
        };

        let serialized = toml::to_string(&config).expect("serialize media config");
        let deserialized: MediaConfig =
            toml::from_str(&serialized).expect("deserialize media config");

        assert!(deserialized.enabled);
        assert_eq!(deserialized.max_file_size_mb, 32);
        assert_eq!(deserialized.stt.enabled, Some(true));
        assert_eq!(deserialized.stt.language.as_deref(), Some("ja"));
        assert_eq!(
            deserialized.stt.prompt.as_deref(),
            Some("domain vocabulary")
        );
        assert_eq!(deserialized.tts.enabled, Some(true));
        assert_eq!(deserialized.tts.voice.as_deref(), Some("alloy"));
        assert_eq!(deserialized.tts.response_format.as_deref(), Some("opus"));
    }

    #[test]
    fn media_type_from_mime_maps_all_variants() {
        assert_eq!(MediaType::from_mime("image/png"), MediaType::Image);
        assert_eq!(MediaType::from_mime("audio/mpeg"), MediaType::Audio);
        assert_eq!(MediaType::from_mime("video/mp4"), MediaType::Video);
        assert_eq!(MediaType::from_mime("application/pdf"), MediaType::Document);
        assert_eq!(MediaType::from_mime("text/plain"), MediaType::Document);
        assert_eq!(
            MediaType::from_mime("application/octet-stream"),
            MediaType::Unknown
        );
    }
}
