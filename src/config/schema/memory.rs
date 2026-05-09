//! Memory subsystem configuration: backend selection (Postgres,
//! Markdown, None), embedding provider, hybrid search weights,
//! retention policies, and connection pool tuning.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Memory storage backend.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryBackend {
    /// `PostgreSQL` typed graph backend (default).
    #[default]
    Postgres,
    /// Human-readable Markdown file backend.
    Markdown,
    /// Compatibility selector that currently routes to the Markdown fallback
    /// backend rather than a truly stateless store.
    None,
}

impl fmt::Display for MemoryBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postgres => write!(f, "postgres"),
            Self::Markdown => write!(f, "markdown"),
            Self::None => write!(f, "none"),
        }
    }
}

impl MemoryBackend {
    /// Returns the backend name as a static string slice.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::Markdown => "markdown",
            Self::None => "none",
        }
    }
}

/// Embedding provider configuration.
///
/// Serializes to/from the string format used in TOML config:
/// `"none"`, `"openai"`, `"gemini"`, `"cohere"`, `"voyage"`,
/// `"jina"`, `"mistral"`, `"bedrock"`, `"nomic"`, or `"custom:URL"`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum EmbeddingProvider {
    /// No embedding provider (vector search disabled).
    #[default]
    None,
    /// `OpenAI` embeddings API.
    OpenAi,
    /// Google Gemini embeddings API.
    Gemini,
    /// Cohere embeddings API.
    Cohere,
    /// Voyage AI embeddings API.
    Voyage,
    /// Jina AI embeddings API.
    Jina,
    /// Mistral embeddings API.
    Mistral,
    /// Amazon Bedrock embeddings API.
    Bedrock,
    /// Nomic embeddings API.
    Nomic,
    /// Custom OpenAI-compatible endpoint URL.
    OpenAiCompatible(String),
}

impl EmbeddingProvider {
    /// Parse from the string format used in config files.
    ///
    /// # Errors
    ///
    /// Returns an error when the provider string is not a supported
    /// embedding backend selector.
    pub fn from_config_str(s: &str) -> Result<Self, String> {
        let normalized = s.trim();
        match normalized {
            "" | "none" => Ok(Self::None),
            "openai" => Ok(Self::OpenAi),
            "gemini" | "google" | "google-gemini" => Ok(Self::Gemini),
            "cohere" => Ok(Self::Cohere),
            "voyage" | "voyageai" | "voyage-ai" => Ok(Self::Voyage),
            "jina" | "jinaai" | "jina-ai" => Ok(Self::Jina),
            "mistral" => Ok(Self::Mistral),
            "bedrock" | "aws-bedrock" => Ok(Self::Bedrock),
            "nomic" => Ok(Self::Nomic),
            value if value.starts_with("custom:") => Ok(Self::OpenAiCompatible(
                value.strip_prefix("custom:").unwrap_or("").to_string(),
            )),
            value if value.starts_with("openai-compatible:") => Ok(Self::OpenAiCompatible(
                value
                    .strip_prefix("openai-compatible:")
                    .unwrap_or("")
                    .to_string(),
            )),
            _ => Err(format!(
                "unknown embedding provider `{normalized}`; expected one of \
                 none, openai, gemini, cohere, voyage, jina, mistral, bedrock, nomic, \
                 or custom:https://..."
            )),
        }
    }

    /// Returns `true` if this provider requires an API key.
    #[must_use]
    pub fn needs_api_key(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// Returns the auth provider selector used to resolve credentials for this
    /// embedding backend.
    #[must_use]
    pub fn credential_provider_selector(&self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::OpenAi | Self::OpenAiCompatible(_) => Some("openai"),
            Self::Gemini => Some("gemini"),
            Self::Cohere => Some("cohere"),
            Self::Voyage => Some("voyage"),
            Self::Jina => Some("jina"),
            Self::Mistral => Some("mistral"),
            Self::Bedrock => Some("bedrock"),
            Self::Nomic => Some("nomic"),
        }
    }

    /// Resolve the effective embedding dimensions. A configured value of `0`
    /// means "provider/model default".
    #[must_use]
    pub fn effective_dimensions(&self, configured_dimensions: usize, model: &str) -> usize {
        if configured_dimensions > 0 {
            configured_dimensions
        } else {
            self.default_dimensions_for_model(model)
        }
    }

