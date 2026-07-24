//! Bridge between rmcp protocol types and `Asterel` internal types.
//!
//! `ContentBlock` variants are text, image, embedded resource, audio,
//! and resource link.

use super::content::ToolContent;

/// Convert an rmcp `Content` item to a `ToolContent`.
///
/// Handles text, image, and embedded resource content.
/// Audio and resource-link variants produce text placeholders.
#[must_use]
pub fn from_rmcp_content(content: &rmcp::model::ContentBlock) -> ToolContent {
    use rmcp::model::ContentBlock;
    match content {
        ContentBlock::Text(text_content) => ToolContent::Text {
            text: text_content.text.clone(),
        },
        ContentBlock::Image(image_content) => ToolContent::Image {
            mime_type: image_content.mime_type.clone(),
            data: image_content.data.clone(),
        },
        ContentBlock::Resource(embedded) => {
            let (uri, mime_type) = match &embedded.resource {
                rmcp::model::ResourceContents::TextResourceContents { uri, mime_type, .. }
                | rmcp::model::ResourceContents::BlobResourceContents { uri, mime_type, .. } => {
                    (uri.clone(), mime_type.clone())
                }
                _ => {
                    return ToolContent::Text {
                        text: "[Unsupported MCP resource]".to_string(),
                    };
                }
            };
            ToolContent::Resource {
                uri,
                mime_type,
                name: None,
            }
        }
        ContentBlock::Audio(audio) => ToolContent::Text {
            text: format!("[Audio: {}]", audio.mime_type),
        },
        ContentBlock::ResourceLink(link) => ToolContent::Resource {
            uri: link.uri.clone(),
            mime_type: link.mime_type.clone(),
            name: Some(link.name.clone()),
        },
        _ => ToolContent::Text {
            text: "[Unsupported MCP content]".to_string(),
        },
    }
}

/// Convert a slice of rmcp `ContentBlock` items to `ToolContent` values.
pub fn from_rmcp_contents(contents: &[rmcp::model::ContentBlock]) -> Vec<ToolContent> {
    contents.iter().map(from_rmcp_content).collect()
}

/// Convert a `ToolContent` to an rmcp `ContentBlock` item.
#[must_use]
pub fn to_rmcp_content(content: &ToolContent) -> rmcp::model::ContentBlock {
    match content {
        ToolContent::Text { text } => rmcp::model::ContentBlock::text(text),
        ToolContent::Image { mime_type, data } => rmcp::model::ContentBlock::image(data, mime_type),
        ToolContent::Resource { uri, name, .. } => {
            let label = name.as_deref().unwrap_or(uri.as_str());
            rmcp::model::ContentBlock::text(format!("[Resource: {label}]"))
        }
    }
}

/// Convert a slice of `ToolContent` items to rmcp `ContentBlock` values.
pub fn to_rmcp_contents(contents: &[ToolContent]) -> Vec<rmcp::model::ContentBlock> {
    contents.iter().map(to_rmcp_content).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trip() {
        let rmcp_content = rmcp::model::ContentBlock::text("hello world");
        let tool = from_rmcp_content(&rmcp_content);
        assert_eq!(
            tool,
            ToolContent::Text {
                text: "hello world".to_string()
            }
        );

        let back = to_rmcp_content(&tool);
        assert_eq!(back.as_text().unwrap().text, "hello world");
    }

    #[test]
    fn image_round_trip() {
        let rmcp_content = rmcp::model::ContentBlock::image("aGVsbG8=", "image/png");
        let tool = from_rmcp_content(&rmcp_content);
        assert_eq!(
            tool,
            ToolContent::Image {
                data: "aGVsbG8=".to_string(),
                mime_type: "image/png".to_string(),
            }
        );

        let back = to_rmcp_content(&tool);
        let img = back.as_image().unwrap();
        assert_eq!(img.data, "aGVsbG8=");
        assert_eq!(img.mime_type, "image/png");
    }

    #[test]
    fn resource_converts_to_text_fallback() {
        let tool = ToolContent::Resource {
            uri: "file:///data.csv".to_string(),
            mime_type: Some("text/csv".to_string()),
            name: Some("data.csv".to_string()),
        };
        let back = to_rmcp_content(&tool);
        let text = back.as_text().unwrap();
        assert!(text.text.contains("data.csv"));
    }

    #[test]
    fn embedded_resource_extracts_uri() {
        let resource =
            rmcp::model::ResourceContents::text("some text content", "file:///notes.txt");
        let rmcp_content = rmcp::model::ContentBlock::resource(resource);
        let tool = from_rmcp_content(&rmcp_content);
        match &tool {
            ToolContent::Resource { uri, .. } => {
                assert_eq!(uri, "file:///notes.txt");
            }
            other => panic!("Expected Resource, got {other:?}"),
        }
    }

    #[test]
    fn batch_conversion() {
        let items = vec![
            rmcp::model::ContentBlock::text("one"),
            rmcp::model::ContentBlock::text("two"),
        ];
        let tools = from_rmcp_contents(&items);
        assert_eq!(tools.len(), 2);

        let back = to_rmcp_contents(&tools);
        assert_eq!(back.len(), 2);
    }
}
