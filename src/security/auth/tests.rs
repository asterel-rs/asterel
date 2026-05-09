//! Unit tests for the auth subsystem (broker, store, profiles).

use std::fs;

use super::*;
use crate::config::Config;

fn test_config(tmp: &tempfile::TempDir) -> Config {
    Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Config::default()
    }
}

#[test]
fn broker_prefers_provider_profile_over_config_api_key() {
    let tmp = tempfile::TempDir::new().unwrap();
    let _generic_guard =
        crate::utils::test_env::EnvVarGuard::unset("ASTEREL_API_KEY").expect("env guard");
    let mut config = test_config(&tmp);
    config.default_provider = Some("openrouter".into());
    config.api_key = Some("sk-config".into());
    config.secrets.encrypt = true;

    let path = auth_profiles_path(&config);
    fs::write(
        &path,
        r#"{
  "version": 1,
  "defaults": {
    "openrouter": "or-default",
    "openai": "oa-default"
  },
  "profiles": [
    {
      "id": "or-default",
      "provider": "openrouter",
      "api_key": "sk-openrouter-profile"
    },
    {
      "id": "oa-default",
      "provider": "openai",
      "api_key": "sk-openai-profile"
    }
  ]
}"#,
    )
    .unwrap();

    let broker = AuthBroker::load_or_init(&config).unwrap();
    assert_eq!(
        broker.resolve_provider_key("openrouter").as_deref(),
        Some("sk-openrouter-profile")
    );
    assert_eq!(
        broker.resolve_provider_key("openai").as_deref(),
        Some("sk-openai-profile")
    );
    assert_eq!(
        broker.resolve_provider_key("anthropic").as_deref(),
        Some("sk-config")
    );

    let persisted = fs::read_to_string(path).unwrap();
    assert!(persisted.contains("enc2:"));
    assert!(!persisted.contains("sk-openrouter-profile"));
    assert!(!persisted.contains("sk-openai-profile"));
}

#[test]
fn broker_resolves_embedding_key_from_openai_profile() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.default_provider = Some("openrouter".into());
    config.api_key = Some("sk-config".into());
    config.memory.embedding_provider = crate::config::EmbeddingProvider::OpenAi;

    let path = auth_profiles_path(&config);
    fs::write(
        &path,
        r#"{
  "version": 1,
  "defaults": {
    "openai": "oa-default"
  },
  "profiles": [
    {
      "id": "oa-default",
      "provider": "openai",
      "api_key": "sk-openai-profile"
    }
  ]
}"#,
    )
    .unwrap();

    let broker = AuthBroker::load_or_init(&config).unwrap();
    assert_eq!(
        broker.resolve_memory_api_key(&config.memory).as_deref(),
        Some("sk-openai-profile")
    );
}

#[test]
fn broker_resolves_embedding_key_from_custom_openai_compatible_provider() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.default_provider = Some("openrouter".into());
    config.memory.embedding_provider =
        crate::config::EmbeddingProvider::OpenAiCompatible("https://embed.example".into());

    let path = auth_profiles_path(&config);
    fs::write(
        &path,
        r#"{
  "version": 1,
  "defaults": {
    "openai": "oa-default"
  },
  "profiles": [
    {
      "id": "oa-default",
      "provider": "openai",
      "api_key": "sk-openai-profile"
    }
  ]
}"#,
    )
    .unwrap();

    let broker = AuthBroker::load_or_init(&config).unwrap();
    assert_eq!(
        broker.resolve_memory_api_key(&config.memory).as_deref(),
        Some("sk-openai-profile")
    );
}

#[test]
fn broker_resolves_embedding_key_from_provider_specific_env_var() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let _env_guard = crate::utils::test_env::EnvVarGuard::set("GEMINI_API_KEY", "gem-env-key")
        .expect("env guard should acquire lock");

    let mut config = config;
    config.memory.embedding_provider = crate::config::EmbeddingProvider::Gemini;

    let broker = AuthBroker::load_or_init(&config).unwrap();
    assert_eq!(
        broker.resolve_memory_api_key(&config.memory).as_deref(),
        Some("gem-env-key")
    );
}

