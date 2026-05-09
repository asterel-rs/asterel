//! Auth helper utilities: secret store access, profile ID validation,
//! provider name canonicalization, and file path resolution.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::AUTH_PROFILES_FILENAME;
use crate::config::Config;
use crate::core::providers::catalog::{
    canonical_provider_name as canonical_catalog_provider_name,
    requested_auth_route as requested_catalog_auth_route,
};
use crate::security::SecretStore;

/// Build a `SecretStore` rooted at the config directory.
pub(super) fn auth_secret_store(config: &Config) -> SecretStore {
    let secret_root = config
        .config_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    SecretStore::new(secret_root, config.secrets.encrypt)
}

/// Check whether a secret value is present and non-empty.
pub(crate) fn has_secret(secret: Option<&str>) -> bool {
    secret.map(str::trim).is_some_and(|value| !value.is_empty())
}

/// Check whether a profile ID contains only safe characters
/// (alphanumeric, `-`, `_`, `.`).
pub(super) fn is_valid_profile_id(profile_id: &str) -> bool {
    profile_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Return the filesystem path for the auth profiles JSON file.
#[must_use]
pub fn auth_profiles_path(config: &Config) -> PathBuf {
    config
        .config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(AUTH_PROFILES_FILENAME)
}

/// Canonicalize a provider name (e.g., "google" -> "gemini").
pub(crate) fn canonical_provider_name(name: &str) -> String {
    canonical_catalog_provider_name(name)
}

/// Resolve a requested auth route from an external provider selector.
#[must_use]
pub(crate) fn requested_auth_route(name: &str) -> Option<&'static str> {
    requested_catalog_auth_route(name)
}

/// Canonicalize an auth route for persisted auth profiles.
#[must_use]
pub(crate) fn canonical_auth_route(
    provider: &str,
    auth_route: Option<&str>,
    auth_scheme: Option<&str>,
    oauth_source: Option<&str>,
) -> Option<String> {
    if let Some(route) = auth_route
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
    {
        return Some(route);
    }

    let normalized_scheme = auth_scheme
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    let normalized_oauth_source = oauth_source
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);

    match normalized_scheme.as_deref() {
        Some("oauth") => normalized_oauth_source.or_else(|| Some("oauth".to_string())),
        Some("api_key" | "api-key") => Some("api".to_string()),
        _ => requested_auth_route(provider)
            .map(str::to_string)
            .or(normalized_oauth_source),
    }
}

/// Build the stable store key for a provider/auth-route target.
#[must_use]
pub(crate) fn auth_target_key(provider: &str, auth_route: Option<&str>) -> String {
    let canonical_provider = canonical_provider_name(provider);
    let canonical_route = auth_route
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase);
    match canonical_route {
        Some(route) => format!("{canonical_provider}@{route}"),
        None => canonical_provider,
    }
}

/// Decrypt an optional secret value in place, returning whether
/// the plaintext needs to be re-encrypted for persistence.
///
/// # Errors
///
/// Returns an error if decryption fails.
pub(super) fn decrypt_opt_secret(
    value: &mut Option<String>,
    store: &SecretStore,
    encrypt_enabled: bool,
) -> Result<bool> {
    let Some(current) = value.as_deref() else {
        return Ok(false);
    };

    let trimmed = current.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }

    let needs_encrypt_persist = encrypt_enabled && !SecretStore::is_encrypted(trimmed);
    let decrypted = store.decrypt(trimmed)?;
    *value = Some(decrypted);

    Ok(needs_encrypt_persist)
}

/// Encrypt an optional secret value in place, skipping values
/// that are already encrypted or empty.
///
/// # Errors
///
/// Returns an error if encryption fails.
pub(super) fn encrypt_opt_secret(value: &mut Option<String>, store: &SecretStore) -> Result<()> {
    let Some(current) = value.as_deref() else {
        return Ok(());
    };

    let trimmed = current.trim();
    if trimmed.is_empty() || SecretStore::is_encrypted(trimmed) {
        if trimmed != current {
            *value = Some(trimmed.to_string());
        }
        return Ok(());
    }

    *value = Some(store.encrypt(trimmed)?);
    Ok(())
}
