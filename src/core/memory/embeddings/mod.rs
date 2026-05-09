//! Embedding provider trait and implementations for dense retrieval.
//!
//! Defines the [`EmbeddingProvider`] interface and concrete HTTP-based clients
//! for the following providers:
//!
//! | Provider | Sub-module | Notes |
//! |----------|------------|-------|
//! | `OpenAI` | `http_providers` | Default; supports `text-embedding-3-*` |
//! | `Cohere` | `http_providers` | Asymmetric: separate `search_document` / `search_query` input types |
//! | `Voyage` | `http_providers` | Asymmetric: `document` / `query` input types |
//! | `Jina` | `http_providers` | Prefix-based asymmetric (`passage:` / `query:`) |
//! | `Mistral` | `http_providers` | Symmetric embedding API |
//! | `Nomic` | `http_providers` | Task-typed asymmetric (`search_document` / `search_query`) |
//! | `Gemini` | `http_providers` | Task-typed via `RETRIEVAL_DOCUMENT` / `RETRIEVAL_QUERY` |
//! | `AWS Bedrock` | `bedrock` | HMAC-SHA256 SigV4 auth; supports `amazon.titan-embed-*` |
//!
//! [`EmbeddingRole`] distinguishes document (indexing) from query (search) embeddings.
//! Providers that support asymmetric roles use it to select the appropriate input
//! type or prefix automatically.
//!
//! References: [DPR] Karpukhin et al., 2020 — Dense Passage Retrieval. See the
//! public research reference index in the docs site.

mod bedrock;
mod http_providers;
#[cfg(test)]
mod tests;

use std::future::Future;
use std::pin::Pin;

use hmac::Hmac;
use sha2::Sha256;

use bedrock::BedrockEmbedding;
use http_providers::{
    CohereEmbedding, CustomBaseUrlPolicy, GeminiEmbedding, JinaEmbedding, MistralEmbedding,
    NomicEmbedding, VoyageEmbedding, require_non_empty_secret, validate_custom_base_url,
};

pub use http_providers::OpenAiEmbedding;

type HmacSha256 = Hmac<Sha256>;

/// Shared boxed future shape used by embedding provider APIs.
pub type EmbeddingFuture<'a, T> = Pin<Box<dyn Future<Output = anyhow::Result<T>> + Send + 'a>>;

/// Retrieval role for an embedding request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmbeddingRole {
    Document,
    Query,
}

impl EmbeddingRole {
    /// Returns the `Cohere` `input_type` string for this role.
    fn cohere_input_type(self) -> &'static str {
        match self {
            Self::Document => "search_document",
            Self::Query => "search_query",
        }
    }

    /// Returns the `Voyage` `input_type` string for this role.
    fn voyage_input_type(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Query => "query",
        }
    }

    /// Returns the `Nomic` `task_type` string for this role.
    fn nomic_task_type(self) -> &'static str {
        match self {
            Self::Document => "search_document",
            Self::Query => "search_query",
        }
    }

    /// Returns the `Gemini` `task_type` string for this role.
    fn gemini_task_type(self) -> &'static str {
        match self {
            Self::Document => "RETRIEVAL_DOCUMENT",
            Self::Query => "RETRIEVAL_QUERY",
        }
    }

    /// Returns the `Jina` instruction prefix prepended to each text.
    fn jina_prefix(self) -> &'static str {
        match self {
            Self::Document => "passage: ",
            Self::Query => "query: ",
        }
    }
}

/// Trait for embedding providers — convert text to vectors.
pub trait EmbeddingProvider: Send + Sync {
    /// Provider name.
    fn name(&self) -> &'static str;

    /// Embedding dimensions.
    fn dimensions(&self) -> usize;

    /// Embed a batch of texts into vectors.
    fn embed<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>>;

    /// Embed documents for retrieval/indexing.
    fn embed_documents<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed(texts)
    }

    /// Embed queries for retrieval.
    fn embed_queries<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        self.embed(texts)
    }

    /// Embed a single text with the provider's generic path.
    fn embed_one<'a>(&'a self, text: &'a str) -> EmbeddingFuture<'a, Vec<f32>> {
        Box::pin(async move {
            let mut results = self.embed(&[text]).await?;
            results
                .pop()
                .ok_or_else(|| anyhow::anyhow!("empty embedding result"))
        })
    }

    /// Embed a single document.
    fn embed_one_document<'a>(&'a self, text: &'a str) -> EmbeddingFuture<'a, Vec<f32>> {
        Box::pin(async move {
            let mut results = self.embed_documents(&[text]).await?;
            results
                .pop()
                .ok_or_else(|| anyhow::anyhow!("empty document embedding result"))
        })
    }

    /// Embed a single query.
    fn embed_one_query<'a>(&'a self, text: &'a str) -> EmbeddingFuture<'a, Vec<f32>> {
        Box::pin(async move {
            let mut results = self.embed_queries(&[text]).await?;
            results
                .pop()
                .ok_or_else(|| anyhow::anyhow!("empty query embedding result"))
        })
    }
}

#[cfg(test)]
pub(crate) struct DeterministicEmbedding {
    dims: usize,
    seed: u64,
}

#[cfg(test)]
impl DeterministicEmbedding {
    pub(crate) fn with_seed(dims: usize, seed: u64) -> Self {
        Self { dims, seed }
    }

    fn fnv1a64(seed: u64, bytes: &[u8]) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325 ^ seed;
        for &b in bytes {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        hash
    }

