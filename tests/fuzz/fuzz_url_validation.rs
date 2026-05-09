use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use asterel::contracts::network::{is_private_host, is_private_ip};

use crate::support;

#[test]
fn fuzz_is_private_host() {
    // Constant assertion: localhost must always be private.
    assert!(is_private_host("localhost"));

    support::for_each_fuzz_input(10_000, 256, |data| {
        let Ok(host) = std::str::from_utf8(data) else {
            return;
        };
        // Must not panic on any input.
        let _ = is_private_host(host);
    });
}

#[test]
fn fuzz_is_private_ip() {
    support::for_each_fuzz_input(10_000, 16, |data| {
        if data.len() < 4 {
            return;
        }

        // Test IPv4 — always safe to construct from 4 bytes.
        let v4 = Ipv4Addr::new(data[0], data[1], data[2], data[3]);
        let ip4 = IpAddr::V4(v4);
        let is_private = is_private_ip(&ip4);

        // Loopback (127.x.x.x) must always be detected as private.
        if v4.is_loopback() {
            assert!(is_private, "loopback {v4} must be private");
        }

        // RFC1918 must always be detected as private.
        if v4.is_private() {
            assert!(is_private, "RFC1918 {v4} must be private");
        }

        // Link-local (169.254.x.x) must be detected as private.
        if v4.is_link_local() {
            assert!(is_private, "link-local {v4} must be private");
        }

        // Unspecified (0.0.0.0) must be detected as private.
        if v4.is_unspecified() {
            assert!(is_private, "unspecified {v4} must be private");
        }

        // Broadcast (255.255.255.255) must be detected as private.
        if v4.is_broadcast() {
            assert!(is_private, "broadcast {v4} must be private");
        }

        // Test IPv6 if enough bytes.
        if data.len() >= 16 {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(&data[..16]);
            let v6 = Ipv6Addr::from(octets);
            let ip6 = IpAddr::V6(v6);
            let is_private6 = is_private_ip(&ip6);

            if v6.is_loopback() {
                assert!(is_private6, "loopback {v6} must be private");
            }

            // Unique local (fc00::/7) must be detected as private.
            let first_byte = octets[0];
            if first_byte == 0xfc || first_byte == 0xfd {
                assert!(is_private6, "unique-local {v6} must be private");
            }
        }
    });
}
