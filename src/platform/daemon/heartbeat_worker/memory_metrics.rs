//! Memory and persona health metrics for the heartbeat worker.
//!
//! Evaluates contradiction ratios, belief promotion stats,
//! stale trend purging, persona calibration, and drift SLOs
//! against the `PostgreSQL` memory backend on each heartbeat tick.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use sqlx_core::pool::{Pool, PoolOptions};
use sqlx_core::query::query;
use sqlx_core::row::Row;
use sqlx_postgres::Postgres;

use crate::contracts::strings::data_model::PERSONA_ROLLBACK_LATEST_SLOT_GLOB;
use crate::core::persona::drift_detector::{DriftSeverity, assess_persona_drift, classify_drift};
use crate::core::persona::metacognition::{CALIBRATION_SNAPSHOT_SLOT_KEY, CalibrationGateStatus};
use crate::core::persona::{
    metacognition::CalibrationSnapshot, state_persistence::PersonaTransition,
};
use crate::runtime::observability::traits::{AutonomySignal, EntityKpiAxis, ObserverMetric};

const CONTRADICTION_RATIO_SLO_MAX: f64 = 0.20;

#[derive(Debug, Clone, Copy)]
struct RetrievalUnitStats {
    total: u64,
    contradicted: u64,
    promoted: u64,
    candidate: u64,
    demoted: u64,
}

#[derive(Debug, Clone)]
struct EntityKpiSnapshot {
    axis: EntityKpiAxis,
    score: f64,
    sample_size: u64,
    source: String,
}

#[derive(Debug, Clone, Copy)]
struct PersonaDriftSnapshot {
    avg_continuity_score: f64,
    min_continuity_score: f64,
    evaluated_records: u64,
    warning_records: u64,
    critical_records: u64,
}

/// Executes one memory hygiene cycle: contradiction ratio, SLO
/// evaluation, persona drift, and calibration checks.
pub(super) async fn run_memory_hygiene_tick(
    config: &crate::config::Config,
    observer: &Arc<dyn crate::runtime::observability::Observer>,
) {
    if !config.memory.hygiene_enabled {
        return;
    }

    match crate::core::memory::hygiene::run_if_due(&config.memory, &config.workspace_dir) {
        Ok(()) => {
            crate::runtime::diagnostics::health::mark_component_ok("memory_hygiene");
        }
        Err(error) => {
            crate::runtime::diagnostics::health::mark_component_error(
                "memory_hygiene",
                error.to_string(),
            );
            tracing::warn!(%error, "memory hygiene tick failed");
        }
    }

    match crate::core::sessions::cleanup::reap_stale_sessions(&config.workspace_dir, &config.memory)
    {
        Ok(report) => {
            if report.archived_count > 0 || report.purged_sessions > 0 {
                tracing::info!(
                    archived = report.archived_count,
                    purged_sessions = report.purged_sessions,
                    purged_messages = report.purged_messages,
                    "session cleanup completed"
                );
            }
        }
        Err(error) => tracing::warn!(%error, "session cleanup failed"),
    }

    let Some(pool) = open_memory_pool(config).await.ok() else {
        tracing::debug!("memory metrics skipped: postgres pool unavailable");
        return;
    };

    if let Ok(Some(total)) = contradiction_mark_total(&pool).await {
        observer.record_metric(&ObserverMetric::ContradictionMarkTotal { count: total });
        tracing::info!(
            contradiction_mark_total = total,
            "memory contradiction metric snapshot"
        );
    }

    if let Ok(Some(total)) = belief_promotion_total(&pool).await {
        observer.record_metric(&ObserverMetric::BeliefPromotionTotal { count: total });
        tracing::info!(
            belief_promotion_total = total,
            "memory promotion metric snapshot"
        );
    }

    record_signal_distribution_snapshot(&pool, observer).await;

    evaluate_memory_slo(config, &pool, observer).await;

    if let Ok(Some(total)) = stale_trend_purge_total(&config.workspace_dir) {
        observer.record_metric(&ObserverMetric::StaleTrendPurgeTotal { count: total });
        tracing::info!(
            stale_trend_purge_total = total,
            "memory stale trend metric snapshot"
        );
    }

    evaluate_persona_drift(config, &pool, observer).await;
    evaluate_persona_calibration(config, &pool, observer).await;
    record_entity_kpi_snapshot(&pool, observer).await;
}

async fn open_memory_pool(config: &crate::config::Config) -> Result<Pool<Postgres>> {
    let database_url = crate::utils::postgres::require_postgres_url(
        config.memory.postgres_url.as_deref(),
        Some(&config.workspace_dir),
        "heartbeat memory metrics",
    )?;

    PoolOptions::<Postgres>::new()
        .max_connections(config.memory.pg_max_connections.max(1))
        .connect(&database_url)
        .await
        .context("connect postgres for heartbeat memory metrics")
}

