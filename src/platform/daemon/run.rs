//! Main daemon entry point and lifecycle management.
//!
//! Initializes memory, persona state, supervised components
//! (channels, cron, gateway), config reload polling, and the
//! heartbeat worker, then awaits graceful shutdown.

use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::task::JoinHandle;
use tokio::time::Duration;

use super::reload;
use super::state::spawn_state_writer;
use super::supervisor::{SupervisedHandle, spawn_supervised_components};
use crate::config::Config;
use crate::core::persona::state_persistence::BackendHeaderPersist;
use crate::plugins::skills::skills_watch_fingerprint_with_config;
use crate::runtime::services::bootstrap_runtime_memory;

const CONFIG_RELOAD_POLL_SECONDS: u64 = 3;
const DEFAULT_COMPONENT_SHUTDOWN_GRACE_SECONDS: u64 = 2;
const SCHEDULER_SHUTDOWN_GRACE_SECONDS: u64 = 40;

struct DaemonReloadState {
    active_config: Arc<Config>,
    last_config_modified_at: Option<std::time::SystemTime>,
    state_handles: Vec<JoinHandle<()>>,
    component_handles: Vec<SupervisedHandle>,
    initial_backoff: u64,
    max_backoff: u64,
    last_skills_fingerprint: Option<u64>,
}

/// Computes the `(initial, max)` backoff pair from the config.
pub(super) fn supervisor_backoff(config: &Config) -> (u64, u64) {
    let initial_backoff = config.reliability.channel_initial_backoff_secs.max(1);
    let max_backoff = config
        .reliability
        .channel_max_backoff_secs
        .max(initial_backoff);
    (initial_backoff, max_backoff)
}

async fn stop_handles(handles: &mut Vec<JoinHandle<()>>) {
    for handle in &*handles {
        handle.abort();
    }
    for handle in handles.drain(..) {
        if let Err(error) = handle.await {
            tracing::warn!(%error, "daemon task panicked during shutdown");
        }
    }
}

async fn stop_supervised_handles(handles: &mut Vec<SupervisedHandle>) {
    for handle in &*handles {
        let _ = handle.shutdown.send(true);
    }
    for supervised in handles.drain(..) {
        let mut join = supervised.handle;
        let grace_seconds = if supervised.name == "scheduler" {
            SCHEDULER_SHUTDOWN_GRACE_SECONDS
        } else {
            DEFAULT_COMPONENT_SHUTDOWN_GRACE_SECONDS
        };
        match tokio::time::timeout(Duration::from_secs(grace_seconds), &mut join).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(component = supervised.name, %error, "daemon task panicked during shutdown");
            }
            Err(_) => {
                tracing::warn!(
                    component = supervised.name,
                    "daemon task did not stop gracefully; aborting"
                );
                join.abort();
                let _ = join.await;
            }
        }
    }
}

/// Runs the daemon lifecycle: initializes components, starts
/// supervised workers, and enters the reload / shutdown loop.
///
/// # Errors
///
/// Returns an error on unrecoverable startup or shutdown
/// failures.
pub async fn run(config: Arc<Config>, host: String, port: u16) -> Result<()> {
    let (initial_backoff, max_backoff) = supervisor_backoff(config.as_ref());
    crate::utils::http::sync_runtime_http_proxy(config.network.proxy.as_deref())
        .context("apply daemon network proxy")?;

    crate::runtime::diagnostics::health::mark_component_ok("daemon");
    if let Err(error) = crate::onboard::postgres::try_revive_managed_local_postgres(config.as_ref())
    {
        tracing::warn!(%error, "failed to auto-revive managed local postgres");
    }
    initialize_daemon_prerequisites(&config).await;

    let mut state = DaemonReloadState {
        state_handles: vec![spawn_state_writer(Arc::clone(&config))],
        component_handles: spawn_supervised_components(
            Arc::clone(&config),
            host.clone(),
            port,
            initial_backoff,
            max_backoff,
            has_supervised_channels(config.as_ref()),
        ),
        last_config_modified_at: reload::config_modified_at(&config.config_path),
        last_skills_fingerprint: skills_fingerprint_if_enabled(&config),
        active_config: config,
        initial_backoff,
        max_backoff,
    };

    println!("◆ {}", t!("daemon.started"));
    println!("   {}", t!("daemon.gateway_addr", host = host, port = port));
    println!("   {}", t!("daemon.components"));
    println!("   {}", t!("daemon.stop_hint"));

    let mut reload_interval =
        tokio::time::interval(Duration::from_secs(CONFIG_RELOAD_POLL_SECONDS));

    loop {
        tokio::select! {
            ctrl_c = tokio::signal::ctrl_c() => {
                ctrl_c?;
                crate::runtime::diagnostics::health::mark_component_error("daemon", "shutdown requested");
                break;
            }
            _ = reload_interval.tick() => {
                if !state.active_config.runtime.enable_live_settings_reload {
                    continue;
                }

                let reloaded = try_config_reload(&mut state, &host, port).await;

                if reloaded {
                    continue;
                }

                try_skills_refresh(
                    &state.active_config,
                    &mut state.last_skills_fingerprint,
                    &mut state.component_handles,
                    &host,
                    port,
                    state.initial_backoff,
                    state.max_backoff,
                ).await;
            }
        }
    }

    stop_handles(&mut state.state_handles).await;
    stop_supervised_handles(&mut state.component_handles).await;

    Ok(())
}

