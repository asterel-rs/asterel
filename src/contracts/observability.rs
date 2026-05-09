//! `Observer` trait, `ObserverEvent` and `ObserverMetric` enums, and
//! lifecycle signal types for the observability abstraction layer.

use std::time::Duration;

use super::ids::{EntityId, PersonId, SessionId};

/// Events the observer can record
#[derive(Debug, Clone)]
pub enum ObserverEvent {
    /// An agent turn has started with the given provider and model.
    AgentStart { provider: String, model: String },
    /// An agent turn has ended.
    AgentEnd {
        duration: Duration,
        tokens_used: Option<u64>,
    },
    /// A tool was invoked during an agent turn.
    ToolCall {
        tool: String,
        duration: Duration,
        success: bool,
    },
    /// A message was sent or received on a transport channel.
    ChannelMessage { channel: String, direction: String },
    /// A companion policy rail was active for a turn phase.
    CompanionPolicyRail {
        entity_id: EntityId,
        person_id: PersonId,
        session_id: Option<SessionId>,
        phase: String,
        enforcement: String,
        reason_code: String,
    },
    /// A companion turn emitted phase-level evidence.
    CompanionTurnEvidence {
        entity_id: EntityId,
        person_id: PersonId,
        session_id: Option<SessionId>,
        phase: String,
        decision: String,
        reason_code: String,
        provenance: String,
        summary: String,
    },
    /// Periodic heartbeat tick from the runtime supervisor.
    HeartbeatTick,
    /// An error occurred in a runtime component.
    Error { component: String, message: String },
}

/// Numeric metrics
#[derive(Debug, Clone)]
pub enum ObserverMetric {
    /// End-to-end latency of a single request.
    RequestLatency(Duration),
    /// Total tokens consumed in a request.
    TokensUsed(u64),
    /// Number of currently active sessions.
    ActiveSessions(u64),
    /// Depth of the pending work queue.
    QueueDepth(u64),
    /// Entity-level KPI score on a specific axis.
    EntityKpiScore {
        axis: EntityKpiAxis,
        score: f64,
        sample_size: u64,
        source: String,
    },
    /// Total signals ingested, labeled by source kind.
    SignalIngestTotal { source_kind: String },
    /// Total signals dropped by deduplication, labeled by source kind.
    SignalDedupDropTotal { source_kind: String },
    /// Cumulative count of belief promotions.
    BeliefPromotionTotal { count: u64 },
    /// Cumulative count of contradiction marks.
    ContradictionMarkTotal { count: u64 },
    /// Cumulative count of stale trend purges.
    StaleTrendPurgeTotal { count: u64 },
    /// Point-in-time snapshot of signals per tier.
    SignalTierSnapshot { tier: String, count: u64 },
    /// Point-in-time snapshot of signals per promotion status.
    PromotionStatusSnapshot { status: String, count: u64 },
    /// A memory SLO violation was detected.
    MemorySloViolation,
    /// Memory correction loop triggered a demotion pass.
    MemoryCorrectionTriggered { demoted_count: u64 },
    /// An autonomy subsystem lifecycle signal was emitted.
    AutonomyLifecycle(AutonomySignal),
    /// A memory subsystem lifecycle signal was emitted.
    MemoryLifecycle(MemorySignal),
    /// A post-turn background hook completed, skipped, or failed.
    PostTurnHook {
        /// Stable hook label (for example `relationship_update`).
        hook: String,
        /// Stable status label (`success`, `failure`, `skipped`, or `rejected`).
        status: String,
    },
}

/// Axes for entity-level key performance indicators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityKpiAxis {
    /// Consistency of the persona's identity over time.
    IdentityContinuity,
    /// Reliability of trust signals and commitments.
    TrustReliability,
    /// Coherence of relational context across sessions.
    RelationalCoherence,
    /// Consistency of aesthetic and preference signals.
    TasteConsistency,
}

impl EntityKpiAxis {
    /// Return the `snake_case` string representation of this axis.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IdentityContinuity => "identity_continuity",
            Self::TrustReliability => "trust_reliability",
            Self::RelationalCoherence => "relational_coherence",
            Self::TasteConsistency => "taste_consistency",
        }
    }
}

/// Lifecycle signals emitted by the autonomy subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutonomySignal {
    /// A new signal was ingested into the autonomy pipeline.
    Ingested,
    /// A duplicate signal was dropped.
    Deduplicated,
    /// A signal was promoted to a higher tier.
    Promoted,
    /// A contradiction between signals was detected.
    ContradictionDetected,
    /// The autonomy mode changed (e.g. supervised to full).
    ModeTransition,
    /// A new autonomous intent was created.
    IntentCreated,
    /// An intent was allowed by the security policy.
    IntentPolicyAllowed,
    /// An intent was denied by the security policy.
    IntentPolicyDenied,
    /// An intent was dispatched for execution.
    IntentDispatched,
    /// An intent execution was blocked at runtime.
    IntentExecutionBlocked,
}

/// Lifecycle signals emitted by the memory subsystem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemorySignal {
    /// A memory consolidation cycle started.
    ConsolidationStarted,
    /// A memory consolidation cycle completed.
    ConsolidationCompleted,
    /// A conflict between memory entries was detected.
    ConflictDetected,
    /// A memory conflict was resolved.
    ConflictResolved,
    /// A memory revocation was applied.
    RevocationApplied,
    /// A governance inspection was performed.
    GovernanceInspect,
    /// Memory data was exported for governance.
    GovernanceExport,
    /// Memory data was deleted for governance.
    GovernanceDelete,
}

/// Core observability trait — implement for any backend
pub trait Observer: Send + Sync {
    /// Record a discrete event
    fn record_event(&self, event: &ObserverEvent);

    /// Record a numeric metric
    fn record_metric(&self, metric: &ObserverMetric);

    /// Record an autonomy subsystem lifecycle signal as a metric.
    fn emit_autonomy_signal(&self, signal: AutonomySignal) {
        self.record_metric(&ObserverMetric::AutonomyLifecycle(signal));
    }

    /// Record a memory subsystem lifecycle signal as a metric.
    fn emit_memory_signal(&self, signal: MemorySignal) {
        self.record_metric(&ObserverMetric::MemoryLifecycle(signal));
    }

    /// Flush any buffered data (no-op for most backends)
    fn flush(&self) {}

    /// Human-readable name of this observer
    fn name(&self) -> &str;
}

/// Zero-overhead observer — all methods compile to nothing.
///
/// Defined here in `contracts` so that lower-layer modules (L1 memory
/// ingestion, L3 agent loop) can use it without importing from `runtime`.
pub struct NoopObserver;

impl Observer for NoopObserver {
    #[inline(always)]
    fn record_event(&self, _event: &ObserverEvent) {}

    #[inline(always)]
    fn record_metric(&self, _metric: &ObserverMetric) {}

    fn name(&self) -> &'static str {
        "noop"
    }
}
