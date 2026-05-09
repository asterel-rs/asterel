//! Shared provider catalog metadata used across runtime construction,
//! onboarding, integration display, and auth/key lookup flows.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::Deserialize;

use crate::contracts::providers::normalize_provider_alias;

/// Shared metadata for an OpenAI-compatible provider surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibleProviderSpec {
    pub display_name: String,
    pub base_url: String,
}

/// OAuth/login surface associated with a provider selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderOAuthFlow {
    Codex,
    Claude,
}

impl ProviderOAuthFlow {
    #[must_use]
    pub const fn source_name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

/// Onboarding auth UX for a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ProviderAuthMethod {
    #[serde(rename = "api_key_only")]
    ApiKeyOnly,
    #[serde(rename = "api_key_or_oauth")]
    ApiKeyOrOAuth,
    #[serde(rename = "api_key_or_application_default_credentials")]
    ApiKeyOrApplicationDefaultCredentials,
    #[serde(rename = "api_key_or_setup_token")]
    ApiKeyOrSetupToken,
    #[serde(rename = "no_key_required")]
    NoKeyRequired,
}

/// Logical onboarding grouping for built-in providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderOnboardingTier {
    Recommended,
    Fast,
    Gateway,
    Specialized,
    Local,
}

impl ProviderOnboardingTier {
    #[must_use]
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Recommended),
            1 => Some(Self::Fast),
            2 => Some(Self::Gateway),
            3 => Some(Self::Specialized),
            4 => Some(Self::Local),
            _ => None,
        }
    }
}

/// Curated model choice shown during onboarding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderModelChoice {
    pub model: String,
    pub label: String,
}

/// Integration-registry display metadata for an AI provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderIntegrationSpec {
    pub name: String,
    pub description: String,
}

/// Onboarding metadata for a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderOnboardingSpec {
    pub tier: ProviderOnboardingTier,
    pub label: String,
    pub auth_method: ProviderAuthMethod,
    pub models: Vec<ProviderModelChoice>,
}

/// Static metadata for a supported provider identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCatalogEntry {
    pub id: String,
    pub display_name: String,
    pub aliases: Vec<String>,
    pub api_key_env_vars: Vec<String>,
    pub display_api_key_env_var: String,
    pub key_url: Option<String>,
    pub compatible: Option<CompatibleProviderSpec>,
    pub onboarding: Option<ProviderOnboardingSpec>,
    pub integration: Option<ProviderIntegrationSpec>,
    pub scan_env: bool,
}

#[derive(Debug)]
struct ProviderRegistry {
    entries: Vec<ProviderCatalogEntry>,
    by_id: HashMap<String, usize>,
}

#[derive(Debug, Deserialize)]
struct RawProviderRegistry {
    providers: Vec<RawProviderCatalogEntry>,
}

#[derive(Debug, Deserialize)]
struct RawProviderCatalogEntry {
    id: String,
    display_name: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    api_key_env_vars: Vec<String>,
    display_api_key_env_var: String,
    #[serde(default)]
    key_url: Option<String>,
    #[serde(default)]
    compatible: Option<RawCompatibleProviderSpec>,
    #[serde(default)]
    onboarding: Option<RawProviderOnboardingSpec>,
    #[serde(default)]
    integration: Option<RawProviderIntegrationSpec>,
    #[serde(default)]
    scan_env: bool,
}

#[derive(Debug, Deserialize)]
struct RawCompatibleProviderSpec {
    display_name: String,
    base_url: String,
}

#[derive(Debug, Deserialize)]
struct RawProviderIntegrationSpec {
    name: String,
    description: String,
}

#[derive(Debug, Deserialize)]
struct RawProviderOnboardingSpec {
    tier: ProviderOnboardingTier,
    label: String,
    auth_method: ProviderAuthMethod,
    #[serde(default)]
    models: Vec<RawProviderModelChoice>,
}

#[derive(Debug, Deserialize)]
struct RawProviderModelChoice {
    model: String,
    label: String,
}

static PROVIDER_REGISTRY: OnceLock<ProviderRegistry> = OnceLock::new();

fn provider_registry() -> &'static ProviderRegistry {
    PROVIDER_REGISTRY.get_or_init(load_provider_registry)
}

