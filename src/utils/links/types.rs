//! Data types for extracted link content and link configuration.

use serde::{Deserialize, Serialize};

pub(crate) const DEFAULT_MAX_FETCH_BODY_BYTES: usize = 256 * 1024;

/// Content extracted from a fetched URL.
#[cfg(feature = "link-extraction")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ExtractedContent {
    /// The source URL that was fetched.
    pub url: String,
    /// The page title, if available.
    pub title: Option<String>,
    /// Extracted readable text from the page.
    pub text: String,
    /// The HTTP Content-Type header value, if present.
    pub content_type: Option<String>,
}

/// Configuration for link detection and extraction behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LinkConfig {
    /// Whether link extraction is enabled.
    pub enabled: bool,
    /// Maximum number of links to extract per message.
    pub max_links_per_message: usize,
    /// Maximum characters to retain from each extracted page.
    pub max_content_chars: usize,
    /// Maximum response body bytes to read while fetching a link.
    #[serde(default = "default_max_fetch_body_bytes")]
    pub max_fetch_body_bytes: usize,
    /// HTTP request timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for LinkConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_links_per_message: 3,
            max_content_chars: 2000,
            max_fetch_body_bytes: DEFAULT_MAX_FETCH_BODY_BYTES,
            timeout_secs: 10,
        }
    }
}

const fn default_max_fetch_body_bytes() -> usize {
    DEFAULT_MAX_FETCH_BODY_BYTES
}
