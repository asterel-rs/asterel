//! Unit tests for doctor report generation.

use super::{
    autonomy_governance_lines, memory_rollout_lines, memory_signal_stats_lines, parse_rfc3339,
    persona_calibration_lines, persona_continuity_gate_lines, persona_drift_lines,
    persona_embodied_state_lines,
};
use crate::config::Config;
use crate::utils::test_env::EnvVarGuard;
use tempfile::TempDir;

#[test]
fn doctor_reports_autonomy_gates() {
    let mut config = Config::default();
    config.observability.backend = crate::config::ObservabilityBackend::Prometheus;

    let lines = autonomy_governance_lines(&config);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("external actions") && line.contains("disabled"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("temperature band") && line.contains('['))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("rollout stage") && line.contains("off"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("rollout policy") && line.contains("enabled=off"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("observability backend") && line.contains("prometheus"))
    );
    assert!(
        lines.iter().any(|line| {
            line.contains("autonomy lifecycle metrics") && line.contains("enabled")
        })
    );
}

#[test]
fn doctor_reports_memory_rollout() {
    let mut config = Config::default();
    config.memory.backend = crate::config::MemoryBackend::Markdown;
    config.memory.auto_save = true;
    config.autonomy.rollout.enabled = true;

    let snapshot = serde_json::json!({
        "memory_rollout": {
            "consolidation": "healthy",
            "conflict": "healthy",
            "revocation": "healthy",
            "governance": "healthy"
        },
        "components": {
            "memory_slo": {
                "status": "ok"
            }
        }
    });

    let lines = memory_rollout_lines(&config, &snapshot);

    assert!(lines.iter().any(|line| {
        line.contains("memory backend: markdown") && line.contains("consolidation=on")
    }));
    assert!(lines.iter().any(
        |line| line.contains("revocation=degraded") && line.contains("governance=unsupported")
    ));
    assert!(lines.iter().any(|line| {
        line.contains("daemon lifecycle health") && line.contains("consolidation=healthy")
    }));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("memory_slo component: ok"))
    );
}

#[test]
fn doctor_reports_memory_rollout_missing_config() {
    let config = Config::default();
    let snapshot = serde_json::json!({});

    let lines = memory_rollout_lines(&config, &snapshot);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("missing config") && line.contains("non-fatal"))
    );
    assert!(lines.iter().any(|line| line.contains("action:")));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("memory_slo component: missing"))
    );
}

#[test]
fn doctor_reports_autonomy_rollout_variants_and_disabled_lifecycle_metrics() {
    let mut config = Config::default();
    config.autonomy.external_action_execution = crate::security::ExternalActionExecution::Enabled;
    config.autonomy.rollout.enabled = true;
    config.autonomy.rollout.stage = Some(crate::config::schema::AutonomyRolloutStage::Full);
    config.observability.backend = crate::config::ObservabilityBackend::None;

    let lines = autonomy_governance_lines(&config);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("external actions: enabled"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("rollout stage: full"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("rollout policy") && line.contains("enabled=on"))
    );
    assert!(lines.iter().any(|line| {
        line.contains("observability backend: none")
            && line.contains("autonomy lifecycle metrics: disabled")
    }));
}

#[test]
fn doctor_reports_memory_rollout_partial_snapshot_and_none_backend() {
    let mut config = Config::default();
    config.memory.backend = crate::config::MemoryBackend::None;

    let snapshot = serde_json::json!({
        "memory_rollout": {
            "consolidation": "healthy"
        },
        "components": {}
    });

    let lines = memory_rollout_lines(&config, &snapshot);

    assert!(lines.iter().any(|line| {
        line.contains("memory backend: none") && line.contains("consolidation=off")
    }));
    assert!(lines.iter().any(|line| {
        line.contains("lifecycle support")
            && line.contains("revocation=degraded")
            && line.contains("governance=unsupported")
    }));
    assert!(lines.iter().any(|line| {
        line.contains("daemon lifecycle health")
            && line.contains("consolidation=healthy")
            && line.contains("conflict=unknown")
    }));
    assert!(
        lines
            .iter()
            .any(|line| line.contains("memory_slo component: missing"))
    );
}

