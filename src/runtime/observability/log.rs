//! Log-based observer: records events and metrics via the `tracing`
//! crate with zero external dependencies.

use tracing::info;

use super::traits::{AutonomySignal, MemorySignal, Observer, ObserverEvent, ObserverMetric};

/// Log-based observer — uses tracing, zero external deps
pub struct LogObserver;

impl LogObserver {
    /// Create a new log-based observer.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for LogObserver {
    fn default() -> Self {
        Self::new()
    }
}

impl Observer for LogObserver {
    fn record_event(&self, event: &ObserverEvent) {
        match event {
            ObserverEvent::AgentStart { provider, model } => {
                info!(provider = %provider, model = %model, "agent.start");
            }
            ObserverEvent::AgentEnd {
                duration,
                tokens_used,
            } => {
                let ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
                info!(duration_ms = ms, tokens = ?tokens_used, "agent.end");
            }
            ObserverEvent::ToolCall {
                tool,
                duration,
                success,
            } => {
                let ms = u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
                info!(tool = %tool, duration_ms = ms, success = success, "tool.call");
            }
            ObserverEvent::ChannelMessage { channel, direction } => {
                info!(channel = %channel, direction = %direction, "channel.message");
            }
            ObserverEvent::CompanionPolicyRail {
                entity_id,
                person_id,
                session_id,
                phase,
                enforcement,
                reason_code,
            } => {
                info!(
                    entity_id = %entity_id,
                    person_id = %person_id,
                    session_id = ?session_id,
                    phase = %phase,
                    enforcement = %enforcement,
                    reason_code = %reason_code,
                    "companion.turn.policy_rail"
                );
            }
            ObserverEvent::CompanionTurnEvidence {
                entity_id,
                person_id,
                session_id,
                phase,
                decision,
                reason_code,
                provenance,
                summary,
            } => {
                info!(
                    entity_id = %entity_id,
                    person_id = %person_id,
                    session_id = ?session_id,
                    phase = %phase,
                    decision = %decision,
                    reason_code = %reason_code,
                    provenance = %provenance,
                    summary = %summary,
                    "companion.turn.evidence"
                );
            }
            ObserverEvent::HeartbeatTick => {
                info!("heartbeat.tick");
            }
            ObserverEvent::Error { component, message } => {
                info!(component = %component, error = %message, "error");
            }
        }
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        match metric {
            ObserverMetric::RequestLatency(d) => {
                let ms = u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
                info!(latency_ms = ms, "metric.request_latency");
            }
            ObserverMetric::TokensUsed(t) => {
                info!(tokens = t, "metric.tokens_used");
            }
            ObserverMetric::ActiveSessions(s) => {
                info!(sessions = s, "metric.active_sessions");
            }
            ObserverMetric::QueueDepth(d) => {
                info!(depth = d, "metric.queue_depth");
            }
            ObserverMetric::EntityKpiScore {
                axis,
                score,
                sample_size,
                source,
            } => {
                info!(
                    axis = %axis.as_str(),
                    score = score,
                    sample_size = sample_size,
                    source = %source,
                    "metric.asterel.entity.kpi.score"
                );
            }
            ObserverMetric::SignalIngestTotal { source_kind } => {
                info!(source_kind = %source_kind, "metric.signal_ingest_total");
            }
            ObserverMetric::SignalDedupDropTotal { source_kind } => {
                info!(source_kind = %source_kind, "metric.signal_dedup_drop_total");
            }
            ObserverMetric::BeliefPromotionTotal { count } => {
                info!(count = count, "metric.belief_promotion_total");
            }
            ObserverMetric::ContradictionMarkTotal { count } => {
                info!(count = count, "metric.contradiction_mark_total");
            }
            ObserverMetric::StaleTrendPurgeTotal { count } => {
                info!(count = count, "metric.stale_trend_purge_total");
            }
            ObserverMetric::SignalTierSnapshot { tier, count } => {
                info!(tier = %tier, count = count, "metric.signal_tier_snapshot");
            }
            ObserverMetric::PromotionStatusSnapshot { status, count } => {
                info!(status = %status, count = count, "metric.promotion_status_snapshot");
            }
            ObserverMetric::MemorySloViolation => {
                info!("metric.memory_slo_violation");
            }
            ObserverMetric::MemoryCorrectionTriggered { demoted_count } => {
                info!(
                    demoted_count = demoted_count,
                    "metric.memory_correction_triggered"
                );
            }
            ObserverMetric::AutonomyLifecycle(signal) => {
                info!(signal = %autonomy_signal_name(*signal), "metric.autonomy_lifecycle");
            }
            ObserverMetric::MemoryLifecycle(signal) => {
                info!(signal = %memory_signal_name(*signal), "metric.memory_lifecycle");
            }
            ObserverMetric::PostTurnHook { hook, status } => {
                info!(hook = %hook, status = %status, "metric.post_turn_hook");
            }
        }
    }

    fn name(&self) -> &'static str {
        "log"
    }
}

