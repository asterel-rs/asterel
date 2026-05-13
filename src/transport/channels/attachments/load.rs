use anyhow::{Result, bail};
use futures_util::StreamExt;
use reqwest::StatusCode;
use std::time::Duration;

use super::super::traits::{MediaAttachment, MediaContent};
use crate::core::tools::{AttachmentSource, OutputAttachment};

/// Maximum attachment size (25 MiB) — prevents memory exhaustion from oversized
/// downloads or file reads before the `MediaStore` layer can enforce its own limit.
pub(crate) const MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;
const ATTACHMENT_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

fn max_attachment_bytes_usize() -> usize {
    usize::try_from(MAX_ATTACHMENT_BYTES).expect("attachment byte limit must fit usize")
}

pub(crate) fn is_redirect_status(status: StatusCode) -> bool {
    status.is_redirection()
}

/// Loads attachment bytes, downloading from URL if needed with SSRF
/// validation and size-gating.
///
/// # Errors
///
/// Returns an error if the URL is blocked, the download fails, or the
/// attachment exceeds the size limit.
pub(crate) async fn load_attachment_bytes(attachment: &MediaAttachment) -> Result<Vec<u8>> {
    match &attachment.data {
        MediaContent::Bytes(bytes) => Ok(bytes.clone()),
        MediaContent::Url(url) => {
            let parsed_url = crate::security::validate_fetch_url(url, false).await?;
            let client = crate::utils::http::try_build_pinned_public_fetch_client_with(
                &parsed_url,
                reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(ATTACHMENT_DOWNLOAD_TIMEOUT),
            )
            .await?;
            let response = client.get(parsed_url).send().await?;
            if is_redirect_status(response.status()) {
                bail!(
                    "attachment redirects are not followed; redirect targets must be validated explicitly"
                );
            }
            let response = response.error_for_status()?;
            if let Some(content_length) = response.content_length()
                && content_length > MAX_ATTACHMENT_BYTES
            {
                bail!(
                    "attachment too large: Content-Length {content_length} \
                     exceeds limit of {MAX_ATTACHMENT_BYTES} bytes"
                );
            }

            collect_attachment_body(response).await
        }
    }
}

pub(crate) async fn collect_attachment_body(response: reqwest::Response) -> Result<Vec<u8>> {
    let initial_capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(8192)
        .min(usize::try_from(MAX_ATTACHMENT_BYTES).unwrap_or(usize::MAX));
    let mut body = Vec::with_capacity(initial_capacity);
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len().saturating_add(chunk.len()) > max_attachment_bytes_usize() {
            let attempted = body.len().saturating_add(chunk.len());
            bail!(
                "attachment too large: {attempted} bytes exceeds limit of {MAX_ATTACHMENT_BYTES} bytes"
            );
        }
        body.extend_from_slice(&chunk);
    }

    Ok(body)
}

#[cfg(test)]
pub(crate) async fn collect_attachment_chunks_for_test<I>(chunks: I) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = Vec<u8>>,
{
    let mut body = Vec::new();
    for chunk in chunks {
        if body.len().saturating_add(chunk.len()) > max_attachment_bytes_usize() {
            let attempted = body.len().saturating_add(chunk.len());
            bail!(
                "attachment too large: {attempted} bytes exceeds limit of {MAX_ATTACHMENT_BYTES} bytes"
            );
        }
        body.extend_from_slice(&chunk);
    }

    Ok(body)
}

pub(crate) async fn output_attachment_to_media_attachment(
    attachment: &OutputAttachment,
) -> Option<MediaAttachment> {
    match &attachment.source {
        AttachmentSource::File { path } => read_output_attachment_file(path, attachment).await,
        AttachmentSource::Url { url } => map_output_attachment_url(url, attachment).await,
    }
}

async fn read_output_attachment_file(
    path: &str,
    attachment: &OutputAttachment,
) -> Option<MediaAttachment> {
    match tokio::fs::metadata(path).await {
        Ok(meta) if meta.len() > MAX_ATTACHMENT_BYTES => {
            tracing::warn!(
                path = %path,
                size = meta.len(),
                limit = MAX_ATTACHMENT_BYTES,
                "output attachment exceeds size limit"
            );
            return None;
        }
        Err(error) => {
            tracing::trace!(
                path = %path,
                mime_type = %attachment.mime_type,
                error = %error,
                "failed to stat output attachment path"
            );
            return None;
        }
        Ok(_) => {}
    }

    match tokio::fs::read(path).await {
        Ok(bytes) => Some(MediaAttachment {
            mime_type: attachment.mime_type.clone(),
            data: MediaContent::Bytes(bytes),
            filename: attachment.filename.clone(),
        }),
        Err(error) => {
            tracing::trace!(
                path = %path,
                mime_type = %attachment.mime_type,
                error = %error,
                "failed to read output attachment path"
            );
            None
        }
    }
}

async fn map_output_attachment_url(
    url: &str,
    attachment: &OutputAttachment,
) -> Option<MediaAttachment> {
    let candidate = MediaAttachment {
        mime_type: attachment.mime_type.clone(),
        data: MediaContent::Url(url.to_string()),
        filename: attachment.filename.clone(),
    };

    match load_attachment_bytes(&candidate).await {
        Ok(bytes) => Some(MediaAttachment {
            mime_type: attachment.mime_type.clone(),
            data: MediaContent::Bytes(bytes),
            filename: attachment.filename.clone(),
        }),
        Err(error) => {
            tracing::warn!(
                attachment_url = %url,
                mime_type = %attachment.mime_type,
                error = %error,
                "rejecting output attachment URL due to fetch policy"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EIGHT_MIB: usize = 8 * 1024 * 1024;

    fn bytes_attachment(bytes: Vec<u8>) -> MediaAttachment {
        MediaAttachment {
            mime_type: "application/octet-stream".to_string(),
            data: MediaContent::Bytes(bytes),
            filename: Some("fixture.bin".to_string()),
        }
    }

    #[tokio::test]
    async fn load_attachment_bytes_accepts_empty_in_memory_payload() {
        let loaded = load_attachment_bytes(&bytes_attachment(Vec::new()))
            .await
            .expect("empty in-memory payload should load");

        assert!(loaded.is_empty());
    }

    #[tokio::test]
    async fn load_attachment_bytes_preserves_small_in_memory_payload() {
        let payload = b"small media payload".to_vec();
        let loaded = load_attachment_bytes(&bytes_attachment(payload.clone()))
            .await
            .expect("small in-memory payload should load");

        assert_eq!(loaded, payload);
    }

    #[tokio::test]
    async fn load_attachment_bytes_accepts_generated_8m_in_memory_payload() {
        let payload = vec![0xA5; EIGHT_MIB];
        let loaded = load_attachment_bytes(&bytes_attachment(payload))
            .await
            .expect("8 MiB in-memory payload should load under the 25 MiB limit");

        assert_eq!(loaded.len(), EIGHT_MIB);
        assert!(loaded.iter().all(|byte| *byte == 0xA5));
    }
}
