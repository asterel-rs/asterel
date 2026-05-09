//! Provider factory: resolves API keys and constructs the
//! appropriate `Provider` implementation by name, wrapped in
//! reliability and OAuth recovery layers.

use std::sync::Arc;

use super::catalog::{api_key_env_candidates, compatible_provider_spec};
use super::compatible::{AuthStyle, OpenAiCompatProvider};
use super::oauth_recovery::OAuthRecoveryProvider;
use super::reliable::ReliableProvider;
use super::traits::Provider;
use crate::contracts::providers::normalize_provider_alias;
use crate::security::SecurityPolicy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderCredentialSource {
    ExplicitParameter,
    ProviderEnvironment(String),
    GenericEnvironment(String),
    ProviderLocalCredential(&'static str),
    IntentionallyUnauthenticatedLocal,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderCredentialResolution {
    pub(crate) key: Option<String>,
    pub(crate) source: ProviderCredentialSource,
}

/// Resolve API key for a provider from config and environment variables.
///
/// Resolution order:
/// 1. Explicitly provided `api_key` parameter (trimmed, filtered if empty)
/// 2. Provider-specific environment variable (e.g., `ANTHROPIC_OAUTH_TOKEN`, `OPENROUTER_API_KEY`)
/// 3. Generic fallback variable (`ASTEREL_API_KEY`)
///
/// For Anthropic, the provider-specific env var is `ANTHROPIC_OAUTH_TOKEN`
/// (for setup-tokens) followed by `ANTHROPIC_API_KEY` (for regular API keys).
fn resolve_api_key(name: &str, explicit_api_key: Option<&str>) -> Option<String> {
    resolve_api_key_details(name, explicit_api_key).key
}

fn resolve_api_key_details(
    name: &str,
    explicit_api_key: Option<&str>,
) -> ProviderCredentialResolution {
    let normalized_name = normalize_provider_alias(name);
    if matches!(
        missing_credential_source(normalized_name),
        ProviderCredentialSource::IntentionallyUnauthenticatedLocal
    ) {
        return ProviderCredentialResolution {
            key: None,
            source: ProviderCredentialSource::IntentionallyUnauthenticatedLocal,
        };
    }

    if let Some(key) = explicit_api_key.map(str::trim).filter(|k| !k.is_empty()) {
        return ProviderCredentialResolution {
            key: Some(key.to_string()),
            source: ProviderCredentialSource::ExplicitParameter,
        };
    }

    for env_var in api_key_env_candidates(normalized_name) {
        if let Ok(value) = std::env::var(env_var) {
            let value = value.trim();
            if !value.is_empty() {
                return ProviderCredentialResolution {
                    key: Some(value.to_string()),
                    source: ProviderCredentialSource::ProviderEnvironment(env_var.clone()),
                };
            }
        }
    }

    for env_var in ["ASTEREL_API_KEY"] {
        if let Ok(value) = std::env::var(env_var) {
            let value = value.trim();
            if !value.is_empty() {
                return ProviderCredentialResolution {
                    key: Some(value.to_string()),
                    source: ProviderCredentialSource::GenericEnvironment(env_var.to_string()),
                };
            }
        }
    }

    ProviderCredentialResolution {
        key: None,
        source: missing_credential_source(normalized_name),
    }
}

fn missing_credential_source(normalized_name: &str) -> ProviderCredentialSource {
    match normalized_name {
        "gemini" => ProviderCredentialSource::ProviderLocalCredential("gemini-cli-oauth"),
        "gemini-vertex" => ProviderCredentialSource::ProviderLocalCredential("google-adc"),
        "ollama" => ProviderCredentialSource::IntentionallyUnauthenticatedLocal,
        _ => ProviderCredentialSource::Missing,
    }
}

fn credential_resolution_name_for_selector(normalized_name: &str) -> &str {
    if normalized_name.starts_with("custom:") {
        "custom"
    } else if normalized_name.starts_with("anthropic-custom:") {
        "anthropic-custom"
    } else {
        normalized_name
    }
}

/// Handle `custom:<url>` and `anthropic-custom:<url>` selector prefixes.
///
/// Returns `Some(Ok(...))` when the prefix matches and the URL is non-empty,
/// `Some(Err(...))` when the prefix matches but the URL is empty, or
/// `None` when the name uses no recognized prefix.
fn create_custom_provider(
    name: &str,
    api_key: Option<&str>,
) -> Option<anyhow::Result<Box<dyn Provider>>> {
    if let Some(base_url) = name.strip_prefix("custom:") {
        Some(if base_url.is_empty() {
            Err(anyhow::anyhow!(
                "Custom provider requires a URL. Format: custom:https://your-api.com"
            ))
        } else {
            Ok(Box::new(OpenAiCompatProvider::new(
                "Custom",
                base_url,
                api_key,
                AuthStyle::Bearer,
                true,
            )))
        })
    } else if let Some(base_url) = name.strip_prefix("anthropic-custom:") {
        Some(if base_url.is_empty() {
            Err(anyhow::anyhow!(
                "Anthropic-custom provider requires a URL. Format: anthropic-custom:https://your-api.com"
            ))
        } else {
            Ok(Box::new(
                super::anthropic::AnthropicProvider::with_base_url(api_key, Some(base_url)),
            ))
        })
    } else {
        None
    }
}

fn resolve_gemini_vertex_target(name: &str) -> anyhow::Result<Option<(String, String)>> {
    let trimmed = name.trim();
    if let Some(selector) = trimmed
        .strip_prefix("gemini-vertex:")
        .or_else(|| trimmed.strip_prefix("vertex-gemini:"))
    {
        let (project, location) = selector.split_once('/').ok_or_else(|| {
            anyhow::anyhow!(
                "gemini-vertex selector requires project and location. Format: gemini-vertex:<project>/<location>"
            )
        })?;
        let project = project.trim();
        let location = location.trim();
        if project.is_empty() || location.is_empty() {
            anyhow::bail!(
                "gemini-vertex selector requires non-empty project and location. Format: gemini-vertex:<project>/<location>"
            );
        }
        return Ok(Some((project.to_string(), location.to_string())));
    }

    if trimmed.eq_ignore_ascii_case("gemini-vertex")
        || trimmed.eq_ignore_ascii_case("vertex-gemini")
    {
        let project = ["VERTEX_AI_PROJECT", "GOOGLE_CLOUD_PROJECT", "GCLOUD_PROJECT"]
            .iter()
            .find_map(|key| std::env::var(key).ok())
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!(
                "gemini-vertex requires a project. Set VERTEX_AI_PROJECT / GOOGLE_CLOUD_PROJECT or use gemini-vertex:<project>/<location>"
            ))?;
        let location = [
            "VERTEX_AI_LOCATION",
            "GOOGLE_CLOUD_LOCATION",
            "GOOGLE_CLOUD_REGION",
        ]
        .iter()
        .find_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "global".to_string());
        return Ok(Some((project, location)));
    }

    Ok(None)
}