async fn record_signal_distribution_snapshot(
    pool: &Pool<Postgres>,
    observer: &Arc<dyn crate::runtime::observability::Observer>,
) {
    if let Ok(rows) =
        query("SELECT signal_tier, COUNT(*) AS count FROM retrieval_units GROUP BY signal_tier")
            .fetch_all(pool)
            .await
    {
        for row in rows {
            let tier: String = row.get("signal_tier");
            let count: i64 = row.get("count");
            observer.record_metric(&ObserverMetric::SignalTierSnapshot {
                tier,
                count: u64::try_from(count).unwrap_or(0),
            });
        }
    }

    if let Ok(rows) = query(
        "SELECT promotion_status, COUNT(*) AS count FROM retrieval_units GROUP BY promotion_status",
    )
    .fetch_all(pool)
    .await
    {
        for row in rows {
            let status: String = row.get("promotion_status");
            let count: i64 = row.get("count");
            observer.record_metric(&ObserverMetric::PromotionStatusSnapshot {
                status,
                count: u64::try_from(count).unwrap_or(0),
            });
        }
    }
}

/// Checks the contradiction ratio against the SLO threshold and
/// reports violations to the observer.
async fn evaluate_memory_slo(
    config: &crate::config::Config,
    pool: &Pool<Postgres>,
    observer: &Arc<dyn crate::runtime::observability::Observer>,
) {
    let Ok(Some(ratio)) = contradiction_ratio(pool).await else {
        return;
    };

    if ratio > CONTRADICTION_RATIO_SLO_MAX {
        let message = format!(
            "contradiction_ratio_slo_violation ratio={ratio:.3} threshold={CONTRADICTION_RATIO_SLO_MAX:.3}"
        );
        observer.record_metric(&ObserverMetric::MemorySloViolation);
        crate::runtime::diagnostics::health::mark_component_error("memory_slo", message.clone());
        tracing::warn!(contradiction_ratio = ratio, "{message}");
    } else {
        let _ = config;
        crate::runtime::diagnostics::health::mark_component_ok("memory_slo");
    }
}

/// Assesses persona drift severity and emits lifecycle signals
/// when drift exceeds acceptable thresholds.
async fn evaluate_persona_drift(
    config: &crate::config::Config,
    pool: &Pool<Postgres>,
    observer: &Arc<dyn crate::runtime::observability::Observer>,
) {
    if !config.persona.enable_drift_detection_loop {
        crate::runtime::diagnostics::health::mark_component_ok("persona_drift");
        return;
    }

    let Some(snapshot) = latest_persona_drift_snapshot(config, pool).await else {
        crate::runtime::diagnostics::health::mark_component_ok("persona_drift");
        return;
    };

    observer.record_metric(&ObserverMetric::EntityKpiScore {
        axis: EntityKpiAxis::IdentityContinuity,
        score: snapshot.avg_continuity_score,
        sample_size: snapshot.evaluated_records,
        source: "persona_transition_records.drift_detector".to_string(),
    });

    if snapshot.critical_records > 0 {
        let message = format!(
            "persona_drift_critical min_score={:.3} avg_score={:.3} critical_count={} threshold={:.3}",
            snapshot.min_continuity_score,
            snapshot.avg_continuity_score,
            snapshot.critical_records,
            config.persona.drift_critical_threshold
        );
        crate::runtime::diagnostics::health::mark_component_error("persona_drift", message.clone());
        observer.emit_autonomy_signal(AutonomySignal::ContradictionDetected);
        observer.emit_autonomy_signal(AutonomySignal::IntentCreated);
        tracing::warn!(
            min_score = snapshot.min_continuity_score,
            avg_score = snapshot.avg_continuity_score,
            critical_count = snapshot.critical_records,
            "{message}"
        );
        return;
    }

    if snapshot.warning_records > 0 {
        tracing::warn!(
            min_score = snapshot.min_continuity_score,
            avg_score = snapshot.avg_continuity_score,
            warning_count = snapshot.warning_records,
            "persona_drift_warning continuity score below warning threshold"
        );
    }
    crate::runtime::diagnostics::health::mark_component_ok("persona_drift");
}

