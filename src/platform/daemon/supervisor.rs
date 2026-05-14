//! Generic component supervisor with exponential backoff.
//!
//! Spawns async components and restarts them on failure with
//! capped exponential backoff up to a maximum restart count.

use std::future::Future;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Duration;

use crate::config::Config;
use crate::runtime::services::load_runtime_operational_snapshot;

pub(super) struct SupervisedHandle {
    pub(super) name: &'static str,
    pub(super) shutdown: watch::Sender<bool>,
    pub(super) handle: JoinHandle<()>,
}

/// Spawns an async task that runs `run_component` in a loop with
/// exponential backoff and a circuit breaker.
pub(super) fn spawn_component_supervisor<F, Fut>(
    name: &'static str,
    initial_backoff_secs: u64,
    max_backoff_secs: u64,
    max_restarts: u32,
    mut run_component: F,
) -> SupervisedHandle
where
    F: FnMut(watch::Receiver<bool>) -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let handle = tokio::spawn(async move {
        let mut backoff = initial_backoff_secs.max(1);
        let max_backoff = max_backoff_secs.max(backoff);
        let mut consecutive_failures: u32 = 0;
        let shutdown = shutdown_rx;

        loop {
            if *shutdown.borrow() {
                break;
            }
            tracing::info!("Daemon component '{name}' starting");
            match run_component(shutdown.clone()).await {
                Ok(()) => {
                    crate::runtime::diagnostics::health::mark_component_error(
                        name,
                        "component exited unexpectedly",
                    );
                    tracing::warn!("Daemon component '{name}' exited unexpectedly");
                    // Unexpected Ok(()) exit should count toward the circuit
                    // breaker — otherwise a component that repeatedly crashes
                    // with Ok(()) restarts infinitely.
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    backoff = initial_backoff_secs.max(1);
                }
                Err(e) => {
                    crate::runtime::diagnostics::health::mark_component_error(name, e.to_string());
                    tracing::error!("Daemon component '{name}' failed: {e}");
                    consecutive_failures = consecutive_failures.saturating_add(1);
                }
            }

            crate::runtime::diagnostics::health::bump_component_restart(name);
            if max_restarts > 0 && consecutive_failures > max_restarts {
                tracing::error!(
                    "Daemon component '{name}' exceeded max restarts ({max_restarts}), circuit open"
                );
                break;
            }
            let mut sleep_shutdown = shutdown.clone();
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(backoff)) => {}
                changed = sleep_shutdown.changed() => {
                    if changed.is_ok() {
                        break;
                    }
                }
            }
            backoff = backoff.saturating_mul(2).min(max_backoff);
        }
    });
    SupervisedHandle {
        name,
        shutdown: shutdown_tx,
        handle,
    }
}

/// Spawns supervised component tasks for the gateway, channels,
/// heartbeat, and scheduler.
pub(super) fn spawn_supervised_components(
    config: Arc<Config>,
    host: String,
    port: u16,
    initial_backoff: u64,
    max_backoff: u64,
    supervise_channels: bool,
) -> Vec<SupervisedHandle> {
    let mut handles = Vec::new();

    let gateway_cfg = Arc::clone(&config);
    handles.push(spawn_component_supervisor(
        "gateway",
        initial_backoff,
        max_backoff,
        10,
        move |_| {
            let cfg = Arc::clone(&gateway_cfg);
            let host = host.clone();
            async move {
                crate::transport::gateway::run_gateway_with_profile(
                    &host,
                    port,
                    cfg,
                    crate::runtime::services::GatewayReadinessProfile::DaemonSupervised,
                )
                .await
            }
        },
    ));

    if supervise_channels {
        let channels_cfg = Arc::clone(&config);
        handles.push(spawn_component_supervisor(
            "channels",
            initial_backoff,
            max_backoff,
            10,
            move |_| {
                let cfg = Arc::clone(&channels_cfg);
                async move { crate::runtime::services::run_channels_surface(cfg).await }
            },
        ));
    } else {
        crate::runtime::diagnostics::health::mark_component_ok("channels");
        tracing::info!("No real-time channels configured; channel supervisor disabled");
    }

    if config.heartbeat.enabled {
        let heartbeat_cfg = Arc::clone(&config);
        handles.push(spawn_component_supervisor(
            "heartbeat",
            initial_backoff,
            max_backoff,
            10,
            move |_| {
                let cfg = Arc::clone(&heartbeat_cfg);
                async move { super::heartbeat_worker::run_heartbeat_worker(cfg).await }
            },
        ));
    }

    let scheduler_cfg = config;
    let cron_support = load_runtime_operational_snapshot(scheduler_cfg.as_ref()).cron;
    if cron_support.is_runtime_required() {
        handles.push(spawn_component_supervisor(
            "scheduler",
            initial_backoff,
            max_backoff,
            10,
            move |shutdown| {
                let cfg = Arc::clone(&scheduler_cfg);
                async move { crate::platform::cron::scheduler::run_until_shutdown(cfg, shutdown).await }
            },
        ));
    } else {
        tracing::info!(
            reason = cron_support
                .reason
                .as_deref()
                .unwrap_or("cron scheduler unsupported"),
            "Scheduler supervisor disabled"
        );
    }

    handles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn supervisor_marks_error_and_restart_on_failure() {
        let handle = spawn_component_supervisor("daemon-test-fail", 1, 1, 0, |_| async {
            anyhow::bail!("boom")
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.handle.abort();
        let _ = handle.handle.await;

        let snapshot = crate::runtime::diagnostics::health::snapshot_json();
        let component = &snapshot["components"]["daemon-test-fail"];
        assert_eq!(component["status"], "error");
        assert!(component["restart_count"].as_u64().unwrap_or(0) >= 1);
        assert!(
            component["last_error"]
                .as_str()
                .unwrap_or("")
                .contains("boom")
        );
    }

    #[tokio::test]
    async fn supervisor_marks_unexpected_exit_as_error() {
        let handle = spawn_component_supervisor("daemon-test-exit", 1, 1, 0, |_| async { Ok(()) });

        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.handle.abort();
        let _ = handle.handle.await;

        let snapshot = crate::runtime::diagnostics::health::snapshot_json();
        let component = &snapshot["components"]["daemon-test-exit"];
        assert_eq!(component["status"], "error");
        assert!(component["restart_count"].as_u64().unwrap_or(0) >= 1);
        assert!(
            component["last_error"]
                .as_str()
                .unwrap_or("")
                .contains("component exited unexpectedly")
        );
    }

    #[tokio::test]
    async fn supervisor_graceful_shutdown_completes_side_effect() {
        let completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let completed_for_task = Arc::clone(&completed);
        let handle =
            spawn_component_supervisor("daemon-test-graceful", 1, 1, 0, move |mut shutdown| {
                let completed = Arc::clone(&completed_for_task);
                async move {
                    let _ = shutdown.changed().await;
                    completed.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                }
            });

        tokio::time::sleep(Duration::from_millis(10)).await;
        let _ = handle.shutdown.send(true);
        handle.handle.await.expect("supervisor should exit cleanly");
        assert!(completed.load(std::sync::atomic::Ordering::SeqCst));
    }
}