/// Return `true` when the provider name selects the local `Codex` CLI subprocess backend.
fn is_codex_cli_selector(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case("codex-cli")
}

/// Return `true` when the provider name selects the `OpenAI` Codex Responses API backend.
fn is_openai_codex_selector(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case("openai-codex")
}

/// Create a provider instance by name, resolving API keys
/// automatically.
///
/// # Errors
///
/// Returns an error if the provider name is unsupported or
/// construction fails.
pub fn create_provider(name: &str, api_key: Option<&str>) -> anyhow::Result<Box<dyn Provider>> {
    create_provider_with_security(name, api_key, None)
}

/// Create a provider instance by name with an optional runtime security policy.
///
/// # Errors
///
/// Returns an error if the provider name is unsupported or construction fails.
pub fn create_provider_with_security(
    name: &str,
    api_key: Option<&str>,
    security: Option<&SecurityPolicy>,
) -> anyhow::Result<Box<dyn Provider>> {
    if is_openai_codex_selector(name) {
        let credential = resolve_api_key_details("openai", api_key);
        let base_url = codex_responses_base_url(credential.key.as_deref());
        tracing::info!(
            base_url,
            oauth = !is_openai_api_key(credential.key.as_deref()),
            credential_source = ?credential.source,
            "OpenAI Codex provider route"
        );
        return Ok(Box::new(OpenAiCompatProvider::new(
            "OpenAI Codex",
            base_url,
            credential.key.as_deref(),
            AuthStyle::Bearer,
            false,
        )));
    }

    if is_codex_cli_selector(name) {
        let Some(security) = security else {
            anyhow::bail!(
                "codex-cli provider requires a runtime SecurityPolicy for process-spawn enforcement"
            );
        };
        tracing::debug!(
            provider = "codex-cli",
            credential_source = ?ProviderCredentialSource::ProviderLocalCredential("codex-cli"),
            "resolved provider credential source"
        );
        return Ok(Box::new(super::codex_cli::CodexCliProvider::new(Some(
            security,
        ))));
    }

    if let Some((project, location)) = resolve_gemini_vertex_target(name)? {
        let credential = resolve_api_key_details("gemini-vertex", api_key);
        tracing::debug!(
            provider = name,
            credential_source = ?credential.source,
            "resolved provider credential source"
        );
        return Ok(Box::new(super::gemini::GeminiProvider::new_vertex(
            project,
            location,
            credential.key.as_deref(),
        )));
    }

    let normalized_name = normalize_provider_alias(name);
    let credential_name = credential_resolution_name_for_selector(normalized_name);
    let credential = resolve_api_key_details(credential_name, api_key);
    tracing::debug!(
        provider = normalized_name,
        credential_source = ?credential.source,
        "resolved provider credential source"
    );
    let api_key = credential.key.as_deref();

    // ── Primary providers (custom implementations) ───────
    match normalized_name {
        "openrouter" => {
            return Ok(Box::new(super::openrouter::OpenRouterProvider::new(
                api_key,
            )));
        }
        "anthropic" => return Ok(Box::new(super::anthropic::AnthropicProvider::new(api_key))),
        "openai" => return Ok(Box::new(super::openai::OpenAiProvider::new(api_key))),
        "ollama" => return Ok(Box::new(super::ollama::OllamaProvider::new(None))),
        "gemini" => {
            return Ok(Box::new(super::gemini::GeminiProvider::new(api_key)));
        }
        "minimax" => return Ok(Box::new(super::minimax::MiniMaxProvider::new(api_key))),
        _ => {}
    }

    // ── OpenAI-compatible providers ──────────────────────
    if let Some(spec) = compatible_provider_spec(normalized_name) {
        return Ok(Box::new(OpenAiCompatProvider::new(
            spec.display_name.as_str(),
            spec.base_url.as_str(),
            api_key,
            AuthStyle::Bearer,
            true,
        )));
    }

    if let Some(result) = create_custom_provider(normalized_name, api_key) {
        return result;
    }

    anyhow::bail!(
        "Unknown provider: {name}. Check README for supported providers or run `asterel onboard --interactive` to reconfigure.\n\
         Tip: Use \"custom:https://your-api.com\" for OpenAI-compatible endpoints.\n\
         Tip: Use \"anthropic-custom:https://your-api.com\" for Anthropic-compatible endpoints."
    )
}

