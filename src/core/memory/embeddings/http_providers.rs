//! HTTP embedding provider implementations (OpenAI-compatible, Ollama).
//!
//! Provides [`OpenAiEmbedding`] and [`OllamaEmbedding`] with SSRF protection,
//! shared HTTP client construction, and configurable base-URL policy.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use anyhow::Context;
use serde_json::{Value, json};

use super::{EmbeddingFuture, EmbeddingProvider, EmbeddingRole};

#[derive(Copy, Clone, Debug)]
pub(super) struct CustomBaseUrlPolicy {
    pub(super) allow_http: bool,
}

pub(super) fn build_embedding_http_client() -> reqwest::Client {
    crate::utils::http::build_http_client_with(
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60)),
    )
}

fn is_ssrf_blocked_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
}

fn is_ssrf_blocked_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() {
        return true;
    }

    let seg0 = ip.segments()[0];
    let is_link_local = (seg0 & 0xffc0) == 0xfe80;
    let is_unique_local = (seg0 & 0xfe00) == 0xfc00;

    is_link_local || is_unique_local
}

fn is_ssrf_blocked_host(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    let host = host.trim_start_matches('[').trim_end_matches(']');

    if host.eq_ignore_ascii_case("metadata.google.internal") {
        return true;
    }

    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }

    let host_lc = host.to_ascii_lowercase();
    if let Ok(ip) = host_lc.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => is_ssrf_blocked_ipv4(v4),
            IpAddr::V6(v6) => is_ssrf_blocked_ipv6(v6),
        };
    }

    false
}

pub(super) fn validate_custom_base_url(
    raw: &str,
    policy: CustomBaseUrlPolicy,
) -> anyhow::Result<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        anyhow::bail!("custom embedding base URL is empty");
    }

    let url = reqwest::Url::parse(raw).context("invalid custom embedding base URL")?;

    match url.scheme() {
        "https" => {}
        "http" if policy.allow_http => {}
        "http" => anyhow::bail!("custom embedding base URL must use https"),
        _ => anyhow::bail!("custom embedding base URL must use http(s)"),
    }

    if !url.username().is_empty() || url.password().is_some() {
        anyhow::bail!("custom embedding base URL must not include userinfo");
    }

    if url.query().is_some() || url.fragment().is_some() {
        anyhow::bail!("custom embedding base URL must not include query or fragment");
    }

    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("custom embedding base URL missing host"))?;

    if is_ssrf_blocked_host(host) {
        anyhow::bail!("custom embedding base URL host is blocked");
    }

    Ok(url.as_str().trim_end_matches('/').to_string())
}

pub(super) fn require_non_empty_secret(label: &str, value: Option<&str>) -> anyhow::Result<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!("{label} is not configured"))
}

pub(super) async fn send_json_request(
    provider_name: &str,
    request: reqwest::RequestBuilder,
) -> anyhow::Result<Value> {
    let response = request
        .send()
        .await
        .with_context(|| format!("{provider_name} embedding HTTP request failed"))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        anyhow::bail!("{provider_name} embedding API error ({status}): {error_text}");
    }

    response
        .json()
        .await
        .with_context(|| format!("{provider_name} embedding response JSON decode failed"))
}

pub(super) fn extract_float_vector(value: &Value, provider_name: &str) -> anyhow::Result<Vec<f32>> {
    let array = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("{provider_name} embedding item was not an array"))?;

    #[allow(clippy::cast_possible_truncation)]
    let vector: Vec<f32> = array
        .iter()
        .map(|item| {
            item.as_f64().map(|float| float as f32).ok_or_else(|| {
                anyhow::anyhow!("{provider_name} embedding vector contained a non-number")
            })
        })
        .collect::<anyhow::Result<Vec<f32>>>()?;

    Ok(vector)
}

fn ensure_embedding_count(
    provider_name: &str,
    embeddings: Vec<Vec<f32>>,
    expected: usize,
) -> anyhow::Result<Vec<Vec<f32>>> {
    if embeddings.len() != expected {
        anyhow::bail!(
            "{provider_name} embedding API returned {} embeddings for {} inputs",
            embeddings.len(),
            expected
        );
    }

    Ok(embeddings)
}

