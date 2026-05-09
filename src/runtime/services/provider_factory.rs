use std::sync::Arc;

use anyhow::{Context, Result};

use super::bootstrap::RuntimeModelSelection;
use super::plugins::runtime_mcp_tool_provider;
use crate::config::Config;
use crate::contracts::channels::ChannelCapabilities;
use crate::contracts::providers::normalize_provider_alias;
use crate::core::memory::Memory;
use crate::core::providers::{self, Provider};
use crate::core::tools::{self, ToolRegistry, build_tool_registry_from_parts};
use crate::security::SecurityPolicy;
use crate::security::auth::AuthBroker;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeProviderCredentialSource {
    PreferredApiKey,
    AuthBroker,
    ConfigApiKeyFallback,
    IntentionallyUnauthenticatedLocal,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeProviderCredential {
    pub(crate) key: Option<String>,
    pub(crate) source: RuntimeProviderCredentialSource,
}

fn resolve_runtime_provider_credential(
    provider_name: &str,
    preferred_api_key: Option<&str>,
    broker_key: Option<&str>,
    config_api_key: Option<&str>,
) -> RuntimeProviderCredential {
    if normalize_provider_alias(provider_name) == "ollama" {
        return RuntimeProviderCredential {
            key: None,
            source: RuntimeProviderCredentialSource::IntentionallyUnauthenticatedLocal,
        };
    }

    if let Some(key) = preferred_api_key
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        return RuntimeProviderCredential {
            key: Some(key.to_owned()),
            source: RuntimeProviderCredentialSource::PreferredApiKey,
        };
    }

    if let Some(key) = broker_key.map(str::trim).filter(|key| !key.is_empty()) {
        return RuntimeProviderCredential {
            key: Some(key.to_owned()),
            source: RuntimeProviderCredentialSource::AuthBroker,
        };
    }

    if let Some(key) = config_api_key.map(str::trim).filter(|key| !key.is_empty()) {
        return RuntimeProviderCredential {
            key: Some(key.to_owned()),
            source: RuntimeProviderCredentialSource::ConfigApiKeyFallback,
        };
    }

    RuntimeProviderCredential {
        key: None,
        source: RuntimeProviderCredentialSource::Missing,
    }
}

#[must_use]
pub fn provider_selector_with_api_base(provider: &str, api_base: Option<&str>) -> String {
    let Some(api_base) = api_base.map(str::trim).filter(|value| !value.is_empty()) else {
        return provider.to_string();
    };
    if provider.starts_with("custom:") || provider.starts_with("anthropic-custom:") {
        return provider.to_string();
    }

    if provider.eq_ignore_ascii_case("openai-codex") {
        tracing::warn!(
            provider,
            api_base,
            "ignoring api_base for OpenAI Codex provider because it uses a provider-specific route"
        );
        return provider.to_string();
    }

    let normalized = normalize_provider_alias(provider);
    if normalized == "custom" {
        format!("custom:{api_base}")
    } else if normalized == "anthropic-custom" || normalized == "anthropic" {
        format!("anthropic-custom:{api_base}")
    } else if normalized == "openai"
        || providers::catalog::compatible_provider_spec(normalized).is_some()
    {
        format!("custom:{api_base}")
    } else {
        tracing::warn!(
            provider,
            api_base,
            "ignoring api_base for provider without openai-compatible custom-base routing"
        );
        provider.to_string()
    }
}

/// Create a resilient provider with OAuth recovery semantics.
///
/// # Errors
///
/// Returns an error if the provider cannot be created.
pub fn create_resilient_provider(
    config: &Config,
    auth_broker: &AuthBroker,
    security: &SecurityPolicy,
    provider_name: &str,
    preferred_api_key: Option<&str>,
) -> Result<Arc<dyn Provider>> {
    create_resilient_provider_with_credential_provider(
        config,
        auth_broker,
        security,
        provider_name,
        provider_name,
        preferred_api_key,
    )
}

/// Create a resilient provider with OAuth recovery semantics while using a
/// separate provider name for credential lookup.
///
/// # Errors
///
/// Returns an error if the provider cannot be created.
pub fn create_resilient_provider_with_credential_provider(
    config: &Config,
    auth_broker: &AuthBroker,
    security: &SecurityPolicy,
    provider_name: &str,
    credential_provider_name: &str,
    preferred_api_key: Option<&str>,
) -> Result<Arc<dyn Provider>> {
    Ok(Arc::from(
        create_resilient_provider_box_with_credential_provider(
            config,
            auth_broker,
            security,
            provider_name,
            credential_provider_name,
            preferred_api_key,
        )?,
    ))
}

