//! Channel listener orchestrator: spawns supervised listeners for all
//! configured channels, coalesces messages, routes events through the
//! routing queue, and handles event-trigger debouncing.
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use super::super::coalescer::MessageCoalescer;
use super::super::message_handler::{handle_channel_event, handle_channel_message};
use super::super::runtime::{channel_backoff_settings, spawn_supervised_listener};
use super::super::traits::ChannelEvent;
use super::routing_queue::{RoutingHandler, RoutingQueueManager};
use super::runtime::{ChannelRuntime, init_channel_runtime};
use crate::config::Config;

static EVENT_DEBOUNCE: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static CHANNEL_SURFACE_RELOAD: LazyLock<broadcast::Sender<()>> = LazyLock::new(|| {
    let (tx, _rx) = broadcast::channel(32);
    tx
});

pub(crate) fn request_channel_surface_reload() -> bool {
    match CHANNEL_SURFACE_RELOAD.send(()) {
        Ok(receiver_count) => receiver_count > 0,
        Err(_error) => false,
    }
}

fn subscribe_channel_surface_reload() -> broadcast::Receiver<()> {
    CHANNEL_SURFACE_RELOAD.subscribe()
}

// Called from transport::gateway::tests — clippy dead_code lint fires under --all-targets
// because the call site is in a separate cfg(test) module that clippy does not cross-scan.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn subscribe_channel_surface_reload_for_tests() -> broadcast::Receiver<()> {
    subscribe_channel_surface_reload()
}

fn event_trigger_key(event: &ChannelEvent) -> Option<String> {
    let sender = event.sender()?;
    let event_type = match event {
        ChannelEvent::ReactionAdd { .. } => "reaction_add",
        ChannelEvent::ReactionRemove { .. } => "reaction_remove",
        ChannelEvent::MessageEdit { .. } => "message_edit",
        _ => return None,
    };
    Some(format!("{sender}:{event_type}"))
}

fn is_event_trigger_enabled(
    config: &crate::config::EventTriggerConfig,
    event: &ChannelEvent,
) -> bool {
    match event {
        ChannelEvent::ReactionAdd { .. } => config.reaction_add,
        ChannelEvent::ReactionRemove { .. } => config.reaction_remove,
        ChannelEvent::MessageEdit { .. } => config.message_edit,
        _ => false,
    }
}

fn is_event_throttled(event: &ChannelEvent, cooldown_secs: u64) -> bool {
    let Some(key) = event_trigger_key(event) else {
        return false;
    };

    let now = Instant::now();
    let cooldown = Duration::from_secs(cooldown_secs);
    let mut guard = EVENT_DEBOUNCE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if let Some(last_seen) = guard.get(&key)
        && now.duration_since(*last_seen) < cooldown
    {
        tracing::debug!("event throttled");
        return true;
    }

    guard.insert(key, now);
    false
}

#[cfg(test)]
fn clear_event_throttle_state() {
    let mut guard = EVENT_DEBOUNCE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.clear();
}

async fn maybe_dispatch_event_trigger(rt: &Arc<ChannelRuntime>, event: &ChannelEvent) {
    let trigger_config = &rt.config.channels_config.event_triggers;
    if is_event_trigger_enabled(trigger_config, event)
        && !is_event_throttled(event, trigger_config.cooldown_secs)
    {
        handle_channel_event(rt, event).await;
    }
}

async fn run_channel_health_monitor(rt: Arc<ChannelRuntime>, interval_minutes: u32) {
    let mut ticker = tokio::time::interval(Duration::from_secs(u64::from(interval_minutes) * 60));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        ticker.tick().await;
        for channel in &rt.channels {
            let result =
                tokio::time::timeout(Duration::from_secs(10), channel.health_check()).await;
            match result {
                Ok(true) => {
                    tracing::debug!(channel = channel.name(), "channel health check passed");
                }
                Ok(false) => {
                    tracing::warn!(channel = channel.name(), "channel health check failed");
                }
                Err(_) => {
                    tracing::warn!(channel = channel.name(), "channel health check timed out");
                }
            }
        }
    }
}

fn load_live_channel_config(current: &Arc<Config>) -> Result<Arc<Config>> {
    if !current.config_path.exists() {
        return Ok(Arc::clone(current));
    }

    Ok(Arc::new(Config::load_from_path(
        &current.config_path,
        &current.workspace_dir,
    )?))
}

fn drain_pending_channel_surface_reloads(reload_rx: &mut broadcast::Receiver<()>) {
    while matches!(
        reload_rx.try_recv(),
        Ok(()) | Err(broadcast::error::TryRecvError::Lagged(_))
    ) {}
}

async fn stop_channel_runtime_tasks(
    listener_tasks: &mut Vec<JoinHandle<()>>,
    health_monitor_handle: &mut Option<JoinHandle<()>>,
) {
    for handle in listener_tasks.iter() {
        handle.abort();
    }
    for handle in listener_tasks.drain(..) {
        let _ = handle.await;
    }

    if let Some(handle) = health_monitor_handle.take() {
        handle.abort();
        let _ = handle.await;
    }
}