pub(super) fn parse_openai_like_embeddings(
    provider_name: &str,
    json: &Value,
    expected: usize,
) -> anyhow::Result<Vec<Vec<f32>>> {
    let data = json
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("{provider_name} response missing `data`"))?;

    let embeddings = data
        .iter()
        .map(|item| {
            let embedding = item
                .get("embedding")
                .ok_or_else(|| anyhow::anyhow!("{provider_name} item missing `embedding`"))?;
            extract_float_vector(embedding, provider_name)
        })
        .collect::<anyhow::Result<Vec<Vec<f32>>>>()?;

    ensure_embedding_count(provider_name, embeddings, expected)
}

pub(super) fn parse_named_array_embeddings(
    provider_name: &str,
    value: &Value,
    expected: usize,
) -> anyhow::Result<Vec<Vec<f32>>> {
    let embeddings = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("{provider_name} embedding array missing"))?
        .iter()
        .map(|item| extract_float_vector(item, provider_name))
        .collect::<anyhow::Result<Vec<Vec<f32>>>>()?;

    ensure_embedding_count(provider_name, embeddings, expected)
}

pub struct OpenAiEmbedding {
    pub(super) provider_name: &'static str,
    pub(super) client: reqwest::Client,
    pub(super) embeddings_url: String,
    pub(super) auth_header: String,
    pub(super) model: String,
    pub(super) dims: usize,
    pub(super) dimensions_field: Option<&'static str>,
}

impl OpenAiEmbedding {
    // TODO(embeddings): expose via EmbeddingFactory::build() so callers don't need new_with_options.
    #[allow(dead_code)]
    #[must_use]
    pub fn new(base_url: &str, api_key: &str, model: &str, dims: usize) -> Self {
        Self::new_with_options("openai", base_url, api_key, model, dims, None)
    }

    pub(super) fn new_with_options(
        provider_name: &'static str,
        base_url: &str,
        api_key: &str,
        model: &str,
        dims: usize,
        dimensions_field: Option<&'static str>,
    ) -> Self {
        let base = base_url.trim_end_matches('/');
        Self {
            provider_name,
            client: build_embedding_http_client(),
            embeddings_url: format!("{base}/v1/embeddings"),
            auth_header: format!("Bearer {api_key}"),
            model: model.to_string(),
            dims,
            dimensions_field,
        }
    }

    fn build_body(&self, texts: &[&str]) -> Value {
        let mut body = json!({
            "model": self.model,
            "input": texts,
        });
        if let Some(field) = self.dimensions_field {
            body[field] = json!(self.dims);
        }
        body
    }

    fn embed_internal<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        Box::pin(async move {
            if texts.is_empty() {
                return Ok(Vec::new());
            }

            let body = self.build_body(texts);
            let json = send_json_request(
                self.provider_name,
                self.client
                    .post(&self.embeddings_url)
                    .header("Authorization", &self.auth_header)
                    .header("Content-Type", "application/json")
                    .json(&body),
            )
            .await?;

            parse_openai_like_embeddings(self.provider_name, &json, texts.len())
        })
    }
}

impl EmbeddingProvider for OpenAiEmbedding {
    fn name(&self) -> &'static str {
        self.provider_name
    }
    fn dimensions(&self) -> usize {
        self.dims
    }
    fn embed<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_internal(texts)
    }
}

pub(super) struct JinaEmbedding {
    inner: OpenAiEmbedding,
}

impl JinaEmbedding {
    pub(super) fn new(api_key: &str, model: &str, dims: usize) -> Self {
        Self {
            inner: OpenAiEmbedding::new_with_options(
                "jina",
                "https://api.jina.ai",
                api_key,
                model,
                dims,
                Some("dimensions"),
            ),
        }
    }

    fn prefixed_texts<'a>(role: EmbeddingRole, texts: &'a [&'a str]) -> Vec<String> {
        texts
            .iter()
            .map(|text| format!("{}{}", role.jina_prefix(), text))
            .collect()
    }

    fn embed_with_role<'a>(
        &'a self,
        role: EmbeddingRole,
        texts: &'a [&'a str],
    ) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        Box::pin(async move {
            if texts.is_empty() {
                return Ok(Vec::new());
            }
            let prefixed = Self::prefixed_texts(role, texts);
            let borrowed = prefixed.iter().map(String::as_str).collect::<Vec<&str>>();
            self.inner.embed_internal(&borrowed).await
        })
    }
}

impl EmbeddingProvider for JinaEmbedding {
    fn name(&self) -> &'static str {
        "jina"
    }
    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }
    fn embed<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_documents(texts)
    }
    fn embed_documents<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_with_role(EmbeddingRole::Document, texts)
    }
    fn embed_queries<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_with_role(EmbeddingRole::Query, texts)
    }
}