    fn default_dimensions_for_model(&self, model: &str) -> usize {
        let normalized = model.trim().to_ascii_lowercase();
        match self {
            Self::None => 0,
            Self::OpenAi | Self::OpenAiCompatible(_) => {
                if normalized.contains("text-embedding-3-large") {
                    3072
                } else {
                    1536
                }
            }
            Self::Gemini => 3072,
            Self::Cohere => {
                if normalized.contains("light") {
                    384
                } else if normalized.contains("embed-v4") {
                    1536
                } else {
                    1024
                }
            }
            Self::Voyage => {
                if normalized.contains("voyage-code-2") {
                    1536
                } else if normalized.contains("lite") {
                    512
                } else {
                    1024
                }
            }
            Self::Jina => {
                if normalized.contains("v5-text-nano") {
                    768
                } else if normalized.contains("v5-text-small") {
                    1024
                } else if normalized.contains("v4") {
                    2048
                } else {
                    1024
                }
            }
            Self::Mistral | Self::Bedrock => 1024,
            Self::Nomic => 768,
        }
    }
}

impl fmt::Display for EmbeddingProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::OpenAi => write!(f, "openai"),
            Self::Gemini => write!(f, "gemini"),
            Self::Cohere => write!(f, "cohere"),
            Self::Voyage => write!(f, "voyage"),
            Self::Jina => write!(f, "jina"),
            Self::Mistral => write!(f, "mistral"),
            Self::Bedrock => write!(f, "bedrock"),
            Self::Nomic => write!(f, "nomic"),
            Self::OpenAiCompatible(url) => write!(f, "custom:{url}"),
        }
    }
}

impl Serialize for EmbeddingProvider {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for EmbeddingProvider {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::from_config_str(&s).map_err(serde::de::Error::custom)
    }
}

/// Memory subsystem configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Storage backend selection. Default: postgres.
    #[serde(default)]
    pub backend: MemoryBackend,
    /// `PostgreSQL` connection URL (env: `ASTEREL_POSTGRES_URL`)
    #[serde(default)]
    pub postgres_url: Option<String>,
    /// Maximum number of pool connections (default: 10)
    #[serde(default = "default_pg_max_connections")]
    pub pg_max_connections: u32,
    /// Connection acquisition timeout in seconds (default: 5)
    #[serde(default = "default_pg_connect_timeout_secs")]
    pub pg_connect_timeout_secs: u64,
    /// Idle connection timeout in seconds (default: 300)
    #[serde(default = "default_pg_idle_timeout_secs")]
    pub pg_idle_timeout_secs: u64,
    /// Minimum number of idle pool connections (default: 1)
    #[serde(default = "default_pg_min_connections")]
    pub pg_min_connections: u32,
    /// Maximum lifetime of a connection before replacement (seconds,
    /// default: 1800 = 30 min).
    #[serde(default = "default_pg_max_lifetime_secs")]
    pub pg_max_lifetime_secs: u64,
    /// HNSW `ef_search` parameter set on every new pool connection
    /// via `after_connect` hook (default: 100). Set to 0 to skip.
    #[serde(default = "default_pg_hnsw_ef_search")]
    pub pg_hnsw_ef_search: u32,
    /// Auto-save conversation context to memory
    #[serde(default = "default_auto_save")]
    pub auto_save: bool,
    /// Run memory/session hygiene (archiving + retention cleanup)
    #[serde(default = "default_hygiene_enabled")]
    pub hygiene_enabled: bool,
    /// Archive daily/session files older than this many days
    #[serde(default = "default_archive_after_days")]
    pub archive_after_days: u32,
    /// Purge archived files older than this many days
    #[serde(default = "default_purge_after_days")]
    pub purge_after_days: u32,
    /// For persisted memory backends: prune conversation records older than this many days
    #[serde(default = "default_conversation_retention_days")]
    pub conversation_retention_days: u32,
    /// Working layer retention override (days).
    #[serde(default)]
    pub layer_retention_working_days: Option<u32>,
    /// Episodic layer retention override (days).
    #[serde(default)]
    pub layer_retention_episodic_days: Option<u32>,
    /// Semantic layer retention override (days).
    #[serde(default)]
    pub layer_retention_semantic_days: Option<u32>,
    /// Procedural layer retention override (days).
    #[serde(default)]
    pub layer_retention_procedural_days: Option<u32>,
    /// Identity layer retention override (days).
    #[serde(default)]
    pub layer_retention_identity_days: Option<u32>,
    /// Ledger (audit log) retention override (days).
    #[serde(default)]
    pub ledger_retention_days: Option<u32>,
    /// Embedding provider: "none" | "openai" | "gemini" | "cohere" |
    /// "voyage" | "jina" | "mistral" | "bedrock" | "nomic" |
    /// "custom:URL"
    #[serde(default)]
    pub embedding_provider: EmbeddingProvider,
    /// Embedding model name (e.g. "text-embedding-3-small")
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    /// Embedding vector dimensions. `0` means "provider/model default".
    #[serde(default = "default_embedding_dims")]
    pub embedding_dimensions: usize,
    /// Weight for vector similarity in hybrid search (0.0–1.0)
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f64,
    /// Weight for keyword BM25 in hybrid search (0.0–1.0)
    #[serde(default = "default_keyword_weight")]
    pub keyword_weight: f64,
    /// Enable graph-based retrieval fusion. Default: true.
    #[serde(default = "default_graph_retrieval_fusion_enabled")]
    pub graph_retrieval_fusion_enabled: bool,
    /// Weight for graph retrieval in hybrid search. Default: 0.15.
    #[serde(default = "default_graph_retrieval_weight")]
    pub graph_retrieval_weight: f64,
    /// Max embedding cache entries before LRU eviction
    #[serde(default = "default_cache_size")]
    pub embedding_cache_size: usize,
    /// Max tokens per chunk for document splitting
    #[serde(default = "default_chunk_size")]
    pub chunk_max_tokens: usize,
    /// Minimum confidence for recall items to be injected into context.
    /// Items below this threshold are silently dropped. Default: 0.3.
    #[serde(default = "default_recall_min_confidence")]
    pub recall_min_confidence: f64,
    /// Maximum number of items held in session working memory. Default: 50.
    #[serde(default = "default_working_memory_capacity")]
    pub working_memory_capacity: usize,
}

