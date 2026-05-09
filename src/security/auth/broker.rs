//! Auth broker: resolves the active API key for a given provider.
//!
//! Checks auth profiles, OAuth tokens, and config-level keys in
//! priority order, falling back gracefully when profiles are absent.

use std::borrow::Cow;
use std::path::PathBuf;

use anyhow::Result;

use super::oauth::import_cached_oauth_credential_for_source;
use super::resolution::{
    auth_profiles_path, auth_secret_store, canonical_auth_route, canonical_provider_name,
    requested_auth_route,
};
use super::store::AuthProfileStore;
use crate::config::{Config, MemoryConfig};
use crate::core::providers::catalog::api_key_env_candidates;
use crate::security::SecretStore;

const EMPTY_ENV_VARS: &[&str] = &[];
const VOYAGE_ENV_VARS: &[&str] = &["VOYAGE_API_KEY"];
const JINA_ENV_VARS: &[&str] = &["JINA_API_KEY"];
const NOMIC_ENV_VARS: &[&str] = &["NOMIC_API_KEY"];

fn embedding_only_api_key_env_candidates(provider: &str) -> &'static [&'static str] {
    match provider {
        "voyage" => VOYAGE_ENV_VARS,
        "jina" => JINA_ENV_VARS,
        "nomic" => NOMIC_ENV_VARS,
        _ => EMPTY_ENV_VARS,
    }
}

