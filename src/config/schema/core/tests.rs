//! Integration tests for `Config` round-trip serialisation,
//! validation, and model resolution.

use super::super::identity::{RuntimeConfig, RuntimeKind, SandboxSelectorMode, SecretsConfig};
use super::super::models::SkillSource;
use super::*;

fn assert_close(lhs: f64, rhs: f64) {
    assert!((lhs - rhs).abs() < 1e-9, "lhs={lhs} rhs={rhs}");
}
use crate::config::schema::core::test_env::{ENV_LOCK, EnvVarGuard};

#[test]
fn needs_onboarding_is_true_without_api_key_or_provider() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _asterel_api_key = EnvVarGuard::unset("ASTEREL_API_KEY");
    let _api_key = EnvVarGuard::unset("API_KEY");

    let config = Config {
        api_key: None,
        default_provider: None,
        ..Config::default()
    };

    assert!(config.needs_onboarding());
}

#[test]
fn needs_onboarding_is_false_with_configured_api_key() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _asterel_api_key = EnvVarGuard::unset("ASTEREL_API_KEY");
    let _api_key = EnvVarGuard::unset("API_KEY");

    let config = Config {
        api_key: Some("sk-configured".to_string()),
        default_provider: None,
        ..Config::default()
    };

    assert!(!config.needs_onboarding());
}

#[test]
fn needs_onboarding_is_false_with_env_api_key() {
    let _lock = ENV_LOCK.lock().unwrap();
    let _asterel_api_key = EnvVarGuard::set("ASTEREL_API_KEY", "sk-env");
    let _api_key = EnvVarGuard::unset("API_KEY");

    let config = Config {
        api_key: None,
        default_provider: None,
        ..Config::default()
    };

    assert!(!config.needs_onboarding());
}

#[test]
fn default_config_has_reasonable_values() {
    let config = Config::default();

    assert_eq!(config.api_key, None);
    assert!(config.default_provider.is_some());
    assert!(config.default_model.is_some());
    assert!((0.0..=2.0).contains(&config.default_temperature));
    assert!(config.runtime.enable_live_settings_reload);
    assert!(config.workspace_dir.ends_with("workspace"));
    assert!(config.config_path.ends_with("config.toml"));
    assert_eq!(config.locale, "en");
    assert_eq!(
        config.skills.source_priority,
        vec![
            SkillSource::Workspace,
            SkillSource::ExtraDirs,
            SkillSource::OpenSkills,
        ]
    );
    assert!(config.skills.enforce_requirements);
    assert!(config.skills.watch_refresh);
}

#[test]
fn config_toml_round_trip_preserves_serialized_fields() {
    let config = Config {
        api_key: Some("sk-test".into()),
        default_provider: Some("openrouter".into()),
        default_model: Some(DEFAULT_MODEL.into()),
        default_temperature: 1.1,
        locale: "ja".into(),
        ..Config::default()
    };

    let serialized = toml::to_string(&config).unwrap();
    let deserialized: Config = toml::from_str(&serialized).unwrap();

    assert_eq!(deserialized.api_key, config.api_key);
    assert_eq!(deserialized.default_provider, config.default_provider);
    assert_eq!(deserialized.default_model, config.default_model);
    assert_close(deserialized.default_temperature, config.default_temperature);
    assert_eq!(deserialized.locale, config.locale);
    assert_eq!(deserialized.media.enabled, config.media.enabled);
    assert_eq!(deserialized.media.storage_dir, config.media.storage_dir);
    assert_eq!(
        deserialized.media.max_file_size_mb,
        config.media.max_file_size_mb
    );
    assert_eq!(deserialized.autonomy.level, config.autonomy.level);
    assert_eq!(
        deserialized.autonomy.external_action_execution,
        config.autonomy.external_action_execution
    );
    assert_eq!(deserialized.workspace_dir, PathBuf::new());
    assert_eq!(deserialized.config_path, PathBuf::new());
}

