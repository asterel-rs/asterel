//! Link content extraction for message enrichment.
//!
//! Fetches URLs found in messages, extracts readable text, and
//! appends summaries so the agent has context about linked pages.

use anyhow::{Context, Result};
use futures_util::{Stream, StreamExt};
use reqwest::StatusCode;
use std::fmt::Write as _;
use url::Url;

use super::types::{ExtractedContent, LinkConfig};

const MAX_LINK_FETCH_REDIRECTS: usize = 5;

/// Fetch URL content and extract readable text.
///
/// # Errors
///
/// Returns an error when URL safety validation fails, HTTP request setup or
/// fetch fails, or response body decoding fails.
pub(crate) async fn extract_content(url: &Url, config: &LinkConfig) -> Result<ExtractedContent> {
    crate::security::validate_fetch_url(url.as_str(), false).await?;

    let response = fetch_with_validated_redirects(url, config, MAX_LINK_FETCH_REDIRECTS).await?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    let is_html = content_type
        .as_deref()
        .is_some_and(|ct| ct.contains("text/html"));

    let max_fetch_body_bytes = config.max_fetch_body_bytes;
    let body = read_limited_response_text(response, max_fetch_body_bytes).await?;

    if is_html {
        Ok(extract_from_html(
            url.as_str(),
            &body,
            config.max_content_chars,
        ))
    } else {
        let truncated = truncate_text(&body, config.max_content_chars);
        Ok(ExtractedContent {
            url: url.to_string(),
            title: None,
            text: truncated,
            content_type,
        })
    }
}

async fn read_limited_response_text(response: reqwest::Response, limit: usize) -> Result<String> {
    let content_length = response.content_length();
    let body = collect_limited_body(response.bytes_stream(), content_length, limit).await?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

async fn collect_limited_body<S, C, E>(
    stream: S,
    content_length: Option<u64>,
    limit: usize,
) -> Result<Vec<u8>>
where
    S: Stream<Item = std::result::Result<C, E>>,
    C: AsRef<[u8]>,
    E: std::error::Error + Send + Sync + 'static,
{
    if limit == 0 {
        anyhow::bail!("link fetch response body byte limit must be greater than zero");
    }

    if let Some(length) = content_length
        && length > limit as u64
    {
        anyhow::bail!("link fetch response body exceeds {limit} byte limit");
    }

    let initial_capacity = content_length
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(8192)
        .min(limit);
    let mut body = Vec::with_capacity(initial_capacity);

    futures_util::pin_mut!(stream);
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(anyhow::Error::from)?;
        let chunk = chunk.as_ref();
        if body.len().saturating_add(chunk.len()) > limit {
            anyhow::bail!("link fetch response body exceeds {limit} byte limit");
        }
        body.extend_from_slice(chunk);
    }

    Ok(body)
}

async fn fetch_with_validated_redirects(
    start_url: &Url,
    config: &LinkConfig,
    max_redirects: usize,
) -> Result<reqwest::Response> {
    let mut current_url = start_url.clone();

    for redirect_count in 0..=max_redirects {
        let client = link_fetch_client_for_url(&current_url, config).await?;
        let response = client
            .get(current_url.as_str())
            .send()
            .await
            .with_context(|| format!("request failed for {current_url}"))?;

        if !should_follow_redirect(response.status()) {
            return Ok(response);
        }

        if redirect_count == max_redirects {
            anyhow::bail!(
                "link fetch aborted after {max_redirects} redirects (possible redirect loop): last URL was {current_url}"
            );
        }

        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .context("redirect response missing Location header")?
            .to_str()
            .context("invalid redirect Location header")?;

        current_url = validate_redirect_target(response.url(), location).await?;
    }

    unreachable!("redirect loop always returns before exhausting iterations")
}

async fn link_fetch_client_for_url(url: &Url, config: &LinkConfig) -> Result<reqwest::Client> {
    crate::utils::http::try_build_pinned_public_fetch_client_with(
        url,
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .user_agent("Asterel/0.1")
            .redirect(reqwest::redirect::Policy::none()),
    )
    .await
}

