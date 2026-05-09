//! Web search tool — searches the web via the `DuckDuckGo` HTML endpoint.
//!
//! # What it does
//!
//! `web_search` submits a query to `https://html.duckduckgo.com/html/` and
//! parses the result page with CSS selectors to extract titles, URLs, and
//! snippets. No API key is required. Results are capped at `max_results`
//! (default 5, maximum 10) and returned as a structured `SearchResponse`.
//!
//! # Security surface
//!
//! The only outbound request is to `DuckDuckGo`'s fixed HTML endpoint;
//! user-supplied queries are passed as query parameters, not interpolated
//! into the URL path. Response bodies are capped at 1 MB before buffering.
//!
//! The `extract_result_url` helper decodes `DuckDuckGo`'s `uddg=` redirect
//! parameter to recover the real destination URL so the agent sees the actual
//! target rather than a `DuckDuckGo` redirect link.
//!
//! Requires the `Network` capability on the tool spec.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reqwest::redirect::Policy;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;

use super::extract::html_to_readable_text;
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{
    Tool, ToolResult, ToolResultCompactionTarget, ToolResultTextField, ToolSpec,
};
use crate::security::capability::Capability;

const DEFAULT_MAX_RESULTS: usize = 5;
const MAX_RESULTS_LIMIT: usize = 10;
const DUCKDUCKGO_HTML_URL: &str = "https://html.duckduckgo.com/html/";
const DUCKDUCKGO_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub struct WebSearchTool;

#[derive(Debug, Deserialize)]
struct SearchArgs {
    query: String,
    #[serde(default)]
    max_results: Option<usize>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct SearchEntry {
    title: String,
    url: String,
    snippet: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct SearchResponse {
    query: String,
    results: Vec<SearchEntry>,
    total_found: usize,
}

impl Tool for WebSearchTool {
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn description(&self) -> &'static str {
        "Search the web using DuckDuckGo. Returns titles, URLs, and snippets for top results. No API key required."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Max results to return (default 5, max 10)"
                }
            },
            "required": ["query"]
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
        args: serde_json::Value,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let parsed: SearchArgs = serde_json::from_value(args)?;
            let query = parsed.query.trim();

            if query.is_empty() {
                return Ok(error_tool_result("Query cannot be empty"));
            }

            let max_results = parsed
                .max_results
                .unwrap_or(DEFAULT_MAX_RESULTS)
                .clamp(1, MAX_RESULTS_LIMIT);

            let client = crate::utils::http::build_http_client_with(
                reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .redirect(Policy::limited(5))
                    .user_agent(DUCKDUCKGO_USER_AGENT),
            );

            let response = match client
                .get(DUCKDUCKGO_HTML_URL)
                .query(&[("q", query)])
                .send()
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    return Ok(error_tool_result(format!(
                        "DuckDuckGo request failed: {error}"
                    )));
                }
            };

            let status = response.status();
            if !status.is_success() {
                return Ok(error_tool_result(format!(
                    "DuckDuckGo search failed with status {status}"
                )));
            }

            if response
                .content_length()
                .is_some_and(|len| len > MAX_RESPONSE_BYTES as u64)
            {
                return Ok(error_tool_result("Response too large"));
            }

            let body = response.text().await?;
            if body.len() > MAX_RESPONSE_BYTES {
                return Ok(error_tool_result(format!(
                    "Response body too large: {} bytes",
                    body.len()
                )));
            }

            let search_response = match build_search_response(query, &body, max_results) {
                Ok(response) => response,
                Err(error) => return Ok(error_tool_result(error)),
            };
            Ok(success_tool_result(serde_json::to_string_pretty(
                &search_response,
            )?))
        })
    }
}

fn build_search_response(
    query: &str,
    html: &str,
    max_results: usize,
) -> Result<SearchResponse, &'static str> {
    let document = Html::parse_document(html);
    let Ok(result_selector) = Selector::parse(".result") else {
        return Ok(empty_search_response(query));
    };
    let Ok(link_selector) = Selector::parse(".result__a") else {
        return Ok(empty_search_response(query));
    };
    let Ok(snippet_selector) = Selector::parse(".result__snippet") else {
        return Ok(empty_search_response(query));
    };

    let results: Vec<SearchEntry> = document
        .select(&result_selector)
        .filter_map(|result| parse_search_entry(&result, &link_selector, &snippet_selector))
        .take(max_results)
        .collect();

    if results.is_empty() && search_selector_drift_suspected(html) {
        return Err(
            "DuckDuckGo search parser returned zero results from a result-like page; selector drift suspected",
        );
    }

    Ok(SearchResponse {
        query: query.to_string(),
        total_found: results.len(),
        results,
    })
}