fn default_auto_save() -> bool {
    true
}
fn default_hygiene_enabled() -> bool {
    true
}
fn default_archive_after_days() -> u32 {
    7
}
fn default_purge_after_days() -> u32 {
    30
}
fn default_conversation_retention_days() -> u32 {
    30
}
fn default_embedding_model() -> String {
    "text-embedding-3-small".into()
}
fn default_embedding_dims() -> usize {
    0
}
fn default_vector_weight() -> f64 {
    0.7
}
fn default_keyword_weight() -> f64 {
    0.3
}
fn default_graph_retrieval_fusion_enabled() -> bool {
    true
}
fn default_graph_retrieval_weight() -> f64 {
    0.15
}
fn default_cache_size() -> usize {
    10_000
}
fn default_chunk_size() -> usize {
    512
}
fn default_recall_min_confidence() -> f64 {
    0.3
}
fn default_working_memory_capacity() -> usize {
    50
}
fn default_pg_max_connections() -> u32 {
    10
}
fn default_pg_connect_timeout_secs() -> u64 {
    5
}
fn default_pg_idle_timeout_secs() -> u64 {
    300
}
fn default_pg_min_connections() -> u32 {
    1
}
fn default_pg_max_lifetime_secs() -> u64 {
    1800
}
fn default_pg_hnsw_ef_search() -> u32 {
    100
}

impl MemoryConfig {
    /// Returns retention days for the named layer, falling back to
    /// `conversation_retention_days` if no override is set.
    #[must_use]
    pub fn layer_retention_days(&self, layer: &str) -> u32 {
        match layer {
            "working" => self
                .layer_retention_working_days
                .unwrap_or(self.conversation_retention_days),
            "episodic" => self
                .layer_retention_episodic_days
                .unwrap_or(self.conversation_retention_days),
            "semantic" => self
                .layer_retention_semantic_days
                .unwrap_or(self.conversation_retention_days),
            "procedural" => self
                .layer_retention_procedural_days
                .unwrap_or(self.conversation_retention_days),
            "identity" => self
                .layer_retention_identity_days
                .unwrap_or(self.conversation_retention_days),
            _ => self.conversation_retention_days,
        }
    }