#[test]
fn config_round_trip_with_default_media_config() {
    let config = Config::default();

    let serialized = toml::to_string(&config).unwrap();
    let deserialized: Config = toml::from_str(&serialized).unwrap();

    assert_eq!(deserialized.media.enabled, MediaConfig::default().enabled);
    assert_eq!(
        deserialized.media.storage_dir,
        MediaConfig::default().storage_dir
    );
    assert_eq!(
        deserialized.media.max_file_size_mb,
        MediaConfig::default().max_file_size_mb
    );
}

#[test]
fn config_round_trip_with_custom_media_config() {
    let config = Config {
        media: MediaConfig {
            enabled: true,
            storage_dir: Some("/tmp/custom-media".to_string()),
            max_file_size_mb: 64,
            ..MediaConfig::default()
        },
        ..Config::default()
    };

    let serialized = toml::to_string(&config).unwrap();
    let deserialized: Config = toml::from_str(&serialized).unwrap();

    assert!(deserialized.media.enabled);
    assert_eq!(
        deserialized.media.storage_dir,
        Some("/tmp/custom-media".to_string())
    );
    assert_eq!(deserialized.media.max_file_size_mb, 64);
}

#[test]
fn config_round_trip_with_skills_runtime_overrides() {
    let config = Config {
        skills: SkillsRuntimeConfig {
            source_priority: vec![SkillSource::ExtraDirs, SkillSource::Workspace],
            extra_dirs: vec!["skills-extra".to_string()],
            disabled_skills: vec!["ops-review".to_string()],
            enforce_requirements: false,
            watch_refresh: false,
            prompt_description_chars: 72,
            turn_hint_limit: 2,
        },
        ..Config::default()
    };

    let serialized = toml::to_string(&config).unwrap();
    let deserialized: Config = toml::from_str(&serialized).unwrap();

    assert_eq!(
        deserialized.skills.source_priority,
        vec![SkillSource::ExtraDirs, SkillSource::Workspace]
    );
    assert_eq!(
        deserialized.skills.extra_dirs,
        vec!["skills-extra".to_string()]
    );
    assert_eq!(
        deserialized.skills.disabled_skills,
        vec!["ops-review".to_string()]
    );
    assert!(!deserialized.skills.enforce_requirements);
    assert!(!deserialized.skills.watch_refresh);
    assert_eq!(deserialized.skills.prompt_description_chars, 72);
    assert_eq!(deserialized.skills.turn_hint_limit, 2);
}

#[test]
fn config_round_trip_preserves_provider_and_model_registry() {
    let config = Config {
        api_key: Some("sk-provider".to_string()),
        default_provider: Some("openai".to_string()),
        default_model: Some("ops-default".to_string()),
        model_list: vec![ModelListEntry {
            model_name: "ops-default".to_string(),
            model: "custom/gpt-5.2".to_string(),
            api_key: Some("sk-model".to_string()),
            api_base: Some("https://proxy.example/v1".to_string()),
        }],
        ..Config::default()
    };

    let serialized = toml::to_string(&config).expect("serialize config");
    let deserialized: Config = toml::from_str(&serialized).expect("deserialize config");

    assert_eq!(deserialized.api_key, Some("sk-provider".to_string()));
    assert_eq!(deserialized.default_provider, Some("openai".to_string()));
    assert_eq!(deserialized.default_model, Some("ops-default".to_string()));
    assert_eq!(deserialized.model_list.len(), 1);
    assert_eq!(deserialized.model_list[0].model_name, "ops-default");
    assert_eq!(deserialized.model_list[0].model, "custom/gpt-5.2");
    assert_eq!(
        deserialized.model_list[0].api_base.as_deref(),
        Some("https://proxy.example/v1")
    );
}

