//! SSRF protection — validates outbound URLs against private/internal IP ranges.

use std::net::{IpAddr, SocketAddr};

use crate::contracts::network::{is_private_host, is_private_ip};
use crate::contracts::strings::verdicts::{
    SSRF_BLOCK_PREFIX, URL_HAS_NO_HOST, URL_REQUIRES_HTTP_OR_HTTPS, URL_REQUIRES_HTTPS,
    URL_USERINFO_NOT_ALLOWED,
};

/// Validate a URL for SSRF safety by resolving DNS and checking all IPs.
///
/// **Known limitation — DNS rebinding:** The DNS lookup happens at validation
/// time. A DNS-rebinding attack could return a public IP during validation but
/// resolve to a private IP (e.g. `169.254.169.254`) at actual request time.
/// Full mitigation requires pinning the resolved IPs on the HTTP client
/// connection (e.g. via `reqwest::dns::Resolve`). Callers that perform the
/// actual fetch should additionally validate the connected IP post-connect.
///
/// # Errors
/// Returns an error if URL parsing fails or host validation detects private/internal routing.
pub async fn validate_no_ssrf(url_str: &str) -> anyhow::Result<()> {
    let parsed = url::Url::parse(url_str).map_err(|e| anyhow::anyhow!("invalid URL: {e}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("{URL_HAS_NO_HOST}"))?;
    if is_private_host(host) {
        anyhow::bail!("{SSRF_BLOCK_PREFIX} host '{host}' resolves to private/internal address");
    }

    // Block hostnames that look like IP addresses but use alternate encodings
    // (e.g. octal `0177.0.0.1`, hex `0x7f000001`, or decimal `2130706433`).
    if host
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.' || c == 'x' || c == 'X')
    {
        // Try parsing as a single decimal integer (e.g. `2130706433` = 127.0.0.1).
        if let Ok(numeric) = host.parse::<u32>() {
            let ip = IpAddr::V4(std::net::Ipv4Addr::from(numeric));
            if is_private_ip(&ip) {
                anyhow::bail!(
                    "{SSRF_BLOCK_PREFIX} numeric host '{host}' decodes to private address {ip}"
                );
            }
        }
        // Try parsing as hex (e.g. `0x7f000001`).
        if let Some(hex_str) = host.strip_prefix("0x").or_else(|| host.strip_prefix("0X"))
            && let Ok(numeric) = u32::from_str_radix(hex_str, 16)
        {
            let ip = IpAddr::V4(std::net::Ipv4Addr::from(numeric));
            if is_private_ip(&ip) {
                anyhow::bail!(
                    "{SSRF_BLOCK_PREFIX} hex host '{host}' decodes to private address {ip}"
                );
            }
        }
    }

    let _ = resolve_public_fetch_addrs(&parsed).await?;
    Ok(())
}

/// Resolve a validated fetch URL to public socket addresses that callers can
/// pin into their HTTP client for the actual request.
///
/// This is the fetch-time half of SSRF defense: it rejects DNS answers that
/// include private/internal IPs and returns the exact public addresses that a
/// caller should install with `reqwest::ClientBuilder::resolve_to_addrs`.
/// Pinning these addresses avoids a second, unvalidated resolver lookup during
/// the HTTP connection attempt.
///
/// # Errors
/// Returns an error if the URL has no host, DNS lookup fails, no addresses are
/// returned, or any resolved IP is private/internal.
pub(crate) async fn resolve_public_fetch_addrs(
    parsed: &url::Url,
) -> anyhow::Result<Vec<SocketAddr>> {
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("{URL_HAS_NO_HOST}"))?;
    let port = parsed.port_or_known_default().unwrap_or(443);

    let addrs = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, port)]
    } else {
        let addr_str = format!("{host}:{port}");
        // Fail-closed: DNS lookup failure means we cannot verify the target is
        // safe, so we reject the URL rather than silently allowing it through.
        tokio::net::lookup_host(&addr_str)
            .await
            .map_err(|e| {
                anyhow::anyhow!("{SSRF_BLOCK_PREFIX} DNS lookup failed for '{host}': {e}")
            })?
            .collect()
    };

    if addrs.is_empty() {
        anyhow::bail!("{SSRF_BLOCK_PREFIX} DNS lookup returned no addresses for '{host}'");
    }

    for addr in &addrs {
        if is_private_ip(&addr.ip()) {
            anyhow::bail!(
                "{SSRF_BLOCK_PREFIX} host '{host}' resolves to private address {}",
                addr.ip()
            );
        }
    }

    Ok(addrs)
}

