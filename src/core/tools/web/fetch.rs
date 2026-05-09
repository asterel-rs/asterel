//! Web fetch tool — fetches a URL and extracts human-readable text content.
//!
//! # What it does
//!
//! `web_fetch` performs a validated HTTPS request and returns the page's
//! title, extracted text (HTML → readable text via `extract::html_to_readable_text`),
//! content type, and character count. JSON responses are pretty-printed before
//! truncation. Plain-text responses are returned as-is up to `max_chars`.
//!
//! # Security surface
//!
//! * Only `https://` URLs are accepted; `http://` and other schemes return a
//!   non-success result immediately.
//! * SSRF is blocked by `security::validate_fetch_url` which performs both
//!   syntactic validation and async DNS resolution to catch private IPs.
//! * Redirects are followed manually via `redirects::get_with_validated_redirects`
//!   (max 5 hops, each hop re-validated) rather than using `reqwest`'s
//!   built-in redirect policy, which would bypass SSRF checks.
//! * Response bodies are capped at 2 MB before buffering.
//! * Requires the `Network` capability on the tool spec.

use std::future::Future;
use std::pin::Pin;

use serde_json::{Value, json};

use super::extract::{extract_title, html_to_readable_text};
use crate::core::tools::middleware::ExecutionContext;

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
use crate::core::tools::traits::{Tool, ToolResult, ToolSpec};
use crate::security::capability::Capability;

const DEFAULT_MAX_CHARS: usize = 4_000;
const MAX_MAX_CHARS: usize = 8_000;

pub struct WebFetchTool;

impl WebFetchTool {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for WebFetchTool {
    fn name(&self) -> &'static str {
        "web_fetch"
    }

    fn description(&self) -> &'static str {
        "Fetch a URL and extract readable text content. Handles HTML, plain text, and JSON. Returns title, extracted text, and content type."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to fetch (https only)"
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Max chars to return (default 4000, max 8000)"
                }
            },
            "required": ["url"]
        })
    }

    fn spec(&self) -> ToolSpec {
        let name = self.name().to_string();
        let effect = crate::contracts::tools::ToolEffect::classify(&name);
        ToolSpec {
            name,
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
            required_capabilities: vec![Capability::Network],
            effect,
        }
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let url = args
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?
                .trim();

            if !url.starts_with("https://") {
                return Ok(failed_tool_result("Only https:// URLs are allowed"));
            }

            let max_chars = bounded_usize_arg(&args, "max_chars", DEFAULT_MAX_CHARS, MAX_MAX_CHARS);

            let validated_url = match crate::security::validate_fetch_url(url, true).await {
                Ok(url) => url,
                Err(error) => return Ok(failed_tool_result(error.to_string())),
            };

            let response =
                match super::redirects::get_with_validated_redirects(&validated_url, 5, true).await
                {
                    Ok(response) => response,
                    Err(error) => return Ok(failed_tool_result(error.to_string())),
                };

            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map_or_else(
                    || "application/octet-stream".to_string(),
                    ToString::to_string,
                );

            if response
                .content_length()
                .is_some_and(|len| len > MAX_RESPONSE_BYTES as u64)
            {
                return Ok(failed_tool_result("Response too large"));
            }

            let body = match response.text().await {
                Ok(body) => body,
                Err(error) => return Ok(failed_tool_result(error.to_string())),
            };

            if body.len() > MAX_RESPONSE_BYTES {
                return Ok(failed_tool_result(format!(
                    "Response body too large: {} bytes (max {MAX_RESPONSE_BYTES})",
                    body.len()
                )));
            }

            let (title, text) = if is_html_content_type(&content_type) {
                (
                    extract_title(&body),
                    html_to_readable_text(&body, max_chars),
                )
            } else if is_json_content_type(&content_type) {
                (None, format_json_body(&body, max_chars))
            } else {
                (None, truncate_chars(&body, max_chars))
            };

            let output = json!({
                "url": validated_url.as_str(),
                "title": title,
                "content_type": content_type,
                "text": text,
                "chars": text.chars().count(),
            });

            Ok(ToolResult {
                success: true,
                output: serde_json::to_string_pretty(&output)?,
                error: None,
                attachments: Vec::new(),
                taint_labels: Vec::new(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            })
        })
    }
}

fn bounded_usize_arg(args: &Value, key: &str, default: usize, max: usize) -> usize {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .map_or(default, |value| value.clamp(1, max))
}

fn is_html_content_type(content_type: &str) -> bool {
    content_type.contains("text/html") || content_type.contains("application/xhtml+xml")
}

fn is_json_content_type(content_type: &str) -> bool {
    content_type.contains("application/json") || content_type.contains("+json")
}

fn format_json_body(body: &str, max_chars: usize) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .map_or_else(
            || truncate_chars(body, max_chars),
            |formatted| truncate_chars(&formatted, max_chars),
        )
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    match text.char_indices().nth(max_chars) {
        Some((idx, _)) => text[..idx].to_string(),
        None => text.to_string(),
    }
}

fn failed_tool_result(message: impl Into<String>) -> ToolResult {
    ToolResult {
        success: false,
        output: String::new(),
        error: Some(message.into()),
        attachments: Vec::new(),
        taint_labels: Vec::new(),
        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
    }
}
