//! MCP tool content types: text, image, and resource variants
//! for representing multimodal tool execution results.

use serde::{Deserialize, Serialize};

/// Structured content from a tool execution.
///
/// Used at the MCP boundary to represent multimodal tool results
/// before they are rendered to text for the existing `ToolResult.output`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContent {
    /// Plain text content.
    Text { text: String },

    /// Base64-encoded image content.
    Image {
        mime_type: String,
        /// Base64-encoded image data.
        data: String,
    },

    /// Reference to an external resource.
    Resource {
        uri: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

/// Render a sequence of tool content items to a plain text string.
///
/// Text items are concatenated with newlines.
/// Non-text items produce descriptive placeholders.
#[must_use]
pub fn render_text(content: &[ToolContent]) -> String {
    let mut out = String::new();
    for item in content {
        if !out.is_empty() {
            out.push('\n');
        }
        match item {
            ToolContent::Text { text } => out.push_str(text),
            ToolContent::Image { mime_type, .. } => {
                out.push_str("[Image: ");
                out.push_str(mime_type);
                out.push(']');
            }
            ToolContent::Resource { uri, name, .. } => {
                out.push_str("[Resource: ");
                if let Some(name) = name {
                    out.push_str(name);
                    out.push_str(" (");
                    out.push_str(uri);
                    out.push_str(")]");
                } else {
                    out.push_str(uri);
                    out.push(']');
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_content_text_serde_roundtrip() {
        let content = ToolContent::Text {
            text: "hello".to_string(),
        };
        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(json["type"], "text");
        assert_eq!(json["text"], "hello");
        let decoded: ToolContent = serde_json::from_value(json).unwrap();
        assert!(matches!(decoded, ToolContent::Text { text } if text == "hello"));
    }

    #[test]
    fn tool_content_image_serde_roundtrip() {
        let content = ToolContent::Image {
            mime_type: "image/png".to_string(),
            data: "iVBOR".to_string(),
        };
        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(json["type"], "image");
        assert_eq!(json["mime_type"], "image/png");
        let decoded: ToolContent = serde_json::from_value(json).unwrap();
        assert!(matches!(decoded, ToolContent::Image { .. }));
    }

    #[test]
    fn tool_content_resource_serde_roundtrip() {
        let content = ToolContent::Resource {
            uri: "file:///tmp/data.csv".to_string(),
            mime_type: Some("text/csv".to_string()),
            name: Some("data.csv".to_string()),
        };
        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(json["type"], "resource");
        assert_eq!(json["uri"], "file:///tmp/data.csv");
        let decoded: ToolContent = serde_json::from_value(json).unwrap();
        assert!(matches!(decoded, ToolContent::Resource { .. }));
    }

    #[test]
    fn render_text_only() {
        let content = vec![
            ToolContent::Text {
                text: "Line 1".to_string(),
            },
            ToolContent::Text {
                text: "Line 2".to_string(),
            },
        ];
        assert_eq!(render_text(&content), "Line 1\nLine 2");
    }

    #[test]
    fn render_mixed_content() {
        let content = vec![
            ToolContent::Text {
                text: "Result:".to_string(),
            },
            ToolContent::Image {
                mime_type: "image/png".to_string(),
                data: "abc".to_string(),
            },
            ToolContent::Resource {
                uri: "file:///x".to_string(),
                name: Some("x".to_string()),
                mime_type: None,
            },
        ];
        let rendered = render_text(&content);
        assert!(rendered.contains("Result:"));
        assert!(rendered.contains("[Image: image/png]"));
        assert!(rendered.contains("[Resource: x (file:///x)]"));
    }

    #[test]
    fn render_empty_content() {
        assert_eq!(render_text(&[]), "");
    }

    #[test]
    fn render_resource_without_name() {
        let content = vec![ToolContent::Resource {
            uri: "https://example.com".to_string(),
            name: None,
            mime_type: None,
        }];
        assert_eq!(render_text(&content), "[Resource: https://example.com]");
    }
}
