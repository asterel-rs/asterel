//! MIME type detection from file bytes and extensions.
//!
//! Uses the `infer` crate for magic-byte detection when the `media`
//! feature is enabled, with a fallback extension-to-MIME lookup.

use super::types::MediaType;

/// Detect MIME type from file magic bytes using the `infer` crate.
#[must_use]
#[cfg(feature = "media")]
pub(crate) fn detect_mime(data: &[u8]) -> Option<String> {
    infer::get(data).map(|info| info.mime_type().to_string())
}

/// Stub MIME detection when the `media` feature is disabled.
#[must_use]
#[cfg(not(feature = "media"))]
pub fn detect_mime(_data: &[u8]) -> Option<String> {
    None
}

/// Map a filename extension to its MIME type. Returns `None` for
/// unrecognized extensions.
#[must_use]
pub(crate) fn detect_mime_ext(filename: &str) -> Option<String> {
    let ext = filename.rsplit('.').next()?;
    // If there was no dot, rsplit returns the entire filename — not an extension.
    if ext == filename {
        return None;
    }
    match ext.to_lowercase().as_str() {
        "jpg" | "jpeg" => Some("image/jpeg".into()),
        "png" => Some("image/png".into()),
        "gif" => Some("image/gif".into()),
        "webp" => Some("image/webp".into()),
        "mp3" => Some("audio/mpeg".into()),
        "wav" => Some("audio/wav".into()),
        "ogg" => Some("audio/ogg".into()),
        "mp4" => Some("video/mp4".into()),
        "webm" => Some("video/webm".into()),
        "pdf" => Some("application/pdf".into()),
        _ => None,
    }
}

/// Detect MIME type and [`MediaType`] from raw bytes and an optional
/// filename, trying magic-byte detection first and falling back to
/// extension lookup.
#[must_use]
pub(crate) fn detect_media_type(data: &[u8], filename: Option<&str>) -> (String, MediaType) {
    let mime = detect_mime(data)
        .or_else(|| filename.and_then(detect_mime_ext))
        .unwrap_or_else(|| "application/octet-stream".into());
    let media_type = MediaType::from_mime(&mime);
    (mime, media_type)
}

#[cfg(test)]
mod tests {
    use super::{detect_media_type, detect_mime, detect_mime_ext};
    use crate::media::types::MediaType;

    #[cfg(feature = "media")]
    #[test]
    fn detect_mime_png_magic_bytes() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00];
        assert_eq!(detect_mime(&png).as_deref(), Some("image/png"));
    }

    #[cfg(feature = "media")]
    #[test]
    fn detect_mime_jpeg_magic_bytes() {
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F'];
        assert_eq!(detect_mime(&jpeg).as_deref(), Some("image/jpeg"));
    }

    #[test]
    fn detect_mime_unknown_returns_none() {
        let unknown = [0x00, 0x11, 0x22, 0x33, 0x44];
        assert!(detect_mime(&unknown).is_none());
    }

    #[test]
    fn detect_mime_from_extension_common_types() {
        assert_eq!(detect_mime_ext("photo.JPG").as_deref(), Some("image/jpeg"));
        assert_eq!(detect_mime_ext("clip.webm").as_deref(), Some("video/webm"));
        assert_eq!(detect_mime_ext("voice.mp3").as_deref(), Some("audio/mpeg"));
        assert_eq!(
            detect_mime_ext("report.pdf").as_deref(),
            Some("application/pdf")
        );
    }

    #[test]
    fn detect_mime_from_extension_handles_unusual_or_missing_extensions() {
        assert_eq!(detect_mime_ext("archive.tar.gz"), None);
        assert_eq!(detect_mime_ext("README"), None);
        assert_eq!(detect_mime_ext(".env"), None);
    }

    #[test]
    fn detect_media_type_empty_input_defaults_to_octet_stream() {
        let (mime, media_type) = detect_media_type(&[], None);
        assert_eq!(mime, "application/octet-stream");
        assert_eq!(media_type, MediaType::Unknown);
    }

    #[test]
    fn detect_media_type_binary_payload_without_known_extension_is_unknown() {
        let binary = [0x00, 0xFF, 0x01, 0xFE, 0x02, 0xFD];
        let (mime, media_type) = detect_media_type(&binary, Some("blob.bin"));
        assert_eq!(mime, "application/octet-stream");
        assert_eq!(media_type, MediaType::Unknown);
    }

    #[test]
    fn detect_media_type_combines_magic_and_extension_fallback() {
        let unknown = [0x00, 0x11, 0x22, 0x33];
        let (mime, media_type) = detect_media_type(&unknown, Some("image.png"));
        assert_eq!(mime, "image/png");
        assert_eq!(media_type, MediaType::Image);

        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00];
        let (mime_from_bytes, media_type_from_bytes) = detect_media_type(&png, Some("file.bin"));
        #[cfg(feature = "media")]
        assert_eq!(mime_from_bytes, "image/png");
        #[cfg(not(feature = "media"))]
        assert_eq!(mime_from_bytes, "application/octet-stream");
        #[cfg(feature = "media")]
        assert_eq!(media_type_from_bytes, MediaType::Image);
        #[cfg(not(feature = "media"))]
        assert_eq!(media_type_from_bytes, MediaType::Unknown);
    }

    #[test]
    fn media_type_from_mime_covers_categories() {
        assert_eq!(MediaType::from_mime("image/webp"), MediaType::Image);
        assert_eq!(MediaType::from_mime("audio/wav"), MediaType::Audio);
        assert_eq!(MediaType::from_mime("video/mp4"), MediaType::Video);
        assert_eq!(MediaType::from_mime("text/markdown"), MediaType::Document);
        assert_eq!(
            MediaType::from_mime("application/octet-stream"),
            MediaType::Unknown
        );
    }
}
