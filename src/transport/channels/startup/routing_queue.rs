//! Routing queue manager: groups inbound channel events by sender/channel,
//! enforces concurrency limits, and dispatches to handler tasks.
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

use super::super::traits::ChannelEvent;
#[cfg(test)]
use super::super::traits::ChannelMessage;
use crate::config::ChannelsConfig;
use crate::config::schema::RoutingRuleConfig;

const OVERFLOW_GROUP: &str = "__routing_overflow__";
#[cfg(not(test))]
const ROUTING_GROUP_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
#[cfg(test)]
const ROUTING_GROUP_IDLE_TIMEOUT: Duration = Duration::from_millis(80);

type HandlerFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
pub(super) type RoutingHandler = Arc<dyn Fn(ChannelEvent) -> HandlerFuture + Send + Sync>;

pub(super) struct RoutingQueueManager {
    global_concurrency: Arc<Semaphore>,
    group_queue_capacity: usize,
    max_groups: usize,
    routing_rules: Vec<RoutingRuleConfig>,
    group_senders: HashMap<String, mpsc::Sender<ChannelEvent>>,
    handler: RoutingHandler,
}

impl RoutingQueueManager {
    pub(super) fn from_config(config: &ChannelsConfig, handler: RoutingHandler) -> Self {
        Self {
            global_concurrency: Arc::new(Semaphore::new(config.routing_global_concurrency.max(1))),
            group_queue_capacity: config.routing_group_queue_capacity.max(1),
            max_groups: config.routing_max_groups.max(1),
            routing_rules: config.routing_rules.clone(),
            group_senders: HashMap::new(),
            handler,
        }
    }

    pub(super) async fn enqueue(&mut self, event: ChannelEvent) {
        self.group_senders.retain(|group, sender| {
            let keep = !sender.is_closed();
            if !keep {
                tracing::debug!(group = %group, "pruning closed routing group sender");
            }
            keep
        });

        let group = self.resolve_group(&event);
        let sender = if let Some(existing) = self.group_senders.get(&group) {
            existing.clone()
        } else {
            self.create_group_sender(group)
        };

        if let Err(error) = sender.send(event).await {
            tracing::warn!(%error, "routing queue send failed; dropping channel message");
        }
    }

    fn resolve_group(&self, event: &ChannelEvent) -> String {
        let (channel, sender, conversation_id) = match event {
            ChannelEvent::Message(msg) => (
                msg.channel.as_str(),
                msg.sender.as_str(),
                msg.conversation_id.as_deref(),
            ),
            other => (
                other.channel_name(),
                other.sender().unwrap_or("unknown"),
                other.conversation_id(),
            ),
        };

        for rule in &self.routing_rules {
            if rule.channel != channel {
                continue;
            }
            if let Some(rule_sender) = &rule.sender
                && rule_sender != sender
            {
                continue;
            }
            if let Some(conv_id) = &rule.conversation_id
                && conversation_id != Some::<&str>(conv_id.as_str())
            {
                continue;
            }

            return rule.group.clone();
        }

        format!("{channel}::{sender}")
    }

    fn create_group_sender(&mut self, requested_group: String) -> mpsc::Sender<ChannelEvent> {
        let bounded_group = if self.group_senders.len() >= self.max_groups
            && !self.group_senders.contains_key(OVERFLOW_GROUP)
        {
            tracing::warn!(
                max_groups = self.max_groups,
                "routing group cap reached; diverting to overflow group"
            );
            OVERFLOW_GROUP.to_string()
        } else if self.group_senders.len() >= self.max_groups {
            OVERFLOW_GROUP.to_string()
        } else {
            requested_group
        };

        if let Some(existing) = self.group_senders.get(&bounded_group) {
            return existing.clone();
        }

        let (tx, mut rx) = mpsc::channel::<ChannelEvent>(self.group_queue_capacity);
        let handler = Arc::clone(&self.handler);
        let permits = Arc::clone(&self.global_concurrency);
        let worker_group = bounded_group.clone();
        tokio::spawn(async move {
            loop {
                let event = match tokio::time::timeout(ROUTING_GROUP_IDLE_TIMEOUT, rx.recv()).await
                {
                    Ok(Some(event)) => event,
                    Ok(None) => break,
                    Err(_) => {
                        tracing::debug!(
                            group = %worker_group,
                            "routing worker exiting after idle timeout"
                        );
                        break;
                    }
                };

                let permit = acquire_global_permit(&permits).await;
                if permit.is_none() {
                    tracing::warn!(group = %worker_group, "routing worker stopping: global permits closed");
                    break;
                }

                (handler)(event).await;
            }
        });

        self.group_senders.insert(bounded_group, tx.clone());
        tx
    }
}

