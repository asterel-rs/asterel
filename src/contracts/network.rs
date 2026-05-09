//! SSRF prevention utilities.
//!
//! These functions classify IP addresses and host strings as "private" so
//! that the HTTP client layer can reject requests directed at internal
//! infrastructure before a connection is ever opened. This is the primary
//! defence against server-side request forgery (SSRF) attacks where an
//! adversary tricks the agent into fetching an internal endpoint.

use std::net::IpAddr;

/// Returns `true` when the IP address falls within a range that must not be
/// reached from an agent-initiated outbound request.
///
/// Considered private for both IPv4 and IPv6:
///
/// **IPv4**
/// - Loopback: `127.0.0.0/8` (`is_loopback`)
/// - RFC 1918 private: `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`
///   (`is_private`)
/// - Link-local: `169.254.0.0/16` (`is_link_local`)
/// - Unspecified: `0.0.0.0`
/// - Broadcast: `255.255.255.255`
/// - AWS EC2 metadata endpoint: `169.254.169.254` (explicit octet check,
///   caught by link-local but made explicit for clarity)
///
/// **IPv6**
/// - Loopback: `::1`
/// - Unspecified: `::`
/// - Unique-local: `fc00::/7` (addresses where `(seg[0] & 0xfe00) == 0xfc00`)
/// - Link-local: `fe80::/10` (addresses where `(seg[0] & 0xffc0) == 0xfe80`)
/// - IPv4-mapped addresses (`::ffff:x.x.x.x`) are unwrapped and checked
///   against the IPv4 rules above.
#[must_use]
pub fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.octets() == [169, 254, 169, 254]
        }
        IpAddr::V6(v6) => {
            let segs = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || (segs[0] & 0xfe00) == 0xfc00
                || (segs[0] & 0xffc0) == 0xfe80
                || v6.to_ipv4_mapped().is_some_and(|v4| {
                    v4.is_loopback()
                        || v4.is_private()
                        || v4.is_link_local()
                        || v4.is_unspecified()
                        || v4.is_broadcast()
                        || v4.octets() == [169, 254, 169, 254]
                })
        }
    }
}

/// Returns `true` when the host string resolves to a private address.
///
/// The check is intentionally conservative and operates without performing a
/// DNS lookup, so it catches only statically knowable private targets:
///
/// - The literal string `"localhost"` (case-sensitive after bracket-stripping).
/// - IPv6 literals enclosed in square brackets, e.g. `"[::1]"` — brackets are
///   stripped before parsing.
/// - Any bare IPv4 or IPv6 literal that `is_private_ip` classifies as private.
///
/// Hostnames that require DNS resolution (e.g. `"internal.corp"`) are **not**
/// blocked here; that class of SSRF must be handled at the DNS or connect
/// layer. This function is a fast pre-filter for the obvious cases.
#[must_use]
pub fn is_private_host(host: &str) -> bool {
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if bare == "localhost" {
        return true;
    }
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return is_private_ip(&ip);
    }
    false
}
