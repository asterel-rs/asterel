use super::super::traits::MediaAttachment;

/// Generates a bracketed placeholder description for an attachment that
/// could not be processed (e.g. `[Attachment: doc.pdf (application/pdf)]`).
pub(crate) fn fallback_attachment_description(
    attachment: &MediaAttachment,
    size_bytes: Option<usize>,
) -> String {
    let filename = attachment.filename.as_deref().unwrap_or("unnamed");
    let size_part = size_bytes
        .map(|bytes| format!(", {}KB", bytes.div_ceil(1024)))
        .unwrap_or_default();
    format!(
        "[Attachment: {filename} ({}{size_part})]",
        attachment.mime_type
    )
}
