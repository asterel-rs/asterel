//! Provider-specific defaults and validation helpers for onboarding.
//!
//! Projects the shared provider catalog into onboarding-friendly selections,
//! model menus, and auth prompts.

use anyhow::Result;

pub(crate) use crate::core::providers::catalog::ProviderAuthMethod;
use crate::core::providers::catalog::{
    ProviderOnboardingTier, primary_api_key_env_var, provider_catalog_entries_for_onboarding_tier,
    provider_catalog_entry, provider_key_url,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProviderModelChoice {
    pub(crate) model: &'static str,
    pub(crate) label: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProviderChoice {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
}

const DEFAULT_PROVIDER_MODEL: ProviderModelChoice = ProviderModelChoice {
    model: "anthropic/claude-sonnet-4.6",
    label: "Claude Sonnet 4.6 (balanced, recommended)",
};

const OPENAI_CODEX_MODELS: &[ProviderModelChoice] = &[
    ProviderModelChoice {
        model: "gpt-5.3-codex",
        label: "GPT-5.3 Codex (most capable Codex)",
    },
    ProviderModelChoice {
        model: "gpt-5.2-codex",
        label: "GPT-5.2 Codex (long-horizon coding)",
    },
    ProviderModelChoice {
        model: "gpt-5.4",
        label: "GPT-5.4 (general flagship)",
    },
    ProviderModelChoice {
        model: "gpt-5-mini",
        label: "GPT-5 Mini (fast, cheap)",
    },
];

fn default_provider_model_choices() -> Vec<ProviderModelChoice> {
    provider_catalog_entry("openrouter")
        .and_then(|entry| entry.onboarding.as_ref())
        .map_or_else(
            || vec![DEFAULT_PROVIDER_MODEL],
            |spec| {
                spec.models
                    .iter()
                    .map(|choice| ProviderModelChoice {
                        model: choice.model.as_str(),
                        label: choice.label.as_str(),
                    })
                    .collect()
            },
        )
}

#[must_use]
pub(crate) fn provider_choices_for_tier(tier: usize) -> Vec<ProviderChoice> {
    let Some(tier) = ProviderOnboardingTier::from_index(tier) else {
        return Vec::new();
    };

    provider_catalog_entries_for_onboarding_tier(tier)
        .into_iter()
        .filter_map(|entry| {
            entry.onboarding.as_ref().map(|spec| ProviderChoice {
                id: entry.id.as_str(),
                label: spec.label.as_str(),
            })
        })
        .collect()
}

#[must_use]
pub(crate) fn provider_choice_for_selection(tier: usize, idx: usize) -> Option<ProviderChoice> {
    provider_choices_for_tier(tier).get(idx).copied()
}

#[must_use]
pub(crate) fn model_choices_for_provider(provider: &str) -> Vec<ProviderModelChoice> {
    if provider.trim().eq_ignore_ascii_case("openai-codex") {
        return OPENAI_CODEX_MODELS.to_vec();
    }

    provider_catalog_entry(provider)
        .and_then(|entry| entry.onboarding.as_ref())
        .map(|spec| {
            spec.models
                .iter()
                .map(|choice| ProviderModelChoice {
                    model: choice.model.as_str(),
                    label: choice.label.as_str(),
                })
                .collect::<Vec<_>>()
        })
        .filter(|choices| !choices.is_empty())
        .unwrap_or_else(default_provider_model_choices)
}

/// Returns the default model name for a given provider identifier.
#[must_use]
pub(crate) fn default_model_for_provider(provider: &str) -> String {
    model_choices_for_provider(provider)
        .first()
        .unwrap_or(&DEFAULT_PROVIDER_MODEL)
        .model
        .to_string()
}

/// Resolve the provider to persist after OAuth login.
#[must_use]
pub(crate) fn provider_after_oauth(provider: &str, oauth_source: Option<&str>) -> String {
    if provider.trim().eq_ignore_ascii_case("openai")
        && oauth_source.is_some_and(|source| source.trim().eq_ignore_ascii_case("codex"))
    {
        "openai-codex".to_string()
    } else {
        provider.to_string()
    }
}

#[must_use]
pub(crate) fn provider_auth_method(name: &str) -> ProviderAuthMethod {
    if name.trim().eq_ignore_ascii_case("openai-codex") {
        return ProviderAuthMethod::ApiKeyOrOAuth;
    }

    provider_catalog_entry(name)
        .and_then(|entry| entry.onboarding.as_ref())
        .map_or(ProviderAuthMethod::ApiKeyOnly, |spec| spec.auth_method)
}

#[cfg(test)]
#[must_use]
pub(crate) fn provider_uses_auth_method(name: &str) -> bool {
    matches!(
        provider_auth_method(name),
        ProviderAuthMethod::ApiKeyOrOAuth
            | ProviderAuthMethod::ApiKeyOrSetupToken
            | ProviderAuthMethod::ApiKeyOrApplicationDefaultCredentials
    )
}

#[must_use]
pub(crate) fn oauth_login_provider(provider: &str) -> String {
    if provider.eq_ignore_ascii_case("openai-codex") {
        "openai".to_string()
    } else {
        provider.to_string()
    }
}

#[must_use]
pub(crate) fn provider_api_key_url(name: &str) -> Option<&'static str> {
    provider_key_url(name)
}

/// Returns the environment variable name used for the API key
/// of the given provider.
#[must_use]
pub(crate) fn provider_env_var(name: &str) -> &'static str {
    primary_api_key_env_var(name)
}

