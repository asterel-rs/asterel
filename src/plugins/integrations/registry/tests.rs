//! Unit tests for the integration registry and status functions.

use super::*;
use crate::config::schema::{ChannelSecurityPolicy, IMessageConfig, MatrixConfig, TelegramConfig};
use crate::config::{Config, ModelListEntry};
use crate::core::providers::catalog::ai_integration_provider_entries;
use crate::plugins::integrations::{IntegrationCategory, IntegrationStatus};

#[test]
fn registry_has_entries() {
    let entries = all_integrations();
    assert_eq!(entries.len(), 40, "Expected 40 functional integrations");
}

#[test]
fn implemented_categories_remain_represented() {
    let entries = all_integrations();
    for cat in [
        IntegrationCategory::Chat,
        IntegrationCategory::AiModel,
        IntegrationCategory::ToolsAutomation,
        IntegrationCategory::Platform,
    ] {
        let count = entries.iter().filter(|e| e.category == cat).count();
        assert!(count > 0, "Category {cat:?} has no entries");
    }
}

#[test]
fn status_functions_dont_panic() {
    let config = Config::default();
    let entries = all_integrations();
    for entry in &entries {
        let _ = (entry.status_fn)(&config);
    }
}

#[test]
fn no_duplicate_names() {
    let entries = all_integrations();
    let mut seen = std::collections::HashSet::new();
    for entry in &entries {
        assert!(
            seen.insert(entry.name),
            "Duplicate integration name: {}",
            entry.name
        );
    }
}

#[test]
fn no_empty_names_or_descriptions() {
    let entries = all_integrations();
    for entry in &entries {
        assert!(!entry.name.is_empty(), "Found integration with empty name");
        assert!(
            !entry.description.is_empty(),
            "Integration '{}' has empty description",
            entry.name
        );
    }
}

#[test]
fn ai_model_integrations_use_shared_provider_catalog_metadata() {
    let entries = all_integrations();
    let ai_entries = entries
        .iter()
        .filter(|entry| entry.category == IntegrationCategory::AiModel)
        .collect::<Vec<_>>();

    let curated = ai_integration_provider_entries();
    assert_eq!(ai_entries.len(), curated.len());

    for provider in curated {
        let integration = provider
            .integration
            .as_ref()
            .expect("AI integration provider should expose integration metadata");
        let entry = ai_entries
            .iter()
            .find(|entry| entry.name == integration.name)
            .unwrap_or_else(|| panic!("missing AI integration entry for {}", provider.id));
        assert_eq!(entry.description, integration.description);
    }
}

#[test]
fn telegram_active_when_configured() {
    let mut config = Config::default();
    config.channels_config.telegram = Some(TelegramConfig {
        bot_token: "123:ABC".into(),
        allowed_users: vec!["user".into()],
        default_account: None,
        default_to: None,
        security: ChannelSecurityPolicy::default(),
    });
    let entries = all_integrations();
    let tg = entries.iter().find(|e| e.name == "Telegram").unwrap();
    assert!(matches!((tg.status_fn)(&config), IntegrationStatus::Active));
}

#[test]
fn telegram_available_when_not_configured() {
    let config = Config::default();
    let entries = all_integrations();
    let tg = entries.iter().find(|e| e.name == "Telegram").unwrap();
    assert!(matches!(
        (tg.status_fn)(&config),
        IntegrationStatus::Available
    ));
}

#[test]
fn imessage_active_when_configured() {
    let mut config = Config::default();
    config.channels_config.imessage = Some(IMessageConfig {
        allowed_contacts: vec!["*".into()],
        security: ChannelSecurityPolicy::default(),
    });
    let entries = all_integrations();
    let im = entries.iter().find(|e| e.name == "iMessage").unwrap();
    assert!(matches!((im.status_fn)(&config), IntegrationStatus::Active));
}

#[test]
fn imessage_available_when_not_configured() {
    let config = Config::default();
    let entries = all_integrations();
    let im = entries.iter().find(|e| e.name == "iMessage").unwrap();
    assert!(matches!(
        (im.status_fn)(&config),
        IntegrationStatus::Available
    ));
}

#[test]
fn matrix_active_when_configured() {
    let mut config = Config::default();
    config.channels_config.matrix = Some(MatrixConfig {
        homeserver: "https://m.org".into(),
        access_token: "tok".into(),
        room_id: "!r:m".into(),
        allowed_users: vec![],
        security: ChannelSecurityPolicy::default(),
    });
    let entries = all_integrations();
    let mx = entries.iter().find(|e| e.name == "Matrix").unwrap();
    assert!(matches!((mx.status_fn)(&config), IntegrationStatus::Active));
}