fn normalize_aliases(
    raw_aliases: Vec<String>,
    id: &str,
    seen_aliases: &mut HashSet<String>,
) -> Vec<String> {
    let aliases = raw_aliases
        .into_iter()
        .map(|alias| alias.trim().to_ascii_lowercase())
        .filter(|alias| !alias.is_empty())
        .collect::<Vec<_>>();
    for alias in &aliases {
        assert!(
            seen_aliases.insert(alias.clone()),
            "duplicate provider alias: {alias}"
        );
        assert_ne!(
            alias, id,
            "provider alias must not equal canonical id: {id}"
        );
    }
    aliases
}

fn normalize_compatible_spec(
    id: &str,
    raw: Option<RawCompatibleProviderSpec>,
) -> Option<CompatibleProviderSpec> {
    let compatible = raw.map(|spec| CompatibleProviderSpec {
        display_name: spec.display_name.trim().to_string(),
        base_url: spec.base_url.trim_end_matches('/').to_string(),
    });
    if let Some(spec) = &compatible {
        assert!(
            !spec.display_name.is_empty(),
            "provider {id} compatible.display_name cannot be empty"
        );
        assert!(
            !spec.base_url.is_empty(),
            "provider {id} compatible.base_url cannot be empty"
        );
    }
    compatible
}

fn normalize_onboarding_spec(
    id: &str,
    raw: Option<RawProviderOnboardingSpec>,
) -> Option<ProviderOnboardingSpec> {
    raw.map(|spec| {
        assert!(
            !spec.models.is_empty(),
            "provider {id} onboarding models cannot be empty"
        );
        assert!(
            !spec.label.trim().is_empty(),
            "provider {id} onboarding label cannot be empty"
        );
        ProviderOnboardingSpec {
            tier: spec.tier,
            label: spec.label.trim().to_string(),
            auth_method: spec.auth_method,
            models: spec
                .models
                .into_iter()
                .map(|choice| ProviderModelChoice {
                    model: choice.model.trim().to_string(),
                    label: choice.label.trim().to_string(),
                })
                .collect(),
        }
    })
}

fn normalize_integration_spec(
    id: &str,
    raw: Option<RawProviderIntegrationSpec>,
) -> Option<ProviderIntegrationSpec> {
    let integration = raw.map(|spec| ProviderIntegrationSpec {
        name: spec.name.trim().to_string(),
        description: spec.description.trim().to_string(),
    });
    if let Some(spec) = &integration {
        assert!(
            !spec.name.is_empty(),
            "provider {id} integration name cannot be empty"
        );
        assert!(
            !spec.description.is_empty(),
            "provider {id} integration description cannot be empty"
        );
    }
    integration
}

fn build_provider_catalog_entry(
    raw_entry: RawProviderCatalogEntry,
    seen_aliases: &mut HashSet<String>,
) -> ProviderCatalogEntry {
    let id = raw_entry.id.trim().to_ascii_lowercase();
    let display_name = raw_entry.display_name.trim().to_string();
    assert!(
        !display_name.is_empty(),
        "provider {id} display_name cannot be empty"
    );

    let display_api_key_env_var = raw_entry.display_api_key_env_var.trim().to_string();
    assert!(
        !display_api_key_env_var.is_empty(),
        "provider {id} must declare display_api_key_env_var"
    );

    ProviderCatalogEntry {
        id: id.clone(),
        display_name,
        aliases: normalize_aliases(raw_entry.aliases, &id, seen_aliases),
        api_key_env_vars: raw_entry
            .api_key_env_vars
            .into_iter()
            .map(|var| var.trim().to_string())
            .filter(|var| !var.is_empty())
            .collect(),
        display_api_key_env_var,
        key_url: raw_entry
            .key_url
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty()),
        compatible: normalize_compatible_spec(&id, raw_entry.compatible),
        onboarding: normalize_onboarding_spec(&id, raw_entry.onboarding),
        integration: normalize_integration_spec(&id, raw_entry.integration),
        scan_env: raw_entry.scan_env,
    }
}

