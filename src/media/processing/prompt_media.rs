//! Prompt-oriented media description helpers used by transport
//! attachment adapters.
use anyhow::Result;

use super::MediaProcessor;
use crate::media::types::MediaFile;

pub(crate) async fn describe_media_for_prompt(
    processor: &MediaProcessor,
    file: &MediaFile,
    data: &[u8],
) -> Result<String> {
    processor.describe(file, data).await
}
