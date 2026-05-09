//! Custom tunnel adapter: bring your own binary with `{port}` and
//! `{host}` placeholders, optional URL regex, and health polling.

use anyhow::{Result, bail};
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

use super::{SharedProcess, Tunnel, TunnelProcess, kill_shared, new_shared_process};

/// Custom Tunnel — bring your own tunnel binary.
///
/// Provide a `start_command` with `{port}` and `{host}` placeholders.
/// Optionally provide a `url_pattern` regex to extract the public URL
/// from stdout, and a `health_url` to poll for liveness.
///
/// Examples:
/// - `bore local {port} --to bore.pub`
/// - `frp -c /etc/frp/frpc.ini`
/// - `ssh -R 80:localhost:{port} serveo.net`
pub struct CustomTunnel {
    start_command: String,
    health_url: Option<String>,
    url_pattern: Option<String>,
    proc: SharedProcess,
    http_client: reqwest::Client,
}

impl CustomTunnel {
    /// Create a new custom tunnel with the given start command, optional
    /// health check URL, and optional URL extraction pattern.
    #[must_use]
    pub fn new(
        start_command: String,
        health_url: Option<String>,
        url_pattern: Option<String>,
    ) -> Self {
        Self {
            start_command,
            health_url,
            url_pattern,
            proc: new_shared_process(),
            http_client: crate::utils::http::build_http_client(),
        }
    }
}

impl Tunnel for CustomTunnel {
    fn name(&self) -> &'static str {
        "custom"
    }

    fn start<'a>(
        &'a self,
        local_host: &'a str,
        local_port: u16,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + 'a>> {
        Box::pin(async move {
            let cmd = self
                .start_command
                .replace("{port}", &local_port.to_string())
                .replace("{host}", local_host);

            let parts = shlex::split(&cmd).ok_or_else(|| {
                anyhow::anyhow!("Custom tunnel start_command has unmatched quotes")
            })?;
            if parts.is_empty() {
                bail!("Custom tunnel start_command is empty");
            }

            let mut child = Command::new(&parts[0])
                .args(&parts[1..])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()?;

            let mut public_url = None;

            // If a URL pattern is provided, try to extract the public URL from stdout
            let pattern = self.url_pattern.as_ref().ok_or_else(|| {
                anyhow::anyhow!("custom tunnel requires url_pattern to extract a public HTTPS URL")
            })?;
            if let Some(stdout) = child.stdout.take() {
                let mut reader = tokio::io::BufReader::new(stdout).lines();
                let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(15);

                while tokio::time::Instant::now() < deadline {
                    let line = tokio::time::timeout(
                        tokio::time::Duration::from_secs(3),
                        reader.next_line(),
                    )
                    .await;

                    match line {
                        Ok(Ok(Some(l))) => {
                            tracing::debug!("custom-tunnel: {l}");
                            // Simple substring match on the pattern
                            if (l.contains(pattern) || l.contains("https://"))
                                && let Some(extracted) = super::process::extract_https_url(&l)
                            {
                                public_url = Some(extracted);
                                break;
                            }
                        }
                        Ok(Ok(None) | Err(_)) => break,
                        Err(_) => {
                            tracing::trace!(
                                "custom tunnel: waiting for tunnel URL (line read timed out)"
                            );
                        }
                    }
                }
            }

            let Some(public_url) = public_url else {
                let _ = child.kill().await;
                let _ = child.wait().await;
                bail!("custom tunnel did not emit a public HTTPS URL matching url_pattern");
            };

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
        Box::pin(async move {
            // If a health URL is configured, try to reach it
            if let Some(ref url) = self.health_url {
                return self
                    .http_client
                    .get(url)
                    .timeout(std::time::Duration::from_secs(5))
                    .send()
                    .await
                    .is_ok_and(|response| response.status().is_success());
            }

            // Otherwise check if the process is still alive
            let guard = self.proc.lock().await;
            guard.as_ref().is_some_and(|tp| tp.child.id().is_some())
        })
    }

    fn public_url(&self) -> Option<String> {
        self.proc
            .try_lock()
            .ok()
            .and_then(|g| g.as_ref().map(|tp| tp.public_url.clone()))
    }
}
