//! Tailscale tunnel adapter: uses `tailscale serve` for tailnet-only
//! access or `tailscale funnel` for public internet exposure.

use anyhow::{Context, Result, bail};
use tokio::process::Command;

use super::{SharedProcess, Tunnel, TunnelProcess, kill_shared, new_shared_process};

/// Tailscale Tunnel — uses `tailscale serve` (tailnet-only) or
/// `tailscale funnel` (public internet).
///
/// Requires Tailscale installed and authenticated (`tailscale up`).
pub struct TailscaleTunnel {
    funnel: bool,
    hostname: Option<String>,
    proc: SharedProcess,
}

impl TailscaleTunnel {
    /// Create a new Tailscale tunnel, optionally using funnel mode and
    /// a custom hostname.
    #[must_use]
    pub fn new(funnel: bool, hostname: Option<String>) -> Self {
        Self {
            funnel,
            hostname,
            proc: new_shared_process(),
        }
    }
}

impl Tunnel for TailscaleTunnel {
    fn name(&self) -> &'static str {
        "tailscale"
    }

    fn start<'a>(
        &'a self,
        _local_host: &'a str,
        local_port: u16,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            let subcommand = if self.funnel { "funnel" } else { "serve" };

            // Get the tailscale hostname for URL construction
            let hostname = if let Some(ref h) = self.hostname {
                h.clone()
            } else {
                // Query tailscale for the current hostname
                let output = Command::new("tailscale")
                    .args(["status", "--json"])
                    .output()
                    .await?;

                if !output.status.success() {
                    bail!(
                        "tailscale status failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }

                let status: serde_json::Value = serde_json::from_slice(&output.stdout)
                    .context("failed to parse tailscale status JSON")?;
                status["Self"]["DNSName"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("tailscale status JSON missing Self.DNSName"))?
                    .trim_end_matches('.')
                    .to_string()
            };

            // tailscale serve|funnel <port>
            let child = Command::new("tailscale")
                .args([subcommand, &local_port.to_string()])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()?;

            let public_url = format!("https://{hostname}:{local_port}");

            let mut guard = self.proc.lock().await;
            *guard = Some(TunnelProcess {
                child,
                public_url: public_url.clone(),
            });

            Ok(public_url)
        })
    }

    fn stop(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move {
            // Also reset the tailscale serve/funnel
            let subcommand = if self.funnel { "funnel" } else { "serve" };
            let reset_result = Command::new("tailscale")
                .args([subcommand, "reset"])
                .output()
                .await;

            let kill_result = kill_shared(&self.proc).await;

            match reset_result {
                Ok(output) if output.status.success() => kill_result,
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    tracing::warn!(subcommand, %stderr, "tailscale reset failed; tunnel may persist");
                    kill_result?;
                    bail!("tailscale {subcommand} reset failed: {stderr}")
                }
                Err(error) => {
                    tracing::warn!(%error, subcommand, "tailscale reset failed; tunnel may persist");
                    kill_result?;
                    Err(error).context(format!("tailscale {subcommand} reset failed"))
                }
            }
        })
    }

    fn health_check(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send + '_>> {
        Box::pin(super::process::is_process_alive(&self.proc))
    }

    fn public_url(&self) -> Option<String> {
        super::process::try_read_public_url(&self.proc)
    }
}
