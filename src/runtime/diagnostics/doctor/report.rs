//! Doctor report generators: autonomy governance, memory rollout,
//! signal stats, persona drift, calibration, continuity gate,
//! embodied-state diagnostic lines.

use chrono::{DateTime, Utc};
use sqlx_core::pool::{Pool, PoolOptions};
use sqlx_core::query::query;
use sqlx_core::row::Row;
use sqlx_postgres::Postgres;

use crate::config::Config;
use crate::contracts::strings::data_model::PERSONA_ROLLBACK_LATEST_SLOT_GLOB;
use crate::core::memory::CapabilitySupport;
use crate::core::persona::continuity_gate::{ROLLBACK_DRILL_SLOT_KEY, RollbackDrillResult};
use crate::core::persona::drift_detector::{DriftSeverity, assess_persona_drift, classify_drift};
use crate::core::persona::embodied_state::{EMBODIED_STATE_SLOT_KEY, EmbodiedStateSnapshot};
use crate::core::persona::metacognition::{CALIBRATION_SNAPSHOT_SLOT_KEY, CalibrationSnapshot};
use crate::core::persona::state_persistence::PersonaTransition;
use crate::runtime::services::{RuntimeCapabilityState, load_runtime_operational_snapshot};
use crate::security::ExternalActionExecution;

struct MemorySignalStats {
    total_units: i64,
    raw_units: i64,
    demoted_units: i64,
    candidate_units: i64,
    promoted_units: i64,
    ttl_expired_units: i64,
    contradicted_units: i64,
    source_kind_breakdown: String,
}

/// Build diagnostic lines describing the current autonomy governance state.
pub(crate) fn autonomy_governance_lines(config: &Config) -> Vec<String> {
    let mut lines = Vec::with_capacity(6);

    lines.push(format!(
        "autonomy level: {:?}",
        config.autonomy.effective_autonomy_lvl()
    ));

    let external_actions = match config.autonomy.external_action_execution {
        ExternalActionExecution::Disabled => "disabled",
        ExternalActionExecution::Enabled => "enabled",
    };
    lines.push(format!("external actions: {external_actions}"));

    let selected_band = config.autonomy.selected_temp_band();
    lines.push(format!(
        "temperature band: [{:.2}, {:.2}]",
        selected_band.min, selected_band.max
    ));

    let rollout_stage = match config.autonomy.rollout.stage {
        Some(crate::config::schema::AutonomyRolloutStage::ReadOnly) => "read-only",
        Some(crate::config::schema::AutonomyRolloutStage::Supervised) => "supervised",
        Some(crate::config::schema::AutonomyRolloutStage::Full) => "full",
        None => "off",
    };
    lines.push(format!("rollout stage: {rollout_stage}"));
    lines.push(format!(
        "rollout policy: enabled={}, read_only_days={:?}, supervised_days={:?}",
        if config.autonomy.rollout.enabled {
            "on"
        } else {
            "off"
        },
        config.autonomy.rollout.read_only_days,
        config.autonomy.rollout.supervised_days
    ));
    lines.push(format!(
        "verify/repair caps: max_attempts={}, max_repair_depth={}",
        config.autonomy.verify_repair_max_attempts, config.autonomy.verify_repair_max_repair_depth
    ));

    let backend = config.observability.backend;
    let lifecycle_metrics = if backend.supports_lifecycle_metrics() {
        "enabled"
    } else {
        "disabled"
    };
    lines.push(format!(
        "observability backend: {backend} (autonomy lifecycle metrics: {lifecycle_metrics})"
    ));

    lines
}

