//! Data types for browser tool actions and `agent-browser` CLI responses.
//!
//! `AgentBrowserResponse` is the JSON shape returned by `agent-browser --json`
//! commands. `BrowserAction` is the typed enum that `parse_browser_action`
//! produces from the raw JSON args and that `browser_action_args` converts
//! back into `agent-browser` CLI argument vectors.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Response from agent-browser --json commands
#[derive(Debug, Deserialize)]
pub(super) struct AgentBrowserResponse {
    /// Whether the browser command succeeded.
    pub success: bool,
    /// JSON payload returned by the browser, if any.
    pub data: Option<Value>,
    /// Error message on failure.
    pub error: Option<String>,
}

/// Supported browser actions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAction {
    /// Navigate to a URL
    Open { url: String },
    /// Get accessibility snapshot with refs
    Snapshot {
        #[serde(default)]
        interactive_only: bool,
        #[serde(default)]
        compact: bool,
        #[serde(default)]
        depth: Option<u32>,
    },
    /// Click an element by ref or selector
    Click { selector: String },
    /// Fill a form field
    Fill { selector: String, value: String },
    /// Type text into focused element
    Type { selector: String, text: String },
    /// Get text content of element
    GetText { selector: String },
    /// Get page title
    GetTitle,
    /// Get current URL
    GetUrl,
    /// Take screenshot
    Screenshot {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        full_page: bool,
    },
    /// Wait for element or time
    Wait {
        #[serde(default)]
        selector: Option<String>,
        #[serde(default)]
        ms: Option<u64>,
        #[serde(default)]
        text: Option<String>,
    },
    /// Press a key
    Press { key: String },
    /// Hover over element
    Hover { selector: String },
    /// Scroll page
    Scroll {
        direction: String,
        #[serde(default)]
        pixels: Option<u32>,
    },
    /// Check if element is visible
    IsVisible { selector: String },
    /// Close browser
    Close,
    /// Find element by semantic locator
    Find {
        by: String, // role, text, label, placeholder, testid
        value: String,
        action: String, // click, fill, text, hover
        #[serde(default)]
        fill_value: Option<String>,
    },
}
