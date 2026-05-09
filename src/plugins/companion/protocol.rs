//! Companion protocol: versioned event envelope for module,
//! config, context, capability, and spark event channels.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::contracts::ids::EventId;

const COMPANION_PROTOCOL_SCHEMA_VERSION: &str = "companion-protocol/v1";

/// Named channel for routing companion protocol events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanionEventChannel {
    /// Module lifecycle events.
    Module,
    /// Configuration change events.
    Config,
    /// Ambient context events.
    Context,
    /// Capability discovery events.
    Capability,
    /// Spontaneous spark events.
    Spark,
}

impl CompanionEventChannel {
    /// Returns the `snake_case` string label for this channel.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Module => "module",
            Self::Config => "config",
            Self::Context => "context",
            Self::Capability => "capability",
            Self::Spark => "spark",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "module" => Some(Self::Module),
            "config" => Some(Self::Config),
            "context" => Some(Self::Context),
            "capability" => Some(Self::Capability),
            "spark" => Some(Self::Spark),
            _ => None,
        }
    }
}

/// Lifecycle state of a companion module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanionModuleState {
    /// Module is not loaded.
    Unloaded,
    /// Module binary/code is loaded.
    Loaded,
    /// Module has completed initialization.
    Initialized,
    /// Module is configured but not yet ready.
    Configured,
    /// Module is fully operational.
    Ready,
    /// Module is temporarily suspended.
    Suspended,
    /// Module encountered an unrecoverable error.
    Failed,
}

/// Signal that drives companion module lifecycle transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanionLifeSignal {
    /// Load the module binary/code.
    Load,
    /// Initialize internal state.
    Initialize,
    /// Apply configuration.
    Configure,
    /// Mark the module as ready.
    MarkReady,
    /// Temporarily suspend the module.
    Suspend,
    /// Resume from suspension.
    Resume,
    /// Mark the module as failed.
    Fail,
    /// Recover from a failed state.
    Recover,
    /// Unload the module completely.
    Unload,
}

impl CompanionModuleState {
    /// # Errors
    ///
    /// Returns an error when the lifecycle transition would violate the phase
    /// order (`load -> init -> configure -> ready`) or recovery constraints.
    pub fn transition(self, signal: CompanionLifeSignal) -> Result<Self> {
        let next = match (self, signal) {
            (Self::Unloaded, CompanionLifeSignal::Load)
            | (Self::Failed, CompanionLifeSignal::Recover) => Self::Loaded,
            (Self::Loaded, CompanionLifeSignal::Initialize) => Self::Initialized,
            (Self::Initialized, CompanionLifeSignal::Configure) => Self::Configured,
            (Self::Configured, CompanionLifeSignal::MarkReady)
            | (Self::Suspended, CompanionLifeSignal::Resume) => Self::Ready,
            (Self::Ready, CompanionLifeSignal::Suspend) => Self::Suspended,
            (_, CompanionLifeSignal::Fail) => Self::Failed,
            (_, CompanionLifeSignal::Unload) => Self::Unloaded,
            _ => {
                anyhow::bail!(
                    "invalid companion lifecycle transition: state={self:?}, signal={signal:?}"
                )
            }
        };
        Ok(next)
    }
}

/// Parsed route expression: `companion/<module>/<channel>/<topic>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionRouteExpression {
    /// Module that owns this route.
    pub module_id: String,
    /// Event channel for the route.
    pub channel: CompanionEventChannel,
    /// Topic within the channel.
    pub topic: String,
}

impl CompanionRouteExpression {
    /// # Errors
    ///
    /// Returns an error when the route expression is not in
    /// `companion/<module>/<channel>/<topic>` format.
    pub fn parse(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        let mut segments = raw.split('/');

        let prefix = segments.next().unwrap_or_default();
        let module_id = segments.next().unwrap_or_default();
        let channel = segments.next().unwrap_or_default();
        let topic = segments.next().unwrap_or_default();

        if segments.next().is_some() {
            anyhow::bail!("companion route expression has too many segments: '{raw}'");
        }
        if prefix != "companion" {
            anyhow::bail!("companion route expression must start with 'companion/'");
        }

        validate_route_segment("module_id", module_id)?;
        validate_route_segment("topic", topic)?;

        let Some(channel) = CompanionEventChannel::from_str(channel) else {
            anyhow::bail!(
                "companion route channel must be one of module|config|context|capability|spark"
            );
        };

        Ok(Self {
            module_id: module_id.to_string(),
            channel,
            topic: topic.to_string(),
        })
    }