    fn splitmix64(mut x: u64) -> u64 {
        x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    #[allow(clippy::cast_precision_loss)]
    fn u64_to_unit_f32(x: u64) -> f32 {
        const U24_MAX: f32 = ((1u32 << 24) - 1) as f32;
        let top_u24: u32 = (x >> 40) as u32;
        (top_u24 as f32 / U24_MAX) * 2.0 - 1.0
    }
}

#[cfg(test)]
impl EmbeddingProvider for DeterministicEmbedding {
    fn name(&self) -> &'static str {
        "deterministic_test"
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn embed<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        Box::pin(async move {
            let mut out = Vec::with_capacity(texts.len());
            for &text in texts {
                let base = Self::fnv1a64(self.seed, text.as_bytes());
                let mut vector = Vec::with_capacity(self.dims);
                for i in 0..self.dims {
                    let mixed = Self::splitmix64(base ^ (i as u64));
                    vector.push(Self::u64_to_unit_f32(mixed));
                }
                out.push(vector);
            }
            Ok(out)
        })
    }
}

/// No-op embedding provider that returns empty vectors.
pub struct NoopEmbedding;

impl EmbeddingProvider for NoopEmbedding {
    fn name(&self) -> &'static str {
        "none"
    }

    fn dimensions(&self) -> usize {
        0
    }

    fn embed<'a>(&'a self, _texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        Box::pin(async move { Ok(Vec::new()) })
    }
}

/// Build an embedding provider from configuration settings.
///
/// # Errors
///
/// Returns an error when credentials are missing, a custom endpoint is
/// invalid, or the provider cannot be initialized.
pub fn create_embedding_provider(
    provider: &crate::config::EmbeddingProvider,
    api_key: Option<&str>,
    model: &str,
    configured_dims: usize,
) -> anyhow::Result<Box<dyn EmbeddingProvider>> {
    let dims = provider.effective_dimensions(configured_dims, model);
    match provider {
        crate::config::EmbeddingProvider::None => Ok(Box::new(NoopEmbedding)),
        crate::config::EmbeddingProvider::OpenAi => {
            let key = require_non_empty_secret("OpenAI embedding API key", api_key)?;
            Ok(Box::new(OpenAiEmbedding::new_with_options(
                "openai",
                "https://api.openai.com",
                &key,
                model,
                dims,
                Some("dimensions"),
            )))
        }
        crate::config::EmbeddingProvider::OpenAiCompatible(base_url) => {
            let key = require_non_empty_secret("OpenAI-compatible embedding API key", api_key)?;
            let validated_base_url = validate_custom_base_url(
                base_url,
                CustomBaseUrlPolicy {
                    allow_http: cfg!(test),
                },
            )?;
            Ok(Box::new(OpenAiEmbedding::new_with_options(
                "openai-compatible",
                &validated_base_url,
                &key,
                model,
                dims,
                Some("dimensions"),
            )))
        }
        crate::config::EmbeddingProvider::Gemini => {
            let key = require_non_empty_secret("Gemini embedding API key", api_key)?;
            Ok(Box::new(GeminiEmbedding::new(&key, model, dims)))
        }
        crate::config::EmbeddingProvider::Cohere => {
            let key = require_non_empty_secret("Cohere embedding API key", api_key)?;
            Ok(Box::new(CohereEmbedding::new(&key, model, dims)))
        }
        crate::config::EmbeddingProvider::Voyage => {
            let key = require_non_empty_secret("Voyage embedding API key", api_key)?;
            Ok(Box::new(VoyageEmbedding::new(&key, model, dims)))
        }
        crate::config::EmbeddingProvider::Jina => {
            let key = require_non_empty_secret("Jina embedding API key", api_key)?;
            Ok(Box::new(JinaEmbedding::new(&key, model, dims)))
        }
        crate::config::EmbeddingProvider::Mistral => {
            let key = require_non_empty_secret("Mistral embedding API key", api_key)?;
            Ok(Box::new(MistralEmbedding::new(&key, model, dims)))
        }
        crate::config::EmbeddingProvider::Nomic => {
            let key = require_non_empty_secret("Nomic embedding API key", api_key)?;
            Ok(Box::new(NomicEmbedding::new(&key, model, dims)))
        }
        crate::config::EmbeddingProvider::Bedrock => {
            let env_access_key_id = std::env::var("AWS_ACCESS_KEY_ID")
                .ok()
                .filter(|value| !value.trim().is_empty());
            let access_key_id = require_non_empty_secret(
                "AWS_ACCESS_KEY_ID / Bedrock access key id",
                api_key.or(env_access_key_id.as_deref()),
            )?;
            let secret_access_key = require_non_empty_secret(
                "AWS_SECRET_ACCESS_KEY / Bedrock secret access key",
                std::env::var("AWS_SECRET_ACCESS_KEY").ok().as_deref(),
            )?;
            let region = std::env::var("AWS_REGION")
                .ok()
                .or_else(|| std::env::var("AWS_DEFAULT_REGION").ok())
                .unwrap_or_else(|| "us-east-1".to_string());
            let endpoint = format!("https://bedrock-runtime.{region}.amazonaws.com");
            let session_token = std::env::var("AWS_SESSION_TOKEN")
                .ok()
                .filter(|token| !token.trim().is_empty());

            Ok(Box::new(BedrockEmbedding::new(
                &endpoint,
                &region,
                &access_key_id,
                &secret_access_key,
                session_token,
                model,
                dims,
            )?))
        }
    }
}
