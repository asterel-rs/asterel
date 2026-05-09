//! **[STUB]** OpenTelemetry observer: counter-only implementation.
//!
//! ⚠️ **Not a full OTLP exporter.** This module tracks event and metric counts via atomics only.
//! Real OTLP exporter implementation required for production observability.
//! See [`OtelStubObserver`] for current capabilities.

use std::sync::atomic::{AtomicU64, Ordering};

use super::traits::{AutonomySignal, MemorySignal, Observer, ObserverEvent, ObserverMetric};

/// OpenTelemetry observer stub that tracks event and metric counts
/// via atomics until a real OTLP exporter replaces it.
pub struct OtelStubObserver {
    event_count: AtomicU64,
    metric_count: AtomicU64,
}

impl OtelStubObserver {
    /// Create a new OpenTelemetry observer with zeroed counters.
    #[must_use]
    pub fn new() -> Self {
        Self {
            event_count: AtomicU64::new(0),
            metric_count: AtomicU64::new(0),
        }
    }

    fn event_kind(event: &ObserverEvent) -> &'static str {
        match event {
            ObserverEvent::AgentStart { .. } => "agent_start",
            ObserverEvent::AgentEnd { .. } => "agent_end",
            ObserverEvent::ToolCall { .. } => "tool_call",
            ObserverEvent::ChannelMessage { .. } => "channel_message",
            ObserverEvent::CompanionPolicyRail { .. } => "companion_policy_rail",
            ObserverEvent::CompanionTurnEvidence { .. } => "companion_turn_evidence",
            ObserverEvent::HeartbeatTick => "heartbeat_tick",
            ObserverEvent::Error { .. } => "error",
        }
    }

    fn metric_kind(metric: &ObserverMetric) -> &'static str {
        match metric {
            ObserverMetric::RequestLatency(_) => "request_latency",
            ObserverMetric::TokensUsed(_) => "tokens_used",
            ObserverMetric::ActiveSessions(_) => "active_sessions",
            ObserverMetric::QueueDepth(_) => "queue_depth",
            ObserverMetric::EntityKpiScore { .. } => "asterel.entity.kpi.score",
            ObserverMetric::SignalIngestTotal { .. } => "signal_ingest_total",
            ObserverMetric::SignalDedupDropTotal { .. } => "signal_dedup_drop_total",
            ObserverMetric::BeliefPromotionTotal { .. } => "belief_promotion_total",
            ObserverMetric::ContradictionMarkTotal { .. } => "contradiction_mark_total",
            ObserverMetric::StaleTrendPurgeTotal { .. } => "stale_trend_purge_total",
            ObserverMetric::SignalTierSnapshot { .. } => "signal_tier_snapshot",
            ObserverMetric::PromotionStatusSnapshot { .. } => "promotion_status_snapshot",
            ObserverMetric::MemorySloViolation => "memory_slo_violation",
            ObserverMetric::MemoryCorrectionTriggered { .. } => "memory_correction_triggered",
            ObserverMetric::AutonomyLifecycle(signal) => autonomy_signal_name(*signal),
            ObserverMetric::MemoryLifecycle(signal) => memory_signal_name(*signal),
            ObserverMetric::PostTurnHook { .. } => "post_turn_hook",
        }
    }

    #[cfg(test)]
    fn snapshot_counts(&self) -> (u64, u64) {
        (
            self.event_count.load(Ordering::Relaxed),
            self.metric_count.load(Ordering::Relaxed),
        )
    }
}

impl Default for OtelStubObserver {
    fn default() -> Self {
        Self::new()
    }
}

