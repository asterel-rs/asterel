#![no_main]
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use arbitrary::Arbitrary;
use asterel::contracts::network::{is_private_host, is_private_ip};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct UrlValidationInput {
    host: String,
    ipv4_octets: [u8; 4],
    ipv6_octets: [u8; 16],
}

fuzz_target!(|input: UrlValidationInput| {
    // ── Host-string invariants ───────────────────────────────
    let host = &input.host;
    let result = is_private_host(host);

    // Known-true invariants.
    if host == "localhost" {
        assert!(result, "localhost must be private");
    }
    if host == "127.0.0.1" {
        assert!(result, "127.0.0.1 must be private");
    }
    if host == "::1" {
        assert!(result, "::1 must be private");
    }
    if host == "[::1]" {
        assert!(result, "[::1] must be private");
    }
    if host == "0.0.0.0" {
        assert!(result, "0.0.0.0 (unspecified) must be private");
    }
    if host == "10.0.0.1" {
        assert!(result, "10.0.0.1 must be private");
    }
    if host == "192.168.1.1" {
        assert!(result, "192.168.1.1 must be private");
    }
    if host == "169.254.169.254" {
        assert!(result, "cloud metadata endpoint must be private");
    }

    // Known-false invariants.
    if host == "8.8.8.8" {
        assert!(!result, "8.8.8.8 must not be private");
    }
    if host == "1.1.1.1" {
        assert!(!result, "1.1.1.1 must not be private");
    }

    // Consistency: is_private_host and is_private_ip must agree for
    // inputs that parse as IP addresses.
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = bare.parse::<IpAddr>() {
        assert_eq!(
            result,
            is_private_ip(&ip),
            "is_private_host and is_private_ip must agree for '{host}'"
        );
    }

    // ── IPv4 invariants ──────────────────────────────────────
    let [a, b, c, d] = input.ipv4_octets;
    let v4 = Ipv4Addr::new(a, b, c, d);
    let ip4 = IpAddr::V4(v4);
    let r4 = is_private_ip(&ip4);

    if a == 127 {
        assert!(r4, "127.x.x.x must be private");
    }
    if a == 10 {
        assert!(r4, "10.x.x.x must be private");
    }
    if a == 192 && b == 168 {
        assert!(r4, "192.168.x.x must be private");
    }
    if a == 172 && (16..=31).contains(&b) {
        assert!(r4, "172.16-31.x.x must be private");
    }
    if a == 169 && b == 254 {
        assert!(r4, "169.254.x.x must be private");
    }
    if v4.is_unspecified() {
        assert!(r4, "unspecified must be private");
    }
    if v4.is_broadcast() {
        assert!(r4, "broadcast must be private");
    }

    // ── IPv6 invariants ──────────────────────────────────────
    let v6 = Ipv6Addr::from(input.ipv6_octets);
    let ip6 = IpAddr::V6(v6);
    let r6 = is_private_ip(&ip6);

    if v6.is_loopback() {
        assert!(r6, "IPv6 loopback must be private");
    }
    if v6.is_unspecified() {
        assert!(r6, "IPv6 unspecified must be private");
    }
    let segs = v6.segments();
    if (segs[0] & 0xfe00) == 0xfc00 {
        assert!(r6, "unique-local IPv6 must be private");
    }
    if (segs[0] & 0xffc0) == 0xfe80 {
        assert!(r6, "link-local IPv6 must be private");
    }
});