#[test]
fn broker_resolves_embedding_key_from_embedding_only_env_var() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);
    let _env_guard = crate::utils::test_env::EnvVarGuard::set("VOYAGE_API_KEY", "voyage-env-key")
        .expect("env guard should acquire lock");

    let mut config = config;
    config.memory.embedding_provider = crate::config::EmbeddingProvider::Voyage;

    let broker = AuthBroker::load_or_init(&config).unwrap();
    assert_eq!(
        broker.resolve_memory_api_key(&config.memory).as_deref(),
        Some("voyage-env-key")
    );
}

#[test]
fn upsert_profile_sets_provider_default_and_normalizes_values() {
    let mut store = AuthProfileStore::default();

    let created = store
        .upsert_profile(
            AuthProfile {
                id: "openai-main".into(),
                provider: "OpenAI".into(),
                auth_route: None,
                label: Some("  Primary Key  ".into()),
                api_key: Some("  sk-openai-main  ".into()),
                refresh_token: Some("  refresh-main  ".into()),
                auth_scheme: Some("  OAuth  ".into()),
                oauth_source: Some("  codex  ".into()),
                is_disabled: true,
            },
            true,
        )
        .unwrap();

    assert!(created);
    assert_eq!(store.profiles.len(), 1);
    assert_eq!(store.profiles[0].provider, "openai");
    assert_eq!(store.profiles[0].label.as_deref(), Some("Primary Key"));
    assert_eq!(store.profiles[0].api_key.as_deref(), Some("sk-openai-main"));
    assert_eq!(
        store.profiles[0].refresh_token.as_deref(),
        Some("refresh-main")
    );
    assert_eq!(store.profiles[0].auth_scheme.as_deref(), Some("oauth"));
    assert_eq!(store.profiles[0].oauth_source.as_deref(), Some("codex"));
    assert_eq!(store.profiles[0].auth_route.as_deref(), Some("codex"));
    assert!(!store.profiles[0].is_disabled);
    assert_eq!(
        store.defaults.get("openai@codex"),
        Some(&"openai-main".to_string())
    );
}

#[test]
fn upsert_profile_rejects_invalid_id() {
    let mut store = AuthProfileStore::default();
    let result = store.upsert_profile(
        AuthProfile {
            id: "bad id".into(),
            provider: "openrouter".into(),
            auth_route: None,
            label: None,
            api_key: Some("sk-test".into()),
            refresh_token: None,
            auth_scheme: Some("api_key".into()),
            oauth_source: None,
            is_disabled: false,
        },
        true,
    );

    assert!(result.is_err());
}

#[test]
fn save_encrypts_refresh_token_in_store() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.secrets.encrypt = true;

    let mut store = AuthProfileStore::default();
    store
        .upsert_profile(
            AuthProfile {
                id: "openai-oauth".into(),
                provider: "openai".into(),
                auth_route: None,
                label: Some("OAuth import".into()),
                api_key: Some("access-token-plaintext".into()),
                refresh_token: Some("refresh-token-plaintext".into()),
                auth_scheme: Some("oauth".into()),
                oauth_source: Some("codex".into()),
                is_disabled: false,
            },
            true,
        )
        .unwrap();

    store.save_for_config(&config).unwrap();
    let persisted = fs::read_to_string(auth_profiles_path(&config)).unwrap();

    assert!(persisted.contains("enc2:"));
    assert!(!persisted.contains("access-token-plaintext"));
    assert!(!persisted.contains("refresh-token-plaintext"));
}

#[test]
fn decrypt_failure_records_operator_visible_disabled_reason() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.secrets.encrypt = true;
    let path = auth_profiles_path(&config);
    fs::write(
        &path,
        r#"{
  "version": 1,
  "profiles": [
    {
      "id": "openai-main",
      "provider": "openai",
      "api_key": "enc2:not-valid-ciphertext"
    }
  ]
}"#,
    )
    .unwrap();

    let store = AuthProfileStore::load_or_init_cfg(&config).unwrap();
    let profile = store
        .profiles
        .iter()
        .find(|profile| profile.id == "openai-main")
        .expect("profile should load in disabled state");
    assert!(profile.is_disabled);
    assert_eq!(
        store
            .usage_stats
            .get("openai-main")
            .and_then(|stats| stats.disabled_reason.as_deref()),
        Some("api_key decrypt failed; profile disabled")
    );
}