/// Wrap a freshly-constructed provider in `OAuthRecoveryProvider`.
///
/// Wires up the `recover` callback (runs `recover_oauth_profile_for_provider`)
/// and the `rebuild` callback (reads the refreshed key via `AuthBroker` and
/// reconstructs the provider). Both callbacks run in a blocking thread pool
/// task since they may perform file I/O.
fn create_provider_with_runtime_recovery(
    config: &crate::config::Config,
    name: &str,
    api_key: Option<&str>,
    security: Option<&SecurityPolicy>,
) -> anyhow::Result<Box<dyn Provider>> {
    create_provider_with_runtime_recovery_for_credential_provider(
        config, name, name, api_key, security,
    )
}

fn create_provider_with_runtime_recovery_for_credential_provider(
    config: &crate::config::Config,
    name: &str,
    credential_provider_name: &str,
    api_key: Option<&str>,
    security: Option<&SecurityPolicy>,
) -> anyhow::Result<Box<dyn Provider>> {
    let provider_name = name.to_string();
    let credential_provider_name = credential_provider_name.to_string();
    let runtime_security = security.cloned();
    let initial_provider: Arc<dyn Provider> = Arc::from(create_provider_with_security(
        name,
        api_key,
        runtime_security.as_ref(),
    )?);
    let config = Arc::new(config.clone());

    let recover = {
        let config = Arc::clone(&config);
        let credential_provider_name = credential_provider_name.clone();
        Arc::new(move |_provider: &str| {
            crate::security::auth::recover_oauth_profile_for_provider(
                &config,
                &credential_provider_name,
            )
        })
    };

    let rebuild = {
        let config = Arc::clone(&config);
        let runtime_security = runtime_security.clone();
        let provider_name = provider_name.clone();
        let credential_provider_name = credential_provider_name.clone();
        Arc::new(move |_provider: &str| {
            let broker = crate::security::auth::AuthBroker::load_or_init(&config)?;
            let refreshed_key = broker.resolve_provider_key(&credential_provider_name);
            Ok(Arc::from(create_provider_with_security(
                &provider_name,
                refreshed_key.as_deref(),
                runtime_security.as_ref(),
            )?) as Arc<dyn Provider>)
        })
    };

    Ok(Box::new(OAuthRecoveryProvider::new(
        &provider_name,
        initial_provider,
        recover,
        rebuild,
    )))
}

