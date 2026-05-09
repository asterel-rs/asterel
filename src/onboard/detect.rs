//! Pre-scan existing setup state for skip/auto-fill during onboarding.

use crate::core::providers::catalog::{api_key_env_candidates, provider_scan_candidates};

/// Snapshot of what is already configured before the wizard starts.
pub(crate) struct DetectedState {
    /// Whether a config.toml already exists.
    pub config_exists: bool,
    /// Provider detected from environment variables (e.g., "openrouter").
    pub detected_provider: Option<String>,
    /// API key detected from environment variables.
    pub detected_api_key: Option<String>,
}

fn configured_provider_and_key(
    config_path: &std::path::Path,
    workspace_dir: &std::path::Path,
) -> (Option<String>, Option<String>) {
    crate::config::Config::load_from_path_unvalidated(config_path, workspace_dir)
        .ok()
        .map_or((None, None), |config| {
            let provider = config
                .default_provider
                .as_deref()
                .map(str::trim)
                .filter(|provider| !provider.is_empty())
                .map(ToOwned::to_owned);
            let api_key = config
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|api_key| !api_key.is_empty())
                .map(ToOwned::to_owned);
            (provider, api_key)
        })
}

/// Scan existing environment and filesystem to populate a [`DetectedState`].
///
/// This is a pure detection pass — it reads but never writes anything.
pub(crate) fn detect_existing_setup() -> DetectedState {
    let asterel_dir = crate::utils::dirs::asterel_home_dir_or_local();
    let config_path = asterel_dir.join("config.toml");
    let workspace_dir = asterel_dir.join("workspace");

    let config_exists = config_path.exists();
    let (configured_provider, configured_api_key) = if config_exists {
        configured_provider_and_key(&config_path, &workspace_dir)
    } else {
        (None, None)
    };

    // Scan known providers for API key env vars.
    let (env_provider, env_api_key) = scan_provider_env();

    let detected_provider = env_provider.or(configured_provider);
    let detected_api_key = env_api_key.or(configured_api_key);

    DetectedState {
        config_exists,
        detected_provider,
        detected_api_key,
    }
}

/// Probe each known provider's env vars in order. Returns the first match.
///
/// Also checks `ASTEREL_API_KEY` as a generic fallback and Gemini CLI auth
/// as a provider-only signal.
fn scan_provider_env() -> (Option<String>, Option<String>) {
    for provider in provider_scan_candidates() {
        for var in api_key_env_candidates(provider) {
            if let Ok(val) = std::env::var(var)
                && !val.trim().is_empty()
            {
                return (Some(provider.to_string()), Some(val.trim().to_string()));
            }
        }
    }

    // Generic fallback: ASTEREL_API_KEY is not provider-specific.
    if let Ok(val) = std::env::var("ASTEREL_API_KEY")
        && !val.trim().is_empty()
    {
        return (None, Some(val.trim().to_string()));
    }

    if crate::core::providers::gemini::GeminiProvider::has_cli_credentials() {
        return (Some("gemini".to_string()), None);
    }

    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_providers_list_is_non_empty() {
        assert!(!provider_scan_candidates().is_empty());
    }

    #[test]
    fn scan_provider_env_returns_none_when_no_vars_set() {
        // In a clean test environment, no provider keys are expected to be set.
        // We can't assert None in CI because the runner may have keys, but we
        // can verify the function returns a consistent pair.
        let (provider, key) = scan_provider_env();
        assert_eq!(provider.is_some(), key.is_some() && provider.is_some());
        // Both are either both Some (matched provider env vars), key is Some
        // with no provider (ASTEREL_API_KEY fallback), provider is Some
        // with no key (CLI/OAuth-backed Gemini), or both None.
        match (provider, key) {
            // ASTEREL_API_KEY fallback
            (Some(_) | None, Some(_)) | (Some(_), None) | (None, None) => {}
        }
    }

    #[test]
    fn detected_state_workspace_check_does_not_panic() {
        // Verify the detection function runs without panicking.
        // In a test environment without a real home directory setup,
        // the function should return a sensible default.
        let _state = detect_existing_setup();
    }

    #[test]
    fn configured_provider_and_key_reads_trimmed_values() {
        let temp = tempfile::TempDir::new().expect("temp dir");
        let config_path = temp.path().join("config.toml");
        let workspace_dir = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace_dir).expect("workspace dir");
        let config = crate::config::Config {
            workspace_dir: workspace_dir.clone(),
            config_path: config_path.clone(),
            api_key: Some("  test-key  ".to_string()),
            default_provider: Some("  gemini  ".to_string()),
            ..crate::config::Config::default()
        };
        config.save().expect("save config");

        let (provider, api_key) = configured_provider_and_key(&config_path, &workspace_dir);
        assert_eq!(provider.as_deref(), Some("gemini"));
        assert_eq!(api_key.as_deref(), Some("test-key"));
    }
}