fn autonomy_signal_name(signal: AutonomySignal) -> &'static str {
    match signal {
        AutonomySignal::Ingested => "ingested",
        AutonomySignal::Deduplicated => "deduplicated",
        AutonomySignal::Promoted => "promoted",
        AutonomySignal::ContradictionDetected => "contradiction_detected",
        AutonomySignal::ModeTransition => "mode_transition",
        AutonomySignal::IntentCreated => "intent_created",
        AutonomySignal::IntentPolicyAllowed => "intent_policy_allowed",
        AutonomySignal::IntentPolicyDenied => "intent_policy_denied",
        AutonomySignal::IntentDispatched => "intent_dispatched",
        AutonomySignal::IntentExecutionBlocked => "intent_execution_blocked",
    }
}

fn memory_signal_name(signal: MemorySignal) -> &'static str {
    match signal {
        MemorySignal::ConsolidationStarted => "consolidation_started",
        MemorySignal::ConsolidationCompleted => "consolidation_completed",
        MemorySignal::ConflictDetected => "conflict_detected",
        MemorySignal::ConflictResolved => "conflict_resolved",
        MemorySignal::RevocationApplied => "revocation_applied",
        MemorySignal::GovernanceInspect => "governance_inspect",
        MemorySignal::GovernanceExport => "governance_export",
        MemorySignal::GovernanceDelete => "governance_delete",
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::contracts::ids::{EntityId, PersonId, SessionId};
    use crate::runtime::observability::traits::EntityKpiAxis;

    #[test]
    fn log_observer_name() {
        assert_eq!(LogObserver::new().name(), "log");
    }

    #[test]
    fn log_observer_all_events_no_panic() {
        let obs = LogObserver::new();
        obs.record_event(&ObserverEvent::AgentStart {
            provider: "openrouter".into(),
            model: "claude-sonnet".into(),
        });
        obs.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::from_millis(500),
            tokens_used: Some(100),
        });
        obs.record_event(&ObserverEvent::AgentEnd {
            duration: Duration::ZERO,
            tokens_used: None,
        });
        obs.record_event(&ObserverEvent::ToolCall {
            tool: "shell".into(),
            duration: Duration::from_millis(10),
            success: false,
        });
        obs.record_event(&ObserverEvent::ChannelMessage {
            channel: "telegram".into(),
            direction: "outbound".into(),
        });
        obs.record_event(&ObserverEvent::CompanionPolicyRail {
            entity_id: EntityId::new("person:test"),
            person_id: PersonId::new("test"),
            session_id: Some(SessionId::new("session-test")),
            phase: "tool_action".into(),
            enforcement: "runtime_guard".into(),
            reason_code: "tool_middleware_policy".into(),
        });
        obs.record_event(&ObserverEvent::CompanionTurnEvidence {
            entity_id: EntityId::new("person:test"),
            person_id: PersonId::new("test"),
            session_id: Some(SessionId::new("session-test")),
            phase: "output".into(),
            decision: "allow".into(),
            reason_code: "turn_output_available".into(),
            provenance: "turn_executor".into(),
            summary: "turn produced a user-facing response".into(),
        });
        obs.record_event(&ObserverEvent::HeartbeatTick);
        obs.record_event(&ObserverEvent::Error {
            component: "provider".into(),
            message: "timeout".into(),
        });
    }

    #[test]
    fn log_observer_all_metrics_no_panic() {
        let obs = LogObserver::new();
        obs.record_metric(&ObserverMetric::RequestLatency(Duration::from_secs(2)));
        obs.record_metric(&ObserverMetric::TokensUsed(0));
        obs.record_metric(&ObserverMetric::TokensUsed(u64::MAX));
        obs.record_metric(&ObserverMetric::ActiveSessions(1));
        obs.record_metric(&ObserverMetric::QueueDepth(999));
        obs.record_metric(&ObserverMetric::AutonomyLifecycle(
            AutonomySignal::IntentCreated,
        ));
        obs.record_metric(&ObserverMetric::MemoryLifecycle(
            MemorySignal::ConsolidationCompleted,
        ));
        obs.record_metric(&ObserverMetric::SignalTierSnapshot {
            tier: "raw".to_string(),
            count: 3,
        });
        obs.record_metric(&ObserverMetric::PromotionStatusSnapshot {
            status: "promoted".to_string(),
            count: 2,
        });
        obs.record_metric(&ObserverMetric::BeliefPromotionTotal { count: 2 });
        obs.record_metric(&ObserverMetric::ContradictionMarkTotal { count: 1 });
        obs.record_metric(&ObserverMetric::StaleTrendPurgeTotal { count: 4 });
        obs.record_metric(&ObserverMetric::MemorySloViolation);
        obs.record_metric(&ObserverMetric::MemoryCorrectionTriggered { demoted_count: 3 });
        obs.record_metric(&ObserverMetric::PostTurnHook {
            hook: "relationship_update".to_string(),
            status: "failure".to_string(),
        });
        obs.record_metric(&ObserverMetric::EntityKpiScore {
            axis: EntityKpiAxis::IdentityContinuity,
            score: 0.9,
            sample_size: 10,
            source: "test".to_string(),
        });
    }
}
