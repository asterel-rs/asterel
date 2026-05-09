use crate::config::{Config, MemoryBackend, ObservabilityBackend};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeCapabilityStatus {
    Supported,
    Degraded,
    Unsupported,
}

impl RuntimeCapabilityStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Degraded => "degraded",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilityState {
    pub status: RuntimeCapabilityStatus,
    pub reason: Option<String>,
}

impl RuntimeCapabilityState {
    #[must_use]
    pub fn supported() -> Self {
        Self {
            status: RuntimeCapabilityStatus::Supported,
            reason: None,
        }
    }

    #[must_use]
    pub fn degraded(reason: impl Into<String>) -> Self {
        Self {
            status: RuntimeCapabilityStatus::Degraded,
            reason: Some(reason.into()),
        }
    }

    #[must_use]
    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self {
            status: RuntimeCapabilityStatus::Unsupported,
            reason: Some(reason.into()),
        }
    }

    #[must_use]
    pub const fn is_supported(&self) -> bool {
        matches!(self.status, RuntimeCapabilityStatus::Supported)
    }

    #[must_use]
    pub const fn is_runtime_required(&self) -> bool {
        matches!(
            self.status,
            RuntimeCapabilityStatus::Supported | RuntimeCapabilityStatus::Degraded
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeChannelSurfaceState {
    pub label: &'static str,
    pub configured: bool,
    pub enabled: bool,
    pub listener: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeOperationalSnapshot {
    pub onboarding_required: bool,
    pub channels: Vec<RuntimeChannelSurfaceState>,
    pub cron: RuntimeCapabilityState,
    pub session_persistence: RuntimeCapabilityState,
    pub memory_signal_metrics: RuntimeCapabilityState,
    pub persona_state_metrics: RuntimeCapabilityState,
    pub memory_review: RuntimeCapabilityState,
    pub observability: RuntimeCapabilityState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayReadinessProfile {
    Standalone,
    DaemonSupervised,
}

#[derive(Debug, Clone)]
pub struct GatewayReadinessAssessment {
    pub ready: bool,
    pub required_components: Vec<String>,
    pub failing_components: Vec<String>,
    pub runtime: crate::runtime::diagnostics::health::HealthSnapshot,
}

#[must_use]
pub fn load_runtime_operational_snapshot(config: &Config) -> RuntimeOperationalSnapshot {
    let postgres_url_resolved = crate::utils::postgres::resolve_postgres_url(
        config.memory.postgres_url.as_deref(),
        Some(&config.workspace_dir),
    )
    .is_some();

    RuntimeOperationalSnapshot {
        onboarding_required: runtime_boot_requires_onboarding(config),
        channels: channel_surface_inventory(config),
        cron: postgres_optional_runtime_capability(
            config,
            postgres_url_resolved,
            "cron scheduler requires a PostgreSQL URL",
        ),
        session_persistence: postgres_optional_runtime_capability(
            config,
            postgres_url_resolved,
            "runtime session persistence requires a PostgreSQL URL",
        ),
        memory_signal_metrics: postgres_memory_capability(
            config,
            postgres_url_resolved,
            "memory signal metrics require the postgres memory backend",
        ),
        persona_state_metrics: postgres_memory_capability(
            config,
            postgres_url_resolved,
            "persona calibration/drift metrics require the postgres memory backend",
        ),
        memory_review: if config.memory.backend == MemoryBackend::None {
            RuntimeCapabilityState::unsupported("memory review requires an active memory backend")
        } else {
            RuntimeCapabilityState::supported()
        },
        observability: observability_runtime_capability(config.observability.backend),
    }
}

#[must_use]
fn observability_runtime_capability(backend: ObservabilityBackend) -> RuntimeCapabilityState {
    match backend {
        ObservabilityBackend::Log | ObservabilityBackend::Prometheus => {
            RuntimeCapabilityState::supported()
        }
        ObservabilityBackend::Otel => RuntimeCapabilityState::degraded(
            "OpenTelemetry backend is currently a counter-only stub",
        ),
        ObservabilityBackend::None => {
            RuntimeCapabilityState::unsupported("observability disabled by config")
        }
    }
}

#[must_use]
pub fn runtime_boot_requires_onboarding(config: &Config) -> bool {
    runtime_boot_requires_onboarding_for_provider(config, None)
}

#[must_use]
pub fn runtime_boot_requires_onboarding_for_provider(
    config: &Config,
    provider_override: Option<&str>,
) -> bool {
    let resolved_model = config.resolve_model(provider_override, None);
    if crate::contracts::providers::normalize_provider_alias(&resolved_model.provider) == "ollama" {
        return false;
    }

    let auth_ready = crate::security::auth::AuthBroker::load_or_init(config)
        .ok()
        .and_then(|broker| broker.resolve_provider_key(&resolved_model.provider))
        .is_some();

    !auth_ready
}

#[must_use]
pub fn load_gateway_readiness_assessment(
    config: &Config,
    profile: GatewayReadinessProfile,
    session_persistence_initialized: bool,
) -> GatewayReadinessAssessment {
    let operational = load_runtime_operational_snapshot(config);
    let runtime = crate::runtime::diagnostics::health::snapshot();
    let mut required_components = vec!["gateway".to_string()];
    if matches!(profile, GatewayReadinessProfile::DaemonSupervised) {
        required_components.push("daemon".to_string());
        if operational.cron.is_runtime_required() {
            required_components.push("scheduler".to_string());
        }
        if operational
            .channels
            .iter()
            .any(|channel| channel.listener && channel.enabled)
        {
            required_components.push("channels".to_string());
        }
        if config.heartbeat.enabled {
            required_components.push("heartbeat".to_string());
        }
    }

    let mut failing_components = required_components
        .iter()
        .filter(|component| {
            runtime
                .components
                .get(component.as_str())
                .is_none_or(|health| {
                    !crate::runtime::diagnostics::health::component_is_ready(health)
                })
        })
        .cloned()
        .collect::<Vec<_>>();

    if operational.session_persistence.is_runtime_required() && !session_persistence_initialized {
        failing_components.push("session_persistence".to_string());
    }

    GatewayReadinessAssessment {
        ready: failing_components.is_empty(),
        required_components,
        failing_components,
        runtime,
    }
}

#[must_use]
fn postgres_optional_runtime_capability(
    config: &Config,
    postgres_url_resolved: bool,
    missing_url_reason: &str,
) -> RuntimeCapabilityState {
    if postgres_url_resolved {
        RuntimeCapabilityState::supported()
    } else if config.memory.backend == MemoryBackend::Postgres {
        RuntimeCapabilityState::degraded(missing_url_reason)
    } else {
        RuntimeCapabilityState::unsupported(missing_url_reason)
    }
}

#[must_use]
fn postgres_memory_capability(
    config: &Config,
    postgres_url_resolved: bool,
    unsupported_reason: &str,
) -> RuntimeCapabilityState {
    if config.memory.backend != MemoryBackend::Postgres {
        return RuntimeCapabilityState::unsupported(unsupported_reason);
    }
    if postgres_url_resolved {
        RuntimeCapabilityState::supported()
    } else {
        RuntimeCapabilityState::degraded(
            "postgres memory backend selected, but PostgreSQL URL is unavailable",
        )
    }
}

#[must_use]
fn channel_surface_inventory(config: &Config) -> Vec<RuntimeChannelSurfaceState> {
    let mut items = Vec::with_capacity(11);
    items.push(RuntimeChannelSurfaceState {
        label: "CLI",
        configured: true,
        enabled: config.channels_config.cli,
        listener: false,
    });

    #[cfg(feature = "telegram")]
    push_channel_surface(
        &mut items,
        config,
        "Telegram",
        config.channels_config.telegram.is_some(),
        true,
    );
    #[cfg(feature = "discord")]
    push_channel_surface(
        &mut items,
        config,
        "Discord",
        config.channels_config.discord.is_some(),
        true,
    );
    #[cfg(feature = "slack")]
    push_channel_surface(
        &mut items,
        config,
        "Slack",
        config.channels_config.slack.is_some(),
        true,
    );
    push_channel_surface(
        &mut items,
        config,
        "Webhook",
        config.channels_config.webhook.is_some(),
        false,
    );
    #[cfg(feature = "imessage")]
    push_channel_surface(
        &mut items,
        config,
        "iMessage",
        config.channels_config.imessage.is_some(),
        true,
    );
    #[cfg(feature = "matrix")]
    push_channel_surface(
        &mut items,
        config,
        "Matrix",
        config.channels_config.matrix.is_some(),
        true,
    );
    #[cfg(feature = "whatsapp")]
    push_channel_surface(
        &mut items,
        config,
        "WhatsApp",
        config.channels_config.whatsapp.is_some(),
        true,
    );
    #[cfg(feature = "email")]
    push_channel_surface(
        &mut items,
        config,
        "Email",
        config.channels_config.email.is_some(),
        true,
    );
    #[cfg(feature = "irc")]
    push_channel_surface(
        &mut items,
        config,
        "IRC",
        config.channels_config.irc.is_some(),
        true,
    );
    #[cfg(feature = "twitter")]
    push_channel_surface(
        &mut items,
        config,
        "Twitter",
        config.channels_config.twitter.is_some(),
        true,
    );

    items
}

fn push_channel_surface(
    items: &mut Vec<RuntimeChannelSurfaceState>,
    config: &Config,
    label: &'static str,
    configured: bool,
    listener: bool,
) {
    items.push(RuntimeChannelSurfaceState {
        label,
        configured,
        enabled: configured && config.channels_config.is_channel_enabled(channel_id(label)),
        listener,
    });
}

fn channel_id(label: &str) -> &str {
    match label {
        "Telegram" => "telegram",
        "Discord" => "discord",
        "Slack" => "slack",
        "Webhook" => "webhook",
        "iMessage" => "imessage",
        "Matrix" => "matrix",
        "WhatsApp" => "whatsapp",
        "Email" => "email",
        "IRC" => "irc",
        "Twitter" => "twitter",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{
        GatewayReadinessProfile, RuntimeCapabilityStatus, load_gateway_readiness_assessment,
        load_runtime_operational_snapshot, runtime_boot_requires_onboarding,
        runtime_boot_requires_onboarding_for_provider,
    };
    use crate::config::ChannelSecurityPolicy;
    use crate::config::{Config, MemoryBackend, WebhookConfig};
    use crate::security::auth::{AuthProfile, AuthProfileStore};
    use crate::utils::test_env::EnvVarGuard;

    fn test_config(tmp: &TempDir) -> Config {
        Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        }
    }

    #[test]
    fn markdown_runtime_marks_cron_and_sessions_unsupported() {
        #[cfg(feature = "postgres")]
        let _db_guard = crate::utils::test_env::acquire_test_db_lock_only_blocking();
        let _postgres_url_guard = EnvVarGuard::unset("ASTEREL_POSTGRES_URL");
        let tmp = TempDir::new().expect("tempdir");
        let mut config = test_config(&tmp);
        config.memory.backend = MemoryBackend::Markdown;

        let snapshot = load_runtime_operational_snapshot(&config);

        assert_eq!(snapshot.cron.status, RuntimeCapabilityStatus::Unsupported);
        assert_eq!(
            snapshot.session_persistence.status,
            RuntimeCapabilityStatus::Unsupported
        );
        assert_eq!(
            snapshot.memory_signal_metrics.status,
            RuntimeCapabilityStatus::Unsupported
        );
        assert_eq!(
            snapshot.persona_state_metrics.status,
            RuntimeCapabilityStatus::Unsupported
        );
    }

    #[test]
    fn postgres_runtime_without_url_marks_capabilities_degraded() {
        #[cfg(feature = "postgres")]
        let _db_guard = crate::utils::test_env::acquire_test_db_lock_only_blocking();
        let _postgres_url_guard = EnvVarGuard::unset("ASTEREL_POSTGRES_URL");
        let tmp = TempDir::new().expect("tempdir");
        let mut config = test_config(&tmp);
        config.memory.backend = MemoryBackend::Postgres;
        config.memory.postgres_url = None;

        let snapshot = load_runtime_operational_snapshot(&config);

        assert_eq!(snapshot.cron.status, RuntimeCapabilityStatus::Degraded);
        assert_eq!(
            snapshot.session_persistence.status,
            RuntimeCapabilityStatus::Degraded
        );
        assert_eq!(
            snapshot.memory_signal_metrics.status,
            RuntimeCapabilityStatus::Degraded
        );
        assert_eq!(
            snapshot.persona_state_metrics.status,
            RuntimeCapabilityStatus::Degraded
        );
    }

    #[test]
    fn onboarding_not_required_for_unauthenticated_local_provider() {
        let tmp = TempDir::new().expect("tempdir");
        let config = test_config(&tmp);

        assert!(!runtime_boot_requires_onboarding_for_provider(
            &config,
            Some("ollama")
        ));
    }

    #[test]
    fn readiness_requires_scheduler_and_session_persistence_when_daemon_supervised() {
        let tmp = TempDir::new().expect("tempdir");
        let mut config = test_config(&tmp);
        config.memory.backend = MemoryBackend::Postgres;
        config.memory.postgres_url = Some("postgres://example".to_string());
        crate::runtime::diagnostics::health::mark_component_ok("gateway");
        crate::runtime::diagnostics::health::mark_component_ok("daemon");
        crate::runtime::diagnostics::health::mark_component_error("scheduler", "not running");

        let readiness = load_gateway_readiness_assessment(
            &config,
            GatewayReadinessProfile::DaemonSupervised,
            false,
        );

        assert!(!readiness.ready);
        assert!(
            readiness
                .required_components
                .contains(&"scheduler".to_string())
        );
        assert!(
            readiness
                .failing_components
                .contains(&"scheduler".to_string())
        );
        assert!(
            readiness
                .failing_components
                .contains(&"session_persistence".to_string())
        );
    }

    #[test]
    fn standalone_gateway_readiness_only_requires_gateway_component() {
        #[cfg(feature = "postgres")]
        let _db_guard = crate::utils::test_env::acquire_test_db_lock_only_blocking();
        let _postgres_url_guard = EnvVarGuard::unset("ASTEREL_POSTGRES_URL");
        let tmp = TempDir::new().expect("tempdir");
        let mut config = test_config(&tmp);
        config.memory.backend = MemoryBackend::Markdown;
        crate::runtime::diagnostics::health::mark_component_ok("gateway");

        let readiness =
            load_gateway_readiness_assessment(&config, GatewayReadinessProfile::Standalone, false);

        assert!(readiness.ready);
        assert_eq!(readiness.required_components, vec!["gateway".to_string()]);
        assert!(readiness.failing_components.is_empty());
    }

    #[test]
    fn channel_inventory_reports_configured_but_disabled_webhook() {
        let tmp = TempDir::new().expect("tempdir");
        let mut config = test_config(&tmp);
        config.channels_config.webhook = Some(WebhookConfig {
            port: 3000,
            secret: Some("secret".to_string()),
            security: ChannelSecurityPolicy::default(),
        });
        config.channels_config.disabled_channels = vec!["webhook".to_string()];

        let snapshot = load_runtime_operational_snapshot(&config);
        let webhook = snapshot
            .channels
            .iter()
            .find(|channel| channel.label == "Webhook")
            .expect("webhook row");

        assert!(webhook.configured);
        assert!(!webhook.enabled);
        assert!(!webhook.listener);
    }

    #[test]
    fn runtime_boot_requires_onboarding_without_provider_key() {
        let tmp = TempDir::new().expect("tempdir");
        let mut config = test_config(&tmp);
        config.default_provider = Some("openai".to_string());

        assert!(runtime_boot_requires_onboarding(&config));
        assert!(load_runtime_operational_snapshot(&config).onboarding_required);
    }

    #[test]
    fn runtime_boot_does_not_require_onboarding_when_auth_profile_can_resolve_key() {
        let tmp = TempDir::new().expect("tempdir");
        let mut config = test_config(&tmp);
        config.default_provider = Some("openai".to_string());

        let mut store = AuthProfileStore::load_or_init_cfg(&config).expect("auth store");
        store
            .upsert_profile(
                AuthProfile {
                    id: "openai-default".to_string(),
                    provider: "openai".to_string(),
                    auth_route: None,
                    label: Some("OpenAI".to_string()),
                    api_key: Some("sk-test".to_string()),
                    refresh_token: None,
                    auth_scheme: Some("api_key".to_string()),
                    oauth_source: None,
                    is_disabled: false,
                },
                true,
            )
            .expect("profile insert");
        store.save_for_config(&config).expect("save auth store");

        assert!(!runtime_boot_requires_onboarding(&config));
        assert!(!load_runtime_operational_snapshot(&config).onboarding_required);
    }

    #[test]
    fn runtime_boot_can_use_agent_provider_override_for_auth_resolution() {
        let tmp = TempDir::new().expect("tempdir");
        let config = test_config(&tmp);

        let mut store = AuthProfileStore::load_or_init_cfg(&config).expect("auth store");
        store
            .upsert_profile(
                AuthProfile {
                    id: "openai-default".to_string(),
                    provider: "openai".to_string(),
                    auth_route: None,
                    label: Some("OpenAI".to_string()),
                    api_key: Some("sk-test".to_string()),
                    refresh_token: None,
                    auth_scheme: Some("api_key".to_string()),
                    oauth_source: None,
                    is_disabled: false,
                },
                true,
            )
            .expect("profile insert");
        store.save_for_config(&config).expect("save auth store");

        assert!(!runtime_boot_requires_onboarding_for_provider(
            &config,
            Some("openai")
        ));
    }
}
