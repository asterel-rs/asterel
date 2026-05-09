//! Vision-based image description using multimodal LLM providers.
//!
//! Encodes images as base64 and sends them to a vision-capable
//! provider for concise natural-language descriptions.

use super::MediaProcessor;
use crate::core::providers::response::{ContentBlock, ImageSource, ProviderMessage};
use crate::media::types::MediaFile;
use crate::utils::encoding::encode_base64;

/// System prompt sent to the vision model for image description.
pub(super) const IMAGE_DESCRIPTION_PROMPT: &str = "Describe this image concisely in 1-2 sentences.";

impl MediaProcessor {
    /// Describe an image using a vision provider, falling back to
    /// metadata when unavailable.
    pub(super) async fn describe_image(&self, file: &MediaFile, data: &[u8]) -> String {
        if let Some(provider) = &self.provider
            && let Some(model) = self.model.as_deref()
            && provider.supports_vision_model(model)
        {
            let source = ImageSource::base64(&file.mime_type, encode_base64(data));
            let messages = vec![ProviderMessage::user_with_image(
                "Describe this image.",
                source,
            )];

            match provider
                .chat_with_tools(Some(IMAGE_DESCRIPTION_PROMPT), &messages, &[], model, 0.2)
                .await
            {
                Ok(response) => {
                    if let Some(text) = extract_response_text(&response) {
                        return text;
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        filename = ?file.filename,
                        model,
                        mime_type = %file.mime_type,
                        error = %error,
                        "vision description failed; using image metadata fallback"
                    );
                }
            }
        } else if let (Some(provider), Some(model)) = (&self.provider, self.model.as_deref())
            && provider.supports_vision()
        {
            tracing::debug!(
                filename = ?file.filename,
                model,
                mime_type = %file.mime_type,
                "vision description skipped because model capability policy rejected image input"
            );
        }

        Self::image_metadata(file)
    }

    /// Describe a document file using text preview or metadata.
    pub(super) fn describe_document(file: &MediaFile, data: &[u8]) -> String {
        let filename = file.filename.as_deref().unwrap_or("unnamed");
        if file.mime_type.starts_with("text/") {
            return match std::str::from_utf8(data) {
                Ok(text) => {
                    let preview: String = text.chars().take(500).collect();
                    if preview.is_empty() {
                        format!(
                            "[Document: {filename} ({}, {} bytes)]",
                            file.mime_type, file.size_bytes
                        )
                    } else {
                        format!(
                            "[Document: {filename} ({}, {} bytes)] Preview: {}",
                            file.mime_type, file.size_bytes, preview
                        )
                    }
                }
                Err(_) => format!(
                    "[Document: {filename} ({}, {} bytes)]",
                    file.mime_type, file.size_bytes
                ),
            };
        }

        if file.mime_type == "application/pdf" {
            return format!("[PDF document: {filename} ({} bytes)]", file.size_bytes);
        }

        format!(
            "[Document: {filename} ({}, {} bytes)]",
            file.mime_type, file.size_bytes
        )
    }

    /// Return a metadata-only description of an image file.
    pub(super) fn image_metadata(file: &MediaFile) -> String {
        format!(
            "[Image: {} ({}, {} bytes)]",
            file.filename.as_deref().unwrap_or("unnamed"),
            file.mime_type,
            file.size_bytes,
        )
    }
}

fn extract_response_text(
    response: &crate::core::providers::response::ProviderResponse,
) -> Option<String> {
    let text = response.text.trim();
    if !text.is_empty() {
        return Some(text.to_string());
    }

    response.content_blocks.iter().find_map(|block| {
        if let ContentBlock::Text { text } = block {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        None
    })
}