    /// Encodes the route expression back to its canonical string.
    #[must_use]
    pub fn encode(&self) -> String {
        format!(
            "companion/{}/{}/{}",
            self.module_id,
            self.channel.as_str(),
            self.topic
        )
    }
}

fn validate_route_segment(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("companion route {field} must not be empty");
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        anyhow::bail!("companion route {field} must use only [A-Za-z0-9._-], got '{value}'");
    }
    Ok(())
}

/// Versioned event envelope carrying a companion protocol event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionEventEnvelope {
    /// Protocol schema version string.
    pub schema_version: String,
    /// Unique event identifier.
    pub event_id: EventId,
    /// Monotonic sequence number within the module.
    pub sequence: u64,
    /// Originating module identifier.
    pub module_id: String,
    /// Event channel.
    pub channel: CompanionEventChannel,
    /// Topic within the channel.
    pub topic: String,
    /// Canonical route expression string.
    pub route: String,
    /// Source system that emitted the event.
    pub source: String,
    /// RFC 3339 emission timestamp.
    pub emitted_at: String,
    /// Arbitrary JSON payload.
    #[serde(default)]
    pub payload: Value,
}

impl CompanionEventEnvelope {
    /// # Errors
    ///
    /// Returns an error when module/topic/source are invalid or route
    /// expression generation fails.
    pub fn new(
        module_id: impl Into<String>,
        channel: CompanionEventChannel,
        topic: impl Into<String>,
        source: impl Into<String>,
        payload: Value,
        sequence: u64,
    ) -> Result<Self> {
        let module_id = module_id.into();
        let topic = topic.into();
        let source = source.into();

        validate_route_segment("module_id", &module_id)?;
        validate_route_segment("topic", &topic)?;
        validate_route_segment("source", &source)?;

        let route = CompanionRouteExpression {
            module_id: module_id.clone(),
            channel,
            topic: topic.clone(),
        }
        .encode();

        Ok(Self {
            schema_version: COMPANION_PROTOCOL_SCHEMA_VERSION.to_string(),
            event_id: EventId::new(Uuid::new_v4().to_string()),
            sequence,
            module_id,
            channel,
            topic,
            route,
            source,
            emitted_at: Utc::now().to_rfc3339(),
            payload,
        })
    }

    /// # Errors
    ///
    /// Returns an error when route/channel/topic/module mismatch is detected.
    pub fn validate_contract(&self) -> Result<()> {
        if self.schema_version != COMPANION_PROTOCOL_SCHEMA_VERSION {
            anyhow::bail!(
                "unsupported companion schema version '{}'",
                self.schema_version
            );
        }

        let parsed = CompanionRouteExpression::parse(&self.route)?;
        if parsed.module_id != self.module_id
            || parsed.channel != self.channel
            || parsed.topic != self.topic
        {
            anyhow::bail!("companion event envelope route does not match channel/module/topic");
        }

        Ok(())
    }
}

/// Connection health state for a companion session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanionConnectionState {
    /// Connection is healthy and receiving heartbeats.
    Active,
    /// Heartbeats are delayed beyond the stale threshold.
    Stale,
    /// A reconnection attempt is in progress.
    Reconnecting,
    /// Connection has been lost.
    Disconnected,
    /// Reconnection attempts exhausted.
    Failed,
}

/// Result of evaluating heartbeat health.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionHeartbeatEvaluation {
    /// Current connection state.
    pub state: CompanionConnectionState,
    /// Whether a reconnection attempt should be initiated.
    pub reconnect_required: bool,
    /// Seconds to wait before retrying, if applicable.
    pub retry_after_secs: Option<u64>,
}