/// Parse a comma-separated allowlist, treating `"*"` as a wildcard.
#[must_use]
pub(crate) fn parse_allowlist(input: &str) -> Vec<String> {
    if input.trim() == "*" {
        return vec!["*".into()];
    }
    input
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

// ── Step 5: Tool Mode & Security ────────────────────────────────

/// # Errors
///
/// Returns an error when `value` is empty or whitespace-only.
pub(crate) fn validate_non_empty(label: &str, value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{label} cannot be empty");
    }
    Ok(trimmed.to_string())
}

/// # Errors
///
/// Returns an error when the base URL is empty or whitespace-only.
pub(crate) fn validate_base_url(value: &str) -> Result<String> {
    let normalized = validate_non_empty("base URL", value)?;
    Ok(normalized.trim_end_matches('/').to_string())
}

/// Validate a port string and return the parsed `u16`.
///
/// # Errors
///
/// Returns an error when the port is not a valid integer or is zero.
pub(crate) fn validate_port(value: &str, default: u16) -> Result<u16> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(default);
    }
    let port: u16 = trimmed
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid port number: {trimmed}"))?;
    if port == 0 {
        anyhow::bail!("port must be between 1 and 65535");
    }
    Ok(port)
}

/// Warn (via tracing) if a Slack token does not start with the expected prefix.
pub(crate) fn warn_slack_token_prefix(token: &str, expected_prefix: &str, label: &str) {
    if !token.starts_with(expected_prefix) {
        tracing::warn!(
            "{label} does not start with expected prefix '{expected_prefix}'. \
             This may indicate an incorrect token."
        );
    }
}