async fn acquire_global_permit(permits: &Arc<Semaphore>) -> Option<OwnedSemaphorePermit> {
    permits.clone().acquire_owned().await.ok()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;
    use crate::config::schema::RoutingRuleConfig;

    fn test_event(channel: &str, sender: &str, id: &str) -> ChannelEvent {
        ChannelEvent::Message(ChannelMessage {
            id: id.to_string(),
            sender: sender.to_string(),
            content: format!("msg-{id}"),
            channel: channel.to_string(),
            context_hint: None,
            conversation_id: None,
            thread_id: None,
            reply_to: None,
            message_id: Some(id.to_string()),
            timestamp: 0,
            attachments: Vec::new(),
        })
    }

    #[tokio::test]
    async fn routing_rules_match_first_applicable_rule() {
        let config = ChannelsConfig {
            routing_rules: vec![
                RoutingRuleConfig {
                    channel: "discord".to_string(),
                    sender: Some("ops".to_string()),
                    conversation_id: None,
                    group: "ops-group".to_string(),
                },
                RoutingRuleConfig {
                    channel: "discord".to_string(),
                    sender: None,
                    conversation_id: None,
                    group: "discord-default".to_string(),
                },
            ],
            ..ChannelsConfig::default()
        };

        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen_clone = Arc::clone(&seen);
        let handler: RoutingHandler = Arc::new(move |event| {
            let seen_clone = Arc::clone(&seen_clone);
            Box::pin(async move {
                let mut lock = seen_clone.lock().unwrap();
                lock.push(event.sender().unwrap_or("unknown").to_string());
            })
        });

        let mut manager = RoutingQueueManager::from_config(&config, handler);
        manager.enqueue(test_event("discord", "ops", "1")).await;
        manager.enqueue(test_event("discord", "user", "2")).await;

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(seen.lock().unwrap().len(), 2);
        assert!(manager.group_senders.contains_key("ops-group"));
        assert!(manager.group_senders.contains_key("discord-default"));
    }

    #[tokio::test]
    async fn routing_global_concurrency_cap_is_enforced() {
        let config = ChannelsConfig {
            routing_global_concurrency: 1,
            ..ChannelsConfig::default()
        };

        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));
        let handler: RoutingHandler = {
            let active = Arc::clone(&active);
            let max_active = Arc::clone(&max_active);
            let completed = Arc::clone(&completed);
            Arc::new(move |_message| {
                let active = Arc::clone(&active);
                let max_active = Arc::clone(&max_active);
                let completed = Arc::clone(&completed);
                Box::pin(async move {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    loop {
                        let observed = max_active.load(Ordering::SeqCst);
                        if current <= observed {
                            break;
                        }
                        if max_active
                            .compare_exchange(observed, current, Ordering::SeqCst, Ordering::SeqCst)
                            .is_ok()
                        {
                            break;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    completed.fetch_add(1, Ordering::SeqCst);
                })
            })
        };

        let mut manager = RoutingQueueManager::from_config(&config, handler);
        manager.enqueue(test_event("discord", "a", "1")).await;
        manager.enqueue(test_event("telegram", "b", "2")).await;

        tokio::time::timeout(Duration::from_secs(10), async {
            while completed.load(Ordering::SeqCst) < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("routing workers should complete both events under the concurrency cap");
        assert_eq!(completed.load(Ordering::SeqCst), 2);
        assert_eq!(max_active.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn routing_handles_burst_load_without_drops_under_capacity() {
        let config = ChannelsConfig {
            routing_global_concurrency: 4,
            routing_group_queue_capacity: 256,
            routing_max_groups: 32,
            ..ChannelsConfig::default()
        };

        let total_messages = 200;
        let completed = Arc::new(AtomicUsize::new(0));
        let handler: RoutingHandler = {
            let completed = Arc::clone(&completed);
            Arc::new(move |_message| {
                let completed = Arc::clone(&completed);
                Box::pin(async move {
                    completed.fetch_add(1, Ordering::SeqCst);
                })
            })
        };

        let mut manager = RoutingQueueManager::from_config(&config, handler);
        for idx in 0..total_messages {
            let sender = format!("user-{}", idx % 8);
            let id = idx.to_string();
            manager.enqueue(test_event("discord", &sender, &id)).await;
        }

        let wait_result = tokio::time::timeout(Duration::from_secs(3), async {
            while completed.load(Ordering::SeqCst) < total_messages {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(wait_result.is_ok());
        assert_eq!(completed.load(Ordering::SeqCst), total_messages);
    }

    #[tokio::test]
    async fn routing_uses_overflow_group_when_max_groups_reached() {
        let config = ChannelsConfig {
            routing_max_groups: 1,
            ..ChannelsConfig::default()
        };

        let handler: RoutingHandler = Arc::new(move |_message| Box::pin(async move {}));
        let mut manager = RoutingQueueManager::from_config(&config, handler);

        manager.enqueue(test_event("discord", "first", "1")).await;
        manager.enqueue(test_event("discord", "second", "2")).await;

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(manager.group_senders.contains_key("discord::first"));
        assert!(manager.group_senders.contains_key(OVERFLOW_GROUP));
    }

    #[tokio::test]
    async fn routing_reclaims_idle_group_slots_before_overflow() {
        let config = ChannelsConfig {
            routing_max_groups: 1,
            ..ChannelsConfig::default()
        };

        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen_clone = Arc::clone(&seen);
        let handler: RoutingHandler = Arc::new(move |event| {
            let seen_clone = Arc::clone(&seen_clone);
            Box::pin(async move {
                let mut lock = seen_clone.lock().unwrap();
                lock.push(event.sender().unwrap_or("unknown").to_string());
            })
        });

        let mut manager = RoutingQueueManager::from_config(&config, handler);
        manager.enqueue(test_event("discord", "group-a", "1")).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(manager.group_senders.contains_key("discord::group-a"));

        tokio::time::sleep(Duration::from_millis(120)).await;
        manager.enqueue(test_event("discord", "group-b", "2")).await;
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(manager.group_senders.contains_key("discord::group-b"));
        assert!(!manager.group_senders.contains_key(OVERFLOW_GROUP));
    }
}