#[test]
fn config_round_trip_preserves_channels_section() {
    let config = Config {
        channels_config: crate::config::ChannelsConfig {
            cli: false,
            disabled_channels: vec!["webhook".to_string()],
            coalescing_window_ms: 250,
            coalescing_max_messages: 6,
            routing_global_concurrency: 3,
            routing_group_queue_capacity: 48,
            routing_max_groups: 77,
            routing_rules: vec![crate::config::schema::RoutingRuleConfig {
                channel: "discord".to_string(),
                sender: Some("ops-user".to_string()),
                conversation_id: Some("thread-42".to_string()),
                group: "ops".to_string(),
            }],
            group_isolation_mode: crate::config::GroupIsolationMode::Global,
            group_isolation_rules: vec![crate::config::GroupIsolationRuleConfig {
                group: "ops".to_string(),
                filesystem: crate::config::GroupIsolationLevel::Container,
                process: crate::config::GroupIsolationLevel::Workspace,
                network: crate::config::GroupIsolationLevel::Shared,
            }],
            telegram: Some(crate::config::TelegramConfig {
                bot_token: "telegram-token".to_string(),
                allowed_users: vec!["u1".to_string(), "u2".to_string()],
                default_account: None,
                default_to: None,
                security: crate::config::ChannelSecurityPolicy {
                    autonomy_level: Some(crate::security::AutonomyLevel::Supervised),
                    tool_allowlist: Some(vec!["file_read".to_string()]),
                },
            }),
            webhook: Some(crate::config::WebhookConfig {
                port: 9090,
                secret: Some("webhook-secret".to_string()),
                security: crate::config::ChannelSecurityPolicy {
                    autonomy_level: Some(crate::security::AutonomyLevel::Supervised),
                    tool_allowlist: Some(vec!["http_fetch".to_string()]),
                },
            }),
            ..crate::config::ChannelsConfig::default()
        },
        ..Config::default()
    };

    let serialized = toml::to_string(&config).expect("serialize config");
    let deserialized: Config = toml::from_str(&serialized).expect("deserialize config");

    assert!(!deserialized.channels_config.cli);
    assert_eq!(
        deserialized.channels_config.disabled_channels,
        vec!["webhook".to_string()]
    );
    assert_eq!(deserialized.channels_config.coalescing_window_ms, 250);
    assert_eq!(deserialized.channels_config.routing_rules.len(), 1);
    assert_eq!(deserialized.channels_config.group_isolation_rules.len(), 1);
    assert_eq!(
        deserialized
            .channels_config
            .telegram
            .as_ref()
            .and_then(|cfg| cfg.security.autonomy_level),
        Some(crate::security::AutonomyLevel::Supervised)
    );
    assert_eq!(
        deserialized
            .channels_config
            .webhook
            .as_ref()
            .and_then(|cfg| cfg.secret.as_deref()),
        Some("webhook-secret")
    );
}

#[test]
fn config_round_trip_preserves_memory_and_autonomy_sections() {
    let config = Config {
        memory: crate::config::MemoryConfig {
            backend: crate::config::MemoryBackend::Markdown,
            postgres_url: None,
            pg_max_connections: 10,
            pg_connect_timeout_secs: 5,
            pg_idle_timeout_secs: 300,
            pg_min_connections: 1,
            pg_max_lifetime_secs: 1800,
            pg_hnsw_ef_search: 100,
            auto_save: false,
            hygiene_enabled: false,
            archive_after_days: 2,
            purge_after_days: 9,
            conversation_retention_days: 45,
            layer_retention_working_days: Some(5),
            layer_retention_episodic_days: Some(20),
            layer_retention_semantic_days: Some(60),
            layer_retention_procedural_days: Some(90),
            layer_retention_identity_days: Some(365),
            ledger_retention_days: Some(30),
            embedding_provider: crate::config::EmbeddingProvider::None,
            embedding_model: "text-embedding-3-small".to_string(),
            embedding_dimensions: 1536,
            vector_weight: 0.6,
            keyword_weight: 0.4,
            graph_retrieval_fusion_enabled: true,
            graph_retrieval_weight: 0.2,
            embedding_cache_size: 512,
            chunk_max_tokens: 128,
            recall_min_confidence: 0.3,
            working_memory_capacity: 50,
        },
        autonomy: crate::config::AutonomyConfig {
            level: crate::security::AutonomyLevel::Full,
            external_action_execution: crate::security::ExternalActionExecution::Enabled,
            workspace_only: false,
            allowed_commands: vec!["git".to_string(), "cargo".to_string()],
            forbidden_paths: vec!["/etc".to_string(), "~/.ssh".to_string()],
            max_actions_per_hour: 99,
            max_actions_per_entity_per_hour: 30,
            max_cost_per_day_cents: 999,
            verify_repair_max_attempts: 4,
            verify_repair_max_repair_depth: 2,
            max_tool_loop_iterations: 15,
            ..crate::config::AutonomyConfig::default()
        },
        ..Config::default()
    };

    let serialized = toml::to_string(&config).expect("serialize config");
    let deserialized: Config = toml::from_str(&serialized).expect("deserialize config");

    assert_eq!(
        deserialized.memory.backend,
        crate::config::MemoryBackend::Markdown
    );
    assert!(!deserialized.memory.auto_save);
    assert_eq!(deserialized.memory.ledger_retention_days, Some(30));
    assert_eq!(
        deserialized.autonomy.external_action_execution,
        crate::security::ExternalActionExecution::Enabled
    );
    assert!(!deserialized.autonomy.workspace_only);
    assert_eq!(deserialized.autonomy.max_tool_loop_iterations, 15);
}