/// Evaluates the persona metacognition calibration snapshot and
/// reports KPI scores to the observer.
async fn evaluate_persona_calibration(
    config: &crate::config::Config,
    pool: &Pool<Postgres>,
    observer: &Arc<dyn crate::runtime::observability::Observer>,
) {
    if !config.persona.enable_metacognitive_logging {
        crate::runtime::diagnostics::health::mark_component_ok("persona_calibration");
        return;
    }

    let Some(snapshot) = latest_persona_calibration_snapshot(pool).await else {
        crate::runtime::diagnostics::health::mark_component_ok("persona_calibration");
        return;
    };

    let min_samples = config.persona.calibration_gate_min_samples.max(1);
    let thresholds_exceeded = snapshot.sample_count >= min_samples
        && (snapshot.mean_error > config.persona.calibration_gate_mean_error_max
            || snapshot.p95_error > config.persona.calibration_gate_p95_error_max);
    let status_blocked = snapshot.gate_status == CalibrationGateStatus::Blocked;

    if status_blocked || thresholds_exceeded {
        let message = format!(
            "persona_calibration_blocked status={} samples={} mean_error={:.3} p95_error={:.3} thresholds=({:.3},{:.3})",
            snapshot.gate_status,
            snapshot.sample_count,
            snapshot.mean_error,
            snapshot.p95_error,
            config.persona.calibration_gate_mean_error_max,
            config.persona.calibration_gate_p95_error_max
        );
        crate::runtime::diagnostics::health::mark_component_error(
            "persona_calibration",
            message.clone(),
        );
        observer.emit_autonomy_signal(AutonomySignal::IntentExecutionBlocked);
        tracing::warn!(
            sample_count = snapshot.sample_count,
            mean_error = snapshot.mean_error,
            p95_error = snapshot.p95_error,
            status = %snapshot.gate_status,
            "{message}"
        );
        return;
    }

    crate::runtime::diagnostics::health::mark_component_ok("persona_calibration");
}

async fn latest_persona_calibration_snapshot(pool: &Pool<Postgres>) -> Option<CalibrationSnapshot> {
    let raw = query(
        "SELECT value FROM belief_slots WHERE slot_key = $1 ORDER BY updated_at DESC LIMIT 1",
    )
    .bind(CALIBRATION_SNAPSHOT_SLOT_KEY)
    .fetch_optional(pool)
    .await
    .ok()?
    .map(|row| row.get::<String, _>("value"))?;

    serde_json::from_str::<CalibrationSnapshot>(&raw).ok()
}

async fn latest_persona_drift_snapshot(
    config: &crate::config::Config,
    pool: &Pool<Postgres>,
) -> Option<PersonaDriftSnapshot> {
    let rows =
        query("SELECT value FROM belief_slots WHERE slot_key LIKE $1 ORDER BY updated_at DESC")
            .bind(PERSONA_ROLLBACK_LATEST_SLOT_GLOB)
            .fetch_all(pool)
            .await
            .ok()?;

    let mut evaluated = 0_u64;
    let mut warning_records = 0_u64;
    let mut critical_records = 0_u64;
    let mut score_sum = 0.0_f64;
    let mut min_score = 1.0_f64;

    for row in rows {
        let raw: String = row.get("value");
        let Ok(record) = serde_json::from_str::<PersonaTransition>(&raw) else {
            continue;
        };

        let assessment = assess_persona_drift(&record.previous, &record.next);
        let severity = classify_drift(
            assessment.continuity_score,
            config.persona.drift_warning_threshold,
            config.persona.drift_critical_threshold,
        );

        evaluated = evaluated.saturating_add(1);
        score_sum += assessment.continuity_score;
        min_score = min_score.min(assessment.continuity_score);
        match severity {
            DriftSeverity::Stable => {}
            DriftSeverity::Warning => warning_records = warning_records.saturating_add(1),
            DriftSeverity::Critical => critical_records = critical_records.saturating_add(1),
        }
    }

    if evaluated == 0 {
        return None;
    }

    let evaluated_u32 = u32::try_from(evaluated).unwrap_or(u32::MAX).max(1);
    Some(PersonaDriftSnapshot {
        avg_continuity_score: score_sum / f64::from(evaluated_u32),
        min_continuity_score: min_score,
        evaluated_records: evaluated,
        warning_records,
        critical_records,
    })
}

async fn record_entity_kpi_snapshot(
    pool: &Pool<Postgres>,
    observer: &Arc<dyn crate::runtime::observability::Observer>,
) {
    let snapshots = collect_entity_kpi_snapshot(pool).await;
    let Ok(snapshots) = snapshots else {
        return;
    };

    for metric in snapshots {
        observer.record_metric(&ObserverMetric::EntityKpiScore {
            axis: metric.axis,
            score: metric.score,
            sample_size: metric.sample_size,
            source: metric.source.clone(),
        });
        tracing::info!(
            axis = %metric.axis.as_str(),
            score = metric.score,
            sample_size = metric.sample_size,
            source = %metric.source,
            "entity kpi snapshot"
        );
    }
}

