//! Browser automation tool — AI-optimized web browsing via the `agent-browser` CLI.
//!
//! # What it does
//!
//! `BrowserTool` wraps the `agent-browser` CLI to provide a headless browser
//! with semantic element selection, accessibility tree snapshots, and JSON
//! output designed for LLM consumption.
//!
//! # Security model
//!
//! The tool enforces three layers of protection before any URL reaches the
//! browser process:
//!
//! 1. **Domain allowlist** — configured via `[browser].allowed_domains` in
//!    `config.toml`. The host extracted from the URL must match at least one
//!    entry. An empty allowlist blocks all navigation.
//! 2. **Private-IP rejection** — `domain::is_private_host` rejects loopback,
//!    private-range, and link-local hosts (IPv4 and IPv6, including
//!    IPv4-mapped IPv6 addresses like `::ffff:127.0.0.1`).
//! 3. **SSRF DNS check** — `security::url_validation::validate_no_ssrf`
//!    performs an async DNS resolution check on the host, catching cases where
//!    a public hostname resolves to a private address at request time.
//!
//! `file://` URLs are always blocked regardless of the allowlist.
//!
//! # Availability check
//!
//! `BrowserTool::is_available` checks whether the `agent-browser` binary is
//! on `PATH` by running `agent-browser --version`. The result is cached for
//! the process lifetime via a `OnceCell` so subsequent calls are free.
//!
//! # Autonomy integration
//!
//! The tool checks `security.can_act()` and `security.record_action()` before
//! executing any browser action. `ReadOnly` autonomy or a tripped rate limit
//! returns an error result immediately.

mod domain;
mod tool_impl;
mod types;

pub use tool_impl::BrowserTool;
pub use types::BrowserAction;

#[cfg(test)]
mod tests;