/// Normalize IRC channel names: ensure each starts with `#`.
#[must_use]
pub(crate) fn normalize_irc_channels(channels: &[String]) -> Vec<String> {
    channels
        .iter()
        .map(|c| {
            let trimmed = c.trim();
            if trimmed.starts_with('#') {
                trimmed.to_string()
            } else {
                format!("#{trimmed}")
            }
        })
        .filter(|c| c.len() > 1)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_env_var_known_providers() {
        assert_eq!(provider_env_var("openrouter"), "OPENROUTER_API_KEY");
        assert_eq!(provider_env_var("anthropic"), "ANTHROPIC_API_KEY");
        assert_eq!(provider_env_var("openai"), "OPENAI_API_KEY");
        assert_eq!(provider_env_var("openai-codex"), "OPENAI_API_KEY");
    }

    #[test]
    fn default_model_for_provider_returns_expected_defaults() {
        assert_eq!(
            default_model_for_provider("openrouter"),
            "anthropic/claude-sonnet-4.6"
        );
        assert_eq!(
            default_model_for_provider("anthropic"),
            "claude-sonnet-4-20250514"
        );
        assert_eq!(default_model_for_provider("openai"), "gpt-5.4");
        assert_eq!(default_model_for_provider("openai-codex"), "gpt-5.3-codex");
        assert_eq!(default_model_for_provider("moonshot"), "kimi-k2-0905");
        assert_eq!(default_model_for_provider("glm"), "glm-5");
        assert_eq!(default_model_for_provider("minimax"), "MiniMax-M2.7");
        assert_eq!(
            default_model_for_provider("unknown-provider"),
            "anthropic/claude-sonnet-4.6"
        );
    }

    #[test]
    fn provider_after_oauth_promotes_codex_selector() {
        assert_eq!(
            provider_after_oauth("openai", Some("codex")),
            "openai-codex"
        );
        assert_eq!(provider_after_oauth("openai", Some("oauth")), "openai");
        assert_eq!(
            provider_after_oauth("anthropic", Some("claude")),
            "anthropic"
        );
        assert_eq!(provider_after_oauth("openai", None), "openai");
    }

    #[test]
    fn provider_choice_catalog_is_shared_across_tiers() {
        assert_eq!(
            provider_choice_for_selection(0, 0),
            Some(ProviderChoice {
                id: "openrouter",
                label: "OpenRouter — 200+ models, 1 API key (recommended)",
            })
        );
        assert_eq!(
            provider_choice_for_selection(2, 3).map(|choice| choice.id),
            Some("copilot")
        );
        assert!(provider_choices_for_tier(99).is_empty());
    }

    #[test]
    fn provider_auth_helpers_match_supported_flows() {
        assert_eq!(
            provider_auth_method("openai"),
            ProviderAuthMethod::ApiKeyOrOAuth
        );
        assert_eq!(
            provider_auth_method("anthropic"),
            ProviderAuthMethod::ApiKeyOrSetupToken
        );
        assert_eq!(
            provider_auth_method("ollama"),
            ProviderAuthMethod::NoKeyRequired
        );
        assert!(provider_uses_auth_method("openai-codex"));
        assert!(!provider_uses_auth_method("gemini"));
        assert_eq!(oauth_login_provider("openai-codex"), "openai");
        assert_eq!(oauth_login_provider("anthropic"), "anthropic");
    }

    #[test]
    fn provider_api_key_url_uses_shared_provider_catalog() {
        assert_eq!(
            provider_api_key_url("openai-codex"),
            Some("https://platform.openai.com/api-keys")
        );
        assert_eq!(provider_api_key_url("ollama"), None);
    }

    #[test]
    fn validate_non_empty_rejects_blank() {
        assert!(validate_non_empty("x", "   ").is_err());
    }

    #[test]
    fn validate_base_url_trims_trailing_slash() {
        assert_eq!(
            validate_base_url("https://ex.com/").unwrap(),
            "https://ex.com"
        );
    }

    #[test]
    fn validate_base_url_rejects_empty() {
        assert!(validate_base_url("   ").is_err());
    }

    #[test]
    fn validate_base_url_preserves_path() {
        assert_eq!(
            validate_base_url("https://ex.com/v1").unwrap(),
            "https://ex.com/v1"
        );
    }

    #[test]
    fn validate_base_url_trims_whitespace() {
        assert_eq!(
            validate_base_url("  https://ex.com  ").unwrap(),
            "https://ex.com"
        );
    }

    #[test]
    fn validate_port_accepts_valid_port() {
        assert_eq!(validate_port("8080", 3000).unwrap(), 8080);
    }

    #[test]
    fn validate_port_returns_default_on_empty() {
        assert_eq!(validate_port("", 3000).unwrap(), 3000);
        assert_eq!(validate_port("  ", 3000).unwrap(), 3000);
    }

    #[test]
    fn validate_port_rejects_non_numeric() {
        assert!(validate_port("abc", 3000).is_err());
    }

    #[test]
    fn validate_port_rejects_zero() {
        assert!(validate_port("0", 3000).is_err());
    }

    #[test]
    fn validate_port_rejects_overflow() {
        assert!(validate_port("99999", 3000).is_err());
    }

    #[test]
    fn normalize_irc_channels_adds_hash() {
        let input = vec!["general".into(), "#existing".into()];
        let result = normalize_irc_channels(&input);
        assert_eq!(result, vec!["#general", "#existing"]);
    }

    #[test]
    fn normalize_irc_channels_filters_empty() {
        let input = vec![String::new(), "  ".into(), "valid".into()];
        let result = normalize_irc_channels(&input);
        assert_eq!(result, vec!["#valid"]);
    }
}