async fn collect_entity_kpi_snapshot(pool: &Pool<Postgres>) -> Result<Vec<EntityKpiSnapshot>> {
    let mut snapshots = Vec::with_capacity(4);

    let retrieval_stats = retrieval_unit_stats(pool).await.ok();
    match retrieval_stats {
        Some(stats) if stats.total > 0 => {
            let contradiction_ratio = ratio(stats.contradicted, stats.total);
            let identity_score = (1.0 - contradiction_ratio).clamp(0.0, 1.0);

            let weighted_trust_numerator =
                bounded_count_to_f64(stats.promoted) + 0.5 * bounded_count_to_f64(stats.candidate);
            let trust_score = (weighted_trust_numerator / bounded_count_to_f64(stats.total.max(1)))
                .clamp(0.0, 1.0);

            let relational_penalty = ratio(stats.demoted, stats.total);
            let relational_score = (1.0 - relational_penalty).clamp(0.0, 1.0);

            snapshots.push(EntityKpiSnapshot {
                axis: EntityKpiAxis::IdentityContinuity,
                score: identity_score,
                sample_size: stats.total,
                source: "retrieval_units.contradiction_penalty".to_string(),
            });
            snapshots.push(EntityKpiSnapshot {
                axis: EntityKpiAxis::TrustReliability,
                score: trust_score,
                sample_size: stats.total,
                source: "retrieval_units.promotion_status_weighted".to_string(),
            });
            snapshots.push(EntityKpiSnapshot {
                axis: EntityKpiAxis::RelationalCoherence,
                score: relational_score,
                sample_size: stats.total,
                source: "retrieval_units.promotion_status_demoted".to_string(),
            });
        }
        Some(_) => {
            snapshots.push(EntityKpiSnapshot {
                axis: EntityKpiAxis::IdentityContinuity,
                score: 0.0,
                sample_size: 0,
                source: "retrieval_units.empty".to_string(),
            });
            snapshots.push(EntityKpiSnapshot {
                axis: EntityKpiAxis::TrustReliability,
                score: 0.0,
                sample_size: 0,
                source: "retrieval_units.empty".to_string(),
            });
            snapshots.push(EntityKpiSnapshot {
                axis: EntityKpiAxis::RelationalCoherence,
                score: 0.0,
                sample_size: 0,
                source: "retrieval_units.empty".to_string(),
            });
        }
        None => {
            snapshots.push(EntityKpiSnapshot {
                axis: EntityKpiAxis::IdentityContinuity,
                score: 0.0,
                sample_size: 0,
                source: "retrieval_units.unavailable".to_string(),
            });
            snapshots.push(EntityKpiSnapshot {
                axis: EntityKpiAxis::TrustReliability,
                score: 0.0,
                sample_size: 0,
                source: "retrieval_units.unavailable".to_string(),
            });
            snapshots.push(EntityKpiSnapshot {
                axis: EntityKpiAxis::RelationalCoherence,
                score: 0.0,
                sample_size: 0,
                source: "retrieval_units.unavailable".to_string(),
            });
        }
    }

    let taste_snapshot = taste_consistency_snapshot(pool).await.unwrap_or(None);
    match taste_snapshot {
        Some((score, sample_size)) => {
            snapshots.push(EntityKpiSnapshot {
                axis: EntityKpiAxis::TasteConsistency,
                score,
                sample_size,
                source: "taste_ratings.average_sigmoid".to_string(),
            });
        }
        None => {
            snapshots.push(EntityKpiSnapshot {
                axis: EntityKpiAxis::TasteConsistency,
                score: 0.0,
                sample_size: 0,
                source: "taste_ratings.unavailable".to_string(),
            });
        }
    }

    Ok(snapshots)
}

async fn retrieval_unit_stats(pool: &Pool<Postgres>) -> Result<RetrievalUnitStats> {
    let row = query(
        "SELECT
            COUNT(*) AS total,
            SUM(CASE WHEN contradiction_penalty > 0.0 THEN 1 ELSE 0 END) AS contradicted,
            SUM(CASE WHEN promotion_status = 'promoted' THEN 1 ELSE 0 END) AS promoted,
            SUM(CASE WHEN promotion_status = 'candidate' THEN 1 ELSE 0 END) AS candidate,
            SUM(CASE WHEN promotion_status = 'demoted' THEN 1 ELSE 0 END) AS demoted
         FROM retrieval_units",
    )
    .fetch_one(pool)
    .await?;

    let total: i64 = row.get("total");
    let contradicted: i64 = row.get("contradicted");
    let promoted: i64 = row.get("promoted");
    let candidate: i64 = row.get("candidate");
    let demoted: i64 = row.get("demoted");

    Ok(RetrievalUnitStats {
        total: u64::try_from(total).unwrap_or(0),
        contradicted: u64::try_from(contradicted).unwrap_or(0),
        promoted: u64::try_from(promoted).unwrap_or(0),
        candidate: u64::try_from(candidate).unwrap_or(0),
        demoted: u64::try_from(demoted).unwrap_or(0),
    })
}