/// Build diagnostic lines for memory backend rollout and lifecycle health.
pub(crate) fn memory_rollout_lines(config: &Config, snapshot: &serde_json::Value) -> Vec<String> {
    let mut lines = Vec::with_capacity(6);
    let backend = config.memory.backend;
    let capability = crate::core::memory::capability_matrix_for_backend(backend.as_str());

    let consolidation = if backend != crate::config::MemoryBackend::None && config.memory.auto_save
    {
        "on"
    } else {
        "off"
    };
    let conflict =
        if backend != crate::config::MemoryBackend::None && config.autonomy.rollout.enabled {
            "on"
        } else {
            "off"
        };
    let revocation = capability.map_or("unknown", |matrix| {
        capability_support_label(matrix.forget_tombstone)
    });
    let governance = capability.map_or("unknown", |matrix| {
        capability_support_label(matrix.forget_hard)
    });

    lines.push(format!(
        "memory backend: {backend} (consolidation={consolidation}, conflict={conflict})"
    ));
    lines.push(format!(
        "lifecycle support: revocation={revocation}, governance={governance}"
    ));

    if let Some(rollout) = snapshot
        .get("memory_rollout")
        .and_then(serde_json::Value::as_object)
    {
        let consolidation_health = rollout
            .get("consolidation")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let conflict_health = rollout
            .get("conflict")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let revocation_health = rollout
            .get("revocation")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let governance_health = rollout
            .get("governance")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");

        lines.push(format!(
            "daemon lifecycle health: consolidation={consolidation_health}, conflict={conflict_health}, revocation={revocation_health}, governance={governance_health}"
        ));
    } else {
        lines.push(
            "daemon lifecycle health: missing config in state file; non-fatal, using static capability fallback".to_string(),
        );
        lines.push(
            "action: restart daemon after rollout update to surface consolidation/conflict/revocation/governance telemetry"
                .to_string(),
        );
    }

    if let Some(memory_slo_status) = snapshot
        .get("components")
        .and_then(serde_json::Value::as_object)
        .and_then(|components| components.get("memory_slo"))
        .and_then(serde_json::Value::as_object)
        .and_then(|status| status.get("status"))
        .and_then(serde_json::Value::as_str)
    {
        lines.push(format!("memory_slo component: {memory_slo_status}"));
    } else {
        lines.push("memory_slo component: missing".to_string());
    }

    lines
}

fn capability_support_label(support: CapabilitySupport) -> &'static str {
    match support {
        CapabilitySupport::Supported => "supported",
        CapabilitySupport::Degraded => "degraded",
        CapabilitySupport::Unsupported => "unsupported",
    }
}

/// Parse an RFC 3339 timestamp string into a UTC `DateTime`.
pub(crate) fn parse_rfc3339(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn with_memory_pool<T: Send>(
    config: &Config,
    f: impl FnOnce(&tokio::runtime::Runtime, &Pool<Postgres>) -> anyhow::Result<T> + Send,
) -> anyhow::Result<T> {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread {
            return tokio::task::block_in_place(|| with_memory_pool_inner(config, f));
        }

        return std::thread::scope(|scope| {
            let join = scope.spawn(move || with_memory_pool_inner(config, f));
            join.join().unwrap_or_else(|panic_payload| {
                let message = if let Some(message) = panic_payload.downcast_ref::<&str>() {
                    *message
                } else if let Some(message) = panic_payload.downcast_ref::<String>() {
                    message.as_str()
                } else {
                    "unknown panic"
                };
                Err(anyhow::anyhow!(
                    "doctor report worker thread panicked: {message}"
                ))
            })
        });
    }

    with_memory_pool_inner(config, f)
}

fn with_memory_pool_inner<T: Send>(
    config: &Config,
    f: impl FnOnce(&tokio::runtime::Runtime, &Pool<Postgres>) -> anyhow::Result<T> + Send,
) -> anyhow::Result<T> {
    let database_url = crate::utils::postgres::require_postgres_url(
        config.memory.postgres_url.as_deref(),
        Some(&config.workspace_dir),
        "doctor report",
    )?;
    let runtime = tokio::runtime::Runtime::new()?;
    let pool = crate::utils::postgres::block_on_sync(&runtime, async {
        let pool = PoolOptions::<Postgres>::new()
            .max_connections(config.memory.pg_max_connections.max(1))
            .connect(&database_url)
            .await?;
        Ok::<Pool<Postgres>, anyhow::Error>(pool)
    })?;
    f(&runtime, &pool)
}

fn load_slot_values_like(config: &Config, pattern: &str) -> anyhow::Result<Vec<String>> {
    with_memory_pool(config, |runtime, pool| {
        crate::utils::postgres::block_on_sync(runtime, async {
            let rows = query(
                "SELECT value FROM belief_slots WHERE slot_key LIKE $1 ORDER BY updated_at DESC",
            )
            .bind(pattern)
            .fetch_all(pool)
            .await?;
            Ok(rows
                .into_iter()
                .map(|row| row.get::<String, _>("value"))
                .collect())
        })
    })
}

fn load_latest_slot_value(config: &Config, slot_key: &str) -> anyhow::Result<Option<String>> {
    with_memory_pool(config, |runtime, pool| {
        crate::utils::postgres::block_on_sync(runtime, async {
            Ok(query(
                "SELECT value FROM belief_slots WHERE slot_key = $1 ORDER BY updated_at DESC LIMIT 1",
            )
            .bind(slot_key)
            .fetch_optional(pool)
            .await?
            .map(|row| row.get::<String, _>("value")))
        })
    })
}