#[test]
fn doctor_parse_rfc3339_accepts_valid_and_rejects_invalid_values() {
    let parsed = parse_rfc3339("2026-04-16T12:34:56+09:00").expect("valid timestamp");
    assert_eq!(parsed.to_rfc3339(), "2026-04-16T03:34:56+00:00");
    assert!(parse_rfc3339("not-a-timestamp").is_none());
}

fn temp_config() -> Config {
    let tmp = TempDir::new().expect("tempdir");
    let base = tmp.path().to_path_buf();
    std::mem::forget(tmp);
    let config = Config {
        workspace_dir: base.join("workspace"),
        config_path: base.join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).expect("workspace");
    config
}

#[test]
fn doctor_skips_postgres_metrics_for_markdown_memory() {
    let mut config = temp_config();
    config.memory.backend = crate::config::MemoryBackend::Markdown;

    let lines = memory_signal_stats_lines(&config);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("signal stats: unsupported"))
    );
}

#[test]
fn doctor_waits_for_daemon_boot_before_persona_metric_queries() {
    let mut config = temp_config();
    config.memory.backend = crate::config::MemoryBackend::Postgres;
    config.memory.postgres_url = Some("postgres://example".to_string());

    let lines = persona_calibration_lines(&config);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("awaiting first daemon boot"))
    );
}

#[test]
fn doctor_reports_disabled_drift_detector_without_querying_state() {
    let mut config = temp_config();
    config.memory.backend = crate::config::MemoryBackend::Postgres;
    config.memory.postgres_url = Some("postgres://example".to_string());
    config.persona.enable_drift_detection_loop = false;
    std::fs::write(
        crate::platform::daemon::state_file_path(&config),
        r#"{"components":{}}"#,
    )
    .expect("state file");

    let lines = persona_drift_lines(&config);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("drift summary: detector disabled by config"))
    );
}

#[test]
fn doctor_reports_disabled_drift_detector_before_daemon_boot() {
    let mut config = temp_config();
    config.memory.backend = crate::config::MemoryBackend::Postgres;
    config.memory.postgres_url = Some("postgres://example".to_string());
    config.persona.enable_drift_detection_loop = false;

    let lines = persona_drift_lines(&config);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("drift summary: detector disabled by config"))
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("awaiting first daemon boot"))
    );
}

#[test]
fn doctor_reports_disabled_continuity_gate_without_snapshot_dependency() {
    let mut config = temp_config();
    config.memory.backend = crate::config::MemoryBackend::Postgres;
    config.memory.postgres_url = Some("postgres://example".to_string());
    config.persona.enable_continuity_gate = false;
    config.persona.enable_rollback_drills = false;
    std::fs::write(
        crate::platform::daemon::state_file_path(&config),
        r#"{"components":{}}"#,
    )
    .expect("state file");

    let lines = persona_continuity_gate_lines(&config);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("continuity gate: disabled"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("rollback drill summary: disabled by config"))
    );
}

#[test]
fn doctor_reports_disabled_rollback_drills_before_daemon_boot() {
    let mut config = temp_config();
    config.memory.backend = crate::config::MemoryBackend::Postgres;
    config.memory.postgres_url = Some("postgres://example".to_string());
    config.persona.enable_continuity_gate = false;
    config.persona.enable_rollback_drills = false;

    let lines = persona_continuity_gate_lines(&config);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("rollback drill summary: disabled by config"))
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("awaiting first daemon boot"))
    );
}

