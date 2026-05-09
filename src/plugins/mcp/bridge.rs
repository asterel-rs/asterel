//! Bridge between rmcp protocol types and `Asterel` internal types.
//!
//! `Content` in rmcp is `Annotated<RawContent>` which derefs to `RawContent`.
//! Variants: `Text(RawTextContent)`, `Image(RawImageContent)`,
//! `Resource(RawEmbeddedResource)`, `Audio(RawAudioContent)`,
//! `ResourceLink(RawResource)`.

use super::content::ToolContent;

/// Convert an rmcp `Content` item to a `ToolContent`.
///
/// Handles text, image, and embedded resource content.
/// Audio and resource-link variants produce text placeholders.
#[must_use]
pub fn from_rmcp_content(content: &rmcp::model::Content) -> ToolContent {
    use rmcp::model::RawContent;
    match &content.raw {
        RawContent::Text(text_content) => ToolContent::Text {
            text: text_content.text.clone(),
        },
        RawContent::Image(image_content) => ToolContent::Image {
            mime_type: image_content.mime_type.clone(),
            data: image_content.data.clone(),
        },
        RawContent::Resource(embedded) => {
            let (uri, mime_type) = match &embedded.resource {
                rmcp::model::ResourceContents::TextResourceContents { uri, mime_type, .. }
                | rmcp::model::ResourceContents::BlobResourceContents { uri, mime_type, .. } => {
                    (uri.clone(), mime_type.clone())
                }
            };
            ToolContent::Resource {
                uri,
                mime_type,
                name: None,
            }
        }
        RawContent::Audio(audio) => ToolContent::Text {
            text: format!("[Audio: {}]", audio.mime_type),
        },
        RawContent::ResourceLink(link) => ToolContent::Resource {
            uri: link.uri.clone(),
            mime_type: link.mime_type.clone(),
            name: Some(link.name.clone()),
        },
    }
}

/// Convert a slice of rmcp `Content` items to `ToolContent` values.
pub fn from_rmcp_contents(contents: &[rmcp::model::Content]) -> Vec<ToolContent> {
    contents.iter().map(from_rmcp_content).collect()
}

/// Convert a `ToolContent` to an rmcp `Content` item.
#[must_use]
pub fn to_rmcp_content(content: &ToolContent) -> rmcp::model::Content {
    match content {
        ToolContent::Text { text } => rmcp::model::Content::text(text),
        ToolContent::Image { mime_type, data } => rmcp::model::Content::image(data, mime_type),
        ToolContent::Resource { uri, name, .. } => {
            let label = name.as_deref().unwrap_or(uri.as_str());
            rmcp::model::Content::text(format!("[Resource: {label}]"))
        }
    }
}

/// Convert a slice of `ToolContent` items to rmcp `Content` values.
pub fn to_rmcp_contents(contents: &[ToolContent]) -> Vec<rmcp::model::Content> {
    contents.iter().map(to_rmcp_content).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trip() {
        let rmcp_content = rmcp::model::Content::text("hello world");
        let tool = from_rmcp_content(&rmcp_content);
        assert_eq!(
            tool,
            ToolContent::Text {
                text: "hello world".to_string()
            }
        );

        let back = to_rmcp_content(&tool);
        assert_eq!(back.raw.as_text().unwrap().text, "hello world");
    }

    #[test]
    fn image_round_trip() {
        let rmcp_content = rmcp::model::Content::image("aGVsbG8=", "image/png");
        let tool = from_rmcp_content(&rmcp_content);
        assert_eq!(
            tool,
            ToolContent::Image {
                data: "aGVsbG8=".to_string(),
                mime_type: "image/png".to_string(),
            }
        );

        let back = to_rmcp_content(&tool);
        let img = back.raw.as_image().unwrap();
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
        let text = back.raw.as_text().unwrap();
        assert!(text.text.contains("data.csv"));
    }

    #[test]
    fn embedded_resource_extracts_uri() {
        let resource =
            rmcp::model::ResourceContents::text("some text content", "file:///notes.txt");
        let rmcp_content = rmcp::model::Content::resource(resource);
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
            rmcp::model::Content::text("one"),
            rmcp::model::Content::text("two"),
        ];
        let tools = from_rmcp_contents(&items);
        assert_eq!(tools.len(), 2);

        let back = to_rmcp_contents(&tools);
        assert_eq!(back.len(), 2);
    }
}