pub(super) struct GeminiEmbedding {
    pub(super) client: reqwest::Client,
    pub(super) base_url: String,
    pub(super) model: String,
    pub(super) api_key: String,
    pub(super) dims: usize,
}

impl GeminiEmbedding {
    pub(super) fn new(api_key: &str, model: &str, dims: usize) -> Self {
        Self {
            client: build_embedding_http_client(),
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            model: if model.starts_with("models/") {
                model.to_string()
            } else {
                format!("models/{model}")
            },
            api_key: api_key.to_string(),
            dims,
        }
    }

    fn embed_with_role<'a>(
        &'a self,
        role: EmbeddingRole,
        texts: &'a [&'a str],
    ) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        Box::pin(async move {
            if texts.is_empty() {
                return Ok(Vec::new());
            }

            let requests = texts
                .iter()
                .map(|text| {
                    json!({
                        "model": self.model,
                        "content": { "parts": [{ "text": text }] },
                        "taskType": role.gemini_task_type(),
                        "outputDimensionality": self.dims,
                    })
                })
                .collect::<Vec<Value>>();

            let url = format!(
                "{}/v1beta/{}:batchEmbedContents",
                self.base_url.trim_end_matches('/'),
                self.model
            );
            let body = json!({ "requests": requests });
            let json = send_json_request(
                "gemini",
                self.client
                    .post(url)
                    .header("x-goog-api-key", &self.api_key)
                    .json(&body),
            )
            .await?;

            let embeddings = json
                .get("embeddings")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow::anyhow!("gemini response missing `embeddings`"))?
                .iter()
                .map(|item| {
                    let values = item
                        .get("values")
                        .ok_or_else(|| anyhow::anyhow!("gemini embedding missing `values`"))?;
                    extract_float_vector(values, "gemini")
                })
                .collect::<anyhow::Result<Vec<Vec<f32>>>>()?;

            ensure_embedding_count("gemini", embeddings, texts.len())
        })
    }
}

impl EmbeddingProvider for GeminiEmbedding {
    fn name(&self) -> &'static str {
        "gemini"
    }
    fn dimensions(&self) -> usize {
        self.dims
    }
    fn embed<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_documents(texts)
    }
    fn embed_documents<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_with_role(EmbeddingRole::Document, texts)
    }
    fn embed_queries<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_with_role(EmbeddingRole::Query, texts)
    }
}

pub(super) struct CohereEmbedding {
    pub(super) client: reqwest::Client,
    pub(super) auth_header: String,
    pub(super) model: String,
    pub(super) dims: usize,
}

impl CohereEmbedding {
    pub(super) fn new(api_key: &str, model: &str, dims: usize) -> Self {
        Self {
            client: build_embedding_http_client(),
            auth_header: format!("Bearer {api_key}"),
            model: model.to_string(),
            dims,
        }
    }

    fn embed_with_role<'a>(
        &'a self,
        role: EmbeddingRole,
        texts: &'a [&'a str],
    ) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        Box::pin(async move {
            if texts.is_empty() {
                return Ok(Vec::new());
            }

            let body = json!({
                "model": self.model,
                "texts": texts,
                "input_type": role.cohere_input_type(),
                "embedding_types": ["float"],
                "output_dimension": self.dims,
            });
            let json = send_json_request(
                "cohere",
                self.client
                    .post("https://api.cohere.com/v2/embed")
                    .header("Authorization", &self.auth_header)
                    .header("Content-Type", "application/json")
                    .json(&body),
            )
            .await?;

            let embeddings = json
                .get("embeddings")
                .and_then(|value| value.get("float"))
                .ok_or_else(|| anyhow::anyhow!("cohere response missing `embeddings.float`"))?;
            parse_named_array_embeddings("cohere", embeddings, texts.len())
        })
    }
}

impl EmbeddingProvider for CohereEmbedding {
    fn name(&self) -> &'static str {
        "cohere"
    }
    fn dimensions(&self) -> usize {
        self.dims
    }
    fn embed<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_documents(texts)
    }
    fn embed_documents<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_with_role(EmbeddingRole::Document, texts)
    }
    fn embed_queries<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_with_role(EmbeddingRole::Query, texts)
    }
}

pub(super) struct VoyageEmbedding {
    pub(super) client: reqwest::Client,
    pub(super) auth_header: String,
    pub(super) model: String,
    pub(super) dims: usize,
}