async fn initialize_daemon_prerequisites(config: &Config) {
    if config.heartbeat.enabled
        && let Err(error) =
            crate::runtime::diagnostics::heartbeat::engine::HeartbeatEngine::ensure_heartbeat_file(
                &config.workspace_dir,
            )
            .await
    {
        tracing::warn!(%error, "failed to ensure heartbeat file");
    }

    if let Err(error) = initialize_persona_startup_state(config).await {
        tracing::warn!(%error, "failed to initialize persona startup state");
    }
}

fn skills_fingerprint_if_enabled(config: &Config) -> Option<u64> {
    if config.skills.watch_refresh {
        Some(skills_watch_fingerprint_with_config(
            &config.workspace_dir,
            &config.skills,
        ))
    } else {
        None
    }
}

async fn try_config_reload(state: &mut DaemonReloadState, host: &str, port: u16) -> bool {
    let Some(observed_modified_at) = reload::config_modified_at(&state.active_config.config_path)
    else {
        return false;
    };
    if state
        .last_config_modified_at
        .is_some_and(|previous| observed_modified_at <= previous)
    {
        return false;
    }
    state.last_config_modified_at = Some(observed_modified_at);

    let candidate = match reload::load_candidate_config(state.active_config.as_ref()) {
        Ok(candidate) => candidate,
        Err(error) => {
            crate::runtime::diagnostics::health::mark_component_error(
                "config_reload",
                error.to_string(),
            );
            tracing::warn!(%error, "rejected config reload candidate");
            return false;
        }
    };

    match reload::evaluate_reload(state.active_config.as_ref(), &candidate) {
        Ok(reload::ReloadDecision::NoChanges) => false,
        Ok(reload::ReloadDecision::Apply { changed_sections }) => {
            tracing::info!(sections = ?changed_sections, "applying live config reload");
            if let Err(error) =
                crate::utils::http::sync_runtime_http_proxy(candidate.network.proxy.as_deref())
            {
                crate::runtime::diagnostics::health::mark_component_error(
                    "config_reload",
                    error.to_string(),
                );
                tracing::warn!(%error, "failed to apply live network proxy during reload");
                return false;
            }
            stop_supervised_handles(&mut state.component_handles).await;
            stop_handles(&mut state.state_handles).await;
            state.active_config = Arc::new(candidate);
            state.state_handles = vec![spawn_state_writer(Arc::clone(&state.active_config))];
            (state.initial_backoff, state.max_backoff) =
                supervisor_backoff(state.active_config.as_ref());
            state.component_handles = spawn_supervised_components(
                Arc::clone(&state.active_config),
                host.to_string(),
                port,
                state.initial_backoff,
                state.max_backoff,
                has_supervised_channels(state.active_config.as_ref()),
            );
            state.last_skills_fingerprint = skills_fingerprint_if_enabled(&state.active_config);
            crate::runtime::diagnostics::health::mark_component_ok("config_reload");
            true
        }
        Err(error) => {
            crate::runtime::diagnostics::health::mark_component_error(
                "config_reload",
                error.to_string(),
            );
            tracing::warn!(%error, "failed to evaluate config reload candidate");
            false
        }
    }
}

async fn try_skills_refresh(
    active_config: &Config,
    last_skills_fingerprint: &mut Option<u64>,
    component_handles: &mut Vec<SupervisedHandle>,
    host: &str,
    port: u16,
    initial_backoff: u64,
    max_backoff: u64,
) {
    if active_config.skills.watch_refresh {
        let observed_fingerprint = skills_watch_fingerprint_with_config(
            &active_config.workspace_dir,
            &active_config.skills,
        );
        if should_apply_skills_refresh(*last_skills_fingerprint, observed_fingerprint) {
            tracing::info!("applying live skills refresh");
            stop_supervised_handles(component_handles).await;
            *component_handles = spawn_supervised_components(
                Arc::new(active_config.clone()),
                host.to_string(),
                port,
                initial_backoff,
                max_backoff,
                has_supervised_channels(active_config),
            );
            crate::runtime::diagnostics::health::mark_component_ok("skills_reload");
        }
        *last_skills_fingerprint = Some(observed_fingerprint);
    } else {
        *last_skills_fingerprint = None;
    }
}

/// Reconciles the persona state mirror from the memory backend
/// on daemon startup.
///
/// # Errors
///
/// Returns an error if memory creation or reconciliation fails.
pub(super) async fn initialize_persona_startup_state(config: &Config) -> Result<()> {
    if !config.persona.enabled_main_session {
        return Ok(());
    }

    let memory = bootstrap_runtime_memory(config).await?;
    let person_id = config
        .identity
        .person_id
        .clone()
        .unwrap_or_else(|| "local-default".to_string());
    let persistence = BackendHeaderPersist::new(
        memory,
        config.workspace_dir.clone(),
        config.persona.clone(),
        person_id,
    );
    let _ = persistence
        .reconcile_mirror_from_backend_on_startup()
        .await?;
    Ok(())
}

/// Returns `true` if at least one real-time channel is
/// configured.
pub(super) fn has_supervised_channels(config: &Config) -> bool {
    crate::transport::channels::factory::has_listener_channels(config)
}

/// Returns `true` if the skills fingerprint has changed since
/// the last check.
pub(super) fn should_apply_skills_refresh(
    previous_fingerprint: Option<u64>,
    observed_fingerprint: u64,
) -> bool {
    previous_fingerprint.is_some_and(|previous| previous != observed_fingerprint)
}
