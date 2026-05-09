//! Unit tests for the tunnel factory and security policy enforcement.

use super::cloudflare::CloudflareTunnel;
use super::custom::CustomTunnel;
use super::ngrok::NgrokTunnel;
use super::none::NoneTunnel;
use super::tailscale::TailscaleTunnel;
use super::*;
use crate::config::schema::{
    CloudflareTunnelConfig, CustomTunnelConfig, NgrokTunnelConfig, TunnelConfig,
};
use crate::security::SecurityPolicy;

fn test_security_policy() -> SecurityPolicy {
    let mut policy = SecurityPolicy::default();
    policy.allowed_commands.extend(
        ["cloudflared", "tailscale", "ngrok"]
            .iter()
            .map(ToString::to_string),
    );
    policy
}

/// Helper: assert `create_tunnel` returns an error containing `needle`.
fn assert_tunnel_err(cfg: &TunnelConfig, needle: &str) {
    let security = test_security_policy();
    match create_tunnel(cfg, &security) {
        Err(e) => assert!(
            e.to_string().contains(needle),
            "Expected error containing \"{needle}\", got: {e}"
        ),
        Ok(_) => panic!("Expected error containing \"{needle}\", but got Ok"),
    }
}

#[test]
fn factory_none_returns_none() {
    let cfg = TunnelConfig::default();
    let security = test_security_policy();
    let t = create_tunnel(&cfg, &security).unwrap();
    assert!(t.is_none());
}

#[test]
fn factory_default_returns_none() {
    let cfg = TunnelConfig::default();
    let security = test_security_policy();
    let t = create_tunnel(&cfg, &security).unwrap();
    assert!(t.is_none());
}

#[test]
fn factory_cloudflare_missing_config_errors() {
    let cfg = TunnelConfig {
        provider: crate::config::TunnelProvider::Cloudflare,
        ..TunnelConfig::default()
    };
    assert_tunnel_err(&cfg, "[tunnel.cloudflare]");
}

#[test]
fn factory_cloudflare_with_config_ok() {
    let cfg = TunnelConfig {
        provider: crate::config::TunnelProvider::Cloudflare,
        cloudflare: Some(CloudflareTunnelConfig {
            token: "test-token".into(),
        }),
        ..TunnelConfig::default()
    };
    let security = test_security_policy();
    let t = create_tunnel(&cfg, &security).unwrap();
    assert!(t.is_some());
    assert_eq!(t.unwrap().name(), "cloudflare");
}

#[test]
fn factory_tailscale_defaults_ok() {
    let cfg = TunnelConfig {
        provider: crate::config::TunnelProvider::Tailscale,
        ..TunnelConfig::default()
    };
    let security = test_security_policy();
    let t = create_tunnel(&cfg, &security).unwrap();
    assert!(t.is_some());
    assert_eq!(t.unwrap().name(), "tailscale");
}

#[test]
fn factory_ngrok_missing_config_errors() {
    let cfg = TunnelConfig {
        provider: crate::config::TunnelProvider::Ngrok,
        ..TunnelConfig::default()
    };
    assert_tunnel_err(&cfg, "[tunnel.ngrok]");
}

#[test]
fn factory_ngrok_with_config_ok() {
    let cfg = TunnelConfig {
        provider: crate::config::TunnelProvider::Ngrok,
        ngrok: Some(NgrokTunnelConfig {
            auth_token: "tok".into(),
            domain: None,
        }),
        ..TunnelConfig::default()
    };
    let security = test_security_policy();
    let t = create_tunnel(&cfg, &security).unwrap();
    assert!(t.is_some());
    assert_eq!(t.unwrap().name(), "ngrok");
}

#[test]
fn factory_custom_missing_config_errors() {
    let cfg = TunnelConfig {
        provider: crate::config::TunnelProvider::Custom,
        ..TunnelConfig::default()
    };
    assert_tunnel_err(&cfg, "[tunnel.custom]");
}