fn search_selector_drift_suspected(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower.contains("uddg=")
        || lower.contains("duckduckgo.com/l/")
        || lower.contains("result__a")
        || lower.contains("result-link")
}

fn parse_search_entry(
    result: &scraper::ElementRef<'_>,
    link_selector: &Selector,
    snippet_selector: &Selector,
) -> Option<SearchEntry> {
    let link = result.select(link_selector).next()?;
    let title = normalize_element_text(link.text());
    let href = link.value().attr("href")?;
    let url = extract_result_url(href);

    if title.is_empty() || url.is_empty() {
        return None;
    }

    let snippet = result
        .select(snippet_selector)
        .next()
        .map(|element| normalize_element_text(element.text()))
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| normalize_text(&html_to_readable_text(&result.html(), 280)));

    Some(SearchEntry {
        title,
        url,
        snippet,
    })
}

fn extract_result_url(href: &str) -> String {
    parse_duckduckgo_redirect_url(href).unwrap_or_else(|| href.trim_start_matches("//").to_string())
}

fn parse_duckduckgo_redirect_url(href: &str) -> Option<String> {
    let parsed = parse_href_url(href)?;
    parsed
        .query_pairs()
        .find_map(|(key, value)| (key == "uddg").then(|| value.into_owned()))
        .filter(|value| !value.is_empty())
}

fn parse_href_url(href: &str) -> Option<Url> {
    Url::parse(href).ok().or_else(|| {
        href.strip_prefix("//")
            .and_then(|rest| Url::parse(&format!("https://{rest}")).ok())
    })
}

fn normalize_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for segment in text.split_whitespace() {
        if !result.is_empty() {
            result.push(' ');
        }
        result.push_str(segment);
    }
    result
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

fn empty_search_response(query: &str) -> SearchResponse {
    SearchResponse {
        query: query.to_string(),
        results: Vec::new(),
        total_found: 0,
    }
}

fn success_tool_result(output: String) -> ToolResult {
    ToolResult::success(output)
        .with_output_kind("web_search")
        .with_compaction_target(ToolResultCompactionTarget::Output)
        .with_source_fields([ToolResultTextField::Output])
}

fn error_tool_result(message: impl Into<String>) -> ToolResult {
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
    use super::{
        build_search_response, extract_result_url, parse_duckduckgo_redirect_url,
        success_tool_result,
    };

    #[test]
    fn parses_uddg_redirect_target() {
        let href = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpath%3Fa%3D1&rut=abc";
        let parsed = parse_duckduckgo_redirect_url(href);

        assert_eq!(parsed.as_deref(), Some("https://example.com/path?a=1"));
    }

    #[test]
    fn parses_absolute_duckduckgo_redirect_target() {
        let href = "https://duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org%2Flearn&rut=abc";
        let parsed = parse_duckduckgo_redirect_url(href);

        assert_eq!(parsed.as_deref(), Some("https://rust-lang.org/learn"));
    }

    #[test]
    fn success_result_emits_semantic_metadata() {
        let result = success_tool_result("{\"query\":\"rust\"}".to_string());

        assert!(result.success);
        assert_eq!(result.semantic.output_kind.as_deref(), Some("web_search"));
    }

    #[test]
    fn falls_back_to_href_without_protocol_relative_prefix() {
        let href = "//duckduckgo.com/l/?rut=abc";

        assert_eq!(extract_result_url(href), "duckduckgo.com/l/?rut=abc");
    }

    #[test]
    fn selector_drift_on_result_like_page_returns_error() {
        let html = r#"
            <html><body>
              <div class="new-result">
                <a class="new-result-link" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com">Example</a>
              </div>
            </body></html>
        "#;

        let error = build_search_response("example", html, 5).unwrap_err();

        assert!(error.contains("selector drift suspected"));
    }
}
