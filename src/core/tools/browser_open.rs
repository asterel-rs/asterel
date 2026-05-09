//! Simple URL-open tool that launches approved HTTPS pages in
//! Brave Browser without DOM automation or scraping.

use std::future::Future;
use std::pin::Pin;

use serde_json::json;

use super::traits::{Tool, ToolResult};
use crate::contracts::strings::verdicts::URL_USERINFO_NOT_ALLOWED;
use crate::core::tools::middleware::ExecutionContext;
use crate::security::validate_no_ssrf;

/// Open approved HTTPS URLs in Brave Browser (no scraping, no DOM automation).
pub struct BrowserOpenTool {
    allowed_domains: Vec<String>,
}

impl BrowserOpenTool {
    #[must_use]
    /// Create a browser-open tool restricted to the given domains.
    pub fn new(allowed_domains: Vec<String>) -> Self {
        Self {
            allowed_domains: normalize_allowed_domains(allowed_domains),
        }
    }

    fn validate_url(&self, raw_url: &str) -> anyhow::Result<String> {
        let url = raw_url.trim();

        if url.is_empty() {
            anyhow::bail!(
                "URL cannot be empty: provide a fully-qualified URL starting with https://"
            );
        }

        if url.chars().any(char::is_whitespace) {
            anyhow::bail!("URL cannot contain whitespace");
        }

        if !url.starts_with("https://") {
            anyhow::bail!("Only https:// URLs are allowed");
        }

        if self.allowed_domains.is_empty() {
            anyhow::bail!(
                "Browser tool is enabled but no allowed_domains are configured. Add [browser].allowed_domains in config.toml"
            );
        }

        let host = extract_host(url)?;

        if is_private_or_local_host(&host) {
            anyhow::bail!("Blocked local/private host: {host}");
        }

        if !host_in_allowlist(&host, &self.allowed_domains) {
            anyhow::bail!("Host '{host}' is not in browser.allowed_domains");
        }

        Ok(url.to_string())
    }
}

impl Tool for BrowserOpenTool {
    fn name(&self) -> &'static str {
        "browser_open"
    }

    fn description(&self) -> &'static str {
        "Open an approved HTTPS URL in Brave Browser. Security constraints: allowlist-only domains, no local/private hosts, no scraping."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "HTTPS URL to open in Brave Browser"
                }
            },
            "required": ["url"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;

            let url = match self.validate_url(url) {
                Ok(v) => v,
                Err(e) => {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(e.to_string()),

                        attachments: Vec::new(),
                        taint_labels: Vec::new(),
                        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                    });
                }
            };

            if let Err(e) = validate_no_ssrf(&url).await {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Blocked URL by SSRF policy: {e}")),

                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                });
            }

            match open_in_brave(&url).await {
                Ok(()) => Ok(ToolResult {
                    success: true,
                    output: format!("Opened in Brave: {url}"),
                    error: None,

                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                }),
                Err(e) => Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(format!("Failed to open Brave Browser: {e}")),

                    attachments: Vec::new(),
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                }),
            }
        })
    }
}

async fn open_in_brave(url: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        for app in ["Brave Browser", "Brave"] {
            let status = tokio::process::Command::new("open")
                .arg("-a")
                .arg(app)
                .arg(url)
                .status()
                .await;

            if let Ok(s) = status {
                if s.success() {
                    return Ok(());
                }
            }
        }
        anyhow::bail!(
            "Brave Browser was not found (tried macOS app names 'Brave Browser' and 'Brave')"
        );
    }

    #[cfg(target_os = "linux")]
    {
        let mut last_error = String::new();
        for cmd in ["brave-browser", "brave"] {
            match tokio::process::Command::new(cmd).arg(url).status().await {
                Ok(status) if status.success() => return Ok(()),
                Ok(status) => {
                    last_error = format!("{cmd} exited with status {status}");
                }
                Err(e) => {
                    last_error = format!("{cmd} not runnable: {e}");
                }
            }
        }
        anyhow::bail!("{last_error}");
    }

    #[cfg(target_os = "windows")]
    {
        let status = tokio::process::Command::new("cmd")
            .args(["/C", "start", "", "brave", url])
            .status()
            .await?;

        if status.success() {
            return Ok(());
        }

        anyhow::bail!("cmd start brave exited with status {status}");
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = url;
        anyhow::bail!("browser_open is not supported on this OS");
    }
}

fn normalize_allowed_domains(domains: Vec<String>) -> Vec<String> {
    let mut normalized = domains
        .into_iter()
        .filter_map(|d| normalize_domain(&d))
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    normalized
}

fn normalize_domain(raw: &str) -> Option<String> {
    let trimmed = raw.trim().to_lowercase();
    if trimmed.is_empty() {
        return None;
    }

    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(&trimmed);

    let without_path = without_scheme
        .split_once('/')
        .map_or(without_scheme, |(host, _)| host);

    let without_dots = without_path.trim_start_matches('.').trim_end_matches('.');
    let without_port = without_dots
        .split_once(':')
        .map_or(without_dots, |(host, _)| host);

    if without_port.is_empty() || without_port.chars().any(char::is_whitespace) {
        return None;
    }

    Some(without_port.to_string())
}