async fn query_count(pool: &Pool<Postgres>, sql: &str) -> anyhow::Result<i64> {
    Ok(query(sql).fetch_one(pool).await?.get("count"))
}

async fn query_contradicted_units(pool: &Pool<Postgres>) -> anyhow::Result<i64> {
    Ok(
        query("SELECT COUNT(*) AS count FROM retrieval_units WHERE contradiction_penalty > $1")
            .bind(0.0_f64)
            .fetch_one(pool)
            .await?
            .get("count"),
    )
}

async fn query_source_kind_breakdown(pool: &Pool<Postgres>) -> anyhow::Result<String> {
    let source_rows = query(
        "SELECT source_kind, COUNT(*) AS count
         FROM retrieval_units
         GROUP BY source_kind",
    )
    .fetch_all(pool)
    .await?;

    let mut parts = source_rows
        .into_iter()
        .map(|row| {
            let kind = row
                .get::<Option<String>, _>("source_kind")
                .unwrap_or_else(|| "unknown".to_string());
            let count: i64 = row.get("count");
            format!("{kind}={count}")
        })
        .collect::<Vec<_>>();
    parts.sort();
    Ok(parts.join(","))
}

fn load_memory_signal_stats(config: &Config) -> anyhow::Result<MemorySignalStats> {
    with_memory_pool(config, |runtime, pool| {
        crate::utils::postgres::block_on_sync(runtime, async {
            Ok(MemorySignalStats {
                total_units: query_count(pool, "SELECT COUNT(*) AS count FROM retrieval_units")
                    .await?,
                raw_units: query_count(
                    pool,
                    "SELECT COUNT(*) AS count FROM retrieval_units WHERE signal_tier = 'raw'",
                )
                .await?,
                demoted_units: query_count(
                    pool,
                    "SELECT COUNT(*) AS count FROM retrieval_units WHERE promotion_status = 'demoted'",
                )
                .await?,
                candidate_units: query_count(
                    pool,
                    "SELECT COUNT(*) AS count FROM retrieval_units WHERE promotion_status = 'candidate'",
                )
                .await?,
                promoted_units: query_count(
                    pool,
                    "SELECT COUNT(*) AS count FROM retrieval_units WHERE promotion_status = 'promoted'",
                )
                .await?,
                ttl_expired_units: query_count(
                    pool,
                    "SELECT COUNT(*) AS count
                     FROM retrieval_units
                     WHERE retention_expires_at IS NOT NULL
                       AND retention_expires_at <= NOW()",
                )
                .await?,
                contradicted_units: query_contradicted_units(pool).await?,
                source_kind_breakdown: query_source_kind_breakdown(pool).await?,
            })
        })
    })
}

fn contradiction_ratio(stats: &MemorySignalStats) -> f64 {
    if stats.total_units <= 0 {
        return 0.0;
    }

    let total_u32 = u32::try_from(stats.total_units).unwrap_or(u32::MAX).max(1);
    let contradicted_u32 = u32::try_from(stats.contradicted_units).unwrap_or(u32::MAX);
    f64::from(contradicted_u32) / f64::from(total_u32)
}

fn unsupported_metric_lines(label: &str, capability: &RuntimeCapabilityState) -> Vec<String> {
    vec![format!(
        "{label}: {} ({})",
        capability.status.as_str(),
        capability
            .reason
            .as_deref()
            .unwrap_or("no additional detail")
    )]
}

fn awaiting_first_daemon_boot_lines(label: &str) -> Vec<String> {
    vec![format!(
        "{label}: awaiting first daemon boot to initialize runtime-backed state"
    )]
}