#[test]
fn recover_oauth_profile_returns_false_for_non_oauth_profile() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    let mut store = AuthProfileStore::default();
    store
        .upsert_profile(
            AuthProfile {
                id: "openai-main".into(),
                provider: "openai".into(),
                auth_route: None,
                label: None,
                api_key: Some("sk-main".into()),
                refresh_token: None,
                auth_scheme: Some("api_key".into()),
                oauth_source: None,
                is_disabled: false,
            },
            true,
        )
        .unwrap();
    store.save_for_config(&config).unwrap();

    let recovered = recover_oauth_profile_for_provider(&config, "openai").unwrap();
    assert!(!recovered);

    assert_eq!(
        recover_oauth_profile_for_provider_with_outcome(&config, "openai").unwrap(),
        OAuthRecoveryOutcome::Skipped(OAuthRecoverySkipReason::NonOAuthProfile)
    );
}

#[test]
fn recover_oauth_profile_returns_false_for_unknown_oauth_source() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    let mut store = AuthProfileStore::default();
    store
        .upsert_profile(
            AuthProfile {
                id: "openai-oauth".into(),
                provider: "openai".into(),
                auth_route: None,
                label: None,
                api_key: Some("access-old".into()),
                refresh_token: Some("refresh-old".into()),
                auth_scheme: Some("oauth".into()),
                oauth_source: Some("custom-source".into()),
                is_disabled: false,
            },
            true,
        )
        .unwrap();
    store.save_for_config(&config).unwrap();

    let recovered = recover_oauth_profile_for_provider(&config, "openai").unwrap();
    assert!(!recovered);

    assert_eq!(
        recover_oauth_profile_for_provider_with_outcome(&config, "openai").unwrap(),
        OAuthRecoveryOutcome::Skipped(OAuthRecoverySkipReason::MissingCachedCredential)
    );
}

#[test]
fn recover_oauth_profile_reports_disabled_profile_reason() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    let mut store = AuthProfileStore::default();
    store
        .upsert_profile(
            AuthProfile {
                id: "openai-oauth".into(),
                provider: "openai".into(),
                auth_route: None,
                label: None,
                api_key: Some("access-old".into()),
                refresh_token: Some("refresh-old".into()),
                auth_scheme: Some("oauth".into()),
                oauth_source: Some("codex".into()),
                is_disabled: false,
            },
            true,
        )
        .unwrap();
    store.profiles[0].is_disabled = true;
    store.save_for_config(&config).unwrap();

    assert_eq!(
        recover_oauth_profile_for_provider_with_outcome(&config, "openai").unwrap(),
        OAuthRecoveryOutcome::Skipped(OAuthRecoverySkipReason::ProfileDisabled)
    );
}

#[test]
fn broker_resolve_provider_key_reloads_profile_store_updates() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = test_config(&tmp);

    let mut store = AuthProfileStore::default();
    store
        .upsert_profile(
            AuthProfile {
                id: "openai-main".into(),
                provider: "openai".into(),
                auth_route: None,
                label: None,
                api_key: Some("sk-old".into()),
                refresh_token: None,
                auth_scheme: Some("api_key".into()),
                oauth_source: None,
                is_disabled: false,
            },
            true,
        )
        .unwrap();
    store.save_for_config(&config).unwrap();

    let broker = AuthBroker::load_or_init(&config).unwrap();
    assert_eq!(
        broker.resolve_provider_key("openai").as_deref(),
        Some("sk-old")
    );

    store
        .upsert_profile(
            AuthProfile {
                id: "openai-main".into(),
                provider: "openai".into(),
                auth_route: None,
                label: None,
                api_key: Some("sk-new".into()),
                refresh_token: None,
                auth_scheme: Some("api_key".into()),
                oauth_source: None,
                is_disabled: false,
            },
            true,
        )
        .unwrap();
    store.save_for_config(&config).unwrap();

    assert_eq!(
        broker.resolve_provider_key("openai").as_deref(),
        Some("sk-new")
    );
}

