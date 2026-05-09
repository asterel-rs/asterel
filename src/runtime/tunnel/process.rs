//! Shared tunnel process management: wraps spawned child processes
//! behind `Arc<Mutex>` for safe concurrent start/stop across adapters.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;

/// Wraps a spawned tunnel child process so implementations can share it.
pub(crate) struct TunnelProcess {
    /// Handle to the spawned child process.
    pub child: tokio::process::Child,
    /// The public URL assigned to this tunnel.
    pub public_url: String,
}

/// Thread-safe shared reference to an optional tunnel process.
pub(crate) type SharedProcess = Arc<Mutex<Option<TunnelProcess>>>;

/// Create a new empty shared process handle.
pub(crate) fn new_shared_process() -> SharedProcess {
    Arc::new(Mutex::new(None))
}

/// Check whether the shared process is still alive.
pub(crate) async fn is_process_alive(proc: &SharedProcess) -> bool {
    let guard = proc.lock().await;
    guard.as_ref().is_some_and(|tp| tp.child.id().is_some())
}

/// Try to read the public URL from a shared process (non-blocking).
pub(crate) fn try_read_public_url(proc: &SharedProcess) -> Option<String> {
    proc.try_lock()
        .ok()
        .and_then(|g| g.as_ref().map(|tp| tp.public_url.clone()))
}

/// Extract the first `https://` URL from a tunnel process log line.
#[must_use]
pub(crate) fn extract_https_url(line: &str) -> Option<String> {
    let idx = line.find("https://")?;
    let url_part = &line[idx..];
    let end = url_part
        .find(|c: char| c.is_whitespace())
        .unwrap_or(url_part.len());
    Some(url_part[..end].to_string())
}

/// Kill a shared tunnel process if running.
pub(crate) async fn kill_shared(proc: &SharedProcess) -> Result<()> {
    let mut running = {
        let mut guard = proc.lock().await;
        guard.take()
    };
    if let Some(ref mut tp) = running {
        if let Err(e) = tp.child.kill().await {
            tracing::warn!(error = %e, "failed to kill tunnel process");
        }
        if let Err(e) = tp.child.wait().await {
            tracing::warn!(error = %e, "failed to reap tunnel process");
        }
    }
    Ok(())
}
