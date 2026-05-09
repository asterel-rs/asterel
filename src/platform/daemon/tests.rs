//! Tests for daemon lifecycle, persona startup, and skills
//! refresh logic.

use std::path::Path;
use std::sync::Arc;

use tempfile::TempDir;

use super::run::{
    has_supervised_channels, initialize_persona_startup_state, should_apply_skills_refresh,
};
use crate::config::{ChannelSecurityPolicy, Config};
use crate::core::memory::create_memory;
use crate::core::persona::state_header::StateHeader;
use crate::core::persona::state_persistence::BackendHeaderPersist;

#[test]
fn should_apply_skills_refresh_only_when_fingerprint_changes() {
    assert!(!should_apply_skills_refresh(None, 42));
    assert!(!should_apply_skills_refresh(Some(42), 42));
    assert!(should_apply_skills_refresh(Some(41), 42));
}

fn custom_state() -> StateHeader {
    StateHeader {
        identity_principles_hash: "identity-v1-abcd1234".to_string(),
        safety_posture: "strict".to_string(),
        current_objective: "reconcile from backend canonical".to_string(),
        open_loops: vec!["startup reconcile".to_string()],
        next_actions: vec!["repair mirror".to_string()],
        commitments: vec!["preserve canonical source".to_string()],
        recent_context_summary: "daemon startup test".to_string(),
        last_updated_at: "2026-02-23T00:00:00Z".to_string(),
    }
}

fn test_config(workspace: &Path, enabled_main_session: bool) -> Config {
    Config {
        workspace_dir: workspace.to_path_buf(),
        memory: crate::config::MemoryConfig {
            backend: crate::config::MemoryBackend::Markdown,
            ..crate::config::MemoryConfig::default()
        },
        persona: crate::config::PersonaConfig {
            enabled_main_session,
            ..crate::config::PersonaConfig::default()
        },
        ..Config::default()
    }
}

#[tokio::test]
async fn initialize_persona_startup_state_seeds_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(tmp.path(), true);

    initialize_persona_startup_state(&config)
        .await
        .expect("startup reconcile should succeed");

    let mirror_path = config
        .workspace_dir
        .join(&config.persona.state_mirror_filename);
    assert!(mirror_path.exists());

    let memory = create_memory(&config.memory, &config.workspace_dir, None)
        .await
        .unwrap();
    let slot = memory
        .resolve_slot(
            "person:local-default",
            "persona/local-default/state_header/v1",
        )
        .await
        .unwrap();
    assert!(slot.is_some());
}

#[tokio::test]
async fn initialize_persona_startup_state_noop_when_disabled() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(tmp.path(), false);

    initialize_persona_startup_state(&config)
        .await
        .expect("disabled path should no-op");

    let mirror_path = config
        .workspace_dir
        .join(&config.persona.state_mirror_filename);
    assert!(!mirror_path.exists());
}

#[tokio::test]
async fn initialize_persona_startup_state_disabled_preserves_existing_mirror() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(tmp.path(), false);

    let mirror_path = config
        .workspace_dir
        .join(&config.persona.state_mirror_filename);
    std::fs::write(&mirror_path, "{\"state_header\":\"existing\"}").unwrap();

    initialize_persona_startup_state(&config)
        .await
        .expect("disabled path should preserve existing mirror");

    let mirror_raw = std::fs::read_to_string(&mirror_path).unwrap();
    assert_eq!(mirror_raw, "{\"state_header\":\"existing\"}");
}

#[tokio::test]
async fn initialize_persona_startup_state_repairs_corrupt_mirror_from_backend() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(tmp.path(), true);

    let memory = create_memory(&config.memory, &config.workspace_dir, None)
        .await
        .unwrap();
    let persistence = BackendHeaderPersist::new(
        Arc::from(memory),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "local-default",
    );
    persistence
        .persist_backend_sync(&custom_state())
        .await
        .expect("seed custom canonical state");

    let mirror_path = config
        .workspace_dir
        .join(&config.persona.state_mirror_filename);
    std::fs::write(&mirror_path, "{\"state_header\":\"corrupt\"}").unwrap();

    initialize_persona_startup_state(&config)
        .await
        .expect("startup reconcile should repair mirror");

    let mirror_raw = std::fs::read_to_string(&mirror_path).unwrap();
    assert!(mirror_raw.contains("reconcile from backend canonical"));
    assert!(!mirror_raw.contains("\"state_header\":\"corrupt\""));
}

