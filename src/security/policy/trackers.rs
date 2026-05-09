//! Rate limiting and cost tracking for security policy enforcement.
//!
//! Provides sliding-window action tracking, daily cost caps, and
//! per-entity rate limiters to prevent abuse.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::contracts::ids::EntityId;

/// Sliding-window action tracker for rate limiting.
///
/// Uses `Arc<Mutex<...>>` internally so that clones share the same state,
/// preventing rate-limit bypass when `SecurityPolicy` is cloned.
#[derive(Debug, Clone)]
pub struct ActionTracker {
    actions: Arc<Mutex<Vec<Instant>>>,
}

impl ActionTracker {
    /// Create a new empty action tracker.
    #[must_use]
    pub fn new() -> Self {
        Self {
            actions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Record an action and return the current count within the window.
    pub fn record(&self) -> usize {
        let mut actions = self
            .actions
            .lock()
            .unwrap_or_else(crate::security::poison_recover!());
        let cutoff = Instant::now()
            .checked_sub(std::time::Duration::from_secs(3600))
            .unwrap_or_else(Instant::now);
        actions.retain(|t| *t > cutoff);
        actions.push(Instant::now());
        actions.len()
    }

    /// Count of actions in the current window, pruning expired entries.
    pub fn count_active(&self) -> usize {
        let mut actions = self
            .actions
            .lock()
            .unwrap_or_else(crate::security::poison_recover!());
        let cutoff = Instant::now()
            .checked_sub(std::time::Duration::from_secs(3600))
            .unwrap_or_else(Instant::now);
        actions.retain(|t| *t > cutoff);
        actions.len()
    }
}

impl Default for ActionTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
struct RateLimiterState {
    global_actions: Vec<Instant>,
    per_entity: HashMap<EntityId, Vec<Instant>>,
    per_conversation: HashMap<String, Vec<Instant>>,
    per_workspace: HashMap<String, Vec<Instant>>,
}

/// Per-entity rate limiter with global backstop, conversation/workspace
/// scopes, and per-entity burst window.
#[derive(Debug)]
pub struct EntityRateLimiter {
    state: Mutex<RateLimiterState>,
    global_max: u32,
    per_entity_max: u32,
    per_conversation_max: u32,
    per_workspace_max: u32,
    burst_max: u32,
    burst_window_secs: u64,
}

/// Rate limit exhaustion error.
#[derive(Debug, Clone)]
pub enum RateLimitError {
    /// The global action budget has been exhausted.
    GlobalExhausted,
    /// The per-entity action budget has been exhausted.
    EntityExhausted {
        /// The entity whose budget was exhausted.
        entity_id: EntityId,
    },
    /// The per-conversation action budget has been exhausted.
    ConversationExhausted { conversation_id: String },
    /// The per-workspace action budget has been exhausted.
    WorkspaceExhausted { workspace_id: String },
    /// The per-entity short-window burst limit has been exceeded.
    BurstExhausted { entity_id: EntityId },
}

const HOUR_SECS: u64 = 3600;

impl EntityRateLimiter {
    /// Create a rate limiter with only global and per-entity caps.
    ///
    /// Conversation, workspace, and burst scopes are disabled.
    #[must_use]
    pub fn new(global_max: u32, per_entity_max: u32) -> Self {
        Self::new_with_scopes(global_max, per_entity_max, 0, 0, 0, 0)
    }

    #[must_use]
    pub fn new_with_scopes(
        global_max: u32,
        per_entity_max: u32,
        per_conversation_max: u32,
        per_workspace_max: u32,
        burst_max: u32,
        burst_window_secs: u64,
    ) -> Self {
        Self {
            state: Mutex::new(RateLimiterState {
                global_actions: Vec::new(),
                per_entity: HashMap::new(),
                per_conversation: HashMap::new(),
                per_workspace: HashMap::new(),
            }),
            global_max,
            per_entity_max,
            per_conversation_max,
            per_workspace_max,
            burst_max,
            burst_window_secs,
        }
    }

    /// Check and record an action for the given entity.
    ///
    /// Backward-compatible: checks global + per-entity scopes only.
    ///
    /// # Errors
    ///
    /// Returns an error when global or per-entity action limits are exceeded.
    pub fn check_and_record(&self, entity_id: &str) -> Result<(), RateLimitError> {
        self.check_and_record_scoped(entity_id, None, None)
    }

    /// Check and record an action with optional conversation and workspace
    /// scopes.
    ///
    /// Scope checks are skipped when the corresponding `max` is 0 or the
    /// key is `None`.
    ///
    /// # Errors
    ///
    /// Returns the first scope whose budget is exhausted, checked in order:
    /// global → workspace → conversation → entity (burst) → entity (hourly).
    pub fn check_and_record_scoped(
        &self,
        entity_id: &str,
        conversation_id: Option<&str>,
        workspace_id: Option<&str>,
    ) -> Result<(), RateLimitError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(crate::security::poison_recover!());
        let now = Instant::now();
        let hourly_cutoff = now
            .checked_sub(std::time::Duration::from_secs(HOUR_SECS))
            .unwrap_or(now);

        // 1. Global scope
        state.global_actions.retain(|t| *t > hourly_cutoff);
        prune_expired_scope_maps(&mut state, hourly_cutoff);
        if state.global_actions.len() >= usize::try_from(self.global_max).unwrap_or(usize::MAX) {
            return Err(RateLimitError::GlobalExhausted);
        }

        // 2. Workspace scope (skip when max=0 or key absent)
        if let Some(ws_id) = workspace_id.filter(|_| self.per_workspace_max > 0) {
            let ws_actions = state.per_workspace.entry(ws_id.to_string()).or_default();
            ws_actions.retain(|t| *t > hourly_cutoff);
            if ws_actions.len() >= usize::try_from(self.per_workspace_max).unwrap_or(usize::MAX) {
                return Err(RateLimitError::WorkspaceExhausted {
                    workspace_id: ws_id.to_string(),
                });
            }
        }

        // 3. Conversation scope (skip when max=0 or key absent)
        if let Some(conv_id) = conversation_id.filter(|_| self.per_conversation_max > 0) {
            let conv_actions = state
                .per_conversation
                .entry(conv_id.to_string())
                .or_default();
            conv_actions.retain(|t| *t > hourly_cutoff);
            if conv_actions.len()
                >= usize::try_from(self.per_conversation_max).unwrap_or(usize::MAX)
            {
                return Err(RateLimitError::ConversationExhausted {
                    conversation_id: conv_id.to_string(),
                });
            }
        }

        // 4. Entity scope — burst window first, then hourly
        let entity_key = EntityId::new(entity_id);
        {
            let entity_actions = state.per_entity.entry(entity_key.clone()).or_default();
            entity_actions.retain(|t| *t > hourly_cutoff);

            if self.burst_max > 0 && self.burst_window_secs > 0 {
                let burst_cutoff = now
                    .checked_sub(std::time::Duration::from_secs(self.burst_window_secs))
                    .unwrap_or(now);
                let burst_count = entity_actions.iter().filter(|t| **t > burst_cutoff).count();
                if burst_count >= usize::try_from(self.burst_max).unwrap_or(usize::MAX) {
                    return Err(RateLimitError::BurstExhausted {
                        entity_id: entity_key.clone(),
                    });
                }
            }

            if entity_actions.len() >= usize::try_from(self.per_entity_max).unwrap_or(usize::MAX) {
                return Err(RateLimitError::EntityExhausted {
                    entity_id: entity_key.clone(),
                });
            }
        }

        // Record in all applicable buckets
        state.global_actions.push(now);
        state.per_entity.entry(entity_key).or_default().push(now);
        if let Some(conv_id) = conversation_id {
            state
                .per_conversation
                .entry(conv_id.to_string())
                .or_default()
                .push(now);
        }
        if let Some(ws_id) = workspace_id {
            state
                .per_workspace
                .entry(ws_id.to_string())
                .or_default()
                .push(now);
        }
        Ok(())
    }
}

fn prune_expired_scope_maps(state: &mut RateLimiterState, cutoff: Instant) {
    state.per_entity.retain(|_, actions| {
        actions.retain(|t| *t > cutoff);
        !actions.is_empty()
    });
    state.per_conversation.retain(|_, actions| {
        actions.retain(|t| *t > cutoff);
        !actions.is_empty()
    });
    state.per_workspace.retain(|_, actions| {
        actions.retain(|t| *t > cutoff);
        !actions.is_empty()
    });
}

/// Daily cost tracker with shared state across clones.
#[derive(Debug, Clone)]
pub struct CostTracker {
    state: Arc<Mutex<DailyCostState>>,
}

#[derive(Debug, Clone, Copy)]
struct DailyCostState {
    day_epoch: u64,
    spent_cents: u32,
}

impl CostTracker {
    /// Create a new cost tracker starting at zero for the current day.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(DailyCostState {
                day_epoch: current_day_epoch(),
                spent_cents: 0,
            })),
        }
    }

    /// Record a cost and return whether the daily budget allows it.
    #[must_use]
    pub fn record(&self, additional_cents: u32, max_cents_per_day: u32) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(crate::security::poison_recover!());
        rollover_day_if_needed(&mut state);
        if additional_cents == 0 {
            return state.spent_cents <= max_cents_per_day;
        }
        if state.spent_cents.saturating_add(additional_cents) > max_cents_per_day {
            return false;
        }
        state.spent_cents = state.spent_cents.saturating_add(additional_cents);
        true
    }

    /// Return the total cents spent so far today.
    #[must_use]
    pub fn spent_today(&self) -> u32 {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(crate::security::poison_recover!());
        rollover_day_if_needed(&mut state);
        state.spent_cents
    }
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new()
    }
}