/// Create a provider wrapped in OAuth token recovery logic.
///
/// # Errors
///
/// Returns an error if the base provider cannot be created or
/// recovery wiring fails.
pub fn create_provider_with_oauth_recovery(
    config: &crate::config::Config,
    name: &str,
    api_key: Option<&str>,
) -> anyhow::Result<Box<dyn Provider>> {
    create_provider_with_oauth_recovery_and_security(config, name, api_key, None)
}

/// Create a provider wrapped in OAuth token recovery logic with runtime security.
///
/// # Errors
///
/// Returns an error if the base provider cannot be created or recovery wiring fails.
pub fn create_provider_with_oauth_recovery_and_security(
    config: &crate::config::Config,
    name: &str,
    api_key: Option<&str>,
    security: Option<&SecurityPolicy>,
) -> anyhow::Result<Box<dyn Provider>> {
    create_provider_with_runtime_recovery(config, name, api_key, security)
}

/// Create a provider wrapped in OAuth token recovery logic with a separate
/// provider name for auth recovery and credential lookup.
///
/// # Errors
///
/// Returns an error if the base provider cannot be created or recovery wiring fails.
pub(crate) fn create_provider_with_oauth_recovery_and_security_for_credential_provider(
    config: &crate::config::Config,
    name: &str,
    credential_provider_name: &str,
    api_key: Option<&str>,
    security: Option<&SecurityPolicy>,
) -> anyhow::Result<Box<dyn Provider>> {
    create_provider_with_runtime_recovery_for_credential_provider(
        config,
        name,
        credential_provider_name,
        api_key,
        security,
    )
}

/// Build a resilient provider with retry and fallback using a
/// custom key resolver.
///
/// # Errors
/// Returns an error if the primary provider cannot be created.
pub fn create_resilient_provider_with_resolver<F>(
    primary_name: &str,
    reliability: &crate::config::ReliabilityConfig,
    resolve_api_key_for_provider: F,
) -> anyhow::Result<Box<dyn Provider>>
where
    F: FnMut(&str) -> Option<String>,
{
    create_resilient_provider_with_resolver_and_security(
        primary_name,
        reliability,
        None,
        resolve_api_key_for_provider,
    )
}

/// Build a resilient provider with retry and fallback using a custom key resolver
/// and optional runtime security.
///
/// # Errors
/// Returns an error if the primary provider cannot be created.
pub fn create_resilient_provider_with_resolver_and_security<F>(
    primary_name: &str,
    reliability: &crate::config::ReliabilityConfig,
    security: Option<&SecurityPolicy>,
    mut resolve_api_key_for_provider: F,
) -> anyhow::Result<Box<dyn Provider>>
where
    F: FnMut(&str) -> Option<String>,
{
    let mut providers: Vec<(String, Box<dyn Provider>)> =
        Vec::with_capacity(1 + reliability.fallback_providers.len());

    let primary_key = resolve_api_key_for_provider(primary_name);
    providers.push((
        primary_name.to_string(),
        create_provider_with_security(primary_name, primary_key.as_deref(), security)?,
    ));

    for fallback in &reliability.fallback_providers {
        if fallback == primary_name || providers.iter().any(|(name, _)| name == fallback) {
            continue;
        }

        let fallback_key = resolve_api_key_for_provider(fallback);

        match create_provider_with_security(fallback, fallback_key.as_deref(), security) {
            Ok(provider) => providers.push((fallback.clone(), provider)),
            Err(e) => {
                tracing::warn!(
                    fallback_provider = fallback,
                    "Ignoring invalid fallback provider: {e}"
                );
            }
        }
    }

    Ok(Box::new(ReliableProvider::new(
        providers,
        reliability.provider_retries,
        reliability.provider_backoff_ms,
    )))
}