/// Validate an external fetch URL for scheme safety and SSRF protection.
///
/// - `require_https=true`: only `https://` is allowed.
/// - `require_https=false`: `http://` and `https://` are allowed.
/// - userinfo (`user:pass@host`) is rejected.
/// - private/internal hosts and DNS resolutions are rejected.
///
/// # Errors
/// Returns an error if scheme/userinfo/host checks fail or SSRF validation fails.
pub async fn validate_fetch_url(url_str: &str, require_https: bool) -> anyhow::Result<url::Url> {
    let parsed = url::Url::parse(url_str).map_err(|e| anyhow::anyhow!("invalid URL: {e}"))?;

    match parsed.scheme() {
        "https" => {}
        "http" if !require_https => {}
        _ if require_https => anyhow::bail!("{URL_REQUIRES_HTTPS}"),
        _ => anyhow::bail!("{URL_REQUIRES_HTTP_OR_HTTPS}"),
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        anyhow::bail!("{URL_USERINFO_NOT_ALLOWED}");
    }

    if parsed.host_str().is_none() {
        anyhow::bail!("{URL_HAS_NO_HOST}");
    }

    validate_no_ssrf(url_str).await?;
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_loopback_v4() {
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        assert!(is_private_ip(&ip));
    }

    #[test]
    fn rejects_loopback_v6() {
        let ip: IpAddr = "::1".parse().unwrap();
        assert!(is_private_ip(&ip));
    }

    #[test]
    fn rejects_rfc1918_10() {
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(is_private_ip(&ip));
    }

    #[test]
    fn rejects_rfc1918_172() {
        let ip1: IpAddr = "172.16.0.1".parse().unwrap();
        let ip2: IpAddr = "172.31.255.255".parse().unwrap();
        assert!(is_private_ip(&ip1));
        assert!(is_private_ip(&ip2));
    }

    #[test]
    fn rejects_rfc1918_192() {
        let ip: IpAddr = "192.168.1.1".parse().unwrap();
        assert!(is_private_ip(&ip));
    }

    #[test]
    fn rejects_link_local() {
        let ip: IpAddr = "169.254.1.1".parse().unwrap();
        assert!(is_private_ip(&ip));
    }

    #[test]
    fn rejects_cloud_metadata() {
        let ip: IpAddr = "169.254.169.254".parse().unwrap();
        assert!(is_private_ip(&ip));
    }

    #[test]
    fn rejects_unique_local_v6() {
        let ip1: IpAddr = "fc00::1".parse().unwrap();
        let ip2: IpAddr = "fd00::1".parse().unwrap();
        assert!(is_private_ip(&ip1));
        assert!(is_private_ip(&ip2));
    }

    #[test]
    fn allows_public_ip() {
        let ip1: IpAddr = "8.8.8.8".parse().unwrap();
        let ip2: IpAddr = "1.1.1.1".parse().unwrap();
        assert!(!is_private_ip(&ip1));
        assert!(!is_private_ip(&ip2));
    }

    #[test]
    fn rejects_localhost_string() {
        assert!(is_private_host("localhost"));
    }

    #[test]
    fn allows_hostname() {
        assert!(!is_private_host("example.com"));
    }

    #[tokio::test]
    async fn validate_external_fetch_url_accepts_public_https() {
        let parsed = validate_fetch_url("https://8.8.8.8/path", true)
            .await
            .expect("public https should pass");
        assert_eq!(parsed.scheme(), "https");
    }

    #[tokio::test]
    async fn resolve_public_fetch_addrs_returns_pinnable_public_addresses() {
        let parsed = validate_fetch_url("https://8.8.8.8/path", true)
            .await
            .expect("public https should pass");
        let addrs = resolve_public_fetch_addrs(&parsed)
            .await
            .expect("public IP should resolve to pinnable socket addresses");

        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].ip(), "8.8.8.8".parse::<IpAddr>().unwrap());
        assert_eq!(addrs[0].port(), 443);
    }

    #[tokio::test]
    async fn resolve_public_fetch_addrs_rejects_private_addresses() {
        let parsed = url::Url::parse("https://127.0.0.1/path").unwrap();
        let err = resolve_public_fetch_addrs(&parsed)
            .await
            .expect_err("private pinned address must be rejected");

        assert!(err.to_string().contains("private address"));
    }

    #[tokio::test]
    async fn validate_external_fetch_url_rejects_http_when_https_required() {
        let err = validate_fetch_url("http://8.8.8.8/path", true)
            .await
            .expect_err("http should fail when https is required");
        assert!(err.to_string().contains("https://"));
    }

    #[tokio::test]
    async fn validate_external_fetch_url_rejects_non_http_schemes() {
        let err = validate_fetch_url("file:///tmp/example.txt", false)
            .await
            .expect_err("file:// should be rejected");
        assert!(err.to_string().contains("http:// and https://"));
    }

    #[tokio::test]
    async fn validate_external_fetch_url_rejects_userinfo() {
        let err = validate_fetch_url("https://user:pass@8.8.8.8/path", true)
            .await
            .expect_err("userinfo should be rejected");
        assert!(err.to_string().contains("userinfo"));
    }

    #[tokio::test]
    async fn validate_external_fetch_url_rejects_private_host() {
        let err = validate_fetch_url("https://127.0.0.1/path", true)
            .await
            .expect_err("private hosts should be rejected");
        assert!(err.to_string().contains("private/internal"));
    }

    // ── Coverage gap tests ─────────────────────────────────

    #[tokio::test]
    async fn numeric_decimal_ip_detected() {
        // 2130706433 = 127.0.0.1 in decimal form.
        let err = validate_no_ssrf("https://2130706433/path")
            .await
            .expect_err("decimal loopback must be blocked");
        assert!(err.to_string().contains("SSRF"));
    }

    #[tokio::test]
    async fn numeric_hex_ip_detected() {
        // 0x7f000001 = 127.0.0.1 in hex form.
        let err = validate_no_ssrf("https://0x7f000001/path")
            .await
            .expect_err("hex loopback must be blocked");
        assert!(err.to_string().contains("SSRF"));
    }

    #[test]
    fn ipv4_mapped_ipv6_private() {
        // ::ffff:127.0.0.1 is IPv4-mapped IPv6 for loopback.
        let ip: IpAddr = "::ffff:127.0.0.1".parse().unwrap();
        assert!(is_private_ip(&ip), "IPv4-mapped loopback must be private");
    }

    #[test]
    fn metadata_ip_blocked() {
        // 169.254.169.254 is the cloud metadata endpoint.
        assert!(is_private_host("169.254.169.254"));
    }

    #[test]
    fn bracketed_ipv6_parsed() {
        // is_private_host must handle bracketed IPv6 addresses.
        assert!(is_private_host("[::1]"));
        assert!(!is_private_host("[2001:4860:4860::8888]"));
    }

    #[test]
    fn empty_host_not_private() {
        // Empty string is not a recognized private host.
        assert!(!is_private_host(""));
    }

    #[tokio::test]
    async fn userinfo_special_chars_rejected() {
        let err = validate_fetch_url("https://admin%40evil@8.8.8.8/path", true)
            .await
            .expect_err("percent-encoded userinfo must be rejected");
        assert!(err.to_string().contains("userinfo"));
    }

    mod proptest_cases {
        use proptest::prelude::*;

        use super::*;

        proptest! {
            #[test]
            fn loopback_always_private(last3 in (0u8.., 0u8.., 0u8..)) {
                let ip: IpAddr = format!("127.{}.{}.{}", last3.0, last3.1, last3.2)
                    .parse()
                    .unwrap();
                prop_assert!(is_private_ip(&ip), "loopback {ip} must be private");
            }

            #[test]
            fn rfc1918_always_private(
                range in 0u8..3,
                b in 0u8..=255,
                c in 0u8..=255,
                d in 0u8..=255,
            ) {
                // Generate only valid RFC1918 addresses — no discards.
                let ip_str = match range {
                    0 => format!("10.{b}.{c}.{d}"),
                    1 => {
                        let b_clamped = 16 + (b % 16); // 16..=31
                        format!("172.{b_clamped}.{c}.{d}")
                    }
                    _ => format!("192.168.{c}.{d}"),
                };
                let ip: IpAddr = ip_str.parse().unwrap();
                prop_assert!(is_private_ip(&ip), "RFC1918 {ip} must be private");
            }

            #[test]
            fn numeric_ip_decimal_detected(a in 0u8..=255, b in 0u8..=255, c in 0u8..=255, d in 0u8..=255) {
                let ip = std::net::Ipv4Addr::new(a, b, c, d);
                let numeric: u32 = u32::from(ip);
                let host = numeric.to_string();
                // If the IP is private, is_private_host on the decimal form
                // may or may not detect it (numeric detection is in validate_no_ssrf).
                // Here we just verify no panics.
                let _ = is_private_host(&host);
            }
        }
    }
}
