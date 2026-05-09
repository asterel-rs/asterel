//! Matrix media helpers: `mxc://` URL to HTTP download URL conversion
//! and media-event attachment extraction (image, audio, video, file).
use super::models::EventContent;
use crate::transport::channels::attachments::media_attachment_url;
use crate::transport::channels::traits::MediaAttachment;

/// Converts an `mxc://` content URI to an HTTP download URL via the
/// Matrix media API v3.
pub(super) fn mxc_to_http(homeserver: &str, mxc_url: &str) -> Option<String> {
    let stripped = mxc_url.strip_prefix("mxc://")?;
    let (server, media_id) = stripped.split_once('/')?;
    Some(format!(
        "{homeserver}/_matrix/media/v3/download/{server}/{media_id}"
    ))
}

/// Extracts media attachments from a Matrix event's content, converting
/// `mxc://` URLs to downloadable HTTP URLs.
pub(super) fn parse_media_attachments(
    homeserver: &str,
    content: &EventContent,
) -> Vec<MediaAttachment> {
    let Some(msgtype) = content.msgtype.as_deref() else {
        return Vec::new();
    };

    if !matches!(msgtype, "m.image" | "m.audio" | "m.video" | "m.file") {
        return Vec::new();
    }

    let Some(mxc_url) = content.url.as_deref() else {
        return Vec::new();
    };
    let Some(download_url) = mxc_to_http(homeserver, mxc_url) else {
        return Vec::new();
    };

    let mime_type = content
        .info
        .as_ref()
        .and_then(|info| info.mimetype.as_deref())
        .map(str::to_string);

    vec![media_attachment_url(
        download_url,
        mime_type.as_deref(),
        content.body.clone(),
    )]
}