/// Build a resilient provider with retry, fallback, and OAuth
/// token recovery on each sub-provider.
///
/// # Errors
/// Returns an error if the primary provider with runtime recovery cannot be created.
pub fn create_resilient_provider_with_oauth_recovery<F>(
    config: &crate::config::Config,
    primary_name: &str,
    reliability: &crate::config::ReliabilityConfig,
    resolve_api_key_for_provider: F,
) -> anyhow::Result<Box<dyn Provider>>
where
    F: FnMut(&str) -> Option<String>,
{
    create_resilient_provider_with_oauth_recovery_and_security(
        config,
        primary_name,
        reliability,
        None,
        resolve_api_key_for_provider,
    )
}

/// Build a resilient provider with retry, fallback, and OAuth token recovery on each
/// sub-provider, while preserving runtime security for providers that spawn subprocesses.
///
/// # Errors
/// Returns an error if the primary provider with runtime recovery cannot be created.
pub fn create_resilient_provider_with_oauth_recovery_and_security<F>(
    config: &crate::config::Config,
    primary_name: &str,
    reliability: &crate::config::ReliabilityConfig,
    security: Option<&SecurityPolicy>,
    resolve_api_key_for_provider: F,
) -> anyhow::Result<Box<dyn Provider>>
where
    F: FnMut(&str) -> Option<String>,
{
    create_resilient_provider_with_oauth_recovery_and_security_for_credential_provider(
        config,
        primary_name,
        primary_name,
        reliability,
        security,
        resolve_api_key_for_provider,
    )
}

/// Build a resilient provider with retry, fallback, OAuth token recovery, and a separate
/// primary provider name for auth recovery and credential lookup.
///
/// # Errors
/// Returns an error if the primary provider with runtime recovery cannot be created.
pub(crate) fn create_resilient_provider_with_oauth_recovery_and_security_for_credential_provider<
    F,
>(
    config: &crate::config::Config,
    primary_name: &str,
    primary_credential_provider_name: &str,
    reliability: &crate::config::ReliabilityConfig,
    security: Option<&SecurityPolicy>,
    mut resolve_api_key_for_provider: F,
) -> anyhow::Result<Box<dyn Provider>>
where
    F: FnMut(&str) -> Option<String>,
{
    let mut providers: Vec<(String, Box<dyn Provider>)> = Vec::new();

    let primary_key = resolve_api_key_for_provider(primary_name);
    providers.push((
        primary_name.to_string(),
        create_provider_with_runtime_recovery_for_credential_provider(
            config,
            primary_name,
            primary_credential_provider_name,
            primary_key.as_deref(),
            security,
        )?,
    ));

    for fallback in &reliability.fallback_providers {
        if fallback == primary_name || providers.iter().any(|(name, _)| name == fallback) {
            continue;
        }

        let fallback_key = resolve_api_key_for_provider(fallback);

        match create_provider_with_runtime_recovery(
            config,
            fallback,
            fallback_key.as_deref(),
            security,
        ) {
            Ok(provider) => providers.push((fallback.clone(), provider)),
            Err(e) => {
                tracing::warn!(
                    fallback_provider = fallback,
                    "Ignoring invalid fallback provider: {e}"
                );
            }
        }
    }

    Ok(Box::new(ReliableProvider::new(
        providers,
        reliability.provider_retries,
        reliability.provider_backoff_ms,
    )))
}

/// Standard `OpenAI` API keys start with `sk-`.  OAuth tokens from the
/// Codex CLI do not — they are JWTs or opaque bearer tokens.
fn is_openai_api_key(key: Option<&str>) -> bool {
    key.is_some_and(|k| k.starts_with("sk-"))
}

/// Choose the correct Responses API endpoint based on credential type.
///
/// OAuth tokens lack the `api.responses.write` scope required by the
/// public `/v1/responses` endpoint.  The Codex CLI itself works around
/// this by calling the `ChatGPT` backend-api, which accepts the same
/// request format without requiring that scope.
fn codex_responses_base_url(key: Option<&str>) -> &'static str {
    if is_openai_api_key(key) {
        "https://api.openai.com/v1/responses"
    } else {
        "https://chatgpt.com/backend-api/codex/responses"
    }
}