impl VoyageEmbedding {
    pub(super) fn new(api_key: &str, model: &str, dims: usize) -> Self {
        Self {
            client: build_embedding_http_client(),
            auth_header: format!("Bearer {api_key}"),
            model: model.to_string(),
            dims,
        }
    }

    fn embed_with_role<'a>(
        &'a self,
        role: EmbeddingRole,
        texts: &'a [&'a str],
    ) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        Box::pin(async move {
            if texts.is_empty() {
                return Ok(Vec::new());
            }

            let body = json!({
                "model": self.model,
                "input": texts,
                "input_type": role.voyage_input_type(),
                "output_dimension": self.dims,
                "truncation": true,
            });
            let json = send_json_request(
                "voyage",
                self.client
                    .post("https://api.voyageai.com/v1/embeddings")
                    .header("Authorization", &self.auth_header)
                    .header("Content-Type", "application/json")
                    .json(&body),
            )
            .await?;

            parse_openai_like_embeddings("voyage", &json, texts.len())
        })
    }
}

impl EmbeddingProvider for VoyageEmbedding {
    fn name(&self) -> &'static str {
        "voyage"
    }
    fn dimensions(&self) -> usize {
        self.dims
    }
    fn embed<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_documents(texts)
    }
    fn embed_documents<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_with_role(EmbeddingRole::Document, texts)
    }
    fn embed_queries<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_with_role(EmbeddingRole::Query, texts)
    }
}

pub(super) struct MistralEmbedding {
    client: reqwest::Client,
    auth_header: String,
    model: String,
    dims: usize,
}

impl MistralEmbedding {
    pub(super) fn new(api_key: &str, model: &str, dims: usize) -> Self {
        Self {
            client: build_embedding_http_client(),
            auth_header: format!("Bearer {api_key}"),
            model: model.to_string(),
            dims,
        }
    }
}

impl EmbeddingProvider for MistralEmbedding {
    fn name(&self) -> &'static str {
        "mistral"
    }
    fn dimensions(&self) -> usize {
        self.dims
    }
    fn embed<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        Box::pin(async move {
            if texts.is_empty() {
                return Ok(Vec::new());
            }

            let body =
                json!({ "model": self.model, "inputs": texts, "output_dimension": self.dims });
            let json = send_json_request(
                "mistral",
                self.client
                    .post("https://api.mistral.ai/v1/embeddings")
                    .header("Authorization", &self.auth_header)
                    .header("Content-Type", "application/json")
                    .json(&body),
            )
            .await?;

            parse_openai_like_embeddings("mistral", &json, texts.len())
        })
    }
}

pub(super) struct NomicEmbedding {
    pub(super) client: reqwest::Client,
    pub(super) auth_header: String,
    pub(super) model: String,
    pub(super) dims: usize,
}

impl NomicEmbedding {
    pub(super) fn new(api_key: &str, model: &str, dims: usize) -> Self {
        Self {
            client: build_embedding_http_client(),
            auth_header: format!("Bearer {api_key}"),
            model: model.to_string(),
            dims,
        }
    }

    fn embed_with_role<'a>(
        &'a self,
        role: EmbeddingRole,
        texts: &'a [&'a str],
    ) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        Box::pin(async move {
            if texts.is_empty() {
                return Ok(Vec::new());
            }

            let body = json!({
                "model": self.model,
                "texts": texts,
                "task_type": role.nomic_task_type(),
                "dimensionality": self.dims,
            });
            let json = send_json_request(
                "nomic",
                self.client
                    .post("https://api-atlas.nomic.ai/v1/embedding/text")
                    .header("Authorization", &self.auth_header)
                    .header("Content-Type", "application/json")
                    .json(&body),
            )
            .await?;

            let embeddings = json
                .get("embeddings")
                .ok_or_else(|| anyhow::anyhow!("nomic response missing `embeddings`"))?;
            parse_named_array_embeddings("nomic", embeddings, texts.len())
        })
    }
}

impl EmbeddingProvider for NomicEmbedding {
    fn name(&self) -> &'static str {
        "nomic"
    }
    fn dimensions(&self) -> usize {
        self.dims
    }
    fn embed<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_documents(texts)
    }
    fn embed_documents<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_with_role(EmbeddingRole::Document, texts)
    }
    fn embed_queries<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed_with_role(EmbeddingRole::Query, texts)
    }
}