fn ratio(numerator: u64, denominator: u64) -> f64 {
    if denominator == 0 {
        return 0.0;
    }
    (bounded_count_to_f64(numerator) / bounded_count_to_f64(denominator)).clamp(0.0, 1.0)
}

async fn taste_consistency_snapshot(pool: &Pool<Postgres>) -> Result<Option<(f64, u64)>> {
    let row = query("SELECT AVG(rating) AS avg_rating, COUNT(*) AS count FROM taste_ratings")
        .fetch_one(pool)
        .await?;

    let avg_rating: Option<f64> = row.get("avg_rating");
    let count: i64 = row.get("count");
    if count <= 0 {
        return Ok(None);
    }

    let sample_size = u64::try_from(count).unwrap_or(0);
    let score = sigmoid_unit(avg_rating.unwrap_or(0.0));
    Ok(Some((score, sample_size)))
}

fn sigmoid_unit(value: f64) -> f64 {
    let clamped = value.clamp(-35.0, 35.0);
    (1.0 / (1.0 + (-clamped).exp())).clamp(0.0, 1.0)
}

fn bounded_count_to_f64(value: u64) -> f64 {
    match u32::try_from(value) {
        Ok(value) => f64::from(value),
        Err(_) => f64::from(u32::MAX),
    }
}

/// Returns the total count of contradiction-marked memory events.
///
/// # Errors
///
/// Returns an error if the database query fails.
async fn contradiction_mark_total(pool: &Pool<Postgres>) -> Result<Option<u64>> {
    let count: i64 = query("SELECT COUNT(*) AS count FROM memory_events WHERE event_type = $1")
        .bind("contradiction_marked")
        .fetch_one(pool)
        .await?
        .get("count");
    Ok(Some(u64::try_from(count).unwrap_or(0)))
}

/// Computes the ratio of contradicted retrieval units to total.
///
/// # Errors
///
/// Returns an error if the database query fails.
async fn contradiction_ratio(pool: &Pool<Postgres>) -> Result<Option<f64>> {
    let total: i64 = query("SELECT COUNT(*) AS count FROM retrieval_units")
        .fetch_one(pool)
        .await?
        .get("count");

    if total <= 0 {
        return Ok(Some(0.0));
    }

    let contradicted: i64 =
        query("SELECT COUNT(*) AS count FROM retrieval_units WHERE contradiction_penalty > $1")
            .bind(0.0_f64)
            .fetch_one(pool)
            .await?
            .get("count");

    let total_u32 = u32::try_from(total).unwrap_or(u32::MAX).max(1);
    let contradicted_u32 = u32::try_from(contradicted).unwrap_or(u32::MAX);

    Ok(Some(
        f64::from(contradicted_u32) / f64::from(total_u32.max(1)),
    ))
}

/// Returns the count of candidate or promoted retrieval units.
///
/// # Errors
///
/// Returns an error if the database query fails.
async fn belief_promotion_total(pool: &Pool<Postgres>) -> Result<Option<u64>> {
    let count: i64 = query(
        "SELECT COUNT(*) AS count
         FROM retrieval_units
         WHERE promotion_status IN ('candidate', 'promoted')",
    )
    .fetch_one(pool)
    .await?
    .get("count");
    Ok(Some(u64::try_from(count).unwrap_or(0)))
}

/// Returns the count of stale trends purged from the state file.
///
/// # Errors
///
/// Returns an error if reading the state file fails.
pub(super) fn stale_trend_purge_total(workspace_dir: &Path) -> Result<Option<u64>> {
    let state_path = workspace_dir
        .join("state")
        .join("memory_hygiene_state.json");
    if !state_path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(state_path)?;
    let json: serde_json::Value = serde_json::from_str(&raw)?;
    let total = json
        .get("last_report")
        .and_then(|v| v.get("lifecycle"))
        .and_then(|v| v.get("stale_trend_demoted"))
        .and_then(serde_json::Value::as_u64);
    Ok(total)
}