/// Build diagnostic lines summarizing persona drift detection results.
pub(crate) fn persona_drift_lines(config: &Config) -> Vec<String> {
    let operational = load_runtime_operational_snapshot(config);
    if !operational.persona_state_metrics.is_supported() {
        return unsupported_metric_lines("drift summary", &operational.persona_state_metrics);
    }

    let mut lines = Vec::with_capacity(4);
    lines.push(format!(
        "detector: {} (warning<={:.2}, critical<={:.2})",
        if config.persona.enable_drift_detection_loop {
            "enabled"
        } else {
            "disabled"
        },
        config.persona.drift_warning_threshold,
        config.persona.drift_critical_threshold
    ));

    if !config.persona.enable_drift_detection_loop {
        lines.push("drift summary: detector disabled by config".to_string());
        return lines;
    }

    if !crate::platform::daemon::state_file_path(config).exists() {
        return awaiting_first_daemon_boot_lines("drift summary");
    }

    let rows = match load_slot_values_like(config, PERSONA_ROLLBACK_LATEST_SLOT_GLOB) {
        Ok(rows) => rows,
        Err(error) => {
            lines.push(format!("drift summary: failed to query records ({error})"));
            return lines;
        }
    };

    let mut evaluated = 0_u64;
    let mut warning_records = 0_u64;
    let mut critical_records = 0_u64;
    let mut score_sum = 0.0_f64;
    let mut min_score = 1.0_f64;

    for row in rows {
        let Ok(record) = serde_json::from_str::<PersonaTransition>(&row) else {
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
        lines.push("drift summary: no transition records found".to_string());
        return lines;
    }

    let evaluated_u32 = u32::try_from(evaluated).unwrap_or(u32::MAX).max(1);
    let avg_score = score_sum / f64::from(evaluated_u32);
    lines.push(format!(
        "drift summary: evaluated={evaluated}, warning={warning_records}, critical={critical_records}"
    ));
    lines.push(format!(
        "drift score: avg_continuity={avg_score:.3}, min_continuity={min_score:.3}"
    ));
    lines
}

/// Build diagnostic lines for metacognitive calibration gate status.
pub(crate) fn persona_calibration_lines(config: &Config) -> Vec<String> {
    let operational = load_runtime_operational_snapshot(config);
    if !operational.persona_state_metrics.is_supported() {
        return unsupported_metric_lines("calibration summary", &operational.persona_state_metrics);
    }

    let mut lines = Vec::with_capacity(4);
    lines.push(format!(
        "detector: {} (window={}, min_samples={}, mean<={:.2}, p95<={:.2})",
        if config.persona.enable_metacognitive_logging {
            "enabled"
        } else {
            "disabled"
        },
        config.persona.calibration_gate_window_size.max(1),
        config.persona.calibration_gate_min_samples.max(1),
        config.persona.calibration_gate_mean_error_max,
        config.persona.calibration_gate_p95_error_max
    ));

    if !config.persona.enable_metacognitive_logging {
        lines.push("calibration summary: metacognitive logging disabled by config".to_string());
        return lines;
    }

    if !crate::platform::daemon::state_file_path(config).exists() {
        return awaiting_first_daemon_boot_lines("calibration summary");
    }

    let Some(raw) = (match load_latest_slot_value(config, CALIBRATION_SNAPSHOT_SLOT_KEY) {
        Ok(raw) => raw,
        Err(error) => {
            lines.push(format!(
                "calibration summary: failed to query calibration snapshot ({error})"
            ));
            return lines;
        }
    }) else {
        lines.push("calibration summary: no calibration snapshot found".to_string());
        return lines;
    };

    let snapshot: CalibrationSnapshot = match serde_json::from_str(&raw) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            lines.push(format!(
                "calibration summary: failed to parse snapshot ({error})"
            ));
            return lines;
        }
    };

    lines.push(format!(
        "calibration summary: status={}, samples={}, mean_error={:.3}, p95_error={:.3}",
        snapshot.gate_status, snapshot.sample_count, snapshot.mean_error, snapshot.p95_error
    ));
    lines.push(format!(
        "calibration thresholds: min_samples={}, mean<={:.2}, p95<={:.2}",
        snapshot.gate_min_samples, snapshot.gate_mean_error_max, snapshot.gate_p95_error_max
    ));

    lines
}

/// Build diagnostic lines for the persona continuity gate and rollback drills.
pub(crate) fn persona_continuity_gate_lines(config: &Config) -> Vec<String> {
    let operational = load_runtime_operational_snapshot(config);
    if !operational.persona_state_metrics.is_supported() {
        return unsupported_metric_lines(
            "rollback drill summary",
            &operational.persona_state_metrics,
        );
    }

    let mut lines = Vec::with_capacity(4);
    lines.push(format!(
        "continuity gate: {} (critical<={:.2})",
        if config.persona.enable_continuity_gate {
            "enabled"
        } else {
            "disabled"
        },
        config.persona.drift_critical_threshold
    ));
    lines.push(format!(
        "rollback drills: {}",
        if config.persona.enable_rollback_drills {
            "enabled"
        } else {
            "disabled"
        }
    ));

    if !config.persona.enable_rollback_drills {
        lines.push("rollback drill summary: disabled by config".to_string());
        return lines;
    }

    if !crate::platform::daemon::state_file_path(config).exists() {
        return awaiting_first_daemon_boot_lines("rollback drill summary");
    }

    let Some(raw) = (match load_latest_slot_value(config, ROLLBACK_DRILL_SLOT_KEY) {
        Ok(raw) => raw,
        Err(error) => {
            lines.push(format!(
                "rollback drill summary: failed to query latest result ({error})"
            ));
            return lines;
        }
    }) else {
        lines.push("rollback drill summary: no drill result found".to_string());
        return lines;
    };

    match serde_json::from_str::<RollbackDrillResult>(&raw) {
        Ok(result) => lines.push(format!(
            "rollback drill summary: status={}, trigger={}, checked_at={}",
            result.status, result.trigger, result.checked_at
        )),
        Err(error) => lines.push(format!(
            "rollback drill summary: failed to parse latest result ({error})"
        )),
    }

    lines
}