#[tokio::test]
async fn initialize_persona_startup_state_recreates_missing_mirror_from_backend() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(tmp.path(), true);

    let memory = create_memory(&config.memory, &config.workspace_dir, None)
        .await
        .unwrap();
    let persistence = BackendHeaderPersist::new(
        Arc::from(memory),
        config.workspace_dir.clone(),
        config.persona.clone(),
        "local-default",
    );
    persistence
        .persist_backend_sync(&custom_state())
        .await
        .expect("seed custom canonical state");

    let mirror_path = config
        .workspace_dir
        .join(&config.persona.state_mirror_filename);
    std::fs::remove_file(&mirror_path).unwrap();
    assert!(!mirror_path.exists());

    initialize_persona_startup_state(&config)
        .await
        .expect("startup reconcile should recreate missing mirror");

    let mirror_raw = std::fs::read_to_string(&mirror_path).unwrap();
    assert!(mirror_raw.contains("reconcile from backend canonical"));
}

#[test]
fn supervisor_backoff_clamps_minimum() {
    use super::run::supervisor_backoff;
    let mut config = Config::default();
    config.reliability.channel_initial_backoff_secs = 0;
    config.reliability.channel_max_backoff_secs = 0;
    let (initial, max) = supervisor_backoff(&config);
    assert!(initial >= 1, "initial backoff must be at least 1");
    assert!(max >= initial, "max backoff must be >= initial");
}

#[test]
fn supervisor_backoff_preserves_valid_values() {
    use super::run::supervisor_backoff;
    let mut config = Config::default();
    config.reliability.channel_initial_backoff_secs = 5;
    config.reliability.channel_max_backoff_secs = 30;
    let (initial, max) = supervisor_backoff(&config);
    assert_eq!(initial, 5);
    assert_eq!(max, 30);
}

#[test]
fn supervisor_backoff_max_clamped_to_initial() {
    use super::run::supervisor_backoff;
    let mut config = Config::default();
    config.reliability.channel_initial_backoff_secs = 10;
    config.reliability.channel_max_backoff_secs = 3;
    let (initial, max) = supervisor_backoff(&config);
    assert_eq!(initial, 10);
    assert_eq!(max, 10, "max must be clamped to at least initial");
}

#[test]
fn has_supervised_channels_returns_false_for_defaults() {
    let config = Config::default();
    assert!(!has_supervised_channels(&config));
}

#[test]
#[cfg(feature = "telegram")]
fn has_supervised_channels_detects_telegram() {
    use crate::config::TelegramConfig;
    let mut config = Config::default();
    config.channels_config.telegram = Some(TelegramConfig {
        bot_token: "test-token".into(),
        allowed_users: vec![],
        default_account: None,
        default_to: None,
        security: ChannelSecurityPolicy::default(),
    });
    assert!(has_supervised_channels(&config));
}

#[test]
fn has_supervised_channels_ignores_disabled_listener_channels() {
    use crate::config::TelegramConfig;

    let mut config = Config::default();
    config.channels_config.telegram = Some(TelegramConfig {
        bot_token: "test-token".into(),
        allowed_users: vec![],
        default_account: None,
        default_to: None,
        security: ChannelSecurityPolicy::default(),
    });
    config.channels_config.disabled_channels = vec!["telegram".to_string()];

    assert!(!has_supervised_channels(&config));
}

#[test]
fn has_supervised_channels_detects_discord() {
    use crate::config::DiscordConfig;
    let mut config = Config::default();
    config.channels_config.discord = Some(DiscordConfig {
        bot_token: "test-token".into(),
        application_id: None,
        guild_id: None,
        allowed_users: vec![],
        intents: None,
        status: None,
        default_account: None,
        default_to: None,
        activity_type: None,
        activity_name: None,
        thinking_embed: false,
        thinking_embed_include_preview: false,
        pickup_policy: crate::config::DiscordPickupPolicyConfig::default(),
        security: ChannelSecurityPolicy::default(),
    });
    assert!(has_supervised_channels(&config));
}

#[test]
#[cfg(feature = "irc")]
fn has_supervised_channels_detects_irc() {
    use crate::config::schema::IrcConfig;

    let mut config = Config::default();
    config.channels_config.irc = Some(IrcConfig {
        server: "irc.example.net".to_string(),
        port: 6697,
        nickname: "asterel".to_string(),
        username: Some("asterel".to_string()),
        channels: vec!["#ops".to_string()],
        allowed_users: Vec::new(),
        server_password: None,
        nickserv_password: None,
        sasl_password: None,
        verify_tls: Some(true),
        security: ChannelSecurityPolicy::default(),
    });

    assert!(has_supervised_channels(&config));
}

#[test]
fn should_apply_skills_refresh_none_returns_false() {
    assert!(!should_apply_skills_refresh(None, 0));
    assert!(!should_apply_skills_refresh(None, u64::MAX));
}

#[test]
fn should_apply_skills_refresh_same_returns_false() {
    assert!(!should_apply_skills_refresh(Some(0), 0));
    assert!(!should_apply_skills_refresh(Some(u64::MAX), u64::MAX));
}