fn load_provider_registry() -> ProviderRegistry {
    let raw: RawProviderRegistry = toml::from_str(include_str!("provider_registry.toml"))
        .unwrap_or_else(|error| panic!("failed to parse provider_registry.toml: {error}"));

    let mut seen_ids = HashSet::new();
    let mut seen_aliases = HashSet::new();
    let mut by_id = HashMap::with_capacity(raw.providers.len());
    let mut entries = Vec::with_capacity(raw.providers.len());

    for raw_entry in raw.providers {
        let id = raw_entry.id.trim().to_ascii_lowercase();
        assert!(!id.is_empty(), "provider_registry entry id cannot be empty");
        assert!(seen_ids.insert(id.clone()), "duplicate provider id: {id}");
        by_id.insert(id.clone(), entries.len());
        entries.push(build_provider_catalog_entry(raw_entry, &mut seen_aliases));
    }

    ProviderRegistry { entries, by_id }
}

/// Return the canonical provider name used for persisted auth and config records.
///
/// `custom:…` selectors are stored as `"custom"`, `anthropic-custom:…` as
/// `"anthropic-custom"`, and everything else is passed through
/// `normalize_provider_alias` (e.g. `"openai-codex"` → `"openai"`).
#[must_use]
pub fn canonical_provider_name(name: &str) -> String {
    let normalized = name.trim().to_ascii_lowercase();
    if normalized.starts_with("custom:") {
        "custom".to_string()
    } else if normalized.starts_with("anthropic-custom:") {
        "anthropic-custom".to_string()
    } else if normalized.starts_with("gemini-vertex:") || normalized.starts_with("vertex-gemini:") {
        "gemini-vertex".to_string()
    } else {
        normalize_provider_alias(&normalized).to_string()
    }
}

/// Returns all curated provider catalog entries in display order.
#[must_use]
pub fn all_provider_catalog_entries() -> &'static [ProviderCatalogEntry] {
    provider_registry().entries.as_slice()
}

/// Returns the built-in providers that should be probed during onboarding env scan.
#[must_use]
pub fn provider_scan_candidates() -> Vec<&'static str> {
    all_provider_catalog_entries()
        .iter()
        .filter(|entry| entry.scan_env)
        .map(|entry| entry.id.as_str())
        .collect()
}

/// Returns providers that should appear in onboarding for the given tier.
#[must_use]
pub fn provider_catalog_entries_for_onboarding_tier(
    tier: ProviderOnboardingTier,
) -> Vec<&'static ProviderCatalogEntry> {
    all_provider_catalog_entries()
        .iter()
        .filter(|entry| {
            entry
                .onboarding
                .as_ref()
                .is_some_and(|spec| spec.tier == tier)
        })
        .collect()
}

/// Returns providers that should appear as AI integrations.
#[must_use]
pub fn ai_integration_provider_entries() -> Vec<&'static ProviderCatalogEntry> {
    all_provider_catalog_entries()
        .iter()
        .filter(|entry| entry.integration.is_some())
        .collect()
}

/// Look up the curated `ProviderCatalogEntry` for a canonicalized provider id.
/// Returns `None` for unknown or custom providers.
#[must_use]
pub fn provider_catalog_entry(name: &str) -> Option<&'static ProviderCatalogEntry> {
    let normalized = canonical_provider_name(name);
    let registry = provider_registry();
    registry
        .by_id
        .get(&normalized)
        .and_then(|index| registry.entries.get(*index))
}

/// Resolve the named auth route for an external provider selector.
///
/// Currently only `"openai-codex"` maps to a special route (`"codex"`).
/// All other selectors return `None` and use the default auth flow.
#[must_use]
pub fn requested_auth_route(name: &str) -> Option<&'static str> {
    name.trim()
        .eq_ignore_ascii_case("openai-codex")
        .then_some("codex")
}

/// Return the OAuth/login flow associated with a provider selector.
#[must_use]
pub fn oauth_flow_for_provider(name: &str) -> Option<ProviderOAuthFlow> {
    let normalized = canonical_provider_name(name);
    match normalized.as_str() {
        "openai" => Some(ProviderOAuthFlow::Codex),
        "claude" | "anthropic" => Some(ProviderOAuthFlow::Claude),
        _ => None,
    }
}

