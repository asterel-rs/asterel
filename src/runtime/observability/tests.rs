//! Unit tests for observer factory and backend implementations.

use std::time::Duration;

use super::*;
use crate::runtime::observability::traits::{AutonomySignal, MemorySignal, ObserverMetric};

#[test]
fn factory_none_returns_noop() {
    let cfg = crate::config::ObservabilityConfig {
        backend: crate::config::ObservabilityBackend::None,
    };
    assert_eq!(create_observer(&cfg).name(), "noop");
}

#[test]
fn factory_log_returns_log() {
    let cfg = crate::config::ObservabilityConfig {
        backend: crate::config::ObservabilityBackend::Log,
    };
    assert_eq!(create_observer(&cfg).name(), "log");
}

#[test]
fn factory_prometheus_returns_prometheus() {
    let cfg = crate::config::ObservabilityConfig {
        backend: crate::config::ObservabilityBackend::Prometheus,
    };
    assert_eq!(create_observer(&cfg).name(), "prometheus");
}

#[test]
fn factory_otel_returns_otel() {
    let cfg = crate::config::ObservabilityConfig {
        backend: crate::config::ObservabilityBackend::Otel,
    };
    assert_eq!(create_observer(&cfg).name(), "otel_stub");
}

#[test]
fn factory_expanded_backends_smoke_paths() {
    let prometheus = create_observer(&crate::config::ObservabilityConfig {
        backend: crate::config::ObservabilityBackend::Prometheus,
    });
    prometheus.record_event(&ObserverEvent::HeartbeatTick);
    prometheus.record_metric(&ObserverMetric::QueueDepth(1));
    prometheus.flush();

    let otel = create_observer(&crate::config::ObservabilityConfig {
        backend: crate::config::ObservabilityBackend::Otel,
    });
    otel.record_event(&ObserverEvent::AgentEnd {
        duration: Duration::from_secs(1),
        tokens_used: Some(123),
    });
    otel.record_metric(&ObserverMetric::TokensUsed(123));
    otel.flush();
}

#[test]
fn factory_default_returns_noop() {
    let cfg = crate::config::ObservabilityConfig::default();
    assert_eq!(create_observer(&cfg).name(), "noop");
}

#[test]
fn observability_records_intent_metrics() {
    let observer = PrometheusObserver::new();

    observer.emit_autonomy_signal(AutonomySignal::IntentCreated);
    observer.emit_autonomy_signal(AutonomySignal::IntentPolicyAllowed);
    observer.emit_autonomy_signal(AutonomySignal::ContradictionDetected);

    let autonomy_counts = observer.snapshot_autonomy_counts();
    assert_eq!(autonomy_counts.intent_created, 1);
    assert_eq!(autonomy_counts.intent_policy_allowed, 1);
    assert_eq!(autonomy_counts.contradiction_detected, 1);
    assert_eq!(autonomy_counts.total, 3);
}

#[test]
fn observability_memory_lifecycle_metrics() {
    let observer = PrometheusObserver::new();

    observer.emit_memory_signal(MemorySignal::ConsolidationStarted);
    observer.emit_memory_signal(MemorySignal::ConsolidationCompleted);
    observer.emit_memory_signal(MemorySignal::ConflictDetected);
    observer.emit_memory_signal(MemorySignal::ConflictResolved);
    observer.emit_memory_signal(MemorySignal::RevocationApplied);
    observer.emit_memory_signal(MemorySignal::GovernanceInspect);
    observer.emit_memory_signal(MemorySignal::GovernanceExport);
    observer.emit_memory_signal(MemorySignal::GovernanceDelete);

    let memory_counts = observer.snapshot_memory_counts();
    assert_eq!(memory_counts.total, 8);
    assert_eq!(memory_counts.consolidation_started, 1);
    assert_eq!(memory_counts.consolidation_completed, 1);
    assert_eq!(memory_counts.conflict_detected, 1);
    assert_eq!(memory_counts.conflict_resolved, 1);
    assert_eq!(memory_counts.revocation_applied, 1);
    assert_eq!(memory_counts.governance_inspect, 1);
    assert_eq!(memory_counts.governance_export, 1);
    assert_eq!(memory_counts.governance_delete, 1);
}
