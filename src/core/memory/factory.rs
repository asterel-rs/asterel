//! Memory backend factory.
//!
//! Selects and constructs the concrete [`Memory`] implementation
//! (`Postgres`, `Markdown`, or `None` → `Markdown`) from the user's
//! [`MemoryConfig`].
//!
//! ## Backend selection
//!
//! | Config variant          | Backend produced         | Gate                         |
//! |-------------------------|--------------------------|------------------------------|
//! | `MemoryBackend::Postgres` | [`PostgresMemory`]     | `postgres` feature flag      |
//! | `MemoryBackend::Markdown` | [`MarkdownMemory`]     | always available             |
//! | `MemoryBackend::None`     | [`MarkdownMemory`]     | always available (compatibility fallback; still persists as Markdown)|
//!
//! After constructing the backend, [`hygiene::run_if_due`] is called to
//! perform scheduled maintenance (archival, pruning, sleep consolidation).
//! Hygiene failures are non-fatal — a warning is logged and the backend is
//! returned regardless.

use std::path::Path;
use std::sync::Arc;

#[cfg(feature = "postgres")]
use super::PostgresMemory;
use super::{MarkdownMemory, Memory, MemoryEvent, MemoryInferenceEvent, embeddings, hygiene};
use crate::config::MemoryConfig;

/// # Errors
///
/// Returns an error when the configured backend is unknown or when backend
/// initialization fails.
pub async fn create_memory(
    config: &MemoryConfig,
    workspace_dir: &Path,
    api_key: Option<&str>,
) -> anyhow::Result<Box<dyn Memory>> {
    let memory: Box<dyn Memory> = match config.backend {
        #[cfg(feature = "postgres")]
        crate::config::MemoryBackend::Postgres => {
            let database_url = config
                .postgres_url
                .clone()
                .or_else(|| std::env::var("ASTEREL_POSTGRES_URL").ok())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "postgres backend requires `postgres_url` in config \
                         or ASTEREL_POSTGRES_URL env var"
                    )
                })?;

            let embedder: Arc<dyn embeddings::EmbeddingProvider> =
                Arc::from(embeddings::create_embedding_provider(
                    &config.embedding_provider,
                    api_key,
                    &config.embedding_model,
                    config.embedding_dimensions,
                )?);

            let mem = PostgresMemory::connect_with_options(
                &database_url,
                embedder,
                super::postgres::PostgresConnectOptions {
                    cache_max: config.embedding_cache_size,
                    graph_retrieval_fusion_enabled: config.graph_retrieval_fusion_enabled,
                    graph_retrieval_weight: config.graph_retrieval_weight,
                    max_connections: config.pg_max_connections,
                    min_connections: config.pg_min_connections,
                    connect_timeout: std::time::Duration::from_secs(config.pg_connect_timeout_secs),
                    idle_timeout: std::time::Duration::from_secs(config.pg_idle_timeout_secs),
                    vector_weight: config.vector_weight,
                    keyword_weight: config.keyword_weight,
                    max_lifetime: std::time::Duration::from_secs(config.pg_max_lifetime_secs),
                    hnsw_ef_search: config.pg_hnsw_ef_search,
                },
            )
            .await?;
            Box::new(mem)
        }
        crate::config::MemoryBackend::Markdown | crate::config::MemoryBackend::None => {
            Box::new(MarkdownMemory::new(workspace_dir))
        }
        #[cfg(not(feature = "postgres"))]
        crate::config::MemoryBackend::Postgres => {
            anyhow::bail!("postgres backend requires the 'postgres' feature flag");
        }
    };

    if let Err(e) = hygiene::run_if_due(config, workspace_dir) {
        tracing::warn!("memory hygiene skipped: {e}");
    }

    Ok(memory)
}

/// # Errors
///
/// Returns an error when the memory backend fails to append inference events.
pub async fn persist_inference_events(
    memory: &dyn Memory,
    events: Vec<MemoryInferenceEvent>,
) -> anyhow::Result<Vec<MemoryEvent>> {
    memory
        .append_inference_events(events)
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn factory_markdown() {
        let tmp = TempDir::new().unwrap();
        let cfg = MemoryConfig {
            backend: crate::config::MemoryBackend::Markdown,
            ..MemoryConfig::default()
        };
        let mem = create_memory(&cfg, tmp.path(), None).await.unwrap();
        assert_eq!(mem.name(), "markdown");
    }

    #[tokio::test]
    async fn factory_none_falls_back_to_markdown() {
        let tmp = TempDir::new().unwrap();
        let cfg = MemoryConfig {
            backend: crate::config::MemoryBackend::None,
            ..MemoryConfig::default()
        };
        let mem = create_memory(&cfg, tmp.path(), None).await.unwrap();
        assert_eq!(mem.name(), "markdown");
    }

    #[tokio::test]
    async fn memory_hygiene_failure_nonfatal() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("state"), "not-json").unwrap();

        let cfg = MemoryConfig {
            backend: crate::config::MemoryBackend::Markdown,
            ..MemoryConfig::default()
        };

        let mem = create_memory(&cfg, tmp.path(), None).await.unwrap();
        assert_eq!(mem.name(), "markdown");
    }
}