    /// Returns ledger retention days, falling back to
    /// `conversation_retention_days` if no override is set.
    #[must_use]
    pub fn ledger_retention_or_default(&self) -> u32 {
        self.ledger_retention_days
            .unwrap_or(self.conversation_retention_days)
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: MemoryBackend::default(),
            postgres_url: None,
            pg_max_connections: default_pg_max_connections(),
            pg_connect_timeout_secs: default_pg_connect_timeout_secs(),
            pg_idle_timeout_secs: default_pg_idle_timeout_secs(),
            pg_min_connections: default_pg_min_connections(),
            pg_max_lifetime_secs: default_pg_max_lifetime_secs(),
            pg_hnsw_ef_search: default_pg_hnsw_ef_search(),
            auto_save: true,
            hygiene_enabled: default_hygiene_enabled(),
            archive_after_days: default_archive_after_days(),
            purge_after_days: default_purge_after_days(),
            conversation_retention_days: default_conversation_retention_days(),
            layer_retention_working_days: None,
            layer_retention_episodic_days: None,
            layer_retention_semantic_days: None,
            layer_retention_procedural_days: None,
            layer_retention_identity_days: None,
            ledger_retention_days: None,
            embedding_provider: EmbeddingProvider::default(),
            embedding_model: default_embedding_model(),
            embedding_dimensions: default_embedding_dims(),
            vector_weight: default_vector_weight(),
            keyword_weight: default_keyword_weight(),
            graph_retrieval_fusion_enabled: default_graph_retrieval_fusion_enabled(),
            graph_retrieval_weight: default_graph_retrieval_weight(),
            embedding_cache_size: default_cache_size(),
            chunk_max_tokens: default_chunk_size(),
            recall_min_confidence: default_recall_min_confidence(),
            working_memory_capacity: default_working_memory_capacity(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::memory::MemoryLayer;

    fn assert_close(lhs: f64, rhs: f64) {
        assert!((lhs - rhs).abs() < 1e-9, "lhs={lhs} rhs={rhs}");
    }

    #[test]
    fn default_memory_config_values() {
        let config = MemoryConfig::default();

        assert_eq!(config.backend, MemoryBackend::Postgres);
        assert!(config.auto_save);
        assert!(config.hygiene_enabled);
        assert_eq!(config.archive_after_days, 7);
        assert_eq!(config.purge_after_days, 30);
        assert_eq!(config.conversation_retention_days, 30);
        assert_eq!(config.layer_retention_working_days, None);
        assert_eq!(config.layer_retention_episodic_days, None);
        assert_eq!(config.layer_retention_semantic_days, None);
        assert_eq!(config.layer_retention_procedural_days, None);
        assert_eq!(config.layer_retention_identity_days, None);
        assert_eq!(config.ledger_retention_days, None);
        assert_eq!(config.embedding_provider, EmbeddingProvider::None);
        assert_eq!(config.embedding_model, "text-embedding-3-small");
        assert_eq!(config.embedding_dimensions, 0);
        assert_close(config.vector_weight, 0.7);
        assert_close(config.keyword_weight, 0.3);
        assert!(config.graph_retrieval_fusion_enabled);
        assert_close(config.graph_retrieval_weight, 0.15);
        assert_eq!(config.embedding_cache_size, 10_000);
        assert_eq!(config.chunk_max_tokens, 512);
        assert_eq!(config.pg_max_lifetime_secs, 1800);
        assert_eq!(config.pg_hnsw_ef_search, 100);
    }

    #[test]
    fn embedding_provider_reports_credential_selector() {
        assert_eq!(EmbeddingProvider::None.credential_provider_selector(), None);
        assert_eq!(
            EmbeddingProvider::OpenAi.credential_provider_selector(),
            Some("openai")
        );
        assert_eq!(
            EmbeddingProvider::Gemini.credential_provider_selector(),
            Some("gemini")
        );
        assert_eq!(
            EmbeddingProvider::Cohere.credential_provider_selector(),
            Some("cohere")
        );
        assert_eq!(
            EmbeddingProvider::Voyage.credential_provider_selector(),
            Some("voyage")
        );
        assert_eq!(
            EmbeddingProvider::Jina.credential_provider_selector(),
            Some("jina")
        );
        assert_eq!(
            EmbeddingProvider::Mistral.credential_provider_selector(),
            Some("mistral")
        );
        assert_eq!(
            EmbeddingProvider::Bedrock.credential_provider_selector(),
            Some("bedrock")
        );
        assert_eq!(
            EmbeddingProvider::Nomic.credential_provider_selector(),
            Some("nomic")
        );
        assert_eq!(
            EmbeddingProvider::OpenAiCompatible("https://embed.example".into())
                .credential_provider_selector(),
            Some("openai")
        );
    }

    #[test]
    fn embedding_provider_parses_supported_variants() {
        assert_eq!(
            EmbeddingProvider::from_config_str("openai").unwrap(),
            EmbeddingProvider::OpenAi
        );
        assert_eq!(
            EmbeddingProvider::from_config_str("google").unwrap(),
            EmbeddingProvider::Gemini
        );
        assert_eq!(
            EmbeddingProvider::from_config_str("cohere").unwrap(),
            EmbeddingProvider::Cohere
        );
        assert_eq!(
            EmbeddingProvider::from_config_str("voyage").unwrap(),
            EmbeddingProvider::Voyage
        );
        assert_eq!(
            EmbeddingProvider::from_config_str("jina").unwrap(),
            EmbeddingProvider::Jina
        );
        assert_eq!(
            EmbeddingProvider::from_config_str("mistral").unwrap(),
            EmbeddingProvider::Mistral
        );
        assert_eq!(
            EmbeddingProvider::from_config_str("aws-bedrock").unwrap(),
            EmbeddingProvider::Bedrock
        );
        assert_eq!(
            EmbeddingProvider::from_config_str("nomic").unwrap(),
            EmbeddingProvider::Nomic
        );
        assert_eq!(
            EmbeddingProvider::from_config_str("custom:https://embed.example").unwrap(),
            EmbeddingProvider::OpenAiCompatible("https://embed.example".into())
        );
    }

    #[test]
    fn embedding_provider_rejects_unknown_variant() {
        let error = EmbeddingProvider::from_config_str("unknown-provider").unwrap_err();
        assert!(error.contains("unknown embedding provider"));
    }

    #[test]
    fn embedding_provider_resolves_provider_defaults() {
        assert_eq!(
            EmbeddingProvider::OpenAi.effective_dimensions(0, "text-embedding-3-large"),
            3072
        );
        assert_eq!(
            EmbeddingProvider::Gemini.effective_dimensions(0, "gemini-embedding-001"),
            3072
        );
        assert_eq!(
            EmbeddingProvider::Cohere.effective_dimensions(0, "embed-v4.0"),
            1536
        );
        assert_eq!(
            EmbeddingProvider::Voyage.effective_dimensions(0, "voyage-4"),
            1024
        );
        assert_eq!(
            EmbeddingProvider::Jina.effective_dimensions(0, "jina-embeddings-v5-text-nano"),
            768
        );
        assert_eq!(
            EmbeddingProvider::Mistral.effective_dimensions(0, "mistral-embed"),
            1024
        );
        assert_eq!(
            EmbeddingProvider::Nomic.effective_dimensions(0, "nomic-embed-text-v1.5"),
            768
        );
    }

    #[test]
    fn layer_retention_days_uses_layer_specific_values() {
        let config = MemoryConfig {
            conversation_retention_days: 30,
            layer_retention_working_days: Some(3),
            layer_retention_episodic_days: Some(14),
            layer_retention_semantic_days: Some(90),
            layer_retention_procedural_days: Some(120),
            layer_retention_identity_days: Some(365),
            ..MemoryConfig::default()
        };

        let cases = [
            (MemoryLayer::Working, "working", 3),
            (MemoryLayer::Episodic, "episodic", 14),
            (MemoryLayer::Semantic, "semantic", 90),
            (MemoryLayer::Procedural, "procedural", 120),
            (MemoryLayer::Identity, "identity", 365),
        ];

        for (_layer, layer_name, expected_days) in cases {
            assert_eq!(config.layer_retention_days(layer_name), expected_days);
        }
    }

    #[test]
    fn layer_retention_days_falls_back_to_conversation_retention() {
        let config = MemoryConfig {
            conversation_retention_days: 45,
            ..MemoryConfig::default()
        };

        assert_eq!(config.layer_retention_days("working"), 45);
        assert_eq!(config.layer_retention_days("episodic"), 45);
        assert_eq!(config.layer_retention_days("semantic"), 45);
        assert_eq!(config.layer_retention_days("procedural"), 45);
        assert_eq!(config.layer_retention_days("identity"), 45);
        assert_eq!(config.layer_retention_days("unknown"), 45);
    }

    #[test]
    fn ledger_retention_or_default_respects_override() {
        let with_override = MemoryConfig {
            conversation_retention_days: 30,
            ledger_retention_days: Some(180),
            ..MemoryConfig::default()
        };
        assert_eq!(with_override.ledger_retention_or_default(), 180);

        let without_override = MemoryConfig {
            conversation_retention_days: 60,
            ledger_retention_days: None,
            ..MemoryConfig::default()
        };
        assert_eq!(without_override.ledger_retention_or_default(), 60);
    }

    #[test]
    fn memory_config_toml_round_trip() {
        let original = MemoryConfig {
            backend: MemoryBackend::Markdown,
            postgres_url: Some("postgres://localhost/test".into()),
            pg_max_connections: 20,
            pg_connect_timeout_secs: 10,
            pg_idle_timeout_secs: 600,
            pg_min_connections: 2,
            pg_max_lifetime_secs: 900,
            pg_hnsw_ef_search: 200,
            auto_save: false,
            hygiene_enabled: false,
            archive_after_days: 3,
            purge_after_days: 12,
            conversation_retention_days: 48,
            layer_retention_working_days: Some(2),
            layer_retention_episodic_days: Some(10),
            layer_retention_semantic_days: Some(60),
            layer_retention_procedural_days: Some(120),
            layer_retention_identity_days: Some(365),
            ledger_retention_days: Some(90),
            embedding_provider: EmbeddingProvider::OpenAiCompatible("https://embed.example".into()),
            embedding_model: "example-embed-v1".into(),
            embedding_dimensions: 1024,
            vector_weight: 0.65,
            keyword_weight: 0.35,
            graph_retrieval_fusion_enabled: false,
            graph_retrieval_weight: 0.25,
            embedding_cache_size: 2048,
            chunk_max_tokens: 256,
            recall_min_confidence: 0.4,
            working_memory_capacity: 75,
        };

        let toml = toml::to_string(&original).unwrap();
        let decoded: MemoryConfig = toml::from_str(&toml).unwrap();

        assert_eq!(decoded.backend, original.backend);
        assert_eq!(decoded.auto_save, original.auto_save);
        assert_eq!(decoded.hygiene_enabled, original.hygiene_enabled);
        assert_eq!(decoded.archive_after_days, original.archive_after_days);
        assert_eq!(decoded.purge_after_days, original.purge_after_days);
        assert_eq!(
            decoded.conversation_retention_days,
            original.conversation_retention_days
        );
        assert_eq!(
            decoded.layer_retention_working_days,
            original.layer_retention_working_days
        );
        assert_eq!(
            decoded.layer_retention_episodic_days,
            original.layer_retention_episodic_days
        );
        assert_eq!(
            decoded.layer_retention_semantic_days,
            original.layer_retention_semantic_days
        );
        assert_eq!(
            decoded.layer_retention_procedural_days,
            original.layer_retention_procedural_days
        );
        assert_eq!(
            decoded.layer_retention_identity_days,
            original.layer_retention_identity_days
        );
        assert_eq!(
            decoded.ledger_retention_days,
            original.ledger_retention_days
        );
        assert_eq!(decoded.embedding_provider, original.embedding_provider);
        assert_eq!(decoded.embedding_model, original.embedding_model);
        assert_eq!(decoded.embedding_dimensions, original.embedding_dimensions);
        assert_close(decoded.vector_weight, original.vector_weight);
        assert_close(decoded.keyword_weight, original.keyword_weight);
        assert_eq!(
            decoded.graph_retrieval_fusion_enabled,
            original.graph_retrieval_fusion_enabled
        );
        assert_close(
            decoded.graph_retrieval_weight,
            original.graph_retrieval_weight,
        );
        assert_eq!(decoded.embedding_cache_size, original.embedding_cache_size);
        assert_eq!(decoded.chunk_max_tokens, original.chunk_max_tokens);
        assert_close(
            decoded.recall_min_confidence,
            original.recall_min_confidence,
        );
        assert_eq!(decoded.pg_max_lifetime_secs, original.pg_max_lifetime_secs);
        assert_eq!(decoded.pg_hnsw_ef_search, original.pg_hnsw_ef_search);
        assert_eq!(
            decoded.working_memory_capacity,
            original.working_memory_capacity
        );
    }
}