#[test]
fn factory_custom_with_config_ok() {
    let cfg = TunnelConfig {
        provider: crate::config::TunnelProvider::Custom,
        custom: Some(CustomTunnelConfig {
            start_command: "echo https://example.test/{port}".into(),
            health_url: None,
            url_pattern: Some("https://".into()),
        }),
        ..TunnelConfig::default()
    };
    let security = test_security_policy();
    let t = create_tunnel(&cfg, &security).unwrap();
    assert!(t.is_some());
    assert_eq!(t.unwrap().name(), "custom");
}

#[test]
fn factory_custom_requires_port_placeholder_and_url_pattern() {
    let security = test_security_policy();
    let missing_port = TunnelConfig {
        provider: crate::config::TunnelProvider::Custom,
        custom: Some(CustomTunnelConfig {
            start_command: "echo https://example.test".into(),
            health_url: None,
            url_pattern: Some("https://".into()),
        }),
        ..TunnelConfig::default()
    };
    assert_tunnel_err(&missing_port, "{port}");

    let missing_pattern = TunnelConfig {
        provider: crate::config::TunnelProvider::Custom,
        custom: Some(CustomTunnelConfig {
            start_command: "echo https://example.test/{port}".into(),
            health_url: None,
            url_pattern: None,
        }),
        ..TunnelConfig::default()
    };
    let err = match create_tunnel(&missing_pattern, &security) {
        Err(error) => error.to_string(),
        Ok(_) => panic!("custom tunnel without url_pattern should fail"),
    };
    assert!(err.contains("url_pattern"));
}

#[test]
fn factory_ngrok_blocked_when_not_allowlisted() {
    let cfg = TunnelConfig {
        provider: crate::config::TunnelProvider::Ngrok,
        ngrok: Some(NgrokTunnelConfig {
            auth_token: "tok".into(),
            domain: None,
        }),
        ..TunnelConfig::default()
    };
    let security = SecurityPolicy {
        allowed_commands: vec!["git".to_string()],
        ..SecurityPolicy::default()
    };
    let Err(err) = create_tunnel(&cfg, &security) else {
        panic!("ngrok should be blocked");
    };
    assert!(err.to_string().contains("not allowlisted"));
}

#[test]
fn none_tunnel_name() {
    let t = NoneTunnel;
    assert_eq!(t.name(), "none");
}

#[test]
fn none_tunnel_public_url_is_none() {
    let t = NoneTunnel;
    assert!(t.public_url().is_none());
}

#[tokio::test]
async fn none_tunnel_health_always_true() {
    let t = NoneTunnel;
    assert!(t.health_check().await);
}

#[tokio::test]
async fn none_tunnel_start_returns_local() {
    let t = NoneTunnel;
    let url = t.start("127.0.0.1", 8080).await.unwrap();
    assert_eq!(url, "http://127.0.0.1:8080");
}

#[test]
fn cloudflare_tunnel_name() {
    let t = CloudflareTunnel::new("tok".into());
    assert_eq!(t.name(), "cloudflare");
    assert!(t.public_url().is_none());
}

#[test]
fn tailscale_tunnel_name() {
    let t = TailscaleTunnel::new(false, None);
    assert_eq!(t.name(), "tailscale");
    assert!(t.public_url().is_none());
}

#[test]
fn tailscale_funnel_mode() {
    let t = TailscaleTunnel::new(true, Some("myhost".into()));
    assert_eq!(t.name(), "tailscale");
}

#[test]
fn ngrok_tunnel_name() {
    let t = NgrokTunnel::new("tok".into(), None);
    assert_eq!(t.name(), "ngrok");
    assert!(t.public_url().is_none());
}

#[test]
fn ngrok_with_domain() {
    let t = NgrokTunnel::new("tok".into(), Some("my.ngrok.io".into()));
    assert_eq!(t.name(), "ngrok");
}

#[test]
fn custom_tunnel_name() {
    let t = CustomTunnel::new("echo hi".into(), None, None);
    assert_eq!(t.name(), "custom");
    assert!(t.public_url().is_none());
}
