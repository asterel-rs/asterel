//! Domain and host validation helpers for the browser tool.
//!
//! Provides three pure functions used by `BrowserTool::validate_url`:
//!
//! * `normalize_domains` — trims and lowercases a list of configured domain
//!   patterns, filtering out empty strings.
//! * `extract_host` — extracts the host portion from a URL string without
//!   depending on the `url` crate (handles IPv6 bracket notation).
//! * `is_private_host` — delegates to `contracts::network::is_private_host`
//!   to detect loopback, private-range, and link-local addresses.
//! * `host_in_allowlist` — checks whether a host matches any entry in the
//!   domain allowlist, supporting exact matches, subdomain matches, and
//!   `"*"`/`"*.example.com"` wildcard patterns.

/// Normalize domain strings by trimming whitespace and lowercasing.
pub(super) fn normalize_domains(domains: Vec<String>) -> Vec<String> {
    domains
        .into_iter()
        .map(|d| d.trim().to_lowercase())
        .filter(|d| !d.is_empty())
        .collect()
}

/// Extract the host portion from a URL string.
///
/// # Errors
///
/// Returns an error if the URL contains no valid host.
pub(super) fn extract_host(url_str: &str) -> anyhow::Result<String> {
    // Simple host extraction without url crate
    let url = url_str.trim();
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("file://"))
        .unwrap_or(url);

    // Extract host — handle bracketed IPv6 addresses like [::1]:8080
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);

    let host = if authority.starts_with('[') {
        // IPv6: take everything up to and including the closing ']'
        authority.find(']').map_or(authority, |i| &authority[..=i])
    } else {
        // IPv4 or hostname: take everything before the port separator
        authority.split(':').next().unwrap_or(authority)
    };

    if host.is_empty() {
        anyhow::bail!(
            "invalid URL '{url_str}': no host found — expected a fully-qualified URL such as https://example.com"
        );
    }

    Ok(host.to_lowercase())
}

/// Return `true` if the host resolves to a private or loopback address.
pub(super) fn is_private_host(host: &str) -> bool {
    crate::contracts::network::is_private_host(host)
}

/// Check whether a host matches any entry in the domain allowlist.
pub(super) fn host_in_allowlist(host: &str, allowed: &[String]) -> bool {
    allowed.iter().any(|pattern| {
        if pattern == "*" {
            return true;
        }
        if pattern.starts_with("*.") {
            // Wildcard subdomain match
            let suffix = &pattern[1..]; // ".example.com"
            host.ends_with(suffix) || host == &pattern[2..]
        } else {
            // Exact match or subdomain
            host == pattern || host.ends_with(&format!(".{pattern}"))
        }
    })
}
