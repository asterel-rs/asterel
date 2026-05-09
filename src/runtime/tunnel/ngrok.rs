//! ngrok tunnel adapter: wraps the `ngrok` binary to expose local
//! ports with optional custom domain support.

use anyhow::{Result, bail};
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use zeroize::Zeroizing;

use super::{SharedProcess, Tunnel, TunnelProcess, kill_shared, new_shared_process};

/// ngrok Tunnel — wraps the `ngrok` binary.
///
/// Requires `ngrok` installed. Optionally set a custom domain
/// (requires ngrok paid plan).
pub struct NgrokTunnel {
    /// Zeroized on drop to prevent token lingering in process memory.
    auth_token: Zeroizing<String>,
    domain: Option<String>,
    proc: SharedProcess,
}

impl NgrokTunnel {
    /// Create a new ngrok tunnel with the given auth token and optional
    /// custom domain.
    #[must_use]
    pub fn new(auth_token: String, domain: Option<String>) -> Self {
        Self {
            auth_token: Zeroizing::new(auth_token),
            domain,
            proc: new_shared_process(),
        }
    }
}

impl Tunnel for NgrokTunnel {
    fn name(&self) -> &'static str {
        "ngrok"
    }

    fn start<'a>(
        &'a self,
        _local_host: &'a str,
        local_port: u16,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            // Build command: ngrok http <port> [--domain <domain>]
            // Auth token is passed via NGROK_AUTHTOKEN env var to avoid
            // exposing it in /proc/<pid>/cmdline.
            let mut args = vec!["http".to_string(), local_port.to_string()];
            if let Some(ref domain) = self.domain {
                args.push("--domain".into());
                args.push(domain.clone());
            }
            // Output log to stdout for URL extraction
            args.push("--log".into());
            args.push("stdout".into());
            args.push("--log-format".into());
            args.push("logfmt".into());

            let mut child = Command::new("ngrok")
                .args(&args)
                .env("NGROK_AUTHTOKEN", self.auth_token.as_str())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()?;

            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| anyhow::anyhow!("Failed to capture ngrok stdout"))?;

            let mut reader = tokio::io::BufReader::new(stdout).lines();
            let mut public_url = String::new();

            // Wait up to 15s for the tunnel URL
            let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(15);
            while tokio::time::Instant::now() < deadline {
                let line =
                    tokio::time::timeout(tokio::time::Duration::from_secs(3), reader.next_line())
                        .await;

                match line {
                    Ok(Ok(Some(l))) => {
                        tracing::debug!("ngrok: {l}");
                        // ngrok logfmt: url=https://xxxx.ngrok-free.app
                        if l.contains("url=https://")
                            && let Some(url) = super::process::extract_https_url(&l)
                        {
                            public_url = url;
                            break;
                        }
                    }
                    Ok(Ok(None)) => break,
                    Ok(Err(e)) => bail!("Error reading ngrok output: {e}"),
                    Err(_) => {
                        tracing::trace!("ngrok: waiting for tunnel URL (line read timed out)");
                    }
                }
            }

            if public_url.is_empty() {
                if let Err(e) = child.kill().await {
                    tracing::warn!(error = %e, "failed to kill ngrok process");
                }
                if let Err(e) = child.wait().await {
                    tracing::warn!(error = %e, "failed to reap ngrok process");
                }
                bail!("ngrok did not produce a public URL within 15s. Is the auth token valid?");
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