/// Build a resilient provider using the default key resolution
/// strategy.
///
/// # Errors
/// Returns an error if resilient provider construction fails.
pub fn create_resilient_provider(
    primary_name: &str,
    api_key: Option<&str>,
    reliability: &crate::config::ReliabilityConfig,
) -> anyhow::Result<Box<dyn Provider>> {
    create_resilient_provider_with_resolver(primary_name, reliability, |provider_name| {
        resolve_api_key(provider_name, api_key)
    })
}

#[cfg(test)]
mod credential_source_tests {
    use crate::utils::test_env::EnvVarGuard;

    use super::{
        ProviderCredentialSource, credential_resolution_name_for_selector,
        missing_credential_source, resolve_api_key_details,
    };

    #[test]
    fn credential_source_prefers_explicit_parameter() {
        let credential = resolve_api_key_details("minimax", Some(" explicit-key "));

        assert_eq!(credential.key.as_deref(), Some("explicit-key"));
        assert_eq!(
            credential.source,
            ProviderCredentialSource::ExplicitParameter
        );
    }

    #[test]
    fn credential_source_uses_provider_environment_before_generic_fallback() {
        let _env = EnvVarGuard::set("MINIMAX_API_KEY", " provider-env-key ");

        let credential = resolve_api_key_details("minimax", None);

        assert_eq!(credential.key.as_deref(), Some("provider-env-key"));
        assert_eq!(
            credential.source,
            ProviderCredentialSource::ProviderEnvironment("MINIMAX_API_KEY".to_string())
        );
    }

    #[test]
    fn credential_source_uses_generic_environment_after_provider_environment() {
        let _env = EnvVarGuard::set("ASTEREL_API_KEY", " generic-env-key ");

        let credential = resolve_api_key_details("unknown-custom-provider", None);

        assert_eq!(credential.key.as_deref(), Some("generic-env-key"));
        assert_eq!(
            credential.source,
            ProviderCredentialSource::GenericEnvironment("ASTEREL_API_KEY".to_string())
        );
    }

    #[test]
    fn credential_source_marks_provider_local_auth_exceptions() {
        assert_eq!(
            missing_credential_source("gemini"),
            ProviderCredentialSource::ProviderLocalCredential("gemini-cli-oauth")
        );
        assert_eq!(
            missing_credential_source("gemini-vertex"),
            ProviderCredentialSource::ProviderLocalCredential("google-adc")
        );
    }

    #[test]
    fn credential_source_marks_no_key_local_provider() {
        let _env = EnvVarGuard::set("ASTEREL_API_KEY", " ignored-generic-key ");

        let credential = resolve_api_key_details("ollama", Some("ignored-explicit-key"));

        assert_eq!(credential.key, None);
        assert_eq!(
            credential.source,
            ProviderCredentialSource::IntentionallyUnauthenticatedLocal
        );
    }

    #[test]
    fn custom_selectors_reuse_canonical_provider_credential_names() {
        assert_eq!(
            credential_resolution_name_for_selector("custom:https://proxy.example/v1"),
            "custom"
        );
        assert_eq!(
            credential_resolution_name_for_selector("anthropic-custom:https://proxy.example"),
            "anthropic-custom"
        );
    }
}

#[cfg(test)]
mod codex_route_tests {
    use super::*;

    #[test]
    fn api_key_routes_to_public_api() {
        let url = codex_responses_base_url(Some("sk-proj-abc123"));
        assert_eq!(url, "https://api.openai.com/v1/responses");
    }

    #[test]
    fn oauth_token_routes_to_backend_api() {
        let url = codex_responses_base_url(Some("eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.xxx"));
        assert_eq!(url, "https://chatgpt.com/backend-api/codex/responses");
    }

    #[test]
    fn no_key_routes_to_backend_api() {
        let url = codex_responses_base_url(None);
        assert_eq!(url, "https://chatgpt.com/backend-api/codex/responses");
    }

    #[test]
    fn is_openai_api_key_recognizes_sk_prefix() {
        assert!(is_openai_api_key(Some("sk-abc")));
        assert!(is_openai_api_key(Some("sk-proj-abc")));
        assert!(!is_openai_api_key(Some("eyJhbG...")));
        assert!(!is_openai_api_key(None));
    }
}
