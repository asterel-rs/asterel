//! Tunnel factory: creates the right tunnel adapter from config with
//! security policy enforcement for process spawning.

use anyhow::{Result, bail};

use super::Tunnel;
use super::cloudflare::CloudflareTunnel;
use super::custom::CustomTunnel;
use super::ngrok::NgrokTunnel;
use super::tailscale::TailscaleTunnel;
use crate::config::schema::{TailscaleTunnelConfig, TunnelConfig};
use crate::security::{
    ProcessSpawnClass, SecurityPolicy, enforce_process_spawn_policy_with_args, enforce_spawn_policy,
};

/// Create a tunnel from config. Returns `None` for provider "none".
///
/// # Errors
///
/// Returns an error when tunnel configuration is invalid or when process spawn
/// policy rejects the selected tunnel provider.
pub fn create_tunnel(
    config: &TunnelConfig,
    security: &SecurityPolicy,
) -> Result<Option<Box<dyn Tunnel>>> {
    match config.provider {
        crate::config::TunnelProvider::None => Ok(None),

        crate::config::TunnelProvider::Cloudflare => {
            enforce_spawn_policy(
                security,
                "cloudflared",
                "runtime_tunnel_cloudflare",
                ProcessSpawnClass::ExternalConnector,
            )?;
            let cf = config.cloudflare.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "tunnel.provider = \"cloudflare\" but [tunnel.cloudflare] section is missing"
                )
            })?;
            Ok(Some(Box::new(CloudflareTunnel::new(cf.token.clone()))))
        }

        crate::config::TunnelProvider::Tailscale => {
            enforce_spawn_policy(
                security,
                "tailscale",
                "runtime_tunnel_tailscale",
                ProcessSpawnClass::ExternalConnector,
            )?;
            let ts = config.tailscale.as_ref().unwrap_or(&TailscaleTunnelConfig {
                funnel: false,
                hostname: None,
            });
            Ok(Some(Box::new(TailscaleTunnel::new(
                ts.funnel,
                ts.hostname.clone(),
            ))))
        }

        crate::config::TunnelProvider::Ngrok => {
            enforce_spawn_policy(
                security,
                "ngrok",
                "runtime_tunnel_ngrok",
                ProcessSpawnClass::ExternalConnector,
            )?;
            let ng = config.ngrok.as_ref().ok_or_else(|| {
                anyhow::anyhow!("tunnel.provider = \"ngrok\" but [tunnel.ngrok] section is missing")
            })?;
            Ok(Some(Box::new(NgrokTunnel::new(
                ng.auth_token.clone(),
                ng.domain.clone(),
            ))))
        }

        crate::config::TunnelProvider::Custom => {
            let cu = config.custom.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "tunnel.provider = \"custom\" but [tunnel.custom] section is missing"
                )
            })?;
            let parts = shlex::split(&cu.start_command).ok_or_else(|| {
                anyhow::anyhow!("custom tunnel start_command has unmatched quotes")
            })?;
            let command = parts
                .first()
                .ok_or_else(|| anyhow::anyhow!("custom tunnel start_command is empty"))?;
            let args: Vec<String> = parts[1..].to_vec();
            if !cu.start_command.contains("{port}") {
                bail!("custom tunnel start_command must include the {{port}} placeholder");
            }
            if cu.url_pattern.as_deref().is_none_or(str::is_empty) {
                bail!("custom tunnel requires url_pattern to extract a public HTTPS URL");
            }
            enforce_process_spawn_policy_with_args(
                security,
                command,
                &args,
                "runtime_tunnel_custom",
                ProcessSpawnClass::ExternalConnector,
            )?;
            // Defense-in-depth: also check the full command string
            if !security.is_command_allowed(&cu.start_command) {
                bail!(
                    "custom tunnel start_command rejected by \
                     security policy"
                );
            }
            Ok(Some(Box::new(CustomTunnel::new(
                cu.start_command.clone(),
                cu.health_url.clone(),
                cu.url_pattern.clone(),
            ))))
        }
    }
}