/// Return the OAuth/login flow associated with a persisted OAuth source name.
#[must_use]
pub fn oauth_flow_for_source(name: &str) -> Option<ProviderOAuthFlow> {
    match name.trim().to_ascii_lowercase().as_str() {
        "codex" => Some(ProviderOAuthFlow::Codex),
        "claude" => Some(ProviderOAuthFlow::Claude),
        _ => None,
    }
}

/// Return provider-specific API key env vars, in lookup priority order.
#[must_use]
pub fn api_key_env_candidates(name: &str) -> &'static [String] {
    provider_catalog_entry(name).map_or(&[] as &[String], |entry| entry.api_key_env_vars.as_slice())
}

/// Return the primary API key env var shown to users for a provider.
#[must_use]
pub fn primary_api_key_env_var(name: &str) -> &'static str {
    provider_catalog_entry(name).map_or("ASTEREL_API_KEY", |entry| {
        entry.display_api_key_env_var.as_str()
    })
}

/// Return the provider's API key creation URL when one is known.
#[must_use]
pub fn provider_key_url(name: &str) -> Option<&'static str> {
    provider_catalog_entry(name).and_then(|entry| entry.key_url.as_deref())
}

/// Return OpenAI-compatible transport metadata when a provider uses that surface.
#[must_use]
pub fn compatible_provider_spec(name: &str) -> Option<&'static CompatibleProviderSpec> {
    provider_catalog_entry(name).and_then(|entry| entry.compatible.as_ref())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::contracts::providers::{
        builtin_provider_ids, is_builtin_provider, provider_aliases,
    };

    use super::{
        ProviderOnboardingTier, ai_integration_provider_entries, all_provider_catalog_entries,
        api_key_env_candidates, canonical_provider_name, compatible_provider_spec,
        normalize_provider_alias, oauth_flow_for_provider, oauth_flow_for_source,
        primary_api_key_env_var, provider_catalog_entries_for_onboarding_tier, provider_key_url,
        provider_scan_candidates, requested_auth_route,
    };

    #[test]
    fn normalize_provider_alias_maps_known_aliases_to_canonical_backends() {
        assert_eq!(normalize_provider_alias("openai-codex"), "openai");
        assert_eq!(normalize_provider_alias(" openai-codex "), "openai");
        assert_eq!(normalize_provider_alias("google"), "gemini");
        assert_eq!(normalize_provider_alias("google-gemini"), "gemini");
        assert_eq!(normalize_provider_alias("grok"), "xai");
        assert_eq!(normalize_provider_alias("kimi"), "moonshot");
        assert_eq!(normalize_provider_alias("z.ai"), "zai");
        assert_eq!(normalize_provider_alias("zhipu"), "glm");
        assert_eq!(normalize_provider_alias("baidu"), "qianfan");
        assert_eq!(normalize_provider_alias("vercel-ai"), "vercel");
        assert_eq!(normalize_provider_alias("cloudflare-ai"), "cloudflare");
        assert_eq!(normalize_provider_alias("together-ai"), "together");
        assert_eq!(normalize_provider_alias("fireworks-ai"), "fireworks");
        assert_eq!(normalize_provider_alias("github-copilot"), "copilot");
        assert_eq!(normalize_provider_alias("aws-bedrock"), "bedrock");
        assert_eq!(normalize_provider_alias("opencode-zen"), "opencode");
        assert_eq!(normalize_provider_alias("anthropic"), "anthropic");
        assert_eq!(normalize_provider_alias("vertex-gemini"), "gemini-vertex");
        assert_eq!(
            canonical_provider_name("custom:https://example.com"),
            "custom"
        );
        assert_eq!(
            canonical_provider_name("anthropic-custom:https://example.com"),
            "anthropic-custom"
        );
        assert_eq!(
            canonical_provider_name("gemini-vertex:project/global"),
            "gemini-vertex"
        );
        assert_eq!(canonical_provider_name("Z.AI"), "zai");
        assert_eq!(canonical_provider_name("github-copilot"), "copilot");
    }

    #[test]
    fn provider_registry_stays_in_sync_with_contract_aliases_and_builtins() {
        let manifest_ids = all_provider_catalog_entries()
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<BTreeSet<_>>();
        let builtin_ids = builtin_provider_ids()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        assert_eq!(manifest_ids, builtin_ids);

        let manifest_aliases = all_provider_catalog_entries()
            .iter()
            .flat_map(|entry| {
                entry
                    .aliases
                    .iter()
                    .map(|alias| (alias.as_str(), entry.id.as_str()))
                    .collect::<Vec<_>>()
            })
            .collect::<BTreeSet<_>>();
        let contract_aliases = provider_aliases().iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(manifest_aliases, contract_aliases);
    }

    #[test]
    fn api_key_env_candidates_prioritize_provider_specific_keys() {
        assert_eq!(
            api_key_env_candidates("anthropic")
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["ANTHROPIC_OAUTH_TOKEN", "ANTHROPIC_API_KEY"]
        );
        assert_eq!(
            api_key_env_candidates("openai-codex")
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["OPENAI_API_KEY"]
        );
        assert!(api_key_env_candidates("ollama").is_empty());
    }

    #[test]
    fn primary_api_key_env_var_uses_first_candidate_or_global_fallback() {
        assert_eq!(primary_api_key_env_var("openai"), "OPENAI_API_KEY");
        assert_eq!(
            primary_api_key_env_var("unknown-provider"),
            "ASTEREL_API_KEY"
        );
    }

    #[test]
    fn provider_key_url_and_compatible_spec_match_catalog() {
        assert_eq!(
            provider_key_url("openai-codex"),
            Some("https://platform.openai.com/api-keys")
        );
        let venice = compatible_provider_spec("venice").expect("venice spec should exist");
        assert_eq!(venice.display_name, "Venice");
        assert_eq!(venice.base_url, "https://api.venice.ai");
        assert!(compatible_provider_spec("openai").is_none());
        assert!(is_builtin_provider("openai-codex"));
        assert!(is_builtin_provider("google-gemini"));
        assert!(is_builtin_provider("github-copilot"));
        assert!(!is_builtin_provider("custom"));
        assert_eq!(requested_auth_route("openai-codex"), Some("codex"));
        assert!(requested_auth_route("openai").is_none());
        assert_eq!(
            oauth_flow_for_provider("anthropic"),
            Some(super::ProviderOAuthFlow::Claude)
        );
        assert_eq!(
            oauth_flow_for_provider("OPENAI-CODEX"),
            Some(super::ProviderOAuthFlow::Codex)
        );
        assert_eq!(
            oauth_flow_for_provider("openai-codex"),
            Some(super::ProviderOAuthFlow::Codex)
        );
        assert_eq!(
            oauth_flow_for_source("CoDeX"),
            Some(super::ProviderOAuthFlow::Codex)
        );
        assert_eq!(
            oauth_flow_for_source("claude"),
            Some(super::ProviderOAuthFlow::Claude)
        );
        assert!(oauth_flow_for_source("custom-source").is_none());
    }

    #[test]
    fn onboarding_and_integration_catalog_views_cover_expected_entries() {
        assert_eq!(
            provider_catalog_entries_for_onboarding_tier(ProviderOnboardingTier::Recommended).len(),
            9
        );
        assert_eq!(
            provider_catalog_entries_for_onboarding_tier(ProviderOnboardingTier::Fast).len(),
            3
        );
        assert_eq!(
            provider_catalog_entries_for_onboarding_tier(ProviderOnboardingTier::Gateway).len(),
            4
        );
        assert_eq!(
            provider_catalog_entries_for_onboarding_tier(ProviderOnboardingTier::Specialized).len(),
            8
        );
        assert_eq!(
            provider_catalog_entries_for_onboarding_tier(ProviderOnboardingTier::Local).len(),
            1
        );
        assert_eq!(ai_integration_provider_entries().len(), 24);
    }

    #[test]
    fn provider_scan_candidates_match_curated_env_scan_subset() {
        assert_eq!(
            provider_scan_candidates(),
            vec![
                "openrouter",
                "anthropic",
                "openai",
                "deepseek",
                "mistral",
                "xai",
                "gemini",
                "groq",
                "ollama",
            ]
        );
    }
}
