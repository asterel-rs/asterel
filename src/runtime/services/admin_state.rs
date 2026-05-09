//! Runtime-owned admin config/auth persistence and narrow mutation helpers.

use anyhow::Context;

use crate::config::Config;
use crate::security::auth::{
    AuthProfile, AuthProfileStore, auth_target_key, canonical_provider_name,
};

fn persisted_provider_selector(provider: &str) -> String {
    if provider.eq_ignore_ascii_case("openai-codex")
        || provider.starts_with("custom:")
        || provider.starts_with("anthropic-custom:")
    {
        provider.to_string()
    } else {
        canonical_provider_name(provider).clone()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AdminStateError {
    #[error("failed to load persisted config")]
    ConfigLoad { source: anyhow::Error },
    #[error("failed to save persisted config")]
    ConfigSave { source: anyhow::Error },
    #[error("failed to load persisted auth profiles")]
    AuthProfilesLoad { source: anyhow::Error },
    #[error("failed to save persisted auth profiles")]
    AuthProfilesSave { source: anyhow::Error },
}

type AdminStateResult<T> = std::result::Result<T, AdminStateError>;

#[allow(clippy::missing_errors_doc)]
pub fn load_admin_runtime_config_snapshot(config: &Config) -> AdminStateResult<Config> {
    if !config.config_path.exists() {
        return Ok(config.clone());
    }

    let config_path = config.config_path.clone();
    let workspace_dir = config.workspace_dir.clone();
    let raw = std::fs::read_to_string(&config_path)
        .with_context(|| format!("read persisted config at {}", config_path.display()))
        .map_err(|source| AdminStateError::ConfigLoad { source })?;
    let mut persisted: Config = toml::from_str(&raw)
        .with_context(|| format!("parse persisted config at {}", config_path.display()))
        .map_err(|source| AdminStateError::ConfigLoad { source })?;
    persisted.config_path = config_path;
    persisted.workspace_dir = workspace_dir;
    Ok(persisted)
}

#[allow(clippy::missing_errors_doc)]
pub fn save_admin_runtime_config(config: &Config) -> AdminStateResult<()> {
    config
        .save()
        .context("save persisted runtime config")
        .map_err(|source| AdminStateError::ConfigSave { source })
}

#[allow(clippy::missing_errors_doc)]
pub fn load_admin_auth_profiles(config: &Config) -> AdminStateResult<AuthProfileStore> {
    AuthProfileStore::load_or_init_cfg(config)
        .context("load persisted auth profiles")
        .map_err(|source| AdminStateError::AuthProfilesLoad { source })
}

#[allow(clippy::missing_errors_doc)]
pub fn save_admin_auth_profiles(config: &Config, store: &AuthProfileStore) -> AdminStateResult<()> {
    store
        .save_for_config(config)
        .context("save persisted auth profiles")
        .map_err(|source| AdminStateError::AuthProfilesSave { source })
}

#[allow(clippy::missing_errors_doc)]
pub fn load_admin_auth_profile(
    config: &Config,
    profile_id: &str,
) -> AdminStateResult<Option<AuthProfile>> {
    let store = load_admin_auth_profiles(config)?;
    Ok(store
        .profiles
        .into_iter()
        .find(|profile| profile.id == profile_id))
}

#[allow(clippy::missing_errors_doc)]
pub fn set_admin_auth_profile_disabled(
    config: &Config,
    profile_id: &str,
    disabled: bool,
) -> AdminStateResult<Option<AuthProfile>> {
    let mut store = load_admin_auth_profiles(config)?;
    let Some(profile) = store
        .profiles
        .iter_mut()
        .find(|candidate| candidate.id == profile_id)
    else {
        return Ok(None);
    };

    profile.is_disabled = disabled;
    let response = profile.clone();

    save_admin_auth_profiles(config, &store)?;
    Ok(Some(response))
}

#[allow(clippy::missing_errors_doc)]
pub fn set_admin_provider_enabled(
    config: &Config,
    canonical_provider: &str,
    enabled: bool,
) -> AdminStateResult<()> {
    let mut store = load_admin_auth_profiles(config)?;
    for profile in &mut store.profiles {
        if canonical_provider_name(&profile.provider) == canonical_provider {
            profile.is_disabled = !enabled;
        }
    }

    save_admin_auth_profiles(config, &store)
}

#[allow(clippy::missing_errors_doc)]
pub fn set_admin_provider_default_model(
    config: &Config,
    canonical_provider: &str,
    model: &str,
) -> AdminStateResult<bool> {
    let model = model.trim();
    if model.is_empty() {
        return Ok(false);
    }

    let mut persisted = load_admin_runtime_config_snapshot(config)?;
    persisted.default_provider = Some(canonical_provider.to_string());
    persisted.default_model = Some(model.to_string());
    save_admin_runtime_config(&persisted)?;
    Ok(true)
}

#[allow(clippy::missing_errors_doc)]
pub fn set_admin_provider_auth_profile(
    config: &Config,
    profile_id: &str,
) -> AdminStateResult<bool> {
    let profile_id = profile_id.trim();
    if profile_id.is_empty() {
        return Ok(false);
    }

    let mut store = load_admin_auth_profiles(config)?;
    let target_key = store
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .map(|profile| auth_target_key(&profile.provider, profile.auth_route.as_deref()));

    let Some(target_key) = target_key else {
        return Ok(false);
    };

    store.defaults.insert(target_key, profile_id.to_string());
    save_admin_auth_profiles(config, &store)?;
    Ok(true)
}

#[allow(clippy::missing_errors_doc)]
pub fn update_admin_active_provider_selection(
    config: &Config,
    active_provider: Option<&str>,
    active_model: Option<&str>,
    temperature: Option<f64>,
) -> AdminStateResult<Vec<String>> {
    let mut persisted = load_admin_runtime_config_snapshot(config)?;
    let mut changes = Vec::new();

    if let Some(provider) = active_provider
        .map(str::trim)
        .filter(|provider| !provider.is_empty())
    {
        persisted.default_provider = Some(persisted_provider_selector(provider));
        changes.push("active_provider".to_string());
    }

    if let Some(model) = active_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        persisted.default_model = Some(model.to_string());
        changes.push("active_model".to_string());
    }

    if let Some(temp) = temperature {
        persisted.default_temperature = temp;
        changes.push("temperature".to_string());
    }

    if !changes.is_empty() {
        save_admin_runtime_config(&persisted)?;
    }

    Ok(changes)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{load_admin_runtime_config_snapshot, update_admin_active_provider_selection};
    use crate::config::Config;

    fn test_config(tmp: &TempDir) -> Config {
        Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        }
    }

    #[test]
    fn active_provider_update_preserves_openai_codex_selector() {
        let tmp = TempDir::new().expect("tempdir");
        let config = test_config(&tmp);

        update_admin_active_provider_selection(&config, Some("openai-codex"), None, None)
            .expect("update provider");

        let persisted = load_admin_runtime_config_snapshot(&config).expect("load persisted config");
        assert_eq!(persisted.default_provider.as_deref(), Some("openai-codex"));
    }

    #[test]
    fn active_provider_update_preserves_custom_selector_url() {
        let tmp = TempDir::new().expect("tempdir");
        let config = test_config(&tmp);

        update_admin_active_provider_selection(
            &config,
            Some("custom:https://proxy.example/v1"),
            None,
            None,
        )
        .expect("update provider");

        let persisted = load_admin_runtime_config_snapshot(&config).expect("load persisted config");
        assert_eq!(
            persisted.default_provider.as_deref(),
            Some("custom:https://proxy.example/v1")
        );
    }
}