/// Policy controlling heartbeat evaluation thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompanionHeartbeatPolicy {
    /// Seconds without heartbeat before marking stale.
    pub stale_after_secs: u64,
    /// Seconds without heartbeat before marking disconnected.
    pub disconnect_after_secs: u64,
    /// Minimum seconds between reconnection attempts.
    pub reconnect_cooldown_secs: u64,
    /// Maximum reconnection attempts before marking failed.
    pub max_reconnect_attempts: u32,
}

impl Default for CompanionHeartbeatPolicy {
    fn default() -> Self {
        Self {
            stale_after_secs: 20,
            disconnect_after_secs: 60,
            reconnect_cooldown_secs: 5,
            max_reconnect_attempts: 3,
        }
    }
}

impl CompanionHeartbeatPolicy {
    /// Evaluates heartbeat health and returns connection state.
    #[must_use]
    pub fn evaluate(
        &self,
        now: DateTime<Utc>,
        last_heartbeat_at: Option<DateTime<Utc>>,
        reconnect_attempts: u32,
        last_reconnect_attempt_at: Option<DateTime<Utc>>,
    ) -> CompanionHeartbeatEvaluation {
        if reconnect_attempts >= self.max_reconnect_attempts {
            return CompanionHeartbeatEvaluation {
                state: CompanionConnectionState::Failed,
                reconnect_required: false,
                retry_after_secs: None,
            };
        }

        let Some(last_heartbeat_at) = last_heartbeat_at else {
            return self.evaluate_reconnect_window_with_last_attempt(
                now,
                CompanionConnectionState::Disconnected,
                true,
                last_reconnect_attempt_at,
            );
        };

        let heartbeat_age = now.signed_duration_since(last_heartbeat_at);
        if heartbeat_age <= duration_from_secs(self.stale_after_secs) {
            return CompanionHeartbeatEvaluation {
                state: CompanionConnectionState::Active,
                reconnect_required: false,
                retry_after_secs: None,
            };
        }

        if heartbeat_age <= duration_from_secs(self.disconnect_after_secs) {
            return self.evaluate_reconnect_window_with_last_attempt(
                now,
                CompanionConnectionState::Stale,
                true,
                last_reconnect_attempt_at,
            );
        }

        self.evaluate_reconnect_window_with_last_attempt(
            now,
            CompanionConnectionState::Disconnected,
            true,
            last_reconnect_attempt_at,
        )
    }

    fn evaluate_reconnect_window_with_last_attempt(
        &self,
        now: DateTime<Utc>,
        fallback_state: CompanionConnectionState,
        reconnect_required: bool,
        last_reconnect_attempt_at: Option<DateTime<Utc>>,
    ) -> CompanionHeartbeatEvaluation {
        let Some(last_reconnect_attempt_at) = last_reconnect_attempt_at else {
            return CompanionHeartbeatEvaluation {
                state: fallback_state,
                reconnect_required,
                retry_after_secs: None,
            };
        };

        let elapsed = now.signed_duration_since(last_reconnect_attempt_at);
        let cooldown = duration_from_secs(self.reconnect_cooldown_secs);
        if elapsed < cooldown {
            let remaining = non_negative_i64_to_u64((cooldown - elapsed).num_seconds());
            return CompanionHeartbeatEvaluation {
                state: CompanionConnectionState::Reconnecting,
                reconnect_required: false,
                retry_after_secs: Some(remaining),
            };
        }

        CompanionHeartbeatEvaluation {
            state: fallback_state,
            reconnect_required,
            retry_after_secs: None,
        }
    }

    /// Convenience alias for [`evaluate`](Self::evaluate).
    #[must_use]
    pub fn evaluate_with_last_attempt(
        &self,
        now: DateTime<Utc>,
        last_heartbeat_at: Option<DateTime<Utc>>,
        reconnect_attempts: u32,
        last_reconnect_attempt_at: Option<DateTime<Utc>>,
    ) -> CompanionHeartbeatEvaluation {
        self.evaluate(
            now,
            last_heartbeat_at,
            reconnect_attempts,
            last_reconnect_attempt_at,
        )
    }
}