fn resolve_provider_key_from_env(provider: &str) -> Option<String> {
    let normalized = canonical_provider_name(provider);
    for env_var in api_key_env_candidates(&normalized)
        .iter()
        .map(String::as_str)
        .chain(
            embedding_only_api_key_env_candidates(&normalized)
                .iter()
                .copied(),
        )
    {
        if let Ok(value) = std::env::var(env_var) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    if let Ok(value) = std::env::var("ASTEREL_API_KEY") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    None
}

/// Resolves the active API key for a given provider.
#[derive(Debug, Clone)]
pub struct AuthBroker {
    profile_store: AuthProfileStore,
    auth_profiles_path: PathBuf,
    secret_store: SecretStore,
    encrypt_enabled: bool,
    config_api_key: Option<String>,
}

impl AuthBroker {
    /// # Errors
    /// Returns an error if auth profile storage cannot be loaded.
    pub fn load_or_init(config: &Config) -> Result<Self> {
        let profile_store = AuthProfileStore::load_or_init_cfg(config)?;
        let auth_profiles_path = auth_profiles_path(config);
        let secret_store = auth_secret_store(config);

        Ok(Self {
            profile_store,
            auth_profiles_path,
            secret_store,
            encrypt_enabled: config.secrets.encrypt,
            config_api_key: config
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(ToOwned::to_owned),
        })
    }

    /// Resolve the active API key for a provider, falling back to config.
    #[must_use]
    pub fn resolve_provider_key(&self, provider: &str) -> Option<String> {
        self.current_profile_store()
            .active_api_key_for_provider(provider)
            .or_else(|| resolve_provider_key_from_env(provider))
            .or_else(|| self.config_api_key.clone())
    }

    fn current_profile_store(&self) -> Cow<'_, AuthProfileStore> {
        match AuthProfileStore::load_or_init_at(
            &self.auth_profiles_path,
            &self.secret_store,
            self.encrypt_enabled,
        ) {
            Ok(store) => Cow::Owned(store),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "failed to reload auth profile store; falling back to broker startup snapshot"
                );
                Cow::Borrowed(&self.profile_store)
            }
        }
    }

    /// Resolve the API key for the memory embedding provider if needed.
    #[must_use]
    pub fn resolve_memory_api_key(&self, memory: &MemoryConfig) -> Option<String> {
        memory
            .embedding_provider
            .credential_provider_selector()
            .and_then(|provider| self.resolve_provider_key(provider))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthRecoverySkipReason {
    EmptyProvider,
    NoActiveProfile,
    ProfileDisabled,
    NonOAuthProfile,
    MissingCachedCredential,
    RouteMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthRecoveryOutcome {
    Updated,
    Unchanged,
    Skipped(OAuthRecoverySkipReason),
}

impl OAuthRecoveryOutcome {
    #[must_use]
    pub const fn changed(self) -> bool {
        matches!(self, Self::Updated)
    }
}

/// # Errors
/// Returns an error if OAuth import, profile mutation, or persistence fails.
pub fn recover_oauth_profile_for_provider(config: &Config, provider: &str) -> Result<bool> {
    let outcome = recover_oauth_profile_for_provider_with_outcome(config, provider)?;
    if let OAuthRecoveryOutcome::Skipped(reason) = outcome {
        tracing::debug!(provider, ?reason, "oauth profile recovery skipped");
    }
    Ok(outcome.changed())
}

/// # Errors
/// Returns an error if OAuth import, profile mutation, or persistence fails.
pub fn recover_oauth_profile_for_provider_with_outcome(
    config: &Config,
    provider: &str,
) -> Result<OAuthRecoveryOutcome> {
    let canonical_provider = canonical_provider_name(provider);
    if canonical_provider.is_empty() {
        return Ok(OAuthRecoveryOutcome::Skipped(
            OAuthRecoverySkipReason::EmptyProvider,
        ));
    }

    let mut store = AuthProfileStore::load_or_init_cfg(config)?;
    let Some(index) = store.active_profile_index_for_provider(provider) else {
        if store.disabled_oauth_profile_exists_for_provider(provider) {
            return Ok(OAuthRecoveryOutcome::Skipped(
                OAuthRecoverySkipReason::ProfileDisabled,
            ));
        }
        return Ok(OAuthRecoveryOutcome::Skipped(
            OAuthRecoverySkipReason::NoActiveProfile,
        ));
    };

    let profile = &store.profiles[index];
    if profile.is_disabled {
        return Ok(OAuthRecoveryOutcome::Skipped(
            OAuthRecoverySkipReason::ProfileDisabled,
        ));
    }
    if profile.auth_scheme.as_deref() != Some("oauth") {
        return Ok(OAuthRecoveryOutcome::Skipped(
            OAuthRecoverySkipReason::NonOAuthProfile,
        ));
    }

    let oauth_source = profile
        .oauth_source
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();

    let Some(imported) = import_cached_oauth_credential_for_source(&oauth_source)? else {
        return Ok(OAuthRecoveryOutcome::Skipped(
            OAuthRecoverySkipReason::MissingCachedCredential,
        ));
    };

    let imported_route = canonical_auth_route(
        imported.target_provider,
        requested_auth_route(provider),
        Some("oauth"),
        Some(imported.source_name),
    );
    let profile_route = canonical_auth_route(
        &profile.provider,
        profile.auth_route.as_deref(),
        profile.auth_scheme.as_deref(),
        profile.oauth_source.as_deref(),
    );

    if canonical_provider_name(imported.target_provider) != canonical_provider
        || profile_route != imported_route
    {
        return Ok(OAuthRecoveryOutcome::Skipped(
            OAuthRecoverySkipReason::RouteMismatch,
        ));
    }

    let mut changed = false;
    {
        let profile = &mut store.profiles[index];

        if profile.api_key.as_deref() != Some(imported.access_token.as_str()) {
            profile.api_key = Some(imported.access_token.clone());
            changed = true;
        }

        if let Some(refresh_token) = imported.refresh_token
            && profile.refresh_token.as_deref() != Some(refresh_token.as_str())
        {
            profile.refresh_token = Some(refresh_token);
            changed = true;
        }

        if profile.auth_scheme.as_deref() != Some("oauth") {
            profile.auth_scheme = Some("oauth".into());
            changed = true;
        }

        if profile.oauth_source.as_deref() != Some(imported.source_name) {
            profile.oauth_source = Some(imported.source_name.into());
            changed = true;
        }
    }

    let profile_id = store.profiles[index].id.clone();
    store.mark_profile_used(&profile_id);
    store.save_for_config(config)?;

    Ok(if changed {
        OAuthRecoveryOutcome::Updated
    } else {
        OAuthRecoveryOutcome::Unchanged
    })
}

#[cfg(test)]
mod tests {
    use super::resolve_provider_key_from_env;
    use crate::utils::test_env::EnvVarGuard;

    #[test]
    fn resolve_provider_key_from_env_falls_back_to_generic_api_key() {
        let _generic_guard =
            EnvVarGuard::set("ASTEREL_API_KEY", "generic-key").expect("generic env guard");

        assert_eq!(
            resolve_provider_key_from_env("ollama"),
            Some("generic-key".to_string())
        );
    }
}
