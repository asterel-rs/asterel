//! `Tunnel` trait: provider-agnostic abstraction for exposing local
//! ports via external tunnel binaries or custom commands.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

/// Agnostic tunnel abstraction — bring your own tunnel provider.
///
/// Implementations wrap an external tunnel binary (cloudflared, tailscale,
/// ngrok, etc.) or a custom command. The gateway calls `start()` after
/// binding its local port and `stop()` on shutdown.
pub trait Tunnel: Send + Sync {
    /// Human-readable provider name (e.g. "cloudflare", "tailscale")
    fn name(&self) -> &str;

    /// Start the tunnel, exposing `local_host:local_port` externally.
    /// Returns the public URL on success.
    fn start<'a>(
        &'a self,
        local_host: &'a str,
        local_port: u16,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

    /// Stop the tunnel process gracefully.
    fn stop(&self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;

    /// Check if the tunnel is still alive.
    fn health_check(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;

    /// Return the public URL if the tunnel is running.
    fn public_url(&self) -> Option<String>;
}