/// Create a resilient provider with OAuth recovery semantics.
///
/// # Errors
///
/// Returns an error if the provider cannot be created.
pub fn create_resilient_provider_box(
    config: &Config,
    auth_broker: &AuthBroker,
    security: &SecurityPolicy,
    provider_name: &str,
    preferred_api_key: Option<&str>,
) -> Result<Box<dyn Provider>> {
    create_resilient_provider_box_with_credential_provider(
        config,
        auth_broker,
        security,
        provider_name,
        provider_name,
        preferred_api_key,
    )
}

/// Create a resilient provider with OAuth recovery semantics while using a
/// separate provider name for credential lookup.
///
/// # Errors
///
/// Returns an error if the provider cannot be created.
pub fn create_resilient_provider_box_with_credential_provider(
    config: &Config,
    auth_broker: &AuthBroker,
    security: &SecurityPolicy,
    provider_name: &str,
    credential_provider_name: &str,
    preferred_api_key: Option<&str>,
) -> Result<Box<dyn Provider>> {
    let preferred_key = preferred_api_key.map(ToOwned::to_owned);
    providers::create_resilient_provider_with_oauth_recovery_and_security_for_credential_provider(
        config,
        provider_name,
        credential_provider_name,
        &config.reliability,
        Some(security),
        |name| {
            let credential = if name == provider_name {
                resolve_runtime_provider_credential(
                    credential_provider_name,
                    preferred_key.as_deref(),
                    auth_broker
                        .resolve_provider_key(credential_provider_name)
                        .as_deref(),
                    None,
                )
            } else {
                resolve_runtime_provider_credential(
                    name,
                    None,
                    auth_broker.resolve_provider_key(name).as_deref(),
                    None,
                )
            };
            tracing::debug!(
                provider = name,
                credential_source = ?credential.source,
                "resolved provider credential source"
            );
            credential.key
        },
    )
    .with_context(|| format!("create resilient provider '{provider_name}'"))
}

/// Create a direct provider with OAuth recovery semantics.
///
/// # Errors
///
/// Returns an error if the provider cannot be created.
pub fn create_provider_box(
    config: &Config,
    auth_broker: &AuthBroker,
    security: &SecurityPolicy,
    provider_name: &str,
    preferred_api_key: Option<&str>,
) -> Result<Box<dyn Provider>> {
    let credential = resolve_runtime_provider_credential(
        provider_name,
        preferred_api_key,
        auth_broker.resolve_provider_key(provider_name).as_deref(),
        None,
    );
    tracing::debug!(
        provider = provider_name,
        credential_source = ?credential.source,
        "resolved provider credential source"
    );
    providers::create_provider_with_oauth_recovery_and_security(
        config,
        provider_name,
        credential.key.as_deref(),
        Some(security),
    )
    .with_context(|| format!("create provider '{provider_name}'"))
}

fn create_taste_provider(
    config: &Config,
    auth_broker: Option<&AuthBroker>,
    security: &SecurityPolicy,
    provider_name: &str,
    credential_provider_name: &str,
    preferred_api_key: Option<&str>,
) -> Option<Arc<dyn Provider>> {
    if !config.taste.enabled {
        return None;
    }

    let credential = resolve_runtime_provider_credential(
        credential_provider_name,
        preferred_api_key,
        auth_broker
            .and_then(|broker| broker.resolve_provider_key(credential_provider_name))
            .as_deref(),
        config.api_key.as_deref(),
    );
    tracing::debug!(
        provider = provider_name,
        credential_source = ?credential.source,
        "resolved taste provider credential source"
    );
    providers::create_provider_with_oauth_recovery_and_security_for_credential_provider(
        config,
        provider_name,
        credential_provider_name,
        credential.key.as_deref(),
        Some(security),
    )
    .ok()
    .map(Arc::from)
}

/// Build a shared tool registry from explicit runtime parts.
#[must_use]
pub fn build_tool_registry(
    config: &Config,
    security: &Arc<SecurityPolicy>,
    memory: &Arc<dyn Memory>,
    auth_broker: Option<&AuthBroker>,
    model_selection: &RuntimeModelSelection,
    channel_capabilities: Option<&ChannelCapabilities>,
) -> Arc<ToolRegistry> {
    let composio_key = if config.composio.enabled {
        config.composio.api_key.as_deref()
    } else {
        None
    };

    build_tool_registry_from_parts(tools::ToolRegistryConfig {
        security,
        memory: Arc::clone(memory),
        composio_key,
        browser: &config.browser,
        tools: &config.tools,
        mcp: Some(&config.mcp),
        mcp_tool_provider: runtime_mcp_tool_provider(),
        taste: &config.taste,
        taste_provider: {
            let provider_selector = provider_selector_with_api_base(
                &model_selection.provider,
                model_selection.api_base.as_deref(),
            );
            create_taste_provider(
                config,
                auth_broker,
                security,
                &provider_selector,
                &model_selection.provider,
                model_selection.api_key.as_deref(),
            )
        },
        taste_model: &model_selection.model,
        channel_capabilities,
        codespace: &config.codespace,
    })
}