fn print_channel_runtime_banner(rt: &ChannelRuntime) {
    println!("◆ {}", t!("channels.server_title"));
    println!("  › {} {}", t!("channels.model"), rt.model);
    println!(
        "  › {} {} (auto-save: {})",
        t!("channels.memory"),
        rt.config.memory.backend,
        if rt.config.memory.auto_save {
            "on"
        } else {
            "off"
        }
    );
    let mut channel_names = String::new();
    for c in &rt.channels {
        if !channel_names.is_empty() {
            channel_names.push_str(", ");
        }
        channel_names.push_str(c.name());
    }
    println!("  › {} {}", t!("channels.channels"), channel_names);
    println!();
    println!("  {}", t!("channels.listening"));
    println!();
}

async fn wait_for_idle_channel_surface_reload(
    rt: &Arc<ChannelRuntime>,
    reload_rx: &mut broadcast::Receiver<()>,
    hold_when_idle: bool,
) -> Result<Option<Arc<Config>>> {
    crate::runtime::diagnostics::health::mark_component_ok("channels");
    tracing::info!("channel surface idle: no active listener-backed channels configured");

    if !hold_when_idle || !rt.config.runtime.enable_live_settings_reload {
        println!("{}", t!("channels.no_channels"));
        return Ok(None);
    }

    match reload_rx.recv().await {
        Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
            match load_live_channel_config(&rt.config) {
                Ok(next_config) => {
                    drain_pending_channel_surface_reloads(reload_rx);
                    Ok(Some(next_config))
                }
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "failed to load persisted channel config after reload request"
                    );
                    Ok(Some(Arc::clone(&rt.config)))
                }
            }
        }
        Err(broadcast::error::RecvError::Closed) => Ok(None),
    }
}

