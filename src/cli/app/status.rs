//! Human-readable status report renderer.
//!
//! Formats runtime, memory, security, and channel configuration
//! into a textual summary for the `status` CLI subcommand.

use asterel::config::Config;
use asterel::runtime::services::load_runtime_operational_snapshot;
use asterel::ui::style as ui;

/// Render the full system status report as a formatted string.
pub fn render_status(config: &Config) -> String {
    let mut lines = render_header(config);
    lines.extend(render_runtime_section(config));
    lines.extend(render_memory_section(config));
    lines.extend(render_security_section(config));
    lines.extend(render_channels_section(config));
    lines.join("\n")
}

fn render_header(config: &Config) -> Vec<String> {
    let operational = load_runtime_operational_snapshot(config);
    vec![
        format!("  {}", ui::section(t!("status.title"))),
        String::new(),
        ui::field_line(t!("status.version"), env!("CARGO_PKG_VERSION")),
        ui::field_line(t!("status.workspace"), config.workspace_dir.display()),
        ui::field_line(t!("status.config"), config.config_path.display()),
        ui::field_line(
            "Onboarding",
            if operational.onboarding_required {
                ui::warn_badge("required")
            } else {
                ui::ok_badge("complete")
            },
        ),
        String::new(),
        ui::field_line(
            t!("status.provider"),
            config
                .default_provider
                .as_deref()
                .unwrap_or(asterel::config::DEFAULT_PROVIDER),
        ),
        ui::field_line(
            t!("status.model"),
            config.default_model.as_deref().unwrap_or("(default)"),
        ),
        ui::field_line(t!("status.observability"), config.observability.backend),
        ui::field_line(
            "Observability state",
            capability_badge(&operational.observability),
        ),
    ]
}

fn render_runtime_section(config: &Config) -> Vec<String> {
    let operational = load_runtime_operational_snapshot(config);
    let temperature_band = config.autonomy.selected_temp_band();
    vec![
        String::new(),
        ui::section_with_rule("Runtime"),
        ui::field_line(
            t!("status.autonomy"),
            format!("{:?}", config.autonomy.effective_autonomy_lvl()),
        ),
        ui::field_line(
            t!("status.external_actions"),
            bool_badge(matches!(
                config.autonomy.external_action_execution,
                asterel::security::ExternalActionExecution::Enabled
            )),
        ),
        ui::field_line(
            t!("status.temperature_band"),
            format!("[{:.2}, {:.2}]", temperature_band.min, temperature_band.max),
        ),
        ui::field_line(
            t!("status.rollout_stage"),
            rollout_stage_label(config.autonomy.rollout.stage),
        ),
        ui::field_line(
            t!("status.rollout_policy"),
            format!(
                "{} read_only_days={:?} supervised_days={:?}",
                bool_badge(config.autonomy.rollout.enabled),
                config.autonomy.rollout.read_only_days,
                config.autonomy.rollout.supervised_days
            ),
        ),
        ui::field_line(
            t!("status.verify_repair"),
            format!(
                "max_attempts={} max_repair_depth={}",
                config.autonomy.verify_repair_max_attempts,
                config.autonomy.verify_repair_max_repair_depth
            ),
        ),
        ui::field_line(
            t!("status.autonomy_metrics"),
            bool_badge(observability_backend_supports_lifecycle_metrics(
                config.observability.backend,
            )),
        ),
        ui::field_line(
            "Session persistence",
            capability_badge(&operational.session_persistence),
        ),
        ui::field_line("Cron scheduler", capability_badge(&operational.cron)),
        ui::field_line(t!("status.runtime"), config.runtime.kind),
        ui::field_line("Sandbox selector", config.runtime.sandbox_selector),
        ui::field_line(
            t!("status.heartbeat"),
            if config.heartbeat.enabled {
                ui::ok_badge(format!("every {}min", config.heartbeat.interval_minutes))
            } else {
                ui::muted_badge("disabled")
            },
        ),
    ]
}

fn render_memory_section(config: &Config) -> Vec<String> {
    let operational = load_runtime_operational_snapshot(config);
    let (consolidation, conflict, revocation, governance) = memory_rollout_status(config);
    vec![
        String::new(),
        ui::section_with_rule("Memory"),
        ui::field_line(
            t!("status.memory"),
            format!(
                "{} auto-save={}",
                config.memory.backend,
                bool_badge(config.memory.auto_save)
            ),
        ),
        ui::field_line(
            t!("status.memory_rollout"),
            format!(
                "consolidation={consolidation} conflict={conflict} revocation={revocation} governance={governance}"
            ),
        ),
        ui::field_line(
            t!("status.memory_metrics"),
            format!(
                "{} (observer={})",
                capability_badge(&operational.memory_signal_metrics),
                bool_badge(observability_backend_supports_lifecycle_metrics(
                    config.observability.backend,
                ))
            ),
        ),
        ui::field_line(
            "Persona drift",
            format!(
                "{} warning<={:.2} critical<={:.2}",
                capability_badge(&operational.persona_state_metrics),
                config.persona.drift_warning_threshold,
                config.persona.drift_critical_threshold
            ),
        ),
    ]
}

