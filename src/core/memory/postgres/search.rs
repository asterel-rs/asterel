//! Full-text and vector similarity search for the `PostgreSQL` backend.
//!
//! Provides two complementary search paths, both scoped to a single `entity_id`
//! and `node_tier`:
//!
//! - **`fts_search_scoped`** — combines `tsvector` ranking
//!   (`ts_rank` / `plainto_tsquery`) with `pg_trgm` substring similarity.
//!   Filters to `promoted` or `candidate` retrieval units only.
//! - **`vector_search_scoped`** — cosine distance via the `pgvector` HNSW
//!   index (`halfvec` operator class); returns `1 − distance` as similarity.
//!
//! The recall pipeline in `repository_recall` calls both functions and fuses
//! the ranked lists using Reciprocal Rank Fusion (RRF, k=60) before applying
//! the 6-phase metadata scoring pass.

use pgvector::HalfVector;
use sqlx_core::query::query;
use sqlx_core::row::Row;

use super::PostgresMemory;
use super::error::{PostgresMemoryError, PostgresMemoryResult, PostgresMemoryResultExt};

impl PostgresMemory {
    /// Full-text search using `tsvector` + `pg_trgm` on `retrieval_units`.
    ///
    /// Returns (`unit_id`, score) pairs ordered by relevance.
    pub(super) async fn fts_search_scoped(
        &self,
        entity_id: &str,
        query_text: &str,
        node_tier: &str,
        limit: usize,
        layer_filter: Option<&str>,
    ) -> PostgresMemoryResult<Vec<(String, f32)>> {
        if query_text.trim().is_empty() {
            return Ok(Vec::new());
        }

        let limit_i64 = search_limit_to_i64(limit)?;

        // Use plainto_tsquery to safely handle arbitrary user input (no manual escaping needed).
        // Combine tsvector ranking with pg_trgm similarity for substring matching.
        let rows = query(
            "SELECT unit_id, \
                    (ts_rank(fts_document, plainto_tsquery('simple', $1)) + \
                     similarity(content, $2)) AS score \
              FROM retrieval_units \
              WHERE entity_id = $3 \
                AND visibility != 'secret' \
                AND ($5::text IS NULL OR layer = $5) \
                AND promotion_status IN ('promoted', 'candidate') \
                AND EXISTS ( \
                    SELECT 1 FROM graph_entities ge \
                    WHERE ge.graph_entity_id = ('slot::' || retrieval_units.unit_id) \
                      AND ge.owner_entity_id = retrieval_units.entity_id \
                      AND ge.node_tier = $4 \
                ) \
                AND (fts_document @@ plainto_tsquery('simple', $1) OR similarity(content, $2) > 0.1) \
              ORDER BY score DESC \
              LIMIT $6",
        )
        .bind(query_text)
        .bind(query_text)
        .bind(entity_id)
        .bind(node_tier)
        .bind(layer_filter)
        .bind(limit_i64)
        .fetch_all(&self.pool)
        .await
        .pg_query("fts search scoped")?;

        let results = rows
            .iter()
            .map(|row| {
                let unit_id: String = row.get("unit_id");
                let score: f32 = row.get("score");
                (unit_id, score)
            })
            .collect();

        Ok(results)
    }

    /// Vector similarity search using pgvector HNSW index.
    ///
    /// Returns (`unit_id`, `cosine_similarity`) pairs ordered by similarity.
    pub(super) async fn vector_search_scoped(
        &self,
        entity_id: &str,
        query_embedding: &[f32],
        node_tier: &str,
        limit: usize,
        layer_filter: Option<&str>,
    ) -> PostgresMemoryResult<Vec<(String, f32)>> {
        if query_embedding.is_empty() {
            return Ok(Vec::new());
        }

        let limit_i64 = search_limit_to_i64(limit)?;
        let query_vec = HalfVector::from_f32_slice(query_embedding);

        // pgvector's <=> operator returns distance (0 = identical), convert to similarity
        let rows = query(
            "SELECT unit_id, 1.0 - (embedding <=> $1) AS similarity \
              FROM retrieval_units \
              WHERE entity_id = $2 \
                AND visibility != 'secret' \
                AND ($4::text IS NULL OR layer = $4) \
                AND promotion_status IN ('promoted', 'candidate') \
                AND EXISTS ( \
                    SELECT 1 FROM graph_entities ge \
                    WHERE ge.graph_entity_id = ('slot::' || retrieval_units.unit_id) \
                      AND ge.owner_entity_id = retrieval_units.entity_id \
                      AND ge.node_tier = $3 \
                ) \
                AND embedding IS NOT NULL \
              ORDER BY embedding <=> $1 \
              LIMIT $5",
        )
        .bind(query_vec)
        .bind(entity_id)
        .bind(node_tier)
        .bind(layer_filter)
        .bind(limit_i64)
        .fetch_all(&self.pool)
        .await
        .pg_query("vector search scoped")?;

        let results = rows
            .iter()
            .filter_map(|row| {
                let unit_id: String = row.get("unit_id");
                let similarity: f32 = row.get("similarity");
                if similarity > 0.0 {
                    Some((unit_id, similarity))
                } else {
                    None
                }
            })
            .collect();

        Ok(results)
    }
}

fn search_limit_to_i64(limit: usize) -> PostgresMemoryResult<i64> {
    i64::try_from(limit)
        .map_err(|_| PostgresMemoryError::conversion("search limit exceeded i64 range"))
}