fn duration_from_secs(seconds: u64) -> Duration {
    Duration::seconds(i64::try_from(seconds).unwrap_or(i64::MAX))
}

fn non_negative_i64_to_u64(value: i64) -> u64 {
    u64::try_from(value.max(0)).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use serde_json::json;

    use super::{
        CompanionConnectionState, CompanionEventChannel, CompanionEventEnvelope,
        CompanionHeartbeatPolicy, CompanionLifeSignal, CompanionModuleState,
        CompanionRouteExpression,
    };

    #[test]
    fn lifecycle_allows_nominal_boot_sequence() {
        let mut state = CompanionModuleState::Unloaded;
        state = state.transition(CompanionLifeSignal::Load).unwrap();
        state = state.transition(CompanionLifeSignal::Initialize).unwrap();
        state = state.transition(CompanionLifeSignal::Configure).unwrap();
        state = state.transition(CompanionLifeSignal::MarkReady).unwrap();
        assert_eq!(state, CompanionModuleState::Ready);
    }

    #[test]
    fn lifecycle_rejects_out_of_order_ready_transition() {
        let error = CompanionModuleState::Loaded
            .transition(CompanionLifeSignal::MarkReady)
            .expect_err("ready without configure must fail");
        assert!(
            error
                .to_string()
                .contains("invalid companion lifecycle transition")
        );
    }

    #[test]
    fn route_expression_parses_valid_shape() {
        let route =
            CompanionRouteExpression::parse("companion/widget.context.caption_update").err();
        assert!(route.is_some());

        let route = CompanionRouteExpression::parse("companion/widget/context/caption_update")
            .expect("valid route shape should parse");
        assert_eq!(route.module_id, "widget");
        assert_eq!(route.channel, CompanionEventChannel::Context);
        assert_eq!(route.topic, "caption_update");
        assert_eq!(route.encode(), "companion/widget/context/caption_update");
    }

    #[test]
    fn event_envelope_builds_canonical_route_and_validates() {
        let envelope = CompanionEventEnvelope::new(
            "caption",
            CompanionEventChannel::Spark,
            "utterance",
            "gateway",
            json!({"text":"hello"}),
            9,
        )
        .unwrap();

        assert_eq!(envelope.route, "companion/caption/spark/utterance");
        envelope.validate_contract().unwrap();
    }

    #[test]
    fn heartbeat_marks_stale_before_disconnect() {
        let policy = CompanionHeartbeatPolicy {
            stale_after_secs: 20,
            disconnect_after_secs: 60,
            reconnect_cooldown_secs: 5,
            max_reconnect_attempts: 3,
        };

        let now = Utc::now();
        let last_heartbeat_at = now - Duration::seconds(30);
        let eval = policy.evaluate_with_last_attempt(now, Some(last_heartbeat_at), 0, None);
        assert_eq!(eval.state, CompanionConnectionState::Stale);
        assert!(eval.reconnect_required);
    }

    #[test]
    fn heartbeat_enforces_reconnect_cooldown_window() {
        let policy = CompanionHeartbeatPolicy::default();
        let now = Utc::now();
        let last_heartbeat_at = now - Duration::seconds(90);
        let last_attempt_at = now - Duration::seconds(2);

        let eval = policy.evaluate_with_last_attempt(
            now,
            Some(last_heartbeat_at),
            1,
            Some(last_attempt_at),
        );
        assert_eq!(eval.state, CompanionConnectionState::Reconnecting);
        assert!(!eval.reconnect_required);
        assert!(eval.retry_after_secs.is_some());
    }

    #[test]
    fn heartbeat_limits_reconnect_attempts() {
        let policy = CompanionHeartbeatPolicy {
            max_reconnect_attempts: 2,
            ..CompanionHeartbeatPolicy::default()
        };
        let now = Utc::now();

        let eval = policy.evaluate_with_last_attempt(now, None, 2, None);
        assert_eq!(eval.state, CompanionConnectionState::Failed);
        assert!(!eval.reconnect_required);
    }
}
