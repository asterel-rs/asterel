//! Native web tools — search, fetch, scrape, and summarize.
//!
//! Provides built-in web capabilities without `MCP` server dependencies.
//! All tools in this module are gated behind the `link-extraction` feature flag.
//!
//! # Tool overview
//!
//! | Tool | Purpose |
//! |------|---------|
//! | [`fetch::WebFetchTool`] | Fetch a URL and extract human-readable text. |
//! | [`search::WebSearchTool`] | Search the web via `DuckDuckGo` HTML (no API key). |
//! | [`scrape::WebScrapeTool`] | Fetch a URL and extract elements matching a CSS selector. |
//! | [`summarize::WebSummarizeTool`] | Extractive summarization of text (no LLM call). |
//!
//! # Security model
//!
//! **SSRF prevention** — all URLs are validated by
//! `security::validate_fetch_url` before any network request is made. This
//! function performs both a syntactic check (scheme must be `https://`) and an
//! asynchronous DNS resolution check that rejects private, loopback, and
//! link-local addresses, including IPv4-mapped IPv6 addresses.
//!
//! **Redirect safety** — `fetch` and `scrape` disable `reqwest`'s automatic
//! redirect following and instead use `redirects::get_with_validated_redirects`,
//! which re-validates the SSRF rules at every hop. Up to 5 hops are followed;
//! any hop whose resolved address is private is rejected.
//!
//! **Content size caps** — every response body is capped at 2 MB before
//! buffering to prevent memory exhaustion on large or adversarially crafted
//! responses.
//!
//! **Content sanitization** — HTML bodies are passed through
//! `extract::html_to_readable_text`, which strips `<script>`, `<style>`,
//! `<nav>`, `<footer>`, `<iframe>`, and other non-content tags before
//! returning plain text.

#[cfg(feature = "link-extraction")]
pub mod extract;
#[cfg(feature = "link-extraction")]
pub mod fetch;
#[cfg(feature = "link-extraction")]
mod redirects;
#[cfg(feature = "link-extraction")]
pub mod scrape;
#[cfg(feature = "link-extraction")]
pub mod search;
#[cfg(feature = "link-extraction")]
pub mod summarize;

#[cfg(feature = "link-extraction")]
pub use fetch::WebFetchTool;
#[cfg(feature = "link-extraction")]
pub use scrape::WebScrapeTool;
#[cfg(feature = "link-extraction")]
pub use search::WebSearchTool;
#[cfg(feature = "link-extraction")]
pub use summarize::WebSummarizeTool;
