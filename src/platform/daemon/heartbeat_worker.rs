//! Periodic heartbeat worker for the daemon runtime.
//!
//! Runs memory hygiene, autonomy-state transitions, persona
//! drift detection, and SLO evaluation on each heartbeat tick.

use std::sync::Arc;

use anyhow::Result;
use tokio::time::Duration;

use crate::platform::cron::heartbeat::HeartbeatCheckResult;

mod autonomy_state;
mod memory_metrics;

use autonomy_state::record_autonomy_mode_transition;
use memory_metrics::run_memory_hygiene_tick;

fn heartbeat_temperature(config: &crate::config::Config) -> f64 {
    config
        .autonomy
        .clamp_temperature(config.default_temperature)
}

/// Runs the periodic heartbeat loop: memory hygiene, autonomy
/// transitions, and task collection/execution.
///
/// # Errors
///
/// Returns an error only on unrecoverable failures; transient
/// errors are logged and the loop continues.
pub(super) async fn run_heartbeat_worker(config: Arc<crate::config::Config>) -> Result<()> {
    let observer: Arc<dyn crate::runtime::observability::Observer> = Arc::from(
        crate::runtime::observability::create_observer(&config.observability),
    );
    let engine = crate::runtime::diagnostics::heartbeat::engine::HeartbeatEngine::new(
        config.heartbeat.clone(),
        config.workspace_dir.clone(),
        Arc::clone(&observer),
    );

    let interval_mins = config.heartbeat.interval_minutes.max(5);
    let mut interval = tokio::time::interval(Duration::from_secs(u64::from(interval_mins) * 60));

    loop {
        interval.tick().await;
        run_memory_hygiene_tick(&config, &observer).await;
        record_autonomy_mode_transition(&config, &observer);

        let tasks = match engine.collect_tasks().await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Heartbeat collect_tasks failed: {e}, skipping cycle");
                crate::runtime::diagnostics::health::mark_component_error(
                    "heartbeat",
                    e.to_string(),
                );
                continue;
            }
        };
        let decision = if tasks.is_empty() {
            HeartbeatCheckResult::skip()
        } else {
            HeartbeatCheckResult::run(&tasks.join(", "))
        };
        if !decision.should_run() {
            continue;
        }

        for task in tasks {
            let prompt = format!("[Heartbeat Task] {task}");
            let temp = heartbeat_temperature(&config);
            if let Err(e) = crate::runtime::services::run_agent_surface(
                Arc::clone(&config),
                crate::core::agent::RunRequest {
                    message: Some(prompt),
                    provider_override: None,
                    model_override: None,
                    temperature: temp,
                    system_prompt: crate::transport::channels::gateway_base_prompt(Some(
                        config.workspace_dir.as_path(),
                    )),
                    stream_sink: None,
                    interactive_input_tx: None,
                    approval_broker: None,
                    execution_audit_sink: None,
                    cli_input_rx: None,
                },
            )
            .await
            {
                crate::runtime::diagnostics::health::mark_component_error(
                    "heartbeat",
                    e.to_string(),
                );
                tracing::warn!("Heartbeat task failed: {e}");
            } else {
                crate::runtime::diagnostics::health::mark_component_ok("heartbeat");
            }
        }
    }
}
