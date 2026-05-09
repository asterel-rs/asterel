//! Shared onboarding auth-profile persistence helpers.
//!
//! Ensures re-running onboarding keeps a stable onboarding profile id
//! per provider and updates that profile in place.

use anyhow::Result;

use crate::config::Config;
use crate::security::auth::{AuthProfile, AuthProfileStore};

const ONBOARD_PROFILE_LABEL: &str = "Created by onboarding";
const AUTH_SCHEME_API_KEY: &str = "api_key";
const AUTH_SCHEME_OAUTH: &str = "oauth";

pub(crate) fn upsert_onboard_auth_profile(
    config: &Config,
    provider: &str,
    api_key: &str,
    oauth_source: Option<String>,
) -> Result<()> {
    let mut auth_store = AuthProfileStore::load_or_init_cfg(config)?;
    let profile_id = onboard_profile_base_id(provider);

    auth_store.upsert_profile(
        AuthProfile {
            id: profile_id.clone(),
            provider: provider.to_string(),
            auth_route: None,
            label: Some(ONBOARD_PROFILE_LABEL.into()),
            api_key: Some(api_key.to_string()),
            refresh_token: None,
            auth_scheme: Some(if oauth_source.is_some() {
                AUTH_SCHEME_OAUTH.into()
            } else {
                AUTH_SCHEME_API_KEY.into()
            }),
            oauth_source,
            is_disabled: false,
        },
        true,
    )?;
    auth_store.mark_profile_used(&profile_id);
    auth_store.save_for_config(config)?;
    Ok(())
}

fn onboard_profile_base_id(provider: &str) -> String {
    format!(
        "{}-onboard-default",
        provider.replace([':', '/'], "-").to_ascii_lowercase()
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::upsert_onboard_auth_profile;
    use crate::config::Config;
    use crate::security::auth::AuthProfileStore;

    #[test]
    fn upsert_onboard_auth_profile_uses_stable_id_for_same_provider() {
        let tmp = TempDir::new().expect("temp dir");
        let config_path = tmp.path().join("config.toml");
        fs::write(&config_path, "").expect("create config file");

        let config = Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path,
            secrets: crate::config::SecretsConfig {
                encrypt: false,
                ..crate::config::SecretsConfig::default()
            },
            ..Config::default()
        };

        upsert_onboard_auth_profile(&config, "openai", "sk-openai", None)
            .expect("upsert first run");
        upsert_onboard_auth_profile(&config, "openai", "sk-openai-updated", None)
            .expect("upsert second run");

        let store = AuthProfileStore::load_or_init_cfg(&config).expect("load auth profiles");
        let openai_profiles: Vec<_> = store
            .profiles
            .iter()
            .filter(|profile| profile.provider == "openai")
            .collect();
        assert_eq!(openai_profiles.len(), 1);
        assert_eq!(openai_profiles[0].id, "openai-onboard-default");
        assert_eq!(
            openai_profiles[0].api_key.as_deref(),
            Some("sk-openai-updated")
        );
    }
}