async fn validate_redirect_target(base_url: &Url, location: &str) -> Result<Url> {
    let next_url = resolve_redirect_target(base_url, location)?;
    crate::security::validate_fetch_url(next_url.as_str(), false)
        .await
        .context("redirect target rejected")
}

fn resolve_redirect_target(base_url: &Url, location: &str) -> Result<Url> {
    match Url::parse(location) {
        Ok(url) => Ok(url),
        Err(_) => Ok(base_url.join(location)?),
    }
}

fn should_follow_redirect(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    )
}

#[cfg(feature = "link-extraction")]
fn extract_from_html(url: &str, html: &str, max_chars: usize) -> ExtractedContent {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);

    let title = Selector::parse("title")
        .ok()
        .and_then(|sel| document.select(&sel).next())
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|t| !t.is_empty());

    let content = extract_element_text(&document, "article")
        .or_else(|| extract_element_text(&document, "main"))
        .or_else(|| extract_element_text(&document, "body"))
        .unwrap_or_default();

    let truncated = truncate_text(&content, max_chars);

    ExtractedContent {
        url: url.to_string(),
        title,
        text: truncated,
        content_type: Some("text/html".to_string()),
    }
}

#[cfg(feature = "link-extraction")]
fn extract_element_text(document: &scraper::Html, selector: &str) -> Option<String> {
    let sel = scraper::Selector::parse(selector).ok()?;
    let element = document.select(&sel).next()?;

    let mut normalized = String::new();
    for fragment in element.text() {
        for word in fragment.split_whitespace() {
            if !normalized.is_empty() {
                normalized.push(' ');
            }
            normalized.push_str(word);
        }
    }

    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    crate::utils::text::truncate_ellipsis(text, max_chars)
}

