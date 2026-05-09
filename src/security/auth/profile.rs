//! Auth profile data model and interactive OAuth registration.
//!
//! Defines `AuthProfile` (provider, API key, refresh token, OAuth
//! source) and orchestrates interactive provider authentication.

use std::fmt;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::oauth::{
    import_cached_oauth_credential_for_provider, import_interactive_oauth_credential_for_provider,
};
use crate::security::SecurityPolicy;

/// Filename for the on-disk auth profiles JSON store.
pub(super) const AUTH_PROFILES_FILENAME: &str = "auth-profiles.json";
/// Current schema version for the auth profiles JSON file.
pub(super) const AUTH_PROFILES_VERSION: u32 = 1;

/// A single authentication profile for a provider.
#[derive(Clone, Serialize, Deserialize)]
pub struct AuthProfile {
    /// Unique profile identifier (alphanumeric, `-`, `_`, `.`).
    pub id: String,
    /// Canonical provider name (e.g., "openai", "anthropic").
    pub provider: String,
    /// Authentication/access route for this provider (e.g., "api", "codex").
    #[serde(default)]
    pub auth_route: Option<String>,
    /// Optional human-readable label.
    #[serde(default)]
    pub label: Option<String>,
    /// API key or access token (encrypted on disk).
    #[serde(default)]
    pub(crate) api_key: Option<String>,
    /// OAuth refresh token (encrypted on disk).
    #[serde(default)]
    pub(crate) refresh_token: Option<String>,
    /// Authentication scheme (e.g., "oauth", "api-key").
    #[serde(default)]
    pub auth_scheme: Option<String>,
    /// Name of the OAuth source that provided the token.
    #[serde(default)]
    pub oauth_source: Option<String>,
    /// Whether this profile is disabled (skipped during resolution).
    #[serde(default)]
    pub is_disabled: bool,
}

impl fmt::Debug for AuthProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthProfile")
            .field("id", &self.id)
            .field("provider", &self.provider)
            .field("auth_route", &self.auth_route)
            .field("label", &self.label)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("auth_scheme", &self.auth_scheme)
            .field("oauth_source", &self.oauth_source)
            .field("is_disabled", &self.is_disabled)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_profile_debug_redacts_secrets() {
        let profile = AuthProfile {
            id: "default".to_string(),
            provider: "openai".to_string(),
            auth_route: Some("api".to_string()),
            label: None,
            api_key: Some("sk-secret-value".to_string()),
            refresh_token: Some("refresh-secret-value".to_string()),
            auth_scheme: Some("api-key".to_string()),
            oauth_source: None,
            is_disabled: false,
        };

        let debug = format!("{profile:?}");
        assert!(!debug.contains("sk-secret-value"));
        assert!(!debug.contains("refresh-secret-value"));
        assert!(debug.contains("<redacted>"));
    }
}

/// Try to import OAuth tokens from local cache only (no interactive login).
///
/// # Errors
///
/// Returns an error when provider-specific OAuth token import fails.
pub fn import_oauth_access_token_for_provider(provider: &str) -> Result<Option<(String, String)>> {
    let imported = import_cached_oauth_credential_for_provider(provider)?;

    Ok(imported.map(|cred| (cred.access_token, cred.source_name.to_string())))
}

/// Build a `SecurityPolicy` that permits running `codex` and `claude` CLI
/// tools during onboarding (they are not in the default allowlist).
fn onboarding_oauth_policy() -> SecurityPolicy {
    let mut policy = SecurityPolicy::default();
    policy.allowed_commands.push("codex".into());
    policy.allowed_commands.push("claude".into());
    policy
}

/// Run the interactive auth flow (`codex login` for `OpenAI`, setup-token paste
/// for `Anthropic`).
///
/// # Errors
///
/// Returns an error when the interactive auth flow fails.
pub fn run_interactive_oauth_for_provider(provider: &str) -> Result<Option<(String, String)>> {
    let security = onboarding_oauth_policy();
    let imported = import_interactive_oauth_credential_for_provider(provider, &security)?;

    Ok(imported.map(|cred| (cred.access_token, cred.source_name.to_string())))
}