fn autonomy_signal_name(signal: AutonomySignal) -> &'static str {
    match signal {
        AutonomySignal::Ingested => "autonomy_ingested",
        AutonomySignal::Deduplicated => "autonomy_deduplicated",
        AutonomySignal::Promoted => "autonomy_promoted",
        AutonomySignal::ContradictionDetected => "autonomy_contradiction_detected",
        AutonomySignal::ModeTransition => "autonomy_mode_transition",
        AutonomySignal::IntentCreated => "autonomy_intent_created",
        AutonomySignal::IntentPolicyAllowed => "autonomy_intent_policy_allowed",
        AutonomySignal::IntentPolicyDenied => "autonomy_intent_policy_denied",
        AutonomySignal::IntentDispatched => "autonomy_intent_dispatched",
        AutonomySignal::IntentExecutionBlocked => "autonomy_intent_execution_blocked",
    }
}

fn memory_signal_name(signal: MemorySignal) -> &'static str {
    match signal {
        MemorySignal::ConsolidationStarted => "memory_consolidation_started",
        MemorySignal::ConsolidationCompleted => "memory_consolidation_completed",
        MemorySignal::ConflictDetected => "memory_conflict_detected",
        MemorySignal::ConflictResolved => "memory_conflict_resolved",
        MemorySignal::RevocationApplied => "memory_revocation_applied",
        MemorySignal::GovernanceInspect => "memory_governance_inspect",
        MemorySignal::GovernanceExport => "memory_governance_export",
        MemorySignal::GovernanceDelete => "memory_governance_delete",
    }
}

impl Observer for OtelStubObserver {
    fn record_event(&self, event: &ObserverEvent) {
        self.event_count.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(event = Self::event_kind(event), "observer.otel_stub.event");
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        self.metric_count.fetch_add(1, Ordering::Relaxed);
        if let ObserverMetric::EntityKpiScore {
            axis,
            score,
            sample_size,
            source,
        } = metric
        {
            tracing::debug!(
                metric = Self::metric_kind(metric),
                axis = %axis.as_str(),
                score = score,
                sample_size = sample_size,
                source = %source,
                "observer.otel_stub.metric"
            );
        } else {
            tracing::debug!(
                metric = Self::metric_kind(metric),
                "observer.otel_stub.metric"
            );
        }
    }

    fn flush(&self) {
        tracing::debug!(
            events_total = self.event_count.load(Ordering::Relaxed),
            metrics_total = self.metric_count.load(Ordering::Relaxed),
            "observer.otel_stub.flush"
        );
    }

    fn name(&self) -> &'static str {
        "otel_stub"
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::runtime::observability::traits::EntityKpiAxis;

    #[test]
    fn otel_observer_name() {
        assert_eq!(OtelStubObserver::new().name(), "otel_stub");
    }

    #[test]
    fn otel_observer_smoke_and_counts() {
        let obs = OtelStubObserver::new();

        obs.record_event(&ObserverEvent::AgentStart {
            provider: "openrouter".into(),
            model: "gpt-5".into(),
        });
        obs.record_event(&ObserverEvent::HeartbeatTick);
        obs.record_metric(&ObserverMetric::RequestLatency(Duration::from_millis(5)));
        obs.record_metric(&ObserverMetric::TokensUsed(10));
        obs.record_metric(&ObserverMetric::AutonomyLifecycle(
            AutonomySignal::IntentPolicyDenied,
        ));
        obs.record_metric(&ObserverMetric::MemoryLifecycle(
            MemorySignal::GovernanceDelete,
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
        obs.record_metric(&ObserverMetric::MemoryCorrectionTriggered { demoted_count: 2 });
        obs.record_metric(&ObserverMetric::EntityKpiScore {
            axis: EntityKpiAxis::TasteConsistency,
            score: 0.0,
            sample_size: 0,
            source: "taste_ratings.unavailable".to_string(),
        });
        obs.flush();

        assert_eq!(obs.snapshot_counts(), (2, 12));
    }

    #[test]
    fn otel_entity_kpi_metric_name_is_otel_compatible() {
        assert_eq!(
            OtelStubObserver::metric_kind(&ObserverMetric::EntityKpiScore {
                axis: EntityKpiAxis::IdentityContinuity,
                score: 0.9,
                sample_size: 42,
                source: "test".to_string(),
            }),
            "asterel.entity.kpi.score"
        );
    }
}
