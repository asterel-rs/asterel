//! Prometheus metrics observer: exposes counters, gauges, and
//! histograms for events, lifecycle signals, and entity KPIs.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use super::traits::{
    AutonomySignal, EntityKpiAxis, MemorySignal, Observer, ObserverEvent, ObserverMetric,
};

/// Prometheus metrics observer: maintains atomic counters, gauges, and
/// label-keyed maps for events, lifecycle signals, and entity KPIs.
pub struct PrometheusObserver {
    event_count: AtomicU64,
    metric_count: AtomicU64,
    error_count: AtomicU64,
    autonomy_lifecycle_total: AtomicU64,
    autonomy_ingested_count: AtomicU64,
    autonomy_deduplicated_count: AtomicU64,
    autonomy_promoted_count: AtomicU64,
    autonomy_contradiction_count: AtomicU64,
    autonomy_mode_transition_count: AtomicU64,
    autonomy_intent_created_count: AtomicU64,
    autonomy_intent_policy_allowed_count: AtomicU64,
    autonomy_intent_policy_denied_count: AtomicU64,
    autonomy_intent_dispatched_count: AtomicU64,
    autonomy_intent_execution_blocked_count: AtomicU64,
    memory_lifecycle_total: AtomicU64,
    memory_consolidation_started_count: AtomicU64,
    memory_consolidation_completed_count: AtomicU64,
    memory_conflict_detected_count: AtomicU64,
    memory_conflict_resolved_count: AtomicU64,
    memory_revocation_applied_count: AtomicU64,
    memory_governance_inspect_count: AtomicU64,
    memory_governance_export_count: AtomicU64,
    memory_governance_delete_count: AtomicU64,
    memory_slo_violation_count: AtomicU64,
    signal_ingest_by_source: Mutex<HashMap<String, u64>>,
    signal_dedup_drop_by_source: Mutex<HashMap<String, u64>>,
    belief_promotion_total_snapshot: AtomicU64,
    contradiction_mark_total_snapshot: AtomicU64,
    stale_trend_purge_total_snapshot: AtomicU64,
    signal_tier_snapshot: Mutex<HashMap<String, u64>>,
    promotion_status_snapshot: Mutex<HashMap<String, u64>>,
    entity_kpi_score_snapshot: Mutex<HashMap<String, f64>>,
    entity_kpi_sample_size_snapshot: Mutex<HashMap<String, u64>>,
    entity_kpi_source_snapshot: Mutex<HashMap<String, String>>,
    post_turn_hook_by_status: Mutex<HashMap<String, u64>>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutonomyMetricCounts {
    pub total: u64,
    pub ingested: u64,
    pub deduplicated: u64,
    pub promoted: u64,
    pub contradiction_detected: u64,
    pub mode_transition: u64,
    pub intent_created: u64,
    pub intent_policy_allowed: u64,
    pub intent_policy_denied: u64,
    pub intent_dispatched: u64,
    pub intent_execution_blocked: u64,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryMetricCounts {
    pub total: u64,
    pub consolidation_started: u64,
    pub consolidation_completed: u64,
    pub conflict_detected: u64,
    pub conflict_resolved: u64,
    pub revocation_applied: u64,
    pub governance_inspect: u64,
    pub governance_export: u64,
    pub governance_delete: u64,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalMetricCounts {
    pub ingested_by_source: HashMap<String, u64>,
    pub dedup_drop_by_source: HashMap<String, u64>,
    pub belief_promotion_total_snapshot: u64,
    pub contradiction_mark_total_snapshot: u64,
    pub stale_trend_purge_total_snapshot: u64,
    pub tier_snapshot: HashMap<String, u64>,
    pub promotion_status_snapshot: HashMap<String, u64>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub struct EntityKpiMetricCounts {
    pub score_by_axis: HashMap<String, f64>,
    pub sample_size_by_axis: HashMap<String, u64>,
    pub source_by_axis: HashMap<String, String>,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnHookMetricCounts {
    pub by_hook_status: HashMap<String, u64>,
}

impl PrometheusObserver {
    /// Create a new Prometheus observer with zeroed counters.
    #[must_use]
    pub fn new() -> Self {
        Self {
            event_count: AtomicU64::new(0),
            metric_count: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
            autonomy_lifecycle_total: AtomicU64::new(0),
            autonomy_ingested_count: AtomicU64::new(0),
            autonomy_deduplicated_count: AtomicU64::new(0),
            autonomy_promoted_count: AtomicU64::new(0),
            autonomy_contradiction_count: AtomicU64::new(0),
            autonomy_mode_transition_count: AtomicU64::new(0),
            autonomy_intent_created_count: AtomicU64::new(0),
            autonomy_intent_policy_allowed_count: AtomicU64::new(0),
            autonomy_intent_policy_denied_count: AtomicU64::new(0),
            autonomy_intent_dispatched_count: AtomicU64::new(0),
            autonomy_intent_execution_blocked_count: AtomicU64::new(0),
            memory_lifecycle_total: AtomicU64::new(0),
            memory_consolidation_started_count: AtomicU64::new(0),
            memory_consolidation_completed_count: AtomicU64::new(0),
            memory_conflict_detected_count: AtomicU64::new(0),
            memory_conflict_resolved_count: AtomicU64::new(0),
            memory_revocation_applied_count: AtomicU64::new(0),
            memory_governance_inspect_count: AtomicU64::new(0),
            memory_governance_export_count: AtomicU64::new(0),
            memory_governance_delete_count: AtomicU64::new(0),
            memory_slo_violation_count: AtomicU64::new(0),
            signal_ingest_by_source: Mutex::new(HashMap::new()),
            signal_dedup_drop_by_source: Mutex::new(HashMap::new()),
            belief_promotion_total_snapshot: AtomicU64::new(0),
            contradiction_mark_total_snapshot: AtomicU64::new(0),
            stale_trend_purge_total_snapshot: AtomicU64::new(0),
            signal_tier_snapshot: Mutex::new(HashMap::new()),
            promotion_status_snapshot: Mutex::new(HashMap::new()),
            entity_kpi_score_snapshot: Mutex::new(HashMap::new()),
            entity_kpi_sample_size_snapshot: Mutex::new(HashMap::new()),
            entity_kpi_source_snapshot: Mutex::new(HashMap::new()),
            post_turn_hook_by_status: Mutex::new(HashMap::new()),
        }
    }

    fn increment_post_turn_hook(&self, hook: &str, status: &str) {
        if let Ok(mut guard) = self.post_turn_hook_by_status.lock() {
            let key = format!("{hook}:{status}");
            let entry = guard.entry(key).or_insert(0);
            *entry = entry.saturating_add(1);
        }
    }

    fn increment_ingest_source(&self, source_kind: &str) {
        if let Ok(mut guard) = self.signal_ingest_by_source.lock() {
            let entry = guard.entry(source_kind.to_string()).or_insert(0);
            *entry = entry.saturating_add(1);
        }
    }

    fn increment_dedup_source(&self, source_kind: &str) {
        if let Ok(mut guard) = self.signal_dedup_drop_by_source.lock() {
            let entry = guard.entry(source_kind.to_string()).or_insert(0);
            *entry = entry.saturating_add(1);
        }
    }

    fn set_signal_tier_snapshot(&self, tier: &str, count: u64) {
        if let Ok(mut guard) = self.signal_tier_snapshot.lock() {
            guard.insert(tier.to_string(), count);
        }
    }

    fn set_promotion_status_snapshot(&self, status: &str, count: u64) {
        if let Ok(mut guard) = self.promotion_status_snapshot.lock() {
            guard.insert(status.to_string(), count);
        }
    }

    fn set_belief_promotion_total_snapshot(&self, count: u64) {
        self.belief_promotion_total_snapshot
            .store(count, Ordering::Relaxed);
    }

    fn set_contradiction_mark_total_snapshot(&self, count: u64) {
        self.contradiction_mark_total_snapshot
            .store(count, Ordering::Relaxed);
    }

    fn set_stale_trend_purge_total_snapshot(&self, count: u64) {
        self.stale_trend_purge_total_snapshot
            .store(count, Ordering::Relaxed);
    }

    fn set_entity_kpi_score(
        &self,
        axis: EntityKpiAxis,
        score: f64,
        sample_size: u64,
        source: &str,
    ) {
        if let Ok(mut guard) = self.entity_kpi_score_snapshot.lock() {
            guard.insert(axis.as_str().to_string(), score);
        }

        if let Ok(mut guard) = self.entity_kpi_sample_size_snapshot.lock() {
            guard.insert(axis.as_str().to_string(), sample_size);
        }

        if let Ok(mut guard) = self.entity_kpi_source_snapshot.lock() {
            guard.insert(axis.as_str().to_string(), source.to_string());
        }
    }

    /// Render a Prometheus text exposition snapshot.
    ///
    /// This method is side-effect free and is intended for whichever runtime
    /// surface owns the eventual `/metrics` scrape endpoint. It keeps metric
    /// naming and label encoding close to the observer state so HTTP wiring can
    /// remain a thin adapter.
    #[must_use]
    pub fn render_text(&self) -> String {
        let mut out = String::new();
        push_counter(
            &mut out,
            "asterel_observer_events_total",
            self.event_count.load(Ordering::Relaxed),
        );
        push_counter(
            &mut out,
            "asterel_observer_metrics_total",
            self.metric_count.load(Ordering::Relaxed),
        );
        push_counter(
            &mut out,
            "asterel_observer_errors_total",
            self.error_count.load(Ordering::Relaxed),
        );
        push_counter(
            &mut out,
            "asterel_autonomy_lifecycle_total",
            self.autonomy_lifecycle_total.load(Ordering::Relaxed),
        );
        push_counter(
            &mut out,
            "asterel_memory_lifecycle_total",
            self.memory_lifecycle_total.load(Ordering::Relaxed),
        );
        push_counter(
            &mut out,
            "asterel_memory_slo_violations_total",
            self.memory_slo_violation_count.load(Ordering::Relaxed),
        );

        push_labeled_counters(
            &mut out,
            "asterel_signal_ingest_total",
            "source_kind",
            self.signal_ingest_by_source
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
        );
        push_labeled_counters(
            &mut out,
            "asterel_signal_dedup_drop_total",
            "source_kind",
            self.signal_dedup_drop_by_source
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
        );
        push_labeled_counters(
            &mut out,
            "asterel_signal_tier_snapshot",
            "tier",
            self.signal_tier_snapshot
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
        );
        push_labeled_counters(
            &mut out,
            "asterel_promotion_status_snapshot",
            "status",
            self.promotion_status_snapshot
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
        );
        push_labeled_counters(
            &mut out,
            "asterel_post_turn_hook_total",
            "hook_status",
            self.post_turn_hook_by_status
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
        );

        out
    }

    fn record_autonomy_signal(&self, signal: AutonomySignal) {
        self.autonomy_lifecycle_total
            .fetch_add(1, Ordering::Relaxed);

        match signal {
            AutonomySignal::Ingested => {
                self.autonomy_ingested_count.fetch_add(1, Ordering::Relaxed);
            }
            AutonomySignal::Deduplicated => {
                self.autonomy_deduplicated_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            AutonomySignal::Promoted => {
                self.autonomy_promoted_count.fetch_add(1, Ordering::Relaxed);
            }
            AutonomySignal::ContradictionDetected => {
                self.autonomy_contradiction_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            AutonomySignal::ModeTransition => {
                self.autonomy_mode_transition_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            AutonomySignal::IntentCreated => {
                self.autonomy_intent_created_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            AutonomySignal::IntentPolicyAllowed => {
                self.autonomy_intent_policy_allowed_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            AutonomySignal::IntentPolicyDenied => {
                self.autonomy_intent_policy_denied_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            AutonomySignal::IntentDispatched => {
                self.autonomy_intent_dispatched_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            AutonomySignal::IntentExecutionBlocked => {
                self.autonomy_intent_execution_blocked_count
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    fn record_memory_signal(&self, signal: MemorySignal) {
        self.memory_lifecycle_total.fetch_add(1, Ordering::Relaxed);

        match signal {
            MemorySignal::ConsolidationStarted => {
                self.memory_consolidation_started_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            MemorySignal::ConsolidationCompleted => {
                self.memory_consolidation_completed_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            MemorySignal::ConflictDetected => {
                self.memory_conflict_detected_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            MemorySignal::ConflictResolved => {
                self.memory_conflict_resolved_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            MemorySignal::RevocationApplied => {
                self.memory_revocation_applied_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            MemorySignal::GovernanceInspect => {
                self.memory_governance_inspect_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            MemorySignal::GovernanceExport => {
                self.memory_governance_export_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            MemorySignal::GovernanceDelete => {
                self.memory_governance_delete_count
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    #[cfg(test)]
    fn snapshot_counts(&self) -> (u64, u64, u64) {
        (
            self.event_count.load(Ordering::Relaxed),
            self.metric_count.load(Ordering::Relaxed),
            self.error_count.load(Ordering::Relaxed),
        )
    }

    #[cfg(test)]
    pub fn snapshot_autonomy_counts(&self) -> AutonomyMetricCounts {
        AutonomyMetricCounts {
            total: self.autonomy_lifecycle_total.load(Ordering::Relaxed),
            ingested: self.autonomy_ingested_count.load(Ordering::Relaxed),
            deduplicated: self.autonomy_deduplicated_count.load(Ordering::Relaxed),
            promoted: self.autonomy_promoted_count.load(Ordering::Relaxed),
            contradiction_detected: self.autonomy_contradiction_count.load(Ordering::Relaxed),
            mode_transition: self.autonomy_mode_transition_count.load(Ordering::Relaxed),
            intent_created: self.autonomy_intent_created_count.load(Ordering::Relaxed),
            intent_policy_allowed: self
                .autonomy_intent_policy_allowed_count
                .load(Ordering::Relaxed),
            intent_policy_denied: self
                .autonomy_intent_policy_denied_count
                .load(Ordering::Relaxed),
            intent_dispatched: self
                .autonomy_intent_dispatched_count
                .load(Ordering::Relaxed),
            intent_execution_blocked: self
                .autonomy_intent_execution_blocked_count
                .load(Ordering::Relaxed),
        }
    }

    #[cfg(test)]
    pub fn snapshot_memory_counts(&self) -> MemoryMetricCounts {
        MemoryMetricCounts {
            total: self.memory_lifecycle_total.load(Ordering::Relaxed),
            consolidation_started: self
                .memory_consolidation_started_count
                .load(Ordering::Relaxed),
            consolidation_completed: self
                .memory_consolidation_completed_count
                .load(Ordering::Relaxed),
            conflict_detected: self.memory_conflict_detected_count.load(Ordering::Relaxed),
            conflict_resolved: self.memory_conflict_resolved_count.load(Ordering::Relaxed),
            revocation_applied: self.memory_revocation_applied_count.load(Ordering::Relaxed),
            governance_inspect: self.memory_governance_inspect_count.load(Ordering::Relaxed),
            governance_export: self.memory_governance_export_count.load(Ordering::Relaxed),
            governance_delete: self.memory_governance_delete_count.load(Ordering::Relaxed),
        }
    }

    #[cfg(test)]
    pub fn snapshot_signal_counts(&self) -> SignalMetricCounts {
        SignalMetricCounts {
            ingested_by_source: self
                .signal_ingest_by_source
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
            dedup_drop_by_source: self
                .signal_dedup_drop_by_source
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
            belief_promotion_total_snapshot: self
                .belief_promotion_total_snapshot
                .load(Ordering::Relaxed),
            contradiction_mark_total_snapshot: self
                .contradiction_mark_total_snapshot
                .load(Ordering::Relaxed),
            stale_trend_purge_total_snapshot: self
                .stale_trend_purge_total_snapshot
                .load(Ordering::Relaxed),
            tier_snapshot: self
                .signal_tier_snapshot
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
            promotion_status_snapshot: self
                .promotion_status_snapshot
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
        }
    }

    #[cfg(test)]
    pub fn snapshot_memory_slo_violation_count(&self) -> u64 {
        self.memory_slo_violation_count.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn snapshot_entity_kpi_counts(&self) -> EntityKpiMetricCounts {
        EntityKpiMetricCounts {
            score_by_axis: self
                .entity_kpi_score_snapshot
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
            sample_size_by_axis: self
                .entity_kpi_sample_size_snapshot
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
            source_by_axis: self
                .entity_kpi_source_snapshot
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
        }
    }

    #[cfg(test)]
    pub fn snapshot_post_turn_hook_counts(&self) -> PostTurnHookMetricCounts {
        PostTurnHookMetricCounts {
            by_hook_status: self
                .post_turn_hook_by_status
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_default(),
        }
    }
}

fn push_counter(out: &mut String, name: &str, value: u64) {
    let _ = writeln!(out, "# TYPE {name} counter");
    let _ = writeln!(out, "{name} {value}");
}

fn push_labeled_counters(out: &mut String, name: &str, label: &str, values: HashMap<String, u64>) {
    if values.is_empty() {
        return;
    }

    let _ = writeln!(out, "# TYPE {name} counter");
    let mut values: Vec<_> = values.into_iter().collect();
    values.sort_by(|lhs, rhs| lhs.0.cmp(&rhs.0));
    for (label_value, value) in values {
        let label_value = escape_label_value(&label_value);
        let _ = writeln!(out, "{name}{{{label}=\"{label_value}\"}} {value}");
    }
}

fn escape_label_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

impl Default for PrometheusObserver {
    fn default() -> Self {
        Self::new()
    }
}

impl Observer for PrometheusObserver {
    fn record_event(&self, event: &ObserverEvent) {
        self.event_count.fetch_add(1, Ordering::Relaxed);
        if matches!(event, ObserverEvent::Error { .. }) {
            self.error_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        self.metric_count.fetch_add(1, Ordering::Relaxed);
        match metric {
            ObserverMetric::SignalIngestTotal { source_kind } => {
                self.increment_ingest_source(source_kind);
            }
            ObserverMetric::SignalDedupDropTotal { source_kind } => {
                self.increment_dedup_source(source_kind);
            }
            ObserverMetric::EntityKpiScore {
                axis,
                score,
                sample_size,
                source,
            } => {
                self.set_entity_kpi_score(*axis, *score, *sample_size, source);
            }
            ObserverMetric::BeliefPromotionTotal { count } => {
                self.set_belief_promotion_total_snapshot(*count);
            }
            ObserverMetric::ContradictionMarkTotal { count } => {
                self.set_contradiction_mark_total_snapshot(*count);
            }
            ObserverMetric::StaleTrendPurgeTotal { count } => {
                self.set_stale_trend_purge_total_snapshot(*count);
            }
            ObserverMetric::SignalTierSnapshot { tier, count } => {
                self.set_signal_tier_snapshot(tier, *count);
            }
            ObserverMetric::PromotionStatusSnapshot { status, count } => {
                self.set_promotion_status_snapshot(status, *count);
            }
            ObserverMetric::MemorySloViolation => {
                self.memory_slo_violation_count
                    .fetch_add(1, Ordering::Relaxed);
            }
            ObserverMetric::AutonomyLifecycle(signal) => self.record_autonomy_signal(*signal),
            ObserverMetric::MemoryLifecycle(signal) => self.record_memory_signal(*signal),
            ObserverMetric::PostTurnHook { hook, status } => {
                self.increment_post_turn_hook(hook, status);
            }
            ObserverMetric::RequestLatency(_)
            | ObserverMetric::TokensUsed(_)
            | ObserverMetric::ActiveSessions(_)
            | ObserverMetric::MemoryCorrectionTriggered { .. }
            | ObserverMetric::QueueDepth(_) => {}
        }
    }

    fn flush(&self) {
        tracing::debug!(
            events_total = self.event_count.load(Ordering::Relaxed),
            metrics_total = self.metric_count.load(Ordering::Relaxed),
            errors_total = self.error_count.load(Ordering::Relaxed),
            autonomy_total = self.autonomy_lifecycle_total.load(Ordering::Relaxed),
            memory_total = self.memory_lifecycle_total.load(Ordering::Relaxed),
            "observer.prometheus.flush"
        );
    }

    fn name(&self) -> &'static str {
        "prometheus"
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn prometheus_observer_name() {
        assert_eq!(PrometheusObserver::new().name(), "prometheus");
    }

    #[test]
    fn prometheus_observer_smoke_and_counts() {
        let obs = PrometheusObserver::new();

        obs.record_event(&ObserverEvent::HeartbeatTick);
        obs.record_event(&ObserverEvent::Error {
            component: "health".into(),
            message: "degraded".into(),
        });
        obs.record_metric(&ObserverMetric::AutonomyLifecycle(
            AutonomySignal::IntentCreated,
        ));
        obs.record_metric(&ObserverMetric::MemoryLifecycle(
            MemorySignal::GovernanceDelete,
        ));
        obs.record_metric(&ObserverMetric::SignalIngestTotal {
            source_kind: "api".to_string(),
        });
        obs.record_metric(&ObserverMetric::SignalDedupDropTotal {
            source_kind: "api".to_string(),
        });
        obs.record_metric(&ObserverMetric::BeliefPromotionTotal { count: 2 });
        obs.record_metric(&ObserverMetric::ContradictionMarkTotal { count: 1 });
        obs.record_metric(&ObserverMetric::StaleTrendPurgeTotal { count: 4 });
        obs.record_metric(&ObserverMetric::SignalTierSnapshot {
            tier: "raw".to_string(),
            count: 3,
        });
        obs.record_metric(&ObserverMetric::PromotionStatusSnapshot {
            status: "promoted".to_string(),
            count: 2,
        });
        obs.record_metric(&ObserverMetric::EntityKpiScore {
            axis: EntityKpiAxis::TrustReliability,
            score: 0.75,
            sample_size: 12,
            source: "retrieval_units.promotion_status_weighted".to_string(),
        });
        obs.record_metric(&ObserverMetric::MemorySloViolation);
        obs.record_metric(&ObserverMetric::MemoryCorrectionTriggered { demoted_count: 1 });
        obs.record_metric(&ObserverMetric::PostTurnHook {
            hook: "working_memory_flush".to_string(),
            status: "failure".to_string(),
        });
        obs.record_metric(&ObserverMetric::RequestLatency(Duration::from_millis(10)));
        obs.flush();

        assert_eq!(obs.snapshot_counts(), (2, 14, 1));
        let autonomy = obs.snapshot_autonomy_counts();
        assert_eq!(autonomy.total, 1);
        assert_eq!(autonomy.intent_created, 1);
        let memory = obs.snapshot_memory_counts();
        assert_eq!(memory.total, 1);
        assert_eq!(memory.governance_delete, 1);

        let signal = obs.snapshot_signal_counts();
        assert_eq!(signal.ingested_by_source.get("api"), Some(&1));
        assert_eq!(signal.dedup_drop_by_source.get("api"), Some(&1));
        assert_eq!(signal.belief_promotion_total_snapshot, 2);
        assert_eq!(signal.contradiction_mark_total_snapshot, 1);
        assert_eq!(signal.stale_trend_purge_total_snapshot, 4);
        assert_eq!(signal.tier_snapshot.get("raw"), Some(&3));
        assert_eq!(signal.promotion_status_snapshot.get("promoted"), Some(&2));
        assert_eq!(obs.snapshot_memory_slo_violation_count(), 1);
        assert_eq!(
            obs.snapshot_post_turn_hook_counts()
                .by_hook_status
                .get("working_memory_flush:failure"),
            Some(&1)
        );

        let entity = obs.snapshot_entity_kpi_counts();
        assert_eq!(entity.score_by_axis.get("trust_reliability"), Some(&0.75));
        assert_eq!(
            entity.sample_size_by_axis.get("trust_reliability"),
            Some(&12)
        );
        assert_eq!(
            entity.source_by_axis.get("trust_reliability"),
            Some(&"retrieval_units.promotion_status_weighted".to_string())
        );
    }

    #[test]
    fn prometheus_signal_snapshots_overwrite_per_label() {
        let obs = PrometheusObserver::new();

        obs.record_metric(&ObserverMetric::SignalTierSnapshot {
            tier: "raw".to_string(),
            count: 2,
        });
        obs.record_metric(&ObserverMetric::SignalTierSnapshot {
            tier: "raw".to_string(),
            count: 5,
        });
        obs.record_metric(&ObserverMetric::PromotionStatusSnapshot {
            status: "candidate".to_string(),
            count: 1,
        });
        obs.record_metric(&ObserverMetric::PromotionStatusSnapshot {
            status: "candidate".to_string(),
            count: 4,
        });

        let signal = obs.snapshot_signal_counts();
        assert_eq!(signal.tier_snapshot.get("raw"), Some(&5));
        assert_eq!(signal.promotion_status_snapshot.get("candidate"), Some(&4));
    }

    #[test]
    fn prometheus_entity_kpi_snapshots_overwrite_per_axis() {
        let obs = PrometheusObserver::new();

        obs.record_metric(&ObserverMetric::EntityKpiScore {
            axis: EntityKpiAxis::IdentityContinuity,
            score: 0.4,
            sample_size: 3,
            source: "first".to_string(),
        });
        obs.record_metric(&ObserverMetric::EntityKpiScore {
            axis: EntityKpiAxis::IdentityContinuity,
            score: 0.9,
            sample_size: 10,
            source: "second".to_string(),
        });

        let entity = obs.snapshot_entity_kpi_counts();
        assert_eq!(entity.score_by_axis.get("identity_continuity"), Some(&0.9));
        assert_eq!(
            entity.sample_size_by_axis.get("identity_continuity"),
            Some(&10)
        );
        assert_eq!(
            entity.source_by_axis.get("identity_continuity"),
            Some(&"second".to_string())
        );
    }

    #[test]
    fn prometheus_render_text_exposes_scrapeable_snapshot() {
        let obs = PrometheusObserver::new();
        obs.record_event(&ObserverEvent::HeartbeatTick);
        obs.record_metric(&ObserverMetric::SignalIngestTotal {
            source_kind: "gateway\"edge".to_string(),
        });
        obs.record_metric(&ObserverMetric::PostTurnHook {
            hook: "memory_flush".to_string(),
            status: "ok".to_string(),
        });

        let text = obs.render_text();

        assert!(text.contains("# TYPE asterel_observer_events_total counter"));
        assert!(text.contains("asterel_observer_events_total 1"));
        assert!(text.contains("asterel_signal_ingest_total{source_kind=\"gateway\\\"edge\"} 1"));
        assert!(text.contains("asterel_post_turn_hook_total{hook_status=\"memory_flush:ok\"} 1"));
    }
}