#[test]
fn active_profile_prefers_configured_order_when_default_missing() {
    let mut store = AuthProfileStore::default();
    store
        .upsert_profile(
            AuthProfile {
                id: "openai-a".into(),
                provider: "openai".into(),
                auth_route: None,
                label: None,
                api_key: Some("sk-a".into()),
                refresh_token: None,
                auth_scheme: Some("api_key".into()),
                oauth_source: None,
                is_disabled: false,
            },
            false,
        )
        .unwrap();
    store
        .upsert_profile(
            AuthProfile {
                id: "openai-b".into(),
                provider: "openai".into(),
                auth_route: None,
                label: None,
                api_key: Some("sk-b".into()),
                refresh_token: None,
                auth_scheme: Some("api_key".into()),
                oauth_source: None,
                is_disabled: false,
            },
            false,
        )
        .unwrap();

    store
        .defaults
        .insert(auth_target_key("openai", Some("api")), "missing-id".into());
    let order = vec!["openai-b".to_string(), "openai-a".to_string()];
    store.set_profile_order("openai", &order);

    let active = store.active_profile_for_provider("openai").unwrap();
    assert_eq!(active.id, "openai-b");
}

#[test]
fn active_profile_skips_cooldown_and_falls_back_to_ready_profile() {
    let mut store = AuthProfileStore::default();
    store
        .upsert_profile(
            AuthProfile {
                id: "openai-a".into(),
                provider: "openai".into(),
                auth_route: None,
                label: None,
                api_key: Some("sk-a".into()),
                refresh_token: None,
                auth_scheme: Some("api_key".into()),
                oauth_source: None,
                is_disabled: false,
            },
            false,
        )
        .unwrap();
    store
        .upsert_profile(
            AuthProfile {
                id: "openai-b".into(),
                provider: "openai".into(),
                auth_route: None,
                label: None,
                api_key: Some("sk-b".into()),
                refresh_token: None,
                auth_scheme: Some("api_key".into()),
                oauth_source: None,
                is_disabled: false,
            },
            false,
        )
        .unwrap();

    let order = vec!["openai-a".to_string(), "openai-b".to_string()];
    store.set_profile_order("openai", &order);
    store.mark_profile_failed("openai-a", Some(600));

    let active = store.active_profile_for_provider("openai").unwrap();
    assert_eq!(active.id, "openai-b");
}

#[test]
fn mark_profile_used_updates_last_good_and_usage_stats() {
    let mut store = AuthProfileStore::default();
    store
        .upsert_profile(
            AuthProfile {
                id: "anthropic-main".into(),
                provider: "anthropic".into(),
                auth_route: None,
                label: None,
                api_key: Some("sk-ant".into()),
                refresh_token: None,
                auth_scheme: Some("api_key".into()),
                oauth_source: None,
                is_disabled: false,
            },
            false,
        )
        .unwrap();

    store.mark_profile_failed("anthropic-main", Some(60));
    store.mark_profile_used("anthropic-main");

    assert_eq!(
        store
            .last_good
            .get(&auth_target_key("anthropic", Some("api")))
            .map(String::as_str),
        Some("anthropic-main")
    );
    let stats = store.usage_stats.get("anthropic-main").unwrap();
    assert_eq!(stats.error_count, 0);
    assert!(stats.last_used_at.is_some());
    assert!(stats.cooldown_until.is_none());
}

#[test]
fn openai_codex_selector_prefers_codex_route() {
    let mut store = AuthProfileStore::default();
    store
        .upsert_profile(
            AuthProfile {
                id: "openai-api".into(),
                provider: "openai".into(),
                auth_route: None,
                label: None,
                api_key: Some("sk-api".into()),
                refresh_token: None,
                auth_scheme: Some("api_key".into()),
                oauth_source: None,
                is_disabled: false,
            },
            false,
        )
        .unwrap();
    store
        .upsert_profile(
            AuthProfile {
                id: "openai-codex".into(),
                provider: "openai".into(),
                auth_route: None,
                label: None,
                api_key: Some("sk-codex".into()),
                refresh_token: Some("refresh-codex".into()),
                auth_scheme: Some("oauth".into()),
                oauth_source: Some("codex".into()),
                is_disabled: false,
            },
            false,
        )
        .unwrap();

    let active = store.active_profile_for_provider("openai-codex").unwrap();
    assert_eq!(active.id, "openai-codex");
    assert_eq!(active.api_key.as_deref(), Some("sk-codex"));
}

