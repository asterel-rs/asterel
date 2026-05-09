//! Cloudflare tunnel adapter: wraps the `cloudflared` binary to
//! expose local ports via Cloudflare Zero Trust.

use anyhow::{Result, bail};
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use zeroize::Zeroizing;

use super::{SharedProcess, Tunnel, TunnelProcess, kill_shared, new_shared_process};

/// Cloudflare Tunnel — wraps the `cloudflared` binary.
///
/// Requires `cloudflared` installed and a tunnel token from the
/// Cloudflare Zero Trust dashboard.
pub struct CloudflareTunnel {
    /// Zeroized on drop to prevent token lingering in process memory.
    token: Zeroizing<String>,
    proc: SharedProcess,
}

impl CloudflareTunnel {
    /// Create a new Cloudflare tunnel with the given Zero Trust token.
    #[must_use]
    pub fn new(token: String) -> Self {
        Self {
            token: Zeroizing::new(token),
            proc: new_shared_process(),
        }
    }
}

impl Tunnel for CloudflareTunnel {
    fn name(&self) -> &'static str {
        "cloudflare"
    }

    fn start<'a>(
        &'a self,
        _local_host: &'a str,
        local_port: u16,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            // cloudflared tunnel --no-autoupdate run --token <TOKEN> --url http://localhost:<port>
            // Pass token via environment variable to avoid exposing it
            // in /proc/<pid>/cmdline (same approach as ngrok tunnel).
            let mut child = Command::new("cloudflared")
                .args([
                    "tunnel",
                    "--no-autoupdate",
                    "run",
                    "--url",
                    &format!("http://localhost:{local_port}"),
                ])
                .env("TUNNEL_TOKEN", self.token.as_str())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()?;

            // Read stderr to find the public URL (cloudflared prints it there)
            let stderr = child
                .stderr
                .take()
                .ok_or_else(|| anyhow::anyhow!("failed to capture cloudflared stderr pipe: tunnel process may have exited before the tunnel was established"))?;

            let mut reader = tokio::io::BufReader::new(stderr).lines();
            let mut public_url = String::new();

            // Wait up to 30s for the tunnel URL to appear
            let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(30);
            while tokio::time::Instant::now() < deadline {
                let line =
                    tokio::time::timeout(tokio::time::Duration::from_secs(5), reader.next_line())
                        .await;

                match line {
                    Ok(Ok(Some(l))) => {
                        tracing::debug!("cloudflared: {l}");
                        if let Some(url) = super::process::extract_https_url(&l) {
                            public_url = url;
                            break;
                        }
                    }
                    Ok(Ok(None)) => break,
                    Ok(Err(e)) => bail!("Error reading cloudflared output: {e}"),
                    Err(_) => {
                        tracing::trace!(
                            "cloudflared: waiting for tunnel URL (line read timed out)"
                        );
                    }
                }
            }

            if public_url.is_empty() {
                if let Err(e) = child.kill().await {
                    tracing::warn!(error = %e, "failed to kill cloudflared process");
                }
                if let Err(e) = child.wait().await {
                    tracing::warn!(error = %e, "failed to reap cloudflared process");
                }
                bail!("cloudflared did not produce a public URL within 30s. Is the token valid?");
            }

            let mut guard = self.proc.lock().await;
            *guard = Some(TunnelProcess {
                child,
                public_url: public_url.clone(),
            });

            Ok(public_url)
        })
    }

    fn stop(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move { kill_shared(&self.proc).await })
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