#[test]
fn doctor_reports_disabled_embodied_state_modulation_without_snapshot_dependency() {
    let mut config = temp_config();
    config.memory.backend = crate::config::MemoryBackend::Postgres;
    config.memory.postgres_url = Some("postgres://example".to_string());
    config.persona.enable_embodied_state_policy_modulation = false;
    std::fs::write(
        crate::platform::daemon::state_file_path(&config),
        r#"{"components":{}}"#,
    )
    .expect("state file");

    let lines = persona_embodied_state_lines(&config);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("embodied-state summary: modulation disabled by config"))
    );
}

#[test]
fn doctor_reports_disabled_embodied_state_modulation_before_daemon_boot() {
    let mut config = temp_config();
    config.memory.backend = crate::config::MemoryBackend::Postgres;
    config.memory.postgres_url = Some("postgres://example".to_string());
    config.persona.enable_embodied_state_policy_modulation = false;

    let lines = persona_embodied_state_lines(&config);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("embodied-state summary: modulation disabled by config"))
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("awaiting first daemon boot"))
    );
}

#[test]
fn doctor_reports_unsupported_persona_metrics_for_markdown_backend() {
    let mut config = temp_config();
    config.memory.backend = crate::config::MemoryBackend::Markdown;

    let drift = persona_drift_lines(&config);
    let continuity = persona_continuity_gate_lines(&config);
    let embodied = persona_embodied_state_lines(&config);

    assert!(
        drift
            .iter()
            .any(|line| line.contains("drift summary: unsupported"))
    );
    assert!(
        continuity
            .iter()
            .any(|line| line.contains("rollback drill summary: unsupported"))
    );
    assert!(
        embodied
            .iter()
            .any(|line| line.contains("embodied-state summary: unsupported"))
    );
}

#[test]
fn doctor_memory_signal_stats_waits_for_daemon_boot_on_supported_backend() {
    let mut config = temp_config();
    config.memory.backend = crate::config::MemoryBackend::Postgres;
    config.memory.postgres_url = Some("postgres://example".to_string());

    let lines = memory_signal_stats_lines(&config);

    assert!(
        lines
            .iter()
            .any(|line| line.contains("signal stats: awaiting first daemon boot"))
    );
}

#[test]
fn doctor_reports_disabled_calibration_before_daemon_boot() {
    let mut config = temp_config();
    config.memory.backend = crate::config::MemoryBackend::Postgres;
    config.memory.postgres_url = Some("postgres://example".to_string());
    config.persona.enable_metacognitive_logging = false;

    let lines = persona_calibration_lines(&config);

    assert!(lines.iter().any(|line| {
        line.contains("calibration summary: metacognitive logging disabled by config")
    }));
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("awaiting first daemon boot"))
    );
}

#[test]
fn doctor_reports_degraded_metric_capabilities_without_querying_state() {
    #[cfg(feature = "postgres")]
    let _db_guard = crate::utils::test_env::acquire_test_db_lock_only_blocking();
    let _postgres_url_guard = EnvVarGuard::unset("ASTEREL_POSTGRES_URL");
    let mut config = temp_config();
    config.memory.backend = crate::config::MemoryBackend::Postgres;
    config.memory.postgres_url = None;

    let drift = persona_drift_lines(&config);
    let calibration = persona_calibration_lines(&config);
    let continuity = persona_continuity_gate_lines(&config);
    let embodied = persona_embodied_state_lines(&config);
    let signals = memory_signal_stats_lines(&config);

    assert!(
        drift
            .iter()
            .any(|line| line.contains("drift summary: degraded"))
    );
    assert!(calibration.iter().any(|line| {
        line.contains("calibration summary: degraded")
            && line.contains("PostgreSQL URL is unavailable")
    }));
    assert!(continuity.iter().any(|line| {
        line.contains("rollback drill summary: degraded")
            && line.contains("PostgreSQL URL is unavailable")
    }));
    assert!(embodied.iter().any(|line| {
        line.contains("embodied-state summary: degraded")
            && line.contains("PostgreSQL URL is unavailable")
    }));
    assert!(signals.iter().any(|line| {
        line.contains("signal stats: degraded") && line.contains("PostgreSQL URL is unavailable")
    }));
}