#[test]
fn config_round_trip_preserves_runtime_and_secrets_sections() {
    let config = Config {
        runtime: RuntimeConfig {
            kind: RuntimeKind::Docker,
            enable_docker_runtime: true,
            enable_live_settings_reload: false,
            sandbox_selector: SandboxSelectorMode::Auto,
        },
        secrets: SecretsConfig {
            encrypt: false,
            ..SecretsConfig::default()
        },
        ..Config::default()
    };

    let serialized = toml::to_string(&config).expect("serialize config");
    let deserialized: Config = toml::from_str(&serialized).expect("deserialize config");

    assert_eq!(deserialized.runtime.kind, RuntimeKind::Docker);
    assert!(deserialized.runtime.enable_docker_runtime);
    assert!(!deserialized.runtime.enable_live_settings_reload);
    assert_eq!(
        deserialized.runtime.sandbox_selector,
        SandboxSelectorMode::Auto
    );
    assert!(!deserialized.secrets.encrypt);
}

#[test]
fn validate_autonomy_controls_rejects_partial_scheduler_active_hours() {
    let mut config = Config::default();
    config.reliability.scheduler_active_hours_start_utc = Some("09:00".to_string());
    config.reliability.scheduler_active_hours_end_utc = None;

    let error = config
        .validate_autonomy_controls()
        .expect_err("partial active-hours config should be rejected");
    assert!(error.to_string().contains("must be set together"));
}

#[test]
fn validate_autonomy_controls_rejects_invalid_scheduler_active_hours_format() {
    let mut config = Config::default();
    config.reliability.scheduler_active_hours_start_utc = Some("9am".to_string());
    config.reliability.scheduler_active_hours_end_utc = Some("17:00".to_string());

    let error = config
        .validate_autonomy_controls()
        .expect_err("invalid active-hours format should be rejected");
    assert!(error.to_string().contains("HH:MM"));
}

#[test]
fn validate_autonomy_controls_accepts_valid_scheduler_active_hours() {
    let mut config = Config::default();
    config.reliability.scheduler_active_hours_start_utc = Some("09:00".to_string());
    config.reliability.scheduler_active_hours_end_utc = Some("17:00".to_string());

    assert!(config.validate_autonomy_controls().is_ok());
}

#[test]
fn validate_autonomy_controls_rejects_model_list_without_provider_model_format() {
    let config = Config {
        model_list: vec![ModelListEntry {
            model_name: "alias-model".to_string(),
            model: "gpt-5.2".to_string(),
            api_key: None,
            api_base: None,
        }],
        ..Config::default()
    };

    let error = config
        .validate_autonomy_controls()
        .expect_err("model_list entries must use provider/model format");
    assert!(error.to_string().contains("provider/model format"));
}