fn extract_host(url: &str) -> anyhow::Result<String> {
    let parsed =
        url::Url::parse(url).map_err(|error| anyhow::anyhow!("invalid URL '{url}': {error}"))?;
    if parsed.scheme() != "https" {
        anyhow::bail!("Only https:// URLs are allowed");
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        anyhow::bail!(URL_USERINFO_NOT_ALLOWED);
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("URL must include a host"))?
        .trim_end_matches('.')
        .to_lowercase();

    if host.is_empty() {
        anyhow::bail!("URL must include a valid host");
    }

    if host.contains(':') && !is_private_or_local_host(&host) {
        anyhow::bail!("IPv6 hosts are not supported in browser_open");
    }

    Ok(host)
}

fn host_in_allowlist(host: &str, allowed_domains: &[String]) -> bool {
    allowed_domains.iter().any(|domain| {
        host == domain
            || host
                .strip_suffix(domain)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

fn is_private_or_local_host(host: &str) -> bool {
    let has_local_tld = host
        .rsplit('.')
        .next()
        .is_some_and(|label| label == "local");

    if host == "localhost" || host.ends_with(".localhost") || has_local_tld {
        return true;
    }

    if crate::contracts::network::is_private_host(host) {
        return true;
    }

    if let Some([a, b, _, _]) = parse_ipv4(host) {
        return a == 0
            || a == 10
            || a == 127
            || (a == 169 && b == 254)
            || (a == 172 && (16..=31).contains(&b))
            || (a == 192 && b == 168)
            || (a == 100 && (64..=127).contains(&b));
    }

    false
}

fn parse_ipv4(host: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() != 4 {
        return None;
    }

    let mut octets = [0_u8; 4];
    for (i, part) in parts.iter().enumerate() {
        octets[i] = part.parse::<u8>().ok()?;
    }
    Some(octets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_tool(allowed_domains: Vec<&str>) -> BrowserOpenTool {
        BrowserOpenTool::new(allowed_domains.into_iter().map(String::from).collect())
    }

    #[test]
    fn normalize_domain_strips_scheme_path_and_case() {
        let got = normalize_domain("  HTTPS://Docs.Example.com/path ").unwrap();
        assert_eq!(got, "docs.example.com");
    }

    #[test]
    fn normalize_allowed_domains_deduplicates() {
        let got = normalize_allowed_domains(vec![
            "example.com".into(),
            "EXAMPLE.COM".into(),
            "https://example.com/".into(),
        ]);
        assert_eq!(got, vec!["example.com".to_string()]);
    }

    #[test]
    fn validate_accepts_exact_domain() {
        let tool = test_tool(vec!["example.com"]);
        let got = tool.validate_url("https://example.com/docs").unwrap();
        assert_eq!(got, "https://example.com/docs");
    }

    #[test]
    fn validate_accepts_subdomain() {
        let tool = test_tool(vec!["example.com"]);
        assert!(tool.validate_url("https://api.example.com/v1").is_ok());
    }

    #[test]
    fn validate_rejects_http() {
        let tool = test_tool(vec!["example.com"]);
        let err = tool
            .validate_url("http://example.com")
            .unwrap_err()
            .to_string();
        assert!(err.contains("https://"));
    }

    #[test]
    fn validate_rejects_localhost() {
        let tool = test_tool(vec!["localhost"]);
        let err = tool
            .validate_url("https://localhost:8080")
            .unwrap_err()
            .to_string();
        assert!(err.contains("local/private"));
    }

    #[test]
    fn validate_rejects_private_ipv4() {
        let tool = test_tool(vec!["192.168.1.5"]);
        let err = tool
            .validate_url("https://192.168.1.5")
            .unwrap_err()
            .to_string();
        assert!(err.contains("local/private"));
    }

    #[test]
    fn validate_rejects_allowlist_miss() {
        let tool = test_tool(vec!["example.com"]);
        let err = tool
            .validate_url("https://google.com")
            .unwrap_err()
            .to_string();
        assert!(err.contains("allowed_domains"));
    }

    #[test]
    fn validate_rejects_whitespace() {
        let tool = test_tool(vec!["example.com"]);
        let err = tool
            .validate_url("https://example.com/hello world")
            .unwrap_err()
            .to_string();
        assert!(err.contains("whitespace"));
    }

    #[test]
    fn validate_rejects_userinfo() {
        let tool = test_tool(vec!["example.com"]);
        let err = tool
            .validate_url("https://user@example.com")
            .unwrap_err()
            .to_string();
        assert!(err.contains("userinfo"));
    }

    #[test]
    fn validate_rejects_private_ipv6_ranges() {
        let tool = test_tool(vec!["*"]);

        for url in [
            "https://[fc00::1]/",
            "https://[fd12:3456::1]/",
            "https://[fe80::1]/",
        ] {
            let err = tool.validate_url(url).unwrap_err().to_string();
            assert!(
                err.contains("local/private"),
                "expected private IPv6 rejection for {url}, got {err}"
            );
        }
    }

    #[test]
    fn validate_requires_allowlist() {
        let tool = BrowserOpenTool::new(vec![]);
        let err = tool
            .validate_url("https://example.com")
            .unwrap_err()
            .to_string();
        assert!(err.contains("allowed_domains"));
    }

    #[test]
    fn parse_ipv4_valid() {
        assert_eq!(parse_ipv4("1.2.3.4"), Some([1, 2, 3, 4]));
    }

    #[test]
    fn parse_ipv4_invalid() {
        assert_eq!(parse_ipv4("1.2.3"), None);
        assert_eq!(parse_ipv4("1.2.3.999"), None);
        assert_eq!(parse_ipv4("not-an-ip"), None);
    }
}