fn render_security_section(config: &Config) -> Vec<String> {
    vec![
        String::new(),
        ui::section_with_rule("Security"),
        ui::field_line(
            t!("status.workspace_only"),
            bool_badge(config.autonomy.workspace_only),
        ),
        ui::field_line(
            "workspace_only(effective)",
            bool_badge(
                config
                    .runtime
                    .resolved_workspace_only(config.autonomy.workspace_only),
            ),
        ),
        ui::field_line(
            "Group isolation mode",
            format!("{:?}", config.channels_config.group_isolation_mode),
        ),
        ui::field_line(
            "Group isolation rules",
            config.channels_config.group_isolation_rules.len(),
        ),
        ui::field_line(
            t!("status.allowed_commands"),
            if config.autonomy.allowed_commands.is_empty() {
                "(none)".to_string()
            } else {
                config.autonomy.allowed_commands.join(", ")
            },
        ),
        ui::field_line(
            t!("status.max_actions"),
            config.autonomy.max_actions_per_hour,
        ),
        ui::field_line(
            t!("status.max_cost"),
            format!(
                "${:.2}",
                f64::from(config.autonomy.max_cost_per_day_cents) / 100.0
            ),
        ),
    ]
}

fn render_channels_section(config: &Config) -> Vec<String> {
    let operational = load_runtime_operational_snapshot(config);
    let mut lines = vec![String::new(), ui::section_with_rule("Channels")];

    for channel in operational.channels {
        lines.push(ui::field_line(
            channel.label,
            if channel.enabled {
                ui::ok_badge(t!("common.confirmed"))
            } else if channel.configured {
                ui::warn_badge("configured but disabled")
            } else {
                ui::muted_badge(t!("common.not_configured"))
            },
        ));
    }

    lines
}

fn bool_badge(value: bool) -> String {
    if value {
        ui::ok_badge("enabled")
    } else {
        ui::muted_badge("disabled")
    }
}

fn capability_badge(capability: &asterel::runtime::services::RuntimeCapabilityState) -> String {
    match capability.status {
        asterel::runtime::services::RuntimeCapabilityStatus::Supported => ui::ok_badge("supported"),
        asterel::runtime::services::RuntimeCapabilityStatus::Degraded => {
            ui::warn_badge(capability.reason.as_deref().unwrap_or("degraded"))
        }
        asterel::runtime::services::RuntimeCapabilityStatus::Unsupported => {
            ui::muted_badge(capability.reason.as_deref().unwrap_or("unsupported"))
        }
    }
}

fn rollout_stage_label(stage: Option<asterel::config::schema::AutonomyRolloutStage>) -> String {
    match stage {
        Some(asterel::config::schema::AutonomyRolloutStage::ReadOnly) => {
            ui::warn_badge("read-only")
        }
        Some(asterel::config::schema::AutonomyRolloutStage::Supervised) => {
            ui::warn_badge("supervised")
        }
        Some(asterel::config::schema::AutonomyRolloutStage::Full) => ui::ok_badge("full"),
        None => ui::muted_badge("off"),
    }
}

fn observability_backend_supports_lifecycle_metrics(
    backend: asterel::config::ObservabilityBackend,
) -> bool {
    backend.supports_lifecycle_metrics()
}

fn memory_rollout_status(
    config: &Config,
) -> (&'static str, &'static str, &'static str, &'static str) {
    let backend = config.memory.backend;
    let consolidation =
        if backend != asterel::config::MemoryBackend::None && config.memory.auto_save {
            "on"
        } else {
            "off"
        };
    let conflict =
        if backend != asterel::config::MemoryBackend::None && config.autonomy.rollout.enabled {
            "on"
        } else {
            "off"
        };

    let capability = asterel::core::memory::capability_matrix_for_backend(backend.as_str());
    let revocation = capability.map_or("unknown", |matrix| {
        capability_support_label(matrix.forget_tombstone)
    });
    let governance = capability.map_or("unknown", |matrix| {
        capability_support_label(matrix.forget_hard)
    });

    (consolidation, conflict, revocation, governance)
}

fn capability_support_label(support: asterel::core::memory::CapabilitySupport) -> &'static str {
    match support {
        asterel::core::memory::CapabilitySupport::Supported => "supported",
        asterel::core::memory::CapabilitySupport::Degraded => "degraded",
        asterel::core::memory::CapabilitySupport::Unsupported => "unsupported",
    }
}

#[cfg(test)]
mod tests {
    use asterel::config::Config;

    use super::{memory_rollout_status, render_status};

    #[test]
    fn status_reports_memory_rollout_support() {
        let mut config = Config::default();
        config.memory.backend = asterel::config::MemoryBackend::Postgres;
        config.memory.auto_save = true;
        config.autonomy.rollout.enabled = true;

        let rollout = memory_rollout_status(&config);
        assert_eq!(rollout.0, "on");
        assert_eq!(rollout.1, "on");
        assert_eq!(rollout.2, "supported");
        assert_eq!(rollout.3, "supported");
    }

    #[test]
    fn status_renders_persona_drift_detector_configuration() {
        let config = Config::default();
        let rendered = render_status(&config);
        assert!(rendered.contains("Persona drift"));
        assert!(rendered.contains("enabled"));
        assert!(rendered.contains("warning<=0.70"));
        assert!(rendered.contains("critical<=0.45"));
    }

    #[test]
    fn status_renders_full_channel_inventory() {
        let config = Config::default();
        let rendered = render_status(&config);
        let operational = asterel::runtime::services::load_runtime_operational_snapshot(&config);

        for channel in operational.channels {
            assert!(rendered.contains(channel.label));
        }
    }
}
