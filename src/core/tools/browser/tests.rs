//! Unit tests for browser domain validation, SSRF blocking, and tool construction.
//!
//! These tests cover `normalize_domains`, `extract_host`, `is_private_host`,
//! `host_in_allowlist`, and `BrowserTool::validate_url`. Network-dependent
//! tests (SSRF DNS resolution) are `tokio::test` async tests and hit the
//! system resolver — they run against `localhost` and known-private addresses
//! rather than public hosts to avoid test flakiness.

use std::sync::Arc;

use super::domain::{extract_host, host_in_allowlist, is_private_host, normalize_domains};
use super::tool_impl::BrowserTool;
use crate::core::tools::traits::Tool;
use crate::security::SecurityPolicy;

#[test]
fn normalize_domains_works() {
    let domains = vec![
        "  Example.COM  ".into(),
        "docs.example.com".into(),
        String::new(),
    ];
    let normalized = normalize_domains(domains);
    assert_eq!(normalized, vec!["example.com", "docs.example.com"]);
}

#[test]
fn extract_host_works() {
    assert_eq!(
        extract_host("https://example.com/path").unwrap(),
        "example.com"
    );
    assert_eq!(
        extract_host("https://Sub.Example.COM:8080/").unwrap(),
        "sub.example.com"
    );
}

#[test]
fn extract_host_handles_ipv6() {
    // IPv6 with brackets (required for URLs with ports)
    assert_eq!(extract_host("https://[::1]/path").unwrap(), "[::1]");
    // IPv6 with brackets and port
    assert_eq!(
        extract_host("https://[2001:db8::1]:8080/path").unwrap(),
        "[2001:db8::1]"
    );
    // IPv6 with brackets, trailing slash
    assert_eq!(extract_host("https://[fe80::1]/").unwrap(), "[fe80::1]");
}

#[test]
fn is_private_host_detects_local() {
    assert!(is_private_host("localhost"));
    assert!(is_private_host("127.0.0.1"));
    assert!(is_private_host("192.168.1.1"));
    assert!(is_private_host("10.0.0.1"));
    assert!(!is_private_host("example.com"));
    assert!(!is_private_host("google.com"));
}

#[test]
fn is_private_host_catches_ipv6() {
    assert!(is_private_host("::1"));
    assert!(is_private_host("[::1]"));
    assert!(is_private_host("0.0.0.0"));
}

#[test]
fn is_private_host_catches_mapped_ipv4() {
    // IPv4-mapped IPv6 addresses
    assert!(is_private_host("::ffff:127.0.0.1"));
    assert!(is_private_host("::ffff:10.0.0.1"));
    assert!(is_private_host("::ffff:192.168.1.1"));
}

#[test]
fn is_private_host_catches_ipv6_private_ranges() {
    // Unique-local (fc00::/7)
    assert!(is_private_host("fd00::1"));
    assert!(is_private_host("fc00::1"));
    // Link-local (fe80::/10)
    assert!(is_private_host("fe80::1"));
    // Public IPv6 should pass
    assert!(!is_private_host("2001:db8::1"));
}

#[tokio::test]
async fn validate_url_blocks_ipv6_ssrf() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec!["*".into()], None);
    assert!(tool.validate_url("https://[::1]/").await.is_err());
    assert!(
        tool.validate_url("https://[::ffff:127.0.0.1]/")
            .await
            .is_err()
    );
    assert!(
        tool.validate_url("https://[::ffff:10.0.0.1]:8080/")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn validate_url_blocks_dns_resolved_private_host() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec!["*".into()], None);

    let err = tool
        .validate_url("https://localhost./")
        .await
        .expect_err("localhost. should resolve to loopback and be blocked");
    assert!(err.to_string().contains("SSRF blocked"));
}

#[test]
fn host_matches_allowlist_exact() {
    let allowed = vec!["example.com".into()];
    assert!(host_in_allowlist("example.com", &allowed));
    assert!(host_in_allowlist("sub.example.com", &allowed));
    assert!(!host_in_allowlist("notexample.com", &allowed));
}

#[test]
fn host_matches_allowlist_wildcard() {
    let allowed = vec!["*.example.com".into()];
    assert!(host_in_allowlist("sub.example.com", &allowed));
    assert!(host_in_allowlist("example.com", &allowed));
    assert!(!host_in_allowlist("other.com", &allowed));
}

#[test]
fn host_matches_allowlist_star() {
    let allowed = vec!["*".into()];
    assert!(host_in_allowlist("anything.com", &allowed));
    assert!(host_in_allowlist("example.org", &allowed));
}

#[test]
fn browser_tool_name() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec!["example.com".into()], None);
    assert_eq!(tool.name(), "browser");
}

#[tokio::test]
async fn browser_tool_validates_url() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec!["example.com".into()], None);

    // Valid — use example.com which resolves. Subdomain matching is tested
    // separately in the sync host_in_allowlist tests.
    assert!(tool.validate_url("https://example.com").await.is_ok());
    assert!(tool.validate_url("https://example.com/path").await.is_ok());

    // Invalid - not in allowlist
    assert!(tool.validate_url("https://other.com").await.is_err());

    // Invalid - private host
    assert!(tool.validate_url("https://localhost").await.is_err());
    assert!(tool.validate_url("https://127.0.0.1").await.is_err());

    // Invalid - not https
    assert!(tool.validate_url("ftp://example.com").await.is_err());

    assert!(tool.validate_url("file:///tmp/test.html").await.is_err());
}

#[tokio::test]
async fn browser_tool_empty_allowlist_blocks() {
    let security = Arc::new(SecurityPolicy::default());
    let tool = BrowserTool::new(security, vec![], None);
    assert!(tool.validate_url("https://example.com").await.is_err());
}