#[test]
fn validate_autonomy_controls_rejects_unknown_provider_without_api_base() {
    let config = Config {
        model_list: vec![ModelListEntry {
            model_name: "alias-model".to_string(),
            model: "myproxy/gpt-5.2".to_string(),
            api_key: None,
            api_base: None,
        }],
        ..Config::default()
    };

    let error = config
        .validate_autonomy_controls()
        .expect_err("unknown provider should require api_base");
    assert!(error.to_string().contains("requires api_base"));
}

#[test]
fn validate_autonomy_controls_accepts_unknown_provider_with_api_base() {
    let config = Config {
        model_list: vec![ModelListEntry {
            model_name: "alias-model".to_string(),
            model: "myproxy/gpt-5.2".to_string(),
            api_key: None,
            api_base: Some("https://proxy.example.com/v1".to_string()),
        }],
        ..Config::default()
    };

    assert!(config.validate_autonomy_controls().is_ok());
}

#[test]
fn validate_autonomy_controls_rejects_invalid_loop_detection_thresholds() {
    let mut config = Config::default();
    config.tools.loop_detection = crate::config::LoopDetectionConfig {
        enabled: true,
        history_size: 8,
        warning_threshold: 5,
        critical_threshold: 3,
        repeat: true,
        ping_pong: true,
        no_progress: true,
    };

    let error = config
        .validate_autonomy_controls()
        .expect_err("loop detection warning threshold must not exceed critical threshold");
    assert!(error.to_string().contains("warning_threshold"));
}

#[test]
fn validate_autonomy_controls_rejects_too_small_parent_fork_token_limit() {
    let mut config = Config::default();
    config.session.parent_fork_max_tokens = 512;

    let error = config
        .validate_autonomy_controls()
        .expect_err("small parent fork limit should be rejected");
    assert!(error.to_string().contains("parent_fork_max_tokens"));
}

#[test]
fn validate_autonomy_controls_accepts_email_channel_policy_fields() {
    let mut config = Config::default();
    config.channels_config.email = Some(crate::config::EmailConfig {
        imap_host: "imap.example.com".to_string(),
        smtp_host: "smtp.example.com".to_string(),
        username: "bot@example.com".to_string(),
        password: "secret".to_string(),
        from_address: "bot@example.com".to_string(),
        security: crate::config::ChannelSecurityPolicy {
            autonomy_level: Some(crate::security::AutonomyLevel::ReadOnly),
            tool_allowlist: None,
        },
        ..crate::config::EmailConfig::default()
    });

    assert!(config.validate_autonomy_controls().is_ok());
}

#[test]
fn resolve_model_selection_uses_model_list_alias_provider_and_api_key() {
    let config = Config {
        default_provider: Some("openrouter".to_string()),
        default_model: Some("workspace-default".to_string()),
        model_list: vec![ModelListEntry {
            model_name: "workspace-default".to_string(),
            model: "openai/gpt-5.4".to_string(),
            api_key: Some("sk-registry".to_string()),
            api_base: None,
        }],
        ..Config::default()
    };

    let selection = config.resolve_model(None, None);
    assert_eq!(selection.provider, "openai");
    assert_eq!(selection.model, "gpt-5.4");
    assert_eq!(selection.api_key.as_deref(), Some("sk-registry"));
    assert!(selection.api_base.is_none());
}

#[test]
fn resolve_model_selection_keeps_provider_override_for_alias_model() {
    let config = Config {
        default_provider: Some("openrouter".to_string()),
        default_model: Some("workspace-default".to_string()),
        model_list: vec![ModelListEntry {
            model_name: "workspace-default".to_string(),
            model: "openai/gpt-5.4".to_string(),
            api_key: Some("sk-registry".to_string()),
            api_base: None,
        }],
        ..Config::default()
    };

    let selection = config.resolve_model(Some("openrouter"), None);
    assert_eq!(selection.provider, "openrouter");
    assert_eq!(selection.model, "gpt-5.4");
    assert_eq!(selection.api_key.as_deref(), Some("sk-registry"));
    assert!(selection.api_base.is_none());
}

