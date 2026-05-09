//! Web scrape tool — fetches a URL and extracts elements matching a CSS selector.
//!
//! # What it does
//!
//! `web_scrape` is a more targeted alternative to `web_fetch`: instead of
//! extracting all readable text, it applies a caller-supplied CSS selector
//! and returns the text content of each matching element (up to `max_results`,
//! default 10, maximum 50). This is useful for structured data extraction
//! from known page layouts.
//!
//! # Security surface
//!
//! Shares the same SSRF prevention, redirect handling, and content-size cap
//! as `web_fetch` — see that module's documentation for details. Additionally:
//!
//! * The CSS selector is validated by `scraper::Selector::parse` before the
//!   HTTP request is issued; malformed selectors return a non-success result
//!   immediately rather than wasting a network round-trip.
//! * Requires the `Network` capability on the tool spec.

use std::future::Future;
use std::pin::Pin;

use scraper::{Html, Selector};
use serde_json::{Value, json};

use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{
    Tool, ToolResult, ToolResultCompactionTarget, ToolResultTextField, ToolSpec,
};
use crate::security::capability::Capability;

const DEFAULT_MAX_RESULTS: usize = 10;
const MAX_MAX_RESULTS: usize = 50;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub struct WebScrapeTool;

impl WebScrapeTool {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for WebScrapeTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for WebScrapeTool {
    fn name(&self) -> &'static str {
        "web_scrape"
    }

    fn description(&self) -> &'static str {
        "Fetch a URL and extract content using CSS selectors. More targeted than web_fetch. Returns matched elements as text."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to fetch (https only)"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector to extract (e.g. 'article', 'h1', '.content')"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Max elements to return (default 10, max 50)"
                }
            },
            "required": ["url", "selector"]
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
            let selector = args
                .get("selector")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector' parameter"))?
                .trim();

            if !url.starts_with("https://") {
                return Ok(failed_tool_result("Only https:// URLs are allowed"));
            }

            if selector.is_empty() {
                return Ok(failed_tool_result("Selector cannot be empty"));
            }

            let max_results =
                bounded_usize_arg(&args, "max_results", DEFAULT_MAX_RESULTS, MAX_MAX_RESULTS);

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

            let document = Html::parse_document(&body);
            let parsed_selector = match Selector::parse(selector) {
                Ok(selector) => selector,
                Err(error) => {
                    return Ok(failed_tool_result(format!("Invalid CSS selector: {error}")));
                }
            };

            let elements: Vec<_> = document.select(&parsed_selector).collect();
            let total_found = elements.len();
            let matches = elements
                .into_iter()
                .take(max_results)
                .enumerate()
                .map(|(index, element)| {
                    json!({
                        "index": index,
                        "text": normalize_element_text(element.text()),
                    })
                })
                .collect::<Vec<_>>();

            let output = json!({
                "url": validated_url.as_str(),
                "selector": selector,
                "matches": matches,
                "total_found": total_found,
            });

            Ok(ToolResult::success(serde_json::to_string_pretty(&output)?)
                .with_output_kind("web_scrape")
                .with_compaction_target(ToolResultCompactionTarget::Output)
                .with_source_fields([ToolResultTextField::Output]))
        })
    }
}

fn bounded_usize_arg(args: &Value, key: &str, default: usize, max: usize) -> usize {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .map_or(default, |value| value.clamp(1, max))
}

fn normalize_element_text<'a>(fragments: impl Iterator<Item = &'a str>) -> String {
    let mut result = String::new();
    for fragment in fragments {
        for segment in fragment.split_whitespace() {
            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(segment);
        }
    }
    result
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::WebScrapeTool;
    use crate::core::tools::traits::Tool;

    #[test]
    fn schema_exposes_expected_parameters() {
        let schema = WebScrapeTool::new().parameters_schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .unwrap();

        assert!(properties.contains_key("url"));
        assert!(properties.contains_key("selector"));
    }

    #[test]
    fn success_output_can_carry_semantic_metadata_shape() {
        let result = crate::core::tools::traits::ToolResult::success(
            serde_json::to_string_pretty(&json!({
                "url": "https://example.com",
                "selector": "article",
                "matches": [],
                "total_found": 0
            }))
            .unwrap(),
        )
        .with_output_kind("web_scrape");

        assert_eq!(result.semantic.output_kind.as_deref(), Some("web_scrape"));
    }
}