/// Enrich a user message with extracted link content.
pub(crate) async fn enrich_message_with_links(message: &str, config: &LinkConfig) -> String {
    if !config.enabled {
        return message.to_string();
    }

    let urls = super::detector::detect_urls(message);
    if urls.is_empty() {
        return message.to_string();
    }

    let urls_to_process: Vec<_> = urls
        .into_iter()
        .take(config.max_links_per_message)
        .collect();
    let mut link_buf = String::new();

    for url in &urls_to_process {
        match extract_content(url, config).await {
            Ok(content) => {
                let title_part = content.title.as_deref().unwrap_or("Untitled");
                if !link_buf.is_empty() {
                    link_buf.push('\n');
                }
                let _ = write!(
                    link_buf,
                    "[Link: {title_part}]\nURL: {}\n{}\n",
                    content.url, content.text
                );
            }
            Err(e) => {
                tracing::debug!(url = %url, error = %e, "link extraction failed");
            }
        }
    }

    if link_buf.is_empty() {
        return message.to_string();
    }

    format!("{message}\n\n---\nExtracted link content:\n{link_buf}\n---")
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    #[test]
    fn resolve_redirect_target_absolute() {
        let base = Url::parse("https://example.com/a").expect("valid base URL");
        let resolved = resolve_redirect_target(&base, "https://example.org/path")
            .expect("absolute URL should resolve");
        assert_eq!(resolved.as_str(), "https://example.org/path");
    }

    #[test]
    fn resolve_redirect_target_relative() {
        let base = Url::parse("https://example.com/dir/page").expect("valid base URL");
        let resolved =
            resolve_redirect_target(&base, "../next").expect("relative URL should resolve");
        assert_eq!(resolved.as_str(), "https://example.com/next");
    }

    #[tokio::test]
    async fn validate_redirect_target_accepts_public_host() {
        let base = Url::parse("https://example.com/start").expect("valid base URL");
        let resolved = validate_redirect_target(&base, "https://8.8.8.8/path")
            .await
            .expect("public redirect target should pass validation");
        assert_eq!(resolved.as_str(), "https://8.8.8.8/path");
    }

    #[tokio::test]
    async fn validate_redirect_target_rejects_private_host() {
        let base = Url::parse("https://example.com/start").expect("valid base URL");
        let err = validate_redirect_target(&base, "http://169.254.169.254/latest/meta-data")
            .await
            .expect_err("private redirect target must be rejected");
        assert!(format!("{err:#}").contains("private/internal"));
    }

    #[test]
    fn truncate_within_limit() {
        let result = truncate_text("hello", 10);
        assert_eq!(result, "hello");
    }

    #[test]
    fn truncate_exceeds_limit() {
        let result = truncate_text("hello world", 5);
        assert_eq!(result, "hello...");
    }

    #[test]
    fn truncate_unicode() {
        let result = truncate_text("abcde", 3);
        assert_eq!(result, "abc...");
    }

    #[tokio::test]
    async fn collect_limited_body_accepts_body_within_limit() {
        let chunks = stream::iter([
            Ok::<Vec<u8>, std::io::Error>(b"hello ".to_vec()),
            Ok::<Vec<u8>, std::io::Error>(b"world".to_vec()),
        ]);

        let body = collect_limited_body(chunks, Some(11), 11)
            .await
            .expect("body at the limit should be accepted");

        assert_eq!(body, b"hello world");
    }

    #[tokio::test]
    async fn collect_limited_body_rejects_large_content_length() {
        let chunks = stream::iter([Ok::<Vec<u8>, std::io::Error>(b"small".to_vec())]);

        let err = collect_limited_body(chunks, Some(12), 10)
            .await
            .expect_err("declared body larger than limit should be rejected");

        assert!(format!("{err:#}").contains("10 byte limit"));
    }

    #[tokio::test]
    async fn collect_limited_body_rejects_stream_that_exceeds_limit() {
        let chunks = stream::iter([
            Ok::<Vec<u8>, std::io::Error>(b"12345".to_vec()),
            Ok::<Vec<u8>, std::io::Error>(b"67890".to_vec()),
            Ok::<Vec<u8>, std::io::Error>(b"x".to_vec()),
        ]);

        let err = collect_limited_body(chunks, None, 10)
            .await
            .expect_err("stream that grows beyond the limit should be rejected");

        assert!(format!("{err:#}").contains("10 byte limit"));
    }

    #[test]
    #[cfg(feature = "link-extraction")]
    fn extract_html_title_and_body() {
        let html =
            r"<html><head><title>Test Page</title></head><body><p>Hello world</p></body></html>";
        let result = extract_from_html("https://example.com", html, 2000);
        assert_eq!(result.title.as_deref(), Some("Test Page"));
        assert!(result.text.contains("Hello world"));
    }

    #[test]
    #[cfg(feature = "link-extraction")]
    fn extract_html_article_preferred() {
        let html = r"<html><body><nav>Nav stuff</nav><article><p>Article content</p></article></body></html>";
        let result = extract_from_html("https://example.com", html, 2000);
        assert_eq!(result.text, "Article content");
    }

    #[test]
    #[cfg(feature = "link-extraction")]
    fn extract_html_falls_back_to_body() {
        let html = r"<html><body><p>Body content here</p></body></html>";
        let result = extract_from_html("https://example.com", html, 2000);
        assert!(result.text.contains("Body content here"));
    }

    #[test]
    #[cfg(feature = "link-extraction")]
    fn extract_html_truncates() {
        let html = r"<html><body><p>A long paragraph of text</p></body></html>";
        let result = extract_from_html("https://example.com", html, 10);
        assert!(result.text.ends_with("..."));
        assert!(result.text.chars().count() <= 13); // 10 + "..."
    }

    #[tokio::test]
    async fn enrich_no_urls() {
        let config = LinkConfig::default();
        let result = enrich_message_with_links("just text", &config).await;
        assert_eq!(result, "just text");
    }

    #[tokio::test]
    async fn enrich_disabled() {
        let config = LinkConfig {
            enabled: false,
            ..LinkConfig::default()
        };
        let result = enrich_message_with_links("check https://example.com", &config).await;
        assert_eq!(result, "check https://example.com");
    }
}