#[test]
fn resolve_model_selection_keeps_openai_codex_selector_for_openai_alias_model() {
    let config = Config {
        default_provider: Some("openai-codex".to_string()),
        default_model: Some("workspace-default".to_string()),
        model_list: vec![ModelListEntry {
            model_name: "workspace-default".to_string(),
            model: "openai/gpt-5.3-codex".to_string(),
            api_key: Some("sk-registry".to_string()),
            api_base: None,
        }],
        ..Config::default()
    };

    let selection = config.resolve_model(None, None);
    assert_eq!(selection.provider, "openai-codex");
    assert_eq!(selection.model, "gpt-5.3-codex");
    assert_eq!(selection.api_key.as_deref(), Some("sk-registry"));
    assert!(selection.api_base.is_none());
}

#[test]
fn resolve_model_selection_preserves_api_base_for_alias() {
    let config = Config {
        default_provider: Some("openrouter".to_string()),
        default_model: Some("workspace-default".to_string()),
        model_list: vec![ModelListEntry {
            model_name: "workspace-default".to_string(),
            model: "myproxy/gpt-5.2".to_string(),
            api_key: Some("sk-registry".to_string()),
            api_base: Some("https://proxy.example.com/v1".to_string()),
        }],
        ..Config::default()
    };

    let selection = config.resolve_model(None, None);
    assert_eq!(selection.provider, "custom:https://proxy.example.com/v1");
    assert_eq!(selection.model, "gpt-5.2");
    assert_eq!(selection.api_key.as_deref(), Some("sk-registry"));
    assert_eq!(
        selection.api_base.as_deref(),
        Some("https://proxy.example.com/v1")
    );
}

#[test]
fn runtime_config_auto_kind_resolves_to_native_without_docker_gate() {
    let runtime = RuntimeConfig {
        kind: RuntimeKind::Auto,
        enable_docker_runtime: false,
        ..RuntimeConfig::default()
    };

    assert_eq!(runtime.resolved_runtime_kind(), RuntimeKind::Native);
}

#[test]
fn runtime_config_auto_kind_resolves_to_docker_with_docker_gate() {
    let runtime = RuntimeConfig {
        kind: RuntimeKind::Auto,
        enable_docker_runtime: true,
        ..RuntimeConfig::default()
    };

    assert_eq!(runtime.resolved_runtime_kind(), RuntimeKind::Docker);
}

#[test]
fn runtime_config_auto_sandbox_selector_relaxes_workspace_only_for_docker() {
    let runtime = RuntimeConfig {
        kind: RuntimeKind::Docker,
        enable_docker_runtime: true,
        sandbox_selector: SandboxSelectorMode::Auto,
        ..RuntimeConfig::default()
    };

    assert!(!runtime.resolved_workspace_only(true));
}

mod proptest_cases {
    use proptest::prelude::*;

    use crate::config::AutonomyConfig;

    proptest! {
        #[test]
        fn autonomy_config_toml_roundtrip(
            max_actions in 0u32..10000,
            max_cost in 0u32..10000,
            workspace_only in proptest::bool::ANY,
        ) {
            let config = AutonomyConfig {
                max_actions_per_hour: max_actions,
                max_cost_per_day_cents: max_cost,
                workspace_only,
                ..AutonomyConfig::default()
            };
            let toml_str = toml::to_string(&config).unwrap();
            let parsed: AutonomyConfig = toml::from_str(&toml_str).unwrap();
            prop_assert_eq!(parsed.max_actions_per_hour, config.max_actions_per_hour);
            prop_assert_eq!(parsed.max_cost_per_day_cents, config.max_cost_per_day_cents);
            prop_assert_eq!(parsed.workspace_only, config.workspace_only);
        }
    }
}