fn current_day_epoch() -> u64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or_else(
            |_| {
                tracing::error!(
                    "system clock before UNIX epoch; cost cap assumes day 0 (deny-all)"
                );
                0
            },
            |d| d.as_secs(),
        );
    secs / 86_400
}

fn rollover_day_if_needed(state: &mut DailyCostState) {
    let today = current_day_epoch();
    if state.day_epoch != today {
        state.day_epoch = today;
        state.spent_cents = 0;
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{EntityRateLimiter, RateLimitError};

    #[test]
    fn entity_rate_limiter_allows_independent_entity_buckets() {
        let limiter = EntityRateLimiter::new(10, 2);

        assert!(limiter.check_and_record("entity-a").is_ok());
        assert!(limiter.check_and_record("entity-a").is_ok());
        assert!(matches!(
            limiter.check_and_record("entity-a"),
            Err(RateLimitError::EntityExhausted { .. })
        ));

        assert!(limiter.check_and_record("entity-b").is_ok());
        assert!(limiter.check_and_record("entity-b").is_ok());
    }

    #[test]
    fn entity_rate_limiter_enforces_global_backstop() {
        let limiter = EntityRateLimiter::new(2, 10);

        assert!(limiter.check_and_record("entity-a").is_ok());
        assert!(limiter.check_and_record("entity-b").is_ok());
        assert!(matches!(
            limiter.check_and_record("entity-c"),
            Err(RateLimitError::GlobalExhausted)
        ));
    }

    #[test]
    fn scoped_limiter_enforces_conversation_cap() {
        let limiter = EntityRateLimiter::new_with_scopes(100, 100, 2, 0, 0, 0);

        assert!(
            limiter
                .check_and_record_scoped("e1", Some("conv-a"), None)
                .is_ok()
        );
        assert!(
            limiter
                .check_and_record_scoped("e1", Some("conv-a"), None)
                .is_ok()
        );
        assert!(matches!(
            limiter.check_and_record_scoped("e1", Some("conv-a"), None),
            Err(RateLimitError::ConversationExhausted { .. })
        ));
        assert!(
            limiter
                .check_and_record_scoped("e1", Some("conv-b"), None)
                .is_ok(),
            "different conversation should still pass"
        );
    }

    #[test]
    fn scoped_limiter_enforces_workspace_cap() {
        let limiter = EntityRateLimiter::new_with_scopes(100, 100, 0, 2, 0, 0);

        assert!(
            limiter
                .check_and_record_scoped("e1", None, Some("ws-1"))
                .is_ok()
        );
        assert!(
            limiter
                .check_and_record_scoped("e2", None, Some("ws-1"))
                .is_ok()
        );
        assert!(matches!(
            limiter.check_and_record_scoped("e3", None, Some("ws-1")),
            Err(RateLimitError::WorkspaceExhausted { .. })
        ));
        assert!(
            limiter
                .check_and_record_scoped("e3", None, Some("ws-2"))
                .is_ok(),
            "different workspace should still pass"
        );
    }

    #[test]
    fn scoped_limiter_enforces_burst_cap() {
        let limiter = EntityRateLimiter::new_with_scopes(100, 100, 0, 0, 3, 60);

        assert!(limiter.check_and_record_scoped("e1", None, None).is_ok());
        assert!(limiter.check_and_record_scoped("e1", None, None).is_ok());
        assert!(limiter.check_and_record_scoped("e1", None, None).is_ok());
        assert!(matches!(
            limiter.check_and_record_scoped("e1", None, None),
            Err(RateLimitError::BurstExhausted { .. })
        ));
        assert!(
            limiter.check_and_record_scoped("e2", None, None).is_ok(),
            "different entity should still pass"
        );
    }

    #[test]
    fn scoped_limiter_skips_disabled_scopes() {
        let limiter = EntityRateLimiter::new_with_scopes(100, 100, 0, 0, 0, 0);

        for _ in 0..50 {
            assert!(
                limiter
                    .check_and_record_scoped("e1", Some("conv"), Some("ws"))
                    .is_ok()
            );
        }
    }

    #[test]
    fn backward_compat_new_delegates_to_scoped() {
        let limiter = EntityRateLimiter::new(5, 2);

        assert!(limiter.check_and_record("e1").is_ok());
        assert!(limiter.check_and_record("e1").is_ok());
        assert!(matches!(
            limiter.check_and_record("e1"),
            Err(RateLimitError::EntityExhausted { .. })
        ));
    }

    #[test]
    fn scoped_limiter_prunes_empty_expired_buckets() {
        let limiter = EntityRateLimiter::new_with_scopes(100, 100, 100, 100, 0, 0);
        let expired = Instant::now() - Duration::from_secs(super::HOUR_SECS + 60);
        {
            let mut state = limiter
                .state
                .lock()
                .unwrap_or_else(crate::security::poison_recover!());
            state.per_entity.insert(
                crate::contracts::ids::EntityId::new("expired"),
                vec![expired],
            );
            state
                .per_conversation
                .insert("expired-conv".to_string(), vec![expired]);
            state
                .per_workspace
                .insert("expired-ws".to_string(), vec![expired]);
        }

        limiter
            .check_and_record_scoped("fresh", Some("fresh-conv"), Some("fresh-ws"))
            .unwrap();

        let state = limiter
            .state
            .lock()
            .unwrap_or_else(crate::security::poison_recover!());
        assert!(
            !state
                .per_entity
                .contains_key(&crate::contracts::ids::EntityId::new("expired"))
        );
        assert!(!state.per_conversation.contains_key("expired-conv"));
        assert!(!state.per_workspace.contains_key("expired-ws"));
    }
}
