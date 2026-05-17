use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;

use crate::core::providers::catalog::{canonical_provider_name, compatible_provider_spec};
use crate::security::scrub::sanitize_api_error;

const PROVIDER_DISCOVERY_CACHE_VERSION: u32 = 1;
const PROVIDER_DISCOVERY_TTL_HOURS: i64 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderDiscoveryKind {
    OpenAiCompatible,
    OpenAiDirect,
    OpenRouter,
    Anthropic,
    Gemini,
    Ollama,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderDiscoverySource {
    Live,
    FreshCache,
    StaleCacheFallback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DiscoveredModelCapabilityHints {
    pub supports_tools: Option<bool>,
    pub supports_vision: Option<bool>,
    pub supports_reasoning: Option<bool>,
    pub supports_streaming: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveredModel {
    pub model_id: String,
    pub display_name: Option<String>,
    #[serde(default)]
    pub capabilities: DiscoveredModelCapabilityHints,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderDiscoveryEntry {
    pub provider: String,
    pub api_base: Option<String>,
    pub auth_fingerprint: Option<String>,
    pub refreshed_at: DateTime<Utc>,
    pub models: Vec<DiscoveredModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderDiscoveryCache {
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<ProviderDiscoveryEntry>,
}

impl Default for ProviderDiscoveryCache {
    fn default() -> Self {
        Self {
            version: PROVIDER_DISCOVERY_CACHE_VERSION,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDiscoveryResult {
    pub source: ProviderDiscoverySource,
    pub models: Vec<DiscoveredModel>,
}

#[derive(Debug, Clone, Copy)]
pub struct ProviderDiscoveryRequest<'a> {
    pub workspace_dir: &'a Path,
    pub provider: &'a str,
    pub api_key: Option<&'a str>,
    pub api_base: Option<&'a str>,
    pub force_refresh: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderDiscoveryScope {
    provider: String,
    api_base: Option<String>,
    auth_fingerprint: Option<String>,
}

impl ProviderDiscoveryScope {
    fn new(provider: &str, api_base: Option<&str>, api_key: Option<&str>) -> Self {
        Self {
            provider: canonical_provider_name(provider),
            api_base: effective_api_base(provider, api_base),
            auth_fingerprint: auth_fingerprint(api_key),
        }
    }

    fn matches(&self, entry: &ProviderDiscoveryEntry) -> bool {
        self.provider == entry.provider
            && self.api_base == entry.api_base
            && self.auth_fingerprint == entry.auth_fingerprint
    }
}

fn normalize_api_base(api_base: Option<&str>) -> Option<String> {
    api_base
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_end_matches('/').to_string())
}

fn selector_api_base(provider: &str) -> Option<&str> {
    provider
        .strip_prefix("custom:")
        .or_else(|| provider.strip_prefix("anthropic-custom:"))
}

fn effective_api_base(provider: &str, api_base: Option<&str>) -> Option<String> {
    if provider.trim().eq_ignore_ascii_case("openai-codex") {
        return None;
    }

    normalize_api_base(api_base).or_else(|| normalize_api_base(selector_api_base(provider)))
}

fn auth_fingerprint(api_key: Option<&str>) -> Option<String> {
    let api_key = api_key.map(str::trim).filter(|value| !value.is_empty())?;
    let digest = sha2::Sha256::digest(api_key.as_bytes());
    Some(hex::encode(digest))
}

fn provider_discovery_dir(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join(".asterel").join("provider-discovery")
}

#[must_use]
pub fn provider_discovery_cache_path(workspace_dir: &Path) -> PathBuf {
    provider_discovery_dir(workspace_dir).join("models.json")
}

#[allow(clippy::missing_errors_doc)]
pub fn load_provider_discovery_cache(workspace_dir: &Path) -> Result<ProviderDiscoveryCache> {
    let path = provider_discovery_cache_path(workspace_dir);
    if !path.exists() {
        return Ok(ProviderDiscoveryCache::default());
    }

    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read provider discovery cache {}", path.display()))?;
    let cache: ProviderDiscoveryCache = serde_json::from_str(&raw)
        .with_context(|| format!("parse provider discovery cache {}", path.display()))?;
    if cache.version != PROVIDER_DISCOVERY_CACHE_VERSION {
        return Ok(ProviderDiscoveryCache::default());
    }
    Ok(cache)
}

#[allow(clippy::missing_errors_doc)]
pub fn save_provider_discovery_cache(
    workspace_dir: &Path,
    cache: &ProviderDiscoveryCache,
) -> Result<()> {
    let path = provider_discovery_cache_path(workspace_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create provider discovery dir {}", parent.display()))?;
    }
    let serialized = serde_json::to_vec_pretty(cache)
        .with_context(|| format!("serialize provider discovery cache {}", path.display()))?;
    let tmp_path = provider_discovery_cache_tmp_path(&path);
    std::fs::write(&tmp_path, serialized)
        .with_context(|| format!("write temp provider discovery cache {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, &path).with_context(|| {
        format!(
            "replace provider discovery cache {} from {}",
            path.display(),
            tmp_path.display()
        )
    })
}

fn provider_discovery_cache_tmp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("models.json");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    path.with_file_name(format!(".{file_name}.{nonce}.tmp"))
}

fn discovery_ttl() -> Duration {
    Duration::hours(PROVIDER_DISCOVERY_TTL_HOURS)
}

fn entry_is_fresh(entry: &ProviderDiscoveryEntry) -> bool {
    Utc::now() - entry.refreshed_at <= discovery_ttl()
}

fn upsert_cache_entry(
    cache: &mut ProviderDiscoveryCache,
    scope: &ProviderDiscoveryScope,
    models: Vec<DiscoveredModel>,
) {
    let entry = ProviderDiscoveryEntry {
        provider: scope.provider.clone(),
        api_base: scope.api_base.clone(),
        auth_fingerprint: scope.auth_fingerprint.clone(),
        refreshed_at: Utc::now(),
        models,
    };

    if let Some(existing) = cache
        .entries
        .iter_mut()
        .find(|candidate| scope.matches(candidate))
    {
        *existing = entry;
    } else {
        cache.entries.push(entry);
    }
}

fn discovery_kind(provider: &str) -> Option<ProviderDiscoveryKind> {
    if provider.starts_with("custom:") {
        return Some(ProviderDiscoveryKind::OpenAiCompatible);
    }
    if provider.starts_with("anthropic-custom:") {
        return Some(ProviderDiscoveryKind::Anthropic);
    }

    match canonical_provider_name(provider).as_str() {
        "custom" => Some(ProviderDiscoveryKind::OpenAiCompatible),
        "openai" => Some(ProviderDiscoveryKind::OpenAiDirect),
        "openrouter" => Some(ProviderDiscoveryKind::OpenRouter),
        "anthropic" | "anthropic-custom" => Some(ProviderDiscoveryKind::Anthropic),
        "gemini" => Some(ProviderDiscoveryKind::Gemini),
        "ollama" => Some(ProviderDiscoveryKind::Ollama),
        "gemini-vertex" | "bedrock" | "minimax" => None,
        other if compatible_provider_spec(other).is_some() => {
            Some(ProviderDiscoveryKind::OpenAiCompatible)
        }
        _ => None,
    }
}

fn resolve_api_key(request: &ProviderDiscoveryRequest<'_>) -> Option<String> {
    if let Some(explicit) = request
        .api_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(explicit.to_string());
    }

    let provider = canonical_provider_name(request.provider);
    for env_var in crate::core::providers::catalog::api_key_env_candidates(&provider)
        .iter()
        .map(String::as_str)
    {
        if let Ok(value) = std::env::var(env_var) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }

    std::env::var("ASTEREL_API_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn resolved_discovery_secret(request: &ProviderDiscoveryRequest<'_>) -> Option<String> {
    let provider = canonical_provider_name(request.provider);
    if provider == "ollama" {
        return None;
    }
    if provider == "gemini" {
        return crate::core::providers::gemini::GeminiProvider::resolve_auth(request.api_key).map(
            |auth| match auth {
                crate::core::providers::gemini::GeminiResolvedAuth::ApiKey(value)
                | crate::core::providers::gemini::GeminiResolvedAuth::OAuthBearer(value) => value,
                crate::core::providers::gemini::GeminiResolvedAuth::ApplicationDefaultCredentials => {
                    String::new()
                }
            },
        );
    }

    resolve_api_key(request)
}

fn normalize_openai_compatible_base(provider: &str, api_base: Option<&str>) -> Result<String> {
    if let Some(api_base) = normalize_api_base(api_base) {
        return Ok(api_base);
    }

    match canonical_provider_name(provider).as_str() {
        "openai" => Ok("https://api.openai.com".to_string()),
        "openrouter" => Ok("https://openrouter.ai/api".to_string()),
        other => compatible_provider_spec(other)
            .map(|spec| spec.base_url.clone())
            .ok_or_else(|| anyhow::anyhow!("provider discovery unsupported for {other}")),
    }
}

fn openai_models_endpoint(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.ends_with("/models") {
        base.to_string()
    } else if base.ends_with("/v1") || base.ends_with("/v1beta") {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    }
}

fn anthropic_models_endpoint(api_base: Option<&str>) -> String {
    let base =
        normalize_api_base(api_base).unwrap_or_else(|| "https://api.anthropic.com".to_string());
    openai_models_endpoint(&base)
}

fn gemini_models_endpoint(api_base: Option<&str>) -> String {
    let base = normalize_api_base(api_base)
        .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta".to_string());
    let base = base.trim_end_matches('/');
    if base.ends_with("/models") {
        base.to_string()
    } else if base.ends_with("/v1beta") || base.ends_with("/v1") {
        format!("{base}/models")
    } else {
        format!("{base}/v1beta/models")
    }
}

fn ollama_tags_endpoint(api_base: Option<&str>) -> String {
    let base = normalize_api_base(api_base).unwrap_or_else(|| "http://localhost:11434".to_string());
    let base = base.trim_end_matches('/');
    if base.ends_with("/api/tags") {
        base.to_string()
    } else {
        format!("{base}/api/tags")
    }
}

async fn fetch_json(request: reqwest::RequestBuilder, context: &str) -> Result<Value> {
    let response = request
        .send()
        .await
        .with_context(|| format!("request {context}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let sanitized = sanitize_api_error(&body);
        bail!("{context} failed with HTTP {status}: {sanitized}");
    }
    response
        .json::<Value>()
        .await
        .with_context(|| format!("decode {context} response"))
}

fn bool_hint_array_contains(array: Option<&Vec<Value>>, needle: &str) -> Option<bool> {
    let array = array?;
    Some(
        array
            .iter()
            .filter_map(Value::as_str)
            .any(|value| value.eq_ignore_ascii_case(needle)),
    )
}

fn sort_and_dedup(models: Vec<DiscoveredModel>) -> Vec<DiscoveredModel> {
    let mut deduped = BTreeMap::new();
    for model in models
        .into_iter()
        .filter(|model| !model.model_id.trim().is_empty())
    {
        deduped.entry(model.model_id.clone()).or_insert(model);
    }
    deduped.into_values().collect()
}

fn parse_openai_compatible_models(value: &Value) -> Vec<DiscoveredModel> {
    let items = value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    sort_and_dedup(
        items
            .into_iter()
            .filter_map(|item| {
                let model_id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())?
                    .to_string();
                let display_name = item
                    .get("name")
                    .or_else(|| item.get("display_name"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string);
                let supported_parameters =
                    item.get("supported_parameters").and_then(Value::as_array);
                let modalities = item.get("modalities").and_then(Value::as_array);
                Some(DiscoveredModel {
                    model_id,
                    display_name,
                    capabilities: DiscoveredModelCapabilityHints {
                        supports_tools: bool_hint_array_contains(supported_parameters, "tools"),
                        supports_vision: bool_hint_array_contains(modalities, "image"),
                        supports_reasoning: bool_hint_array_contains(
                            supported_parameters,
                            "reasoning",
                        ),
                        supports_streaming: None,
                    },
                })
            })
            .collect(),
    )
}

fn parse_anthropic_models(value: &Value) -> Vec<DiscoveredModel> {
    let items = value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    sort_and_dedup(
        items
            .into_iter()
            .filter_map(|item| {
                let model_id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())?
                    .to_string();
                let display_name = item
                    .get("display_name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string);
                Some(DiscoveredModel {
                    model_id,
                    display_name,
                    capabilities: DiscoveredModelCapabilityHints::default(),
                })
            })
            .collect(),
    )
}

fn parse_gemini_models(value: &Value) -> Vec<DiscoveredModel> {
    let items = value
        .get("models")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    sort_and_dedup(
        items
            .into_iter()
            .filter_map(|item| {
                let supported_generation_methods = item
                    .get("supportedGenerationMethods")
                    .and_then(Value::as_array)?;
                let supports_generation = supported_generation_methods
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|method| {
                        method.eq_ignore_ascii_case("generateContent")
                            || method.eq_ignore_ascii_case("streamGenerateContent")
                    });
                if !supports_generation {
                    return None;
                }
                let supports_streaming = supported_generation_methods
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|method| method.eq_ignore_ascii_case("streamGenerateContent"));

                let model_id = item
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())?
                    .trim_start_matches("models/")
                    .to_string();
                let display_name = item
                    .get("displayName")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string);
                Some(DiscoveredModel {
                    model_id,
                    display_name,
                    capabilities: DiscoveredModelCapabilityHints {
                        supports_tools: None,
                        supports_vision: None,
                        supports_reasoning: None,
                        supports_streaming: Some(supports_streaming),
                    },
                })
            })
            .collect(),
    )
}

fn parse_ollama_models(value: &Value) -> Vec<DiscoveredModel> {
    let items = value
        .get("models")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    sort_and_dedup(
        items
            .into_iter()
            .filter_map(|item| {
                let model_id = item
                    .get("name")
                    .or_else(|| item.get("model"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())?
                    .to_string();
                Some(DiscoveredModel {
                    model_id,
                    display_name: None,
                    capabilities: DiscoveredModelCapabilityHints::default(),
                })
            })
            .collect(),
    )
}

async fn fetch_openai_compatible_models(
    client: &reqwest::Client,
    provider: &str,
    api_key: &str,
    api_base: Option<&str>,
) -> Result<Vec<DiscoveredModel>> {
    let base = normalize_openai_compatible_base(provider, api_base)?;
    let url = openai_models_endpoint(&base);
    let mut request = client
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"));
    if canonical_provider_name(provider) == "openrouter" {
        request = request
            .header("HTTP-Referer", "https://github.com/asterel-rs/asterel")
            .header("X-Title", "asterel");
    }
    fetch_json(request, &format!("{provider} model discovery"))
        .await
        .map(|value| parse_openai_compatible_models(&value))
}

async fn fetch_anthropic_models(
    client: &reqwest::Client,
    api_key: &str,
    api_base: Option<&str>,
) -> Result<Vec<DiscoveredModel>> {
    let url = anthropic_models_endpoint(api_base);
    let (header_name, header_value) =
        crate::core::providers::anthropic::AnthropicProvider::auth_header_for_token(api_key);
    let request = client
        .get(&url)
        .header(header_name, header_value)
        .header("anthropic-version", "2023-06-01");
    fetch_json(request, "anthropic model discovery")
        .await
        .map(|value| parse_anthropic_models(&value))
}

async fn fetch_gemini_models(
    client: &reqwest::Client,
    auth: &crate::core::providers::gemini::GeminiResolvedAuth,
    api_base: Option<&str>,
) -> Result<Vec<DiscoveredModel>> {
    let url = gemini_models_endpoint(api_base);
    let request = match auth {
        crate::core::providers::gemini::GeminiResolvedAuth::ApiKey(api_key) => {
            client.get(&url).header("x-goog-api-key", api_key)
        }
        crate::core::providers::gemini::GeminiResolvedAuth::OAuthBearer(token) => client
            .get(&url)
            .header("Authorization", format!("Bearer {token}")),
        crate::core::providers::gemini::GeminiResolvedAuth::ApplicationDefaultCredentials => {
            bail!("Gemini discovery does not support Vertex ADC-only auth")
        }
    };
    fetch_json(request, "gemini model discovery")
        .await
        .map(|value| parse_gemini_models(&value))
}

async fn fetch_ollama_models(
    client: &reqwest::Client,
    api_base: Option<&str>,
) -> Result<Vec<DiscoveredModel>> {
    let url = ollama_tags_endpoint(api_base);
    let request = client.get(&url);
    fetch_json(request, "ollama model discovery")
        .await
        .map(|value| parse_ollama_models(&value))
}

async fn fetch_live_models(request: &ProviderDiscoveryRequest<'_>) -> Result<Vec<DiscoveredModel>> {
    let client = crate::core::providers::build_provider_client_with_timeout(15);
    let provider = canonical_provider_name(request.provider);
    let kind = discovery_kind(request.provider)
        .ok_or_else(|| anyhow::anyhow!("provider discovery unsupported for {provider}"))?;
    let api_base = effective_api_base(request.provider, request.api_base);
    match kind {
        ProviderDiscoveryKind::OpenAiDirect
        | ProviderDiscoveryKind::OpenRouter
        | ProviderDiscoveryKind::OpenAiCompatible => {
            let api_key = resolve_api_key(request).ok_or_else(|| {
                anyhow::anyhow!("provider discovery requires API key for {provider}")
            })?;
            if provider == "openai" && !api_key.starts_with("sk-") {
                bail!("provider discovery unavailable for non-API-key OpenAI/Codex credentials")
            }
            fetch_openai_compatible_models(&client, &provider, &api_key, api_base.as_deref()).await
        }
        ProviderDiscoveryKind::Anthropic => {
            let api_key = resolve_api_key(request).ok_or_else(|| {
                anyhow::anyhow!("provider discovery requires API key or setup token for anthropic")
            })?;
            fetch_anthropic_models(&client, &api_key, api_base.as_deref()).await
        }
        ProviderDiscoveryKind::Gemini => {
            let auth = crate::core::providers::gemini::GeminiProvider::resolve_auth(
                request.api_key,
            )
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "provider discovery requires Gemini API key, GOOGLE_API_KEY, or CLI auth"
                )
            })?;
            fetch_gemini_models(&client, &auth, api_base.as_deref()).await
        }
        ProviderDiscoveryKind::Ollama => fetch_ollama_models(&client, api_base.as_deref()).await,
    }
}

#[allow(clippy::missing_errors_doc)]
pub async fn resolve_provider_discovery(
    request: ProviderDiscoveryRequest<'_>,
) -> Result<ProviderDiscoveryResult> {
    let resolved_key = resolved_discovery_secret(&request);
    let scope =
        ProviderDiscoveryScope::new(request.provider, request.api_base, resolved_key.as_deref());
    let mut cache = load_provider_discovery_cache(request.workspace_dir)?;
    let cached_entry = cache
        .entries
        .iter()
        .find(|entry| scope.matches(entry))
        .cloned();

    if !request.force_refresh
        && let Some(entry) = &cached_entry
        && entry_is_fresh(entry)
    {
        return Ok(ProviderDiscoveryResult {
            source: ProviderDiscoverySource::FreshCache,
            models: entry.models.clone(),
        });
    }

    match fetch_live_models(&request).await {
        Ok(models) => {
            upsert_cache_entry(&mut cache, &scope, models.clone());
            save_provider_discovery_cache(request.workspace_dir, &cache)?;
            Ok(ProviderDiscoveryResult {
                source: ProviderDiscoverySource::Live,
                models,
            })
        }
        Err(error) => {
            if let Some(entry) = cached_entry {
                tracing::warn!(
                    provider = %scope.provider,
                    error = %error,
                    "provider discovery live refresh failed; using stale cache"
                );
                return Ok(ProviderDiscoveryResult {
                    source: ProviderDiscoverySource::StaleCacheFallback,
                    models: entry.models,
                });
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn cache_roundtrip_preserves_entries() {
        let tmp = TempDir::new().expect("temp dir");
        let cache = ProviderDiscoveryCache {
            version: PROVIDER_DISCOVERY_CACHE_VERSION,
            entries: vec![ProviderDiscoveryEntry {
                provider: "openai".to_string(),
                api_base: Some("https://api.example.com".to_string()),
                auth_fingerprint: Some("abc".to_string()),
                refreshed_at: Utc::now(),
                models: vec![DiscoveredModel {
                    model_id: "gpt-test".to_string(),
                    display_name: Some("GPT Test".to_string()),
                    capabilities: DiscoveredModelCapabilityHints::default(),
                }],
            }],
        };

        save_provider_discovery_cache(tmp.path(), &cache).expect("save cache");
        let loaded = load_provider_discovery_cache(tmp.path()).expect("load cache");
        assert_eq!(loaded, cache);
    }

    #[test]
    fn cache_save_does_not_leave_temp_file_after_success() {
        let tmp = TempDir::new().expect("temp dir");
        let cache = ProviderDiscoveryCache::default();

        save_provider_discovery_cache(tmp.path(), &cache).expect("save cache");

        let cache_dir = provider_discovery_cache_path(tmp.path())
            .parent()
            .expect("cache dir")
            .to_path_buf();
        let temp_files = std::fs::read_dir(cache_dir)
            .expect("read cache dir")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.ends_with(".tmp"))
            })
            .count();
        assert_eq!(temp_files, 0);
    }

    #[test]
    fn ollama_discovery_secret_ignores_generic_api_key() {
        let tmp = TempDir::new().expect("temp dir");
        let _env = crate::utils::test_env::EnvVarGuard::set("ASTEREL_API_KEY", "generic-key");
        let request = ProviderDiscoveryRequest {
            workspace_dir: tmp.path(),
            provider: "ollama",
            api_key: Some("ignored-explicit-key"),
            api_base: Some("http://127.0.0.1:11434"),
            force_refresh: false,
        };

        assert_eq!(resolved_discovery_secret(&request), None);
        let scope = ProviderDiscoveryScope::new(
            request.provider,
            request.api_base,
            resolved_discovery_secret(&request).as_deref(),
        );
        assert_eq!(scope.auth_fingerprint, None);
    }

    #[test]
    fn custom_selector_discovery_uses_embedded_api_base() {
        let scope = ProviderDiscoveryScope::new(
            "custom:https://proxy.example/v1/",
            None,
            Some("custom-key"),
        );

        assert_eq!(
            discovery_kind("custom:https://proxy.example/v1"),
            Some(ProviderDiscoveryKind::OpenAiCompatible)
        );
        assert_eq!(scope.provider, "custom");
        assert_eq!(scope.api_base.as_deref(), Some("https://proxy.example/v1"));
    }

    #[test]
    fn bare_custom_discovery_uses_separate_api_base() {
        let scope = ProviderDiscoveryScope::new(
            "custom",
            Some("https://proxy.example/v1/"),
            Some("custom-key"),
        );

        assert_eq!(
            discovery_kind("custom"),
            Some(ProviderDiscoveryKind::OpenAiCompatible)
        );
        assert_eq!(scope.provider, "custom");
        assert_eq!(scope.api_base.as_deref(), Some("https://proxy.example/v1"));
    }

    #[test]
    fn anthropic_custom_selector_discovery_uses_embedded_api_base() {
        let scope = ProviderDiscoveryScope::new(
            "anthropic-custom:https://claude.example/",
            None,
            Some("anthropic-key"),
        );

        assert_eq!(
            discovery_kind("anthropic-custom:https://claude.example"),
            Some(ProviderDiscoveryKind::Anthropic)
        );
        assert_eq!(scope.provider, "anthropic-custom");
        assert_eq!(scope.api_base.as_deref(), Some("https://claude.example"));
    }

    #[test]
    fn bare_anthropic_custom_discovery_uses_separate_api_base() {
        let scope = ProviderDiscoveryScope::new(
            "anthropic-custom",
            Some("https://claude.example/"),
            Some("anthropic-key"),
        );

        assert_eq!(
            discovery_kind("anthropic-custom"),
            Some(ProviderDiscoveryKind::Anthropic)
        );
        assert_eq!(scope.provider, "anthropic-custom");
        assert_eq!(scope.api_base.as_deref(), Some("https://claude.example"));
    }

    #[test]
    fn openai_codex_discovery_ignores_api_base_like_runtime() {
        let scope = ProviderDiscoveryScope::new(
            "openai-codex",
            Some("https://proxy.example/v1"),
            Some("sk-test"),
        );

        assert_eq!(scope.provider, "openai");
        assert_eq!(scope.api_base, None);
    }

    #[tokio::test]
    async fn resolve_provider_discovery_uses_fresh_cache_without_network() {
        let tmp = TempDir::new().expect("temp dir");
        let mut cache = ProviderDiscoveryCache::default();
        let scope = ProviderDiscoveryScope {
            provider: "openai".to_string(),
            api_base: Some("http://127.0.0.1:9".to_string()),
            auth_fingerprint: auth_fingerprint(Some("cache-key")),
        };
        upsert_cache_entry(
            &mut cache,
            &scope,
            vec![DiscoveredModel {
                model_id: "gpt-cache".to_string(),
                display_name: None,
                capabilities: DiscoveredModelCapabilityHints::default(),
            }],
        );
        save_provider_discovery_cache(tmp.path(), &cache).expect("save cache");

        let result = resolve_provider_discovery(ProviderDiscoveryRequest {
            workspace_dir: tmp.path(),
            provider: "openai",
            api_key: Some("cache-key"),
            api_base: Some("http://127.0.0.1:9"),
            force_refresh: false,
        })
        .await
        .expect("resolve from cache");

        assert_eq!(result.source, ProviderDiscoverySource::FreshCache);
        assert_eq!(result.models[0].model_id, "gpt-cache");
    }

    #[tokio::test]
    async fn resolve_provider_discovery_falls_back_to_stale_cache_on_live_failure() {
        let tmp = TempDir::new().expect("temp dir");
        let mut cache = ProviderDiscoveryCache::default();
        let scope = ProviderDiscoveryScope {
            provider: "openai".to_string(),
            api_base: Some("http://127.0.0.1:9".to_string()),
            auth_fingerprint: auth_fingerprint(Some("cache-key")),
        };
        let entry = ProviderDiscoveryEntry {
            provider: scope.provider.clone(),
            api_base: scope.api_base.clone(),
            auth_fingerprint: scope.auth_fingerprint.clone(),
            refreshed_at: Utc::now() - Duration::hours(PROVIDER_DISCOVERY_TTL_HOURS + 1),
            models: vec![DiscoveredModel {
                model_id: "gpt-stale".to_string(),
                display_name: None,
                capabilities: DiscoveredModelCapabilityHints::default(),
            }],
        };
        cache.entries.push(entry.clone());
        save_provider_discovery_cache(tmp.path(), &cache).expect("save cache");

        let result = resolve_provider_discovery(ProviderDiscoveryRequest {
            workspace_dir: tmp.path(),
            provider: "openai",
            api_key: Some("cache-key"),
            api_base: Some("http://127.0.0.1:9"),
            force_refresh: false,
        })
        .await
        .expect("resolve with stale fallback");

        assert_eq!(result.source, ProviderDiscoverySource::StaleCacheFallback);
        assert_eq!(result.models, entry.models);
    }

    #[tokio::test]
    async fn fetch_openai_compatible_models_parses_standard_models_payload() {
        let server = MockServer::start().await;
        let tmp = TempDir::new().expect("tmp");
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("Authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"id": "gpt-5", "name": "GPT-5"},
                    {"id": "gpt-4o", "supported_parameters": ["tools", "reasoning"]}
                ]
            })))
            .mount(&server)
            .await;

        let result = resolve_provider_discovery(ProviderDiscoveryRequest {
            workspace_dir: tmp.path(),
            provider: "openai",
            api_key: Some("sk-test"),
            api_base: Some(&server.uri()),
            force_refresh: true,
        })
        .await
        .expect("discover openai models");

        assert_eq!(result.source, ProviderDiscoverySource::Live);
        assert_eq!(result.models.len(), 2);
        assert_eq!(result.models[0].model_id, "gpt-4o");
        assert_eq!(result.models[0].capabilities.supports_tools, Some(true));
        assert_eq!(result.models[1].display_name.as_deref(), Some("GPT-5"));
    }

    #[tokio::test]
    async fn provider_discovery_non_success_error_body_is_sanitized() {
        let server = MockServer::start().await;
        let tmp = TempDir::new().expect("tmp");
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(
                ResponseTemplate::new(500)
                    .set_body_string("upstream echoed sk-leaked-secret-token in error body"),
            )
            .mount(&server)
            .await;

        let error = resolve_provider_discovery(ProviderDiscoveryRequest {
            workspace_dir: tmp.path(),
            provider: "openai",
            api_key: Some("sk-test"),
            api_base: Some(&server.uri()),
            force_refresh: true,
        })
        .await
        .expect_err("non-success discovery should fail");
        let message = error.to_string();

        assert!(message.contains("HTTP 500"));
        assert!(!message.contains("sk-leaked-secret-token"));
        assert!(message.contains("[REDACTED]"));
    }

    #[tokio::test]
    async fn fetch_anthropic_models_parses_direct_models_payload() {
        let server = MockServer::start().await;
        let tmp = TempDir::new().expect("tmp");
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("x-api-key", "sk-ant-key"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [
                    {"id": "claude-sonnet-4-5", "display_name": "Claude Sonnet 4.5"}
                ]
            })))
            .mount(&server)
            .await;

        let result = resolve_provider_discovery(ProviderDiscoveryRequest {
            workspace_dir: tmp.path(),
            provider: "anthropic",
            api_key: Some("sk-ant-key"),
            api_base: Some(&server.uri()),
            force_refresh: true,
        })
        .await
        .expect("discover anthropic models");

        assert_eq!(result.models.len(), 1);
        assert_eq!(result.models[0].model_id, "claude-sonnet-4-5");
        assert_eq!(
            result.models[0].display_name.as_deref(),
            Some("Claude Sonnet 4.5")
        );
    }

    #[tokio::test]
    async fn fetch_gemini_models_filters_non_generation_models() {
        let server = MockServer::start().await;
        let tmp = TempDir::new().expect("tmp");
        Mock::given(method("GET"))
            .and(path("/v1beta/models"))
            .and(header("x-goog-api-key", "gm-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [
                    {
                        "name": "models/gemini-2.5-pro",
                        "displayName": "Gemini 2.5 Pro",
                        "supportedGenerationMethods": ["generateContent", "countTokens"]
                    },
                    {
                        "name": "models/gemini-2.5-flash",
                        "displayName": "Gemini 2.5 Flash",
                        "supportedGenerationMethods": ["generateContent", "streamGenerateContent"]
                    },
                    {
                        "name": "models/text-embedding-004",
                        "displayName": "Embedding",
                        "supportedGenerationMethods": ["embedContent"]
                    }
                ]
            })))
            .mount(&server)
            .await;

        let api_base = format!("{}/v1beta", server.uri());
        let result = resolve_provider_discovery(ProviderDiscoveryRequest {
            workspace_dir: tmp.path(),
            provider: "gemini",
            api_key: Some("gm-key"),
            api_base: Some(&api_base),
            force_refresh: true,
        })
        .await
        .expect("discover gemini models");

        assert_eq!(result.models.len(), 2);
        assert_eq!(result.models[0].model_id, "gemini-2.5-flash");
        assert_eq!(result.models[0].capabilities.supports_streaming, Some(true));
        assert_eq!(result.models[1].model_id, "gemini-2.5-pro");
        assert_eq!(
            result.models[1].display_name.as_deref(),
            Some("Gemini 2.5 Pro")
        );
        assert_eq!(
            result.models[1].capabilities.supports_streaming,
            Some(false)
        );
    }

    #[tokio::test]
    async fn fetch_gemini_models_supports_oauth_bearer_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1beta/models"))
            .and(header("Authorization", "Bearer cli-oauth-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [
                    {
                        "name": "models/gemini-2.5-pro",
                        "displayName": "Gemini 2.5 Pro",
                        "supportedGenerationMethods": ["generateContent"]
                    }
                ]
            })))
            .mount(&server)
            .await;

        let api_base = format!("{}/v1beta", server.uri());
        let models = fetch_gemini_models(
            &crate::core::providers::build_provider_client_with_timeout(15),
            &crate::core::providers::gemini::GeminiResolvedAuth::OAuthBearer(
                "cli-oauth-token".to_string(),
            ),
            Some(&api_base),
        )
        .await
        .expect("discover Gemini models with OAuth bearer");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, "gemini-2.5-pro");
    }

    #[tokio::test]
    async fn fetch_ollama_models_parses_tags_payload() {
        let server = MockServer::start().await;
        let tmp = TempDir::new().expect("tmp");
        Mock::given(method("GET"))
            .and(path("/api/tags"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "models": [
                    {"name": "llama3.3:latest"},
                    {"model": "phi4:14b"}
                ]
            })))
            .mount(&server)
            .await;

        let result = resolve_provider_discovery(ProviderDiscoveryRequest {
            workspace_dir: tmp.path(),
            provider: "ollama",
            api_key: None,
            api_base: Some(&server.uri()),
            force_refresh: true,
        })
        .await
        .expect("discover ollama models");

        assert_eq!(result.models.len(), 2);
        assert_eq!(result.models[0].model_id, "llama3.3:latest");
    }

    #[tokio::test]
    async fn openai_discovery_rejects_non_api_key_credentials() {
        let tmp = TempDir::new().expect("tmp");

        let error = resolve_provider_discovery(ProviderDiscoveryRequest {
            workspace_dir: tmp.path(),
            provider: "openai-codex",
            api_key: Some("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test"),
            api_base: None,
            force_refresh: true,
        })
        .await
        .expect_err("oauth-like Codex token should not hit OpenAI /models");

        assert!(
            error
                .to_string()
                .contains("non-API-key OpenAI/Codex credentials")
        );
    }
}
