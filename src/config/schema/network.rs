//! Process-level network configuration such as outbound proxy settings.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use url::Url;

/// Outbound network settings for runtime surfaces.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkConfig {
    /// Optional HTTP(S) proxy URL applied to outbound integrations.
    #[serde(default)]
    pub proxy: Option<String>,
}

impl NetworkConfig {
    /// Validate the configured network settings.
    ///
    /// # Errors
    ///
    /// Returns an error when the configured proxy is not a valid absolute
    /// HTTP(S) URL.
    pub fn validate(&self) -> Result<()> {
        let Some(proxy) = self.proxy.as_deref() else {
            return Ok(());
        };

        let trimmed = proxy.trim();
        if trimmed.is_empty() {
            anyhow::bail!("network.proxy cannot be empty when set");
        }

        let parsed = Url::parse(trimmed).context("network.proxy must be a valid absolute URL")?;
        if !matches!(parsed.scheme(), "http" | "https") {
            anyhow::bail!("network.proxy must use http:// or https://");
        }
        if parsed.host_str().is_none() {
            anyhow::bail!("network.proxy must include a host");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::NetworkConfig;

    #[test]
    fn default_network_config_has_no_proxy() {
        let config = NetworkConfig::default();
        assert!(config.proxy.is_none());
    }

    #[test]
    fn validate_accepts_http_proxy() {
        let config = NetworkConfig {
            proxy: Some("https://proxy.example:8443".to_string()),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_non_http_proxy() {
        let config = NetworkConfig {
            proxy: Some("socks5://proxy.example:1080".to_string()),
        };
        let error = config
            .validate()
            .expect_err("proxy scheme should be rejected");
        assert!(error.to_string().contains("http:// or https://"));
    }

    #[test]
    fn network_config_toml_round_trip() {
        let original = NetworkConfig {
            proxy: Some("http://127.0.0.1:8080".to_string()),
        };

        let toml = toml::to_string(&original).expect("serialize network config");
        let decoded: NetworkConfig = toml::from_str(&toml).expect("deserialize network config");

        assert_eq!(decoded.proxy, original.proxy);
    }
}