#[test]
fn matrix_available_when_not_configured() {
    let config = Config::default();
    let entries = all_integrations();
    let mx = entries.iter().find(|e| e.name == "Matrix").unwrap();
    assert!(matches!(
        (mx.status_fn)(&config),
        IntegrationStatus::Available
    ));
}

#[test]
fn shell_and_filesystem_always_active() {
    let config = Config::default();
    let entries = all_integrations();
    for name in ["Shell", "File System"] {
        let entry = entries.iter().find(|e| e.name == name).unwrap();
        assert!(
            matches!((entry.status_fn)(&config), IntegrationStatus::Active),
            "{name} should always be Active"
        );
    }
}

#[test]
fn macos_active_on_macos() {
    let config = Config::default();
    let entries = all_integrations();
    let macos = entries.iter().find(|e| e.name == "macOS").unwrap();
    let status = (macos.status_fn)(&config);
    if cfg!(target_os = "macos") {
        assert!(matches!(status, IntegrationStatus::Active));
    } else {
        assert!(matches!(status, IntegrationStatus::Available));
    }
}

#[test]
fn category_counts_reasonable() {
    let entries = all_integrations();
    let chat_count = entries
        .iter()
        .filter(|e| e.category == IntegrationCategory::Chat)
        .count();
    let ai_count = entries
        .iter()
        .filter(|e| e.category == IntegrationCategory::AiModel)
        .count();
    assert!(
        chat_count >= 5,
        "Expected 5+ chat integrations, got {chat_count}"
    );
    assert!(
        ai_count >= 5,
        "Expected 5+ AI model integrations, got {ai_count}"
    );
}

#[test]
fn google_active_for_model_list_alias_with_gemini_provider() {
    let config = Config {
        default_model: Some("workspace-default".to_string()),
        default_provider: None,
        model_list: vec![ModelListEntry {
            model_name: "workspace-default".to_string(),
            model: "gemini/gemini-2.5-pro".to_string(),
            api_key: None,
            api_base: None,
        }],
        ..Config::default()
    };

    let entries = all_integrations();
    let google = entries.iter().find(|e| e.name == "Google").unwrap();
    assert!(matches!(
        (google.status_fn)(&config),
        IntegrationStatus::Active
    ));
}

#[test]
fn google_active_for_legacy_provider_alias() {
    let config = Config {
        default_provider: Some("google-gemini".to_string()),
        default_model: Some("gemini-2.5-pro".to_string()),
        ..Config::default()
    };

    let entries = all_integrations();
    let google = entries.iter().find(|e| e.name == "Google").unwrap();
    assert!(matches!(
        (google.status_fn)(&config),
        IntegrationStatus::Active
    ));
}

#[test]
fn xai_active_for_model_list_with_xai_provider() {
    let config = Config {
        default_model: Some("workspace-default".to_string()),
        default_provider: None,
        model_list: vec![ModelListEntry {
            model_name: "workspace-default".to_string(),
            model: "xai/grok-4".to_string(),
            api_key: None,
            api_base: None,
        }],
        ..Config::default()
    };

    let entries = all_integrations();
    let xai = entries.iter().find(|e| e.name == "xAI").unwrap();
    assert!(matches!(
        (xai.status_fn)(&config),
        IntegrationStatus::Active
    ));
}

#[test]
fn xai_active_for_legacy_provider_alias() {
    let config = Config {
        default_provider: Some("grok".to_string()),
        default_model: Some("grok-4-1-fast-reasoning".to_string()),
        ..Config::default()
    };

    let entries = all_integrations();
    let xai = entries.iter().find(|e| e.name == "xAI").unwrap();
    assert!(matches!(
        (xai.status_fn)(&config),
        IntegrationStatus::Active
    ));
}

#[test]
fn vercel_active_for_legacy_provider_alias() {
    let config = Config {
        default_provider: Some("vercel-ai".to_string()),
        default_model: Some("gpt-5.4".to_string()),
        ..Config::default()
    };

    let entries = all_integrations();
    let vercel = entries.iter().find(|e| e.name == "Vercel AI").unwrap();
    assert!(matches!(
        (vercel.status_fn)(&config),
        IntegrationStatus::Active
    ));
}