/// Build diagnostic lines for embodied-state temperature modulation.
pub(crate) fn persona_embodied_state_lines(config: &Config) -> Vec<String> {
    let operational = load_runtime_operational_snapshot(config);
    if !operational.persona_state_metrics.is_supported() {
        return unsupported_metric_lines(
            "embodied-state summary",
            &operational.persona_state_metrics,
        );
    }

    let mut lines = Vec::with_capacity(3);
    lines.push(format!(
        "embodied-state modulation: {} (|delta_temp|<={:.2})",
        if config.persona.enable_embodied_state_policy_modulation {
            "enabled"
        } else {
            "disabled"
        },
        config.persona.embodied_temperature_delta_max.abs()
    ));

    if !config.persona.enable_embodied_state_policy_modulation {
        lines.push("embodied-state summary: modulation disabled by config".to_string());
        return lines;
    }

    if !crate::platform::daemon::state_file_path(config).exists() {
        return awaiting_first_daemon_boot_lines("embodied-state summary");
    }

    let Some(raw) = (match load_latest_slot_value(config, EMBODIED_STATE_SLOT_KEY) {
        Ok(raw) => raw,
        Err(error) => {
            lines.push(format!(
                "embodied-state summary: failed to query latest snapshot ({error})"
            ));
            return lines;
        }
    }) else {
        lines.push("embodied-state summary: no modulation snapshot found".to_string());
        return lines;
    };

    match serde_json::from_str::<EmbodiedStateSnapshot>(&raw) {
        Ok(snapshot) => lines.push(format!(
            "embodied-state summary: pressure={:.3}, capacity={:.3}, stability={:.3}, coherence={:.3}, delta={:.3}, reason={}, calibration={}",
            snapshot.resource_pressure_index,
            snapshot.runtime_capacity_index,
            snapshot.interaction_stability_index,
            snapshot.coherence_index,
            snapshot.applied_temperature_delta,
            snapshot.modulation_reason,
            snapshot.calibration_status
        )),
        Err(error) => lines.push(format!(
            "embodied-state summary: failed to parse latest snapshot ({error})"
        )),
    }

    lines
}

/// Build diagnostic lines with memory signal tier and promotion stats.
pub(crate) fn memory_signal_stats_lines(config: &Config) -> Vec<String> {
    let operational = load_runtime_operational_snapshot(config);
    if !operational.memory_signal_metrics.is_supported() {
        return unsupported_metric_lines("signal stats", &operational.memory_signal_metrics);
    }
    if !crate::platform::daemon::state_file_path(config).exists() {
        return awaiting_first_daemon_boot_lines("signal stats");
    }

    let stats = match load_memory_signal_stats(config) {
        Ok(stats) => stats,
        Err(error) => {
            return vec![format!(
                "signal stats: failed to query memory metrics ({error})"
            )];
        }
    };

    let ratio = contradiction_ratio(&stats);

    vec![
        format!(
            "signal stats: total_units={}, raw_units={}, demoted_units={}",
            stats.total_units, stats.raw_units, stats.demoted_units
        ),
        format!(
            "signal stats: promotion_breakdown candidate={}, promoted={}, demoted={}",
            stats.candidate_units, stats.promoted_units, stats.demoted_units
        ),
        format!(
            "signal stats: ttl_expired_units={}",
            stats.ttl_expired_units
        ),
        format!(
            "signal stats: source_kind_breakdown={}",
            stats.source_kind_breakdown
        ),
        format!(
            "signal stats: contradicted_units={}, contradiction_ratio={ratio:.3}",
            stats.contradicted_units
        ),
    ]
}