#[test]
fn openai_codex_selector_falls_back_to_api_route() {
    let mut store = AuthProfileStore::default();
    store
        .upsert_profile(
            AuthProfile {
                id: "openai-api".into(),
                provider: "openai".into(),
                auth_route: None,
                label: None,
                api_key: Some("sk-api".into()),
                refresh_token: None,
                auth_scheme: Some("api_key".into()),
                oauth_source: None,
                is_disabled: false,
            },
            false,
        )
        .unwrap();

    let active = store.active_profile_for_provider("openai-codex").unwrap();
    assert_eq!(active.id, "openai-api");
    assert_eq!(active.api_key.as_deref(), Some("sk-api"));
}

#[test]
fn set_profile_order_for_openai_codex_targets_codex_profiles_when_present() {
    let mut store = AuthProfileStore::default();
    for (id, api_key) in [
        ("openai-codex-a", "sk-codex-a"),
        ("openai-codex-b", "sk-codex-b"),
    ] {
        store
            .upsert_profile(
                AuthProfile {
                    id: id.into(),
                    provider: "openai".into(),
                    auth_route: None,
                    label: None,
                    api_key: Some(api_key.into()),
                    refresh_token: Some(format!("refresh-{id}")),
                    auth_scheme: Some("oauth".into()),
                    oauth_source: Some("codex".into()),
                    is_disabled: false,
                },
                false,
            )
            .unwrap();
    }
    store
        .upsert_profile(
            AuthProfile {
                id: "openai-api".into(),
                provider: "openai".into(),
                auth_route: None,
                label: None,
                api_key: Some("sk-api".into()),
                refresh_token: None,
                auth_scheme: Some("api_key".into()),
                oauth_source: None,
                is_disabled: false,
            },
            false,
        )
        .unwrap();

    let order = vec![
        "openai-codex-b".to_string(),
        "openai-api".to_string(),
        "openai-codex-a".to_string(),
    ];
    store.set_profile_order("openai-codex", &order);

    assert_eq!(
        store.order.get(&auth_target_key("openai", Some("codex"))),
        Some(&vec![
            "openai-codex-b".to_string(),
            "openai-codex-a".to_string()
        ])
    );
}

#[test]
fn set_profile_order_for_openai_codex_falls_back_to_api_profiles_when_needed() {
    let mut store = AuthProfileStore::default();
    for (id, api_key) in [("openai-api-a", "sk-api-a"), ("openai-api-b", "sk-api-b")] {
        store
            .upsert_profile(
                AuthProfile {
                    id: id.into(),
                    provider: "openai".into(),
                    auth_route: None,
                    label: None,
                    api_key: Some(api_key.into()),
                    refresh_token: None,
                    auth_scheme: Some("api_key".into()),
                    oauth_source: None,
                    is_disabled: false,
                },
                false,
            )
            .unwrap();
    }

    let order = vec!["openai-api-b".to_string(), "openai-api-a".to_string()];
    store.set_profile_order("openai-codex", &order);

    assert_eq!(
        store.order.get(&auth_target_key("openai", Some("api"))),
        Some(&order)
    );
}

#[test]
fn default_profile_id_for_openai_uses_api_target_key() {
    let mut store = AuthProfileStore::default();
    store
        .upsert_profile(
            AuthProfile {
                id: "openai-default".into(),
                provider: "openai".into(),
                auth_route: None,
                label: None,
                api_key: Some("sk-openai".into()),
                refresh_token: None,
                auth_scheme: Some("api_key".into()),
                oauth_source: None,
                is_disabled: false,
            },
            true,
        )
        .unwrap();

    assert_eq!(
        store.default_profile_id_for_provider("openai"),
        Some("openai-default")
    );
    assert_eq!(
        store.effective_target_key_for_provider("openai"),
        "openai@api"
    );
}

#[test]
fn default_profile_id_for_openai_codex_falls_back_to_api_route_when_needed() {
    let mut store = AuthProfileStore::default();
    store
        .upsert_profile(
            AuthProfile {
                id: "openai-api".into(),
                provider: "openai".into(),
                auth_route: None,
                label: None,
                api_key: Some("sk-openai".into()),
                refresh_token: None,
                auth_scheme: Some("api_key".into()),
                oauth_source: None,
                is_disabled: false,
            },
            true,
        )
        .unwrap();

    assert_eq!(
        store.default_profile_id_for_provider("openai-codex"),
        Some("openai-api")
    );
    assert_eq!(
        store.effective_target_key_for_provider("openai-codex"),
        "openai@api"
    );
}