async fn run_active_channel_surface(
    rt: &Arc<ChannelRuntime>,
    reload_rx: &mut broadcast::Receiver<()>,
) -> Result<Option<Arc<Config>>> {
    print_channel_runtime_banner(rt);
    crate::runtime::diagnostics::health::mark_component_ok("channels");

    let (initial_backoff_secs, max_backoff_secs) = channel_backoff_settings(&rt.config.reliability);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ChannelEvent>(100);

    let mut listener_tasks = Vec::with_capacity(rt.channels.len());
    for ch in &rt.channels {
        listener_tasks.push(spawn_supervised_listener(
            ch.clone(),
            tx.clone(),
            initial_backoff_secs,
            max_backoff_secs,
        ));
    }
    drop(tx);

    let health_check_minutes = rt.config.gateway.channel_health_check_minutes;
    let mut health_monitor_handle = if health_check_minutes > 0 {
        tracing::info!(
            interval_minutes = health_check_minutes,
            "started periodic channel health monitor"
        );
        Some(tokio::spawn(run_channel_health_monitor(
            Arc::clone(rt),
            health_check_minutes,
        )))
    } else {
        None
    };

    let handler_rt = Arc::clone(rt);
    let dispatch_handler: RoutingHandler = Arc::new(move |event| {
        let handler_rt = Arc::clone(&handler_rt);
        Box::pin(async move { dispatch_channel_event(&handler_rt, event).await })
    });
    let mut queue_manager =
        RoutingQueueManager::from_config(&rt.config.channels_config, dispatch_handler);
    let mut coalescer = MessageCoalescer::from_config(&rt.config.channels_config);

    loop {
        tokio::select! {
            maybe_event = coalescer.next_event(&mut rx) => {
                let Some(event) = maybe_event else {
                    break;
                };
                queue_manager.enqueue(event).await;
            }
            reload = reload_rx.recv(), if rt.config.runtime.enable_live_settings_reload => {
                match reload {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {
                        match load_live_channel_config(&rt.config) {
                            Ok(next_config) => {
                                tracing::info!("reloading channel surface from persisted config");
                                stop_channel_runtime_tasks(
                                    &mut listener_tasks,
                                    &mut health_monitor_handle,
                                )
                                .await;
                                drain_pending_channel_surface_reloads(reload_rx);
                                return Ok(Some(next_config));
                            }
                            Err(error) => {
                                tracing::warn!(
                                    %error,
                                    "failed to load persisted channel config for live reload"
                                );
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }

    stop_channel_runtime_tasks(&mut listener_tasks, &mut health_monitor_handle).await;
    Ok(None)
}

/// # Errors
///
/// Returns an error when channel runtime initialization or listener startup
/// fails.
async fn start_channels_with_mode(config: Arc<Config>, hold_when_idle: bool) -> Result<()> {
    let mut active_config = load_live_channel_config(&config)?;
    let mut reload_rx = subscribe_channel_surface_reload();

    loop {
        let rt = Arc::new(init_channel_runtime(&active_config).await?);

        if rt.channels.is_empty() {
            match wait_for_idle_channel_surface_reload(&rt, &mut reload_rx, hold_when_idle).await? {
                Some(next_config) => {
                    active_config = next_config;
                    continue;
                }
                None => return Ok(()),
            };
        }

        match run_active_channel_surface(&rt, &mut reload_rx).await? {
            Some(next_config) => {
                active_config = next_config;
            }
            None => {
                return Ok(());
            }
        }
    }
}

/// # Errors
///
/// Returns an error when channel runtime initialization or listener startup
/// fails.
pub async fn start_channels(config: Arc<Config>) -> Result<()> {
    start_channels_with_mode(config, false).await
}

/// # Errors
///
/// Returns an error when channel surface startup fails.
pub(crate) async fn run_channels_surface(config: Arc<Config>) -> Result<()> {
    start_channels_with_mode(config, true).await
}

async fn dispatch_channel_event(rt: &Arc<ChannelRuntime>, event: ChannelEvent) {
    match &event {
        ChannelEvent::Message(msg) => {
            handle_channel_message(rt, msg).await;
        }
        ChannelEvent::ReactionAdd {
            channel_name,
            channel_id,
            message_id,
            user_id,
            emoji,
        } => {
            maybe_dispatch_event_trigger(rt, &event).await;
            tracing::info!(
                channel = %channel_name,
                channel_id = %channel_id,
                message_id = %message_id,
                user_id = %user_id,
                emoji = %emoji,
                "channel.reaction.add"
            );
        }
        ChannelEvent::ReactionRemove {
            channel_name,
            channel_id,
            message_id,
            user_id,
            emoji,
        } => {
            maybe_dispatch_event_trigger(rt, &event).await;
            tracing::info!(
                channel = %channel_name,
                channel_id = %channel_id,
                message_id = %message_id,
                user_id = %user_id,
                emoji = %emoji,
                "channel.reaction.remove"
            );
        }
        ChannelEvent::MessageEdit {
            channel_name,
            channel_id,
            message_id,
            user_id,
            ..
        } => {
            maybe_dispatch_event_trigger(rt, &event).await;
            tracing::debug!(
                channel = %channel_name,
                channel_id = %channel_id,
                message_id = %message_id,
                user_id = %user_id,
                "channel.message.edit"
            );
        }
        ChannelEvent::MessageDelete {
            channel_name,
            channel_id,
            message_id,
        } => {
            tracing::debug!(
                channel = %channel_name,
                channel_id = %channel_id,
                message_id = %message_id,
                "channel.message.delete"
            );
        }
        ChannelEvent::TypingStart {
            channel_name,
            channel_id,
            user_id,
        } => {
            tracing::debug!(
                channel = %channel_name,
                channel_id = %channel_id,
                user_id = %user_id,
                "channel.typing.start"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::{clear_event_throttle_state, is_event_throttled, is_event_trigger_enabled};
    use crate::config::EventTriggerConfig;
    use crate::contracts::ids::{ChannelId, MessageId, UserId};
    use crate::transport::channels::traits::ChannelEvent;

    static EVENT_THROTTLE_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn reaction_add_event(user_id: &str) -> ChannelEvent {
        ChannelEvent::ReactionAdd {
            channel_name: "discord".to_string(),
            channel_id: ChannelId::new("chan-1"),
            message_id: MessageId::new("msg-1"),
            user_id: UserId::new(user_id),
            emoji: ":thumbs_up:".to_string(),
        }
    }

    #[test]
    fn event_throttle_blocks_repeated_event_for_same_user_and_type() {
        let _guard = EVENT_THROTTLE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        clear_event_throttle_state();
        let event = reaction_add_event("user-1");

        assert!(!is_event_throttled(&event, 5));
        assert!(is_event_throttled(&event, 5));
    }

    #[test]
    fn event_throttle_uses_event_type_in_key() {
        let _guard = EVENT_THROTTLE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        clear_event_throttle_state();
        let reaction_add = reaction_add_event("user-type-key");
        let reaction_remove = ChannelEvent::ReactionRemove {
            channel_name: "discord".to_string(),
            channel_id: ChannelId::new("chan-1"),
            message_id: MessageId::new("msg-1"),
            user_id: UserId::new("user-type-key"),
            emoji: ":thumbs_up:".to_string(),
        };

        assert!(!is_event_throttled(&reaction_add, 5));
        assert!(!is_event_throttled(&reaction_remove, 5));
    }

    #[test]
    fn dispatch_enablement_follows_event_trigger_config() {
        let config = EventTriggerConfig {
            reaction_add: true,
            reaction_remove: false,
            message_edit: true,
            cooldown_secs: 5,
        };

        assert!(is_event_trigger_enabled(&config, &reaction_add_event("u")));
        assert!(!is_event_trigger_enabled(
            &config,
            &ChannelEvent::ReactionRemove {
                channel_name: "discord".to_string(),
                channel_id: ChannelId::new("chan-1"),
                message_id: MessageId::new("msg-1"),
                user_id: UserId::new("u"),
                emoji: ":thumbs_up:".to_string(),
            }
        ));
        assert!(is_event_trigger_enabled(
            &config,
            &ChannelEvent::MessageEdit {
                channel_name: "discord".to_string(),
                channel_id: ChannelId::new("chan-1"),
                message_id: MessageId::new("msg-1"),
                new_content: "updated".to_string(),
                user_id: UserId::new("u"),
            }
        ));
    }
}
