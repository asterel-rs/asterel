//! Channel runtime utilities: exponential-backoff settings and supervised
//! listener spawning with automatic reconnect on transient failures.
use std::sync::Arc;
use std::time::Duration;

use crate::transport::channels::traits::{Channel, ChannelEvent};

const DEFAULT_CHANNEL_INITIAL_BACKOFF_SECS: u64 = 2;
const DEFAULT_CHANNEL_MAX_BACKOFF_SECS: u64 = 60;

/// Returns the (initial, max) exponential-backoff durations in seconds
/// for channel listener restarts.
pub(crate) fn channel_backoff_settings(
    reliability: &crate::config::ReliabilityConfig,
) -> (u64, u64) {
    let initial_backoff_secs = reliability
        .channel_initial_backoff_secs
        .max(DEFAULT_CHANNEL_INITIAL_BACKOFF_SECS);
    let max_backoff_secs = reliability
        .channel_max_backoff_secs
        .max(DEFAULT_CHANNEL_MAX_BACKOFF_SECS);

    (initial_backoff_secs, max_backoff_secs)
}

/// Spawns a channel listener with automatic restart and exponential
/// backoff on failure.
pub(crate) fn spawn_supervised_listener(
    ch: Arc<dyn Channel>,
    tx: tokio::sync::mpsc::Sender<ChannelEvent>,
    initial_backoff_secs: u64,
    max_backoff_secs: u64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let component = format!("channel:{}", ch.name());
        let mut backoff = initial_backoff_secs.max(1);
        let max_backoff = max_backoff_secs.max(backoff);

        loop {
            crate::runtime::diagnostics::health::mark_component_ok(&component);
            let result = ch.listen(tx.clone()).await;

            if tx.is_closed() {
                break;
            }

            match result {
                Ok(()) => {
                    tracing::warn!("Channel {} exited unexpectedly; restarting", ch.name());
                    crate::runtime::diagnostics::health::mark_component_error(
                        &component,
                        "listener exited unexpectedly",
                    );
                    // Clean exit — reset backoff since the listener ran successfully
                    backoff = initial_backoff_secs.max(1);
                }
                Err(e) => {
                    tracing::error!("Channel {} error: {e}; restarting", ch.name());
                    crate::runtime::diagnostics::health::mark_component_error(
                        &component,
                        e.to_string(),
                    );
                }
            }

            crate::runtime::diagnostics::health::bump_component_restart(&component);
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            // Double backoff AFTER sleeping so first error uses initial_backoff
            backoff = backoff.saturating_mul(2).min(max_backoff);
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct AlwaysFailChannel {
        name: &'static str,
        calls: Arc<AtomicUsize>,
    }

    impl Channel for AlwaysFailChannel {
        fn name(&self) -> &str {
            self.name
        }

        fn send<'a>(
            &'a self,
            _message: &'a str,
            _recipient: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>>
        {
            Box::pin(async move { Ok(()) })
        }

        fn listen<'a>(
            &'a self,
            _tx: tokio::sync::mpsc::Sender<ChannelEvent>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send + 'a>>
        {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                anyhow::bail!("listen boom")
            })
        }
    }

    #[tokio::test]
    async fn supervised_listener_marks_error_and_restarts_on_failures() {
        let calls = Arc::new(AtomicUsize::new(0));
        let channel: Arc<dyn Channel> = Arc::new(AlwaysFailChannel {
            name: "test-supervised-fail",
            calls: Arc::clone(&calls),
        });

        let (tx, rx) = tokio::sync::mpsc::channel::<ChannelEvent>(1);
        let handle = spawn_supervised_listener(channel, tx, 1, 1);

        tokio::time::sleep(Duration::from_millis(80)).await;
        drop(rx);
        handle.abort();
        let _ = handle.await;

        let snapshot = crate::runtime::diagnostics::health::snapshot_json();
        let component = &snapshot["components"]["channel:test-supervised-fail"];
        assert_eq!(component["status"], "error");
        assert!(component["restart_count"].as_u64().unwrap_or(0) >= 1);
        assert!(
            component["last_error"]
                .as_str()
                .unwrap_or("")
                .contains("listen boom")
        );
        assert!(calls.load(Ordering::SeqCst) >= 1);
    }
}
