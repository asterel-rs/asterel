//! Tunnel provider configuration for exposing the gateway publicly
//! via Cloudflare, Tailscale, ngrok, or a custom command.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Tunnel provider backend.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TunnelProvider {
    /// No tunnel (gateway is only accessible locally).
    #[default]
    None,
    /// Cloudflare Tunnel via Zero Trust.
    Cloudflare,
    /// Tailscale Serve/Funnel.
    Tailscale,
    /// ngrok tunnel.
    Ngrok,
    /// Custom tunnel command.
    Custom,
}

impl fmt::Display for TunnelProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Cloudflare => write!(f, "cloudflare"),
            Self::Tailscale => write!(f, "tailscale"),
            Self::Ngrok => write!(f, "ngrok"),
            Self::Custom => write!(f, "custom"),
        }
    }
}

/// Top-level tunnel configuration for public gateway exposure.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TunnelConfig {
    /// Selected tunnel provider.
    pub provider: TunnelProvider,

    /// Cloudflare Tunnel configuration.
    #[serde(default)]
    pub cloudflare: Option<CloudflareTunnelConfig>,

    /// Tailscale Serve/Funnel configuration.
    #[serde(default)]
    pub tailscale: Option<TailscaleTunnelConfig>,

    /// ngrok tunnel configuration.
    #[serde(default)]
    pub ngrok: Option<NgrokTunnelConfig>,

    /// Custom tunnel command configuration.
    #[serde(default)]
    pub custom: Option<CustomTunnelConfig>,
}

/// Cloudflare Tunnel (Zero Trust) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudflareTunnelConfig {
    /// Cloudflare Tunnel token (from Zero Trust dashboard)
    pub token: String,
}

/// Tailscale Serve/Funnel tunnel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TailscaleTunnelConfig {
    /// Use Tailscale Funnel (public internet) vs Serve (tailnet only)
    #[serde(default)]
    pub funnel: bool,
    /// Optional hostname override
    pub hostname: Option<String>,
}

/// ngrok tunnel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NgrokTunnelConfig {
    /// ngrok auth token
    pub auth_token: String,
    /// Optional custom domain
    pub domain: Option<String>,
}

/// Custom tunnel via user-defined command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomTunnelConfig {
    /// Command template to start the tunnel. Use {port} and {host} placeholders.
    /// Example: "bore local {port} --to bore.pub"
    pub start_command: String,
    /// Optional URL to check tunnel health
    pub health_url: Option<String>,
    /// Optional regex to extract public URL from command stdout
    pub url_pattern: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tunnel_config() {
        let config = TunnelConfig::default();

        assert_eq!(config.provider, TunnelProvider::None);
        assert!(config.cloudflare.is_none());
        assert!(config.tailscale.is_none());
        assert!(config.ngrok.is_none());
        assert!(config.custom.is_none());
    }

    #[test]
    fn tunnel_config_toml_round_trip() {
        let original = TunnelConfig {
            provider: TunnelProvider::Custom,
            cloudflare: Some(CloudflareTunnelConfig {
                token: "cf-token".into(),
            }),
            tailscale: Some(TailscaleTunnelConfig {
                funnel: true,
                hostname: Some("agent.tailnet.ts.net".into()),
            }),
            ngrok: Some(NgrokTunnelConfig {
                auth_token: "ngrok-token".into(),
                domain: Some("agent.ngrok.app".into()),
            }),
            custom: Some(CustomTunnelConfig {
                start_command: "bore local {port} --to bore.pub".into(),
                health_url: Some("https://agent.example/health".into()),
                url_pattern: Some("https://[^\\s]+".into()),
            }),
        };

        let toml = toml::to_string(&original).unwrap();
        let decoded: TunnelConfig = toml::from_str(&toml).unwrap();

        assert_eq!(decoded.provider, original.provider);
        assert_eq!(
            decoded.cloudflare.as_ref().map(|cfg| cfg.token.as_str()),
            original.cloudflare.as_ref().map(|cfg| cfg.token.as_str())
        );
        assert_eq!(
            decoded.tailscale.as_ref().map(|cfg| cfg.funnel),
            original.tailscale.as_ref().map(|cfg| cfg.funnel)
        );
        assert_eq!(
            decoded
                .tailscale
                .as_ref()
                .and_then(|cfg| cfg.hostname.as_deref()),
            original
                .tailscale
                .as_ref()
                .and_then(|cfg| cfg.hostname.as_deref())
        );
        assert_eq!(
            decoded.ngrok.as_ref().map(|cfg| cfg.auth_token.as_str()),
            original.ngrok.as_ref().map(|cfg| cfg.auth_token.as_str())
        );
        assert_eq!(
            decoded.ngrok.as_ref().and_then(|cfg| cfg.domain.as_deref()),
            original
                .ngrok
                .as_ref()
                .and_then(|cfg| cfg.domain.as_deref())
        );
        assert_eq!(
            decoded
                .custom
                .as_ref()
                .map(|cfg| cfg.start_command.as_str()),
            original
                .custom
                .as_ref()
                .map(|cfg| cfg.start_command.as_str())
        );
        assert_eq!(
            decoded
                .custom
                .as_ref()
                .and_then(|cfg| cfg.health_url.as_deref()),
            original
                .custom
                .as_ref()
                .and_then(|cfg| cfg.health_url.as_deref())
        );
        assert_eq!(
            decoded
                .custom
                .as_ref()
                .and_then(|cfg| cfg.url_pattern.as_deref()),
            original
                .custom
                .as_ref()
                .and_then(|cfg| cfg.url_pattern.as_deref())
        );
    }

    #[test]
    fn tunnel_config_deserializes_all_provider_configs() {
        let toml = r#"
provider = "cloudflare"

[cloudflare]
token = "cf-token"

[tailscale]
funnel = true
hostname = "agent.tailnet.ts.net"

[ngrok]
auth_token = "ngrok-token"
domain = "agent.ngrok.app"

[custom]
start_command = "ssh -R 80:{host}:{port} serveo.net"
health_url = "https://agent.example/health"
url_pattern = "https://[^\\s]+"
"#;

        let config: TunnelConfig = toml::from_str(toml).unwrap();

        assert_eq!(config.provider, TunnelProvider::Cloudflare);
        assert_eq!(
            config.cloudflare.as_ref().map(|cfg| cfg.token.as_str()),
            Some("cf-token")
        );
        assert_eq!(config.tailscale.as_ref().map(|cfg| cfg.funnel), Some(true));
        assert_eq!(
            config
                .tailscale
                .as_ref()
                .and_then(|cfg| cfg.hostname.as_deref()),
            Some("agent.tailnet.ts.net")
        );
        assert_eq!(
            config.ngrok.as_ref().map(|cfg| cfg.auth_token.as_str()),
            Some("ngrok-token")
        );
        assert_eq!(
            config.ngrok.as_ref().and_then(|cfg| cfg.domain.as_deref()),
            Some("agent.ngrok.app")
        );
        assert_eq!(
            config.custom.as_ref().map(|cfg| cfg.start_command.as_str()),
            Some("ssh -R 80:{host}:{port} serveo.net")
        );
        assert_eq!(
            config
                .custom
                .as_ref()
                .and_then(|cfg| cfg.health_url.as_deref()),
            Some("https://agent.example/health")
        );
        assert_eq!(
            config
                .custom
                .as_ref()
                .and_then(|cfg| cfg.url_pattern.as_deref()),
            Some("https://[^\\s]+")
        );
    }
}