#[cfg(test)]
mod credential_source_tests {
    use super::{
        RuntimeProviderCredentialSource, provider_selector_with_api_base,
        resolve_runtime_provider_credential,
    };

    #[test]
    fn credential_source_prefers_explicit_api_key() {
        let credential = resolve_runtime_provider_credential(
            "openai",
            Some("preferred-key"),
            Some("broker-key"),
            Some("config-key"),
        );

        assert_eq!(credential.key.as_deref(), Some("preferred-key"));
        assert_eq!(
            credential.source,
            RuntimeProviderCredentialSource::PreferredApiKey
        );
    }

    #[test]
    fn credential_source_ignores_blank_preferred_key() {
        let credential = resolve_runtime_provider_credential(
            "openai",
            Some("   "),
            Some("broker-key"),
            Some("config-key"),
        );

        assert_eq!(credential.key.as_deref(), Some("broker-key"));
        assert_eq!(
            credential.source,
            RuntimeProviderCredentialSource::AuthBroker
        );
    }

    #[test]
    fn credential_source_uses_broker_when_no_preferred_key() {
        let credential = resolve_runtime_provider_credential(
            "openai",
            None,
            Some("broker-key"),
            Some("config-key"),
        );

        assert_eq!(credential.key.as_deref(), Some("broker-key"));
        assert_eq!(
            credential.source,
            RuntimeProviderCredentialSource::AuthBroker
        );
    }

    #[test]
    fn credential_source_uses_config_fallback_for_taste_path() {
        let credential =
            resolve_runtime_provider_credential("openai", None, None, Some("config-key"));

        assert_eq!(credential.key.as_deref(), Some("config-key"));
        assert_eq!(
            credential.source,
            RuntimeProviderCredentialSource::ConfigApiKeyFallback
        );
    }

    #[test]
    fn credential_source_marks_missing_for_local_or_provider_local_paths() {
        let credential = resolve_runtime_provider_credential("openai", None, None, None);

        assert_eq!(credential.key, None);
        assert_eq!(credential.source, RuntimeProviderCredentialSource::Missing);
    }

    #[test]
    fn credential_source_marks_runtime_local_provider_as_unauthenticated() {
        let credential = resolve_runtime_provider_credential(
            "ollama",
            Some("ignored-preferred"),
            Some("ignored-broker"),
            Some("ignored-config"),
        );

        assert_eq!(credential.key, None);
        assert_eq!(
            credential.source,
            RuntimeProviderCredentialSource::IntentionallyUnauthenticatedLocal
        );
    }

    #[test]
    fn api_base_promotes_provider_to_custom_selector() {
        assert_eq!(
            provider_selector_with_api_base("openai", Some(" https://proxy.example/v1 ")),
            "custom:https://proxy.example/v1"
        );
        assert_eq!(
            provider_selector_with_api_base("custom", Some("https://proxy.example/v1")),
            "custom:https://proxy.example/v1"
        );
        assert_eq!(
            provider_selector_with_api_base("anthropic", Some("https://claude.example")),
            "anthropic-custom:https://claude.example"
        );
        assert_eq!(
            provider_selector_with_api_base("anthropic-custom", Some("https://claude.example")),
            "anthropic-custom:https://claude.example"
        );
    }

    #[test]
    fn api_base_does_not_rewrite_existing_custom_selector() {
        assert_eq!(
            provider_selector_with_api_base(
                "custom:https://already.example",
                Some("https://new.example")
            ),
            "custom:https://already.example"
        );
        assert_eq!(
            provider_selector_with_api_base(
                "anthropic-custom:https://already.example",
                Some("https://new.example")
            ),
            "anthropic-custom:https://already.example"
        );
    }

    #[test]
    fn api_base_does_not_rewrite_provider_specific_protocols() {
        assert_eq!(
            provider_selector_with_api_base("ollama", Some("http://127.0.0.1:11434")),
            "ollama"
        );
        assert_eq!(
            provider_selector_with_api_base("gemini", Some("https://generativelanguage.example")),
            "gemini"
        );
        assert_eq!(
            provider_selector_with_api_base("openai-codex", Some("https://proxy.example/v1")),
            "openai-codex"
        );
    }
}
