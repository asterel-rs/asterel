//! `PostgreSQL` schema migration runner.
//!
//! Applies numbered SQL migrations from `migrations/` in order. Each
//! migration is guarded by a `current_version < N` check so the runner
//! is safe to call on every startup — already-applied migrations are
//! skipped without touching the database.
//!
//! ## Migration history (v1 – v21)
//!
//! | Version | Change                                               |
//! |---------|------------------------------------------------------|
//! | v1      | Initial schema: `memory_events`, `retrieval_units`, `belief_slots`, `schema_version` |
//! | v2      | Vector column migrated from `vector` → `halfvec` (dimension halving) |
//! | v3      | Score/status guardrails on `retrieval_units`         |
//! | v4      | Access pattern tracking (`access_count`, `accessed_at`) |
//! | v5      | Pinned flag and linear temporal decay fields         |
//! | v6      | Emotion fields on `memory_events`                   |
//! | v7      | `GraphRAG` extension tables (`graph_entities`, `graph_edges`) |
//! | v8      | `GraphRAG` ontology tables                           |
//! | v9      | `GraphRAG` bitemporal validity (`valid_from` / `valid_until`) |
//! | v12     | Adaptive forgetting (`deletion_ledger`)              |
//! | v13     | Edge invalidation index                              |
//! | v14     | Episode/note hierarchy (`parent_graph_entity_id`)    |
//! | v15     | Operator trust state table                           |
//! | v20     | Drop planner/simulation legacy tables                |
//! | v21     | Source lineage for derived memory                     |
//!
//! After all migrations, an HNSW index is created on
//! `retrieval_units.embedding` if non-null rows exist (deferred index
//! creation avoids failure on empty tables).

use sqlx_core::pool::Pool;
use sqlx_core::query::query;
use sqlx_core::row::Row;
use sqlx_postgres::Postgres;

use super::error::{PostgresMemoryError, PostgresMemoryResult};

const MIGRATION_V1_SQL: &str = include_str!("../../../../migrations/001_initial_schema.sql");
const MIGRATION_V2_SQL: &str = include_str!("../../../../migrations/002_halfvec_migration.sql");
const MIGRATION_V3_SQL: &str =
    include_str!("../../../../migrations/003_retrieval_units_guardrails.sql");
const MIGRATION_V4_SQL: &str =
    include_str!("../../../../migrations/004_access_pattern_tracking.sql");
const MIGRATION_V5_SQL: &str =
    include_str!("../../../../migrations/005_pinned_and_temporal_decay.sql");
const MIGRATION_V6_SQL: &str =
    include_str!("../../../../migrations/006_memory_event_emotion_fields.sql");
const MIGRATION_V7_SQL: &str = include_str!("../../../../migrations/007_graphrag_extensions.sql");
const MIGRATION_V8_SQL: &str = include_str!("../../../../migrations/008_graphrag_ontology.sql");
const MIGRATION_V9_SQL: &str = include_str!("../../../../migrations/009_graphrag_bitemporal.sql");
const MIGRATION_V12_SQL: &str = include_str!("../../../../migrations/012_adaptive_forgetting.sql");
const MIGRATION_V13_SQL: &str =
    include_str!("../../../../migrations/013_edge_invalidation_index.sql");
const MIGRATION_V14_SQL: &str =
    include_str!("../../../../migrations/014_episode_note_hierarchy.sql");
const MIGRATION_V15_SQL: &str = include_str!("../../../../migrations/015_operator_trust_state.sql");
const MIGRATION_V20_SQL: &str =
    include_str!("../../../../migrations/020_drop_legacy_planner_simulation_tables.sql");
const MIGRATION_V21_SQL: &str = include_str!("../../../../migrations/021_memory_derivations.sql");

/// Run schema migrations up to the latest version. Idempotent.
///
/// # Errors
/// Returns an error if any migration statement fails.
#[allow(clippy::too_many_lines)]
pub(super) async fn run_migrations(pool: &Pool<Postgres>) -> PostgresMemoryResult<()> {
    let has_schema: bool = query(
        "SELECT EXISTS( \
            SELECT 1 FROM information_schema.tables \
            WHERE table_name = 'schema_version' \
         )",
    )
    .fetch_one(pool)
    .await
    .is_ok_and(|row| row.get::<bool, _>(0));

    let current_version = if has_schema {
        query(
            "SELECT version FROM schema_version \
             ORDER BY migrated_at DESC LIMIT 1",
        )
        .fetch_one(pool)
        .await
        .map_or(0, |row| row.get::<i32, _>(0))
    } else {
        0
    };

    if current_version < 1 {
        run_versioned_migration(pool, 1, "schema", MIGRATION_V1_SQL).await?;
    }

    if current_version < 2 {
        run_versioned_migration(pool, 2, "halfvec", MIGRATION_V2_SQL).await?;
    }

    if current_version < 3 {
        run_versioned_migration(pool, 3, "retrieval guardrails", MIGRATION_V3_SQL).await?;
    }

    if current_version < 4 {
        run_versioned_migration(pool, 4, "access pattern tracking", MIGRATION_V4_SQL).await?;
    }

    if current_version < 5 {
        run_versioned_migration(pool, 5, "pinned/temporal decay", MIGRATION_V5_SQL).await?;
    }

    if current_version < 6 {
        run_versioned_migration(pool, 6, "emotion fields", MIGRATION_V6_SQL).await?;
    }

    if current_version < 7 {
        run_versioned_migration(pool, 7, "GraphRAG extensions", MIGRATION_V7_SQL).await?;
    }

    if current_version < 8 {
        run_versioned_migration(pool, 8, "GraphRAG ontology", MIGRATION_V8_SQL).await?;
    }

    if current_version < 9 {
        run_versioned_migration(pool, 9, "GraphRAG bitemporal", MIGRATION_V9_SQL).await?;
    }

    if current_version < 12 {
        run_versioned_migration(pool, 12, "adaptive forgetting", MIGRATION_V12_SQL).await?;
    }

    if current_version < 13 {
        run_versioned_migration(pool, 13, "edge invalidation", MIGRATION_V13_SQL).await?;
    }

    if current_version < 14 {
        run_versioned_migration(pool, 14, "episode/note hierarchy", MIGRATION_V14_SQL).await?;
    }

    if current_version < 15 {
        run_versioned_migration(pool, 15, "operator trust", MIGRATION_V15_SQL).await?;
    }

    if current_version < 20 {
        run_versioned_migration(
            pool,
            20,
            "legacy planner/simulation cleanup",
            MIGRATION_V20_SQL,
        )
        .await?;
    }

    if current_version < 21 {
        let mut tx = pool.begin().await.map_err(PostgresMemoryError::migration)?;
        query("SELECT pg_advisory_xact_lock($1)")
            .bind(0x4173_7465_726F_6E21_i64)
            .execute(&mut *tx)
            .await
            .map_err(PostgresMemoryError::migration)?;
        for statement in MIGRATION_V21_SQL.split(';').map(str::trim) {
            if statement.is_empty() {
                continue;
            }
            query(statement).execute(&mut *tx).await.map_err(|error| {
                PostgresMemoryError::migration(format!(
                    "v21 memory derivation lineage migration failed: {error}"
                ))
            })?;
        }
        super::integrity::rebuild_memory_event_chain(&mut tx).await?;
        super::integrity::rebuild_deletion_ledger_chain(&mut tx).await?;
        tx.commit().await.map_err(PostgresMemoryError::migration)?;
    }

    // Always attempt HNSW index creation (deferred for empty
    // tables, and needed after v2 drops the old index).
    create_hnsw_index_if_absent(pool).await?;

    Ok(())
}

async fn run_versioned_migration(
    pool: &Pool<Postgres>,
    version: i32,
    label: &str,
    sql: &str,
) -> PostgresMemoryResult<()> {
    let wrapped_sql = format!("BEGIN;\n{sql}\nCOMMIT;");
    sqlx_core::raw_sql::raw_sql(&wrapped_sql)
        .execute(pool)
        .await
        .map_err(|e| {
            PostgresMemoryError::migration(format!("v{version} {label} migration failed: {e}"))
        })?;
    Ok(())
}

/// Create HNSW index on `retrieval_units.embedding` if absent.
///
/// Uses `halfvec_cosine_ops` with tuned parameters (m=24,
/// `ef_construction`=200) for improved recall over defaults.
///
/// # Errors
/// Returns an error if index creation fails.
async fn create_hnsw_index_if_absent(pool: &Pool<Postgres>) -> PostgresMemoryResult<()> {
    let has_index: bool = query(
        "SELECT EXISTS( \
            SELECT 1 FROM pg_indexes \
            WHERE indexname = 'idx_retrieval_units_embedding_hnsw' \
         )",
    )
    .fetch_one(pool)
    .await
    .is_ok_and(|row| row.get::<bool, _>(0));

    if !has_index {
        let has_embeddings: bool = query(
            "SELECT EXISTS(\
                SELECT 1 FROM retrieval_units \
                WHERE embedding IS NOT NULL LIMIT 1\
            )",
        )
        .fetch_one(pool)
        .await
        .is_ok_and(|row| row.get::<bool, _>(0));

        if has_embeddings {
            query(
                "CREATE INDEX CONCURRENTLY IF NOT EXISTS \
                 idx_retrieval_units_embedding_hnsw \
                 ON retrieval_units \
                 USING hnsw (embedding halfvec_cosine_ops) \
                 WITH (m = 24, ef_construction = 200)",
            )
            .execute(pool)
            .await
            .map_err(|e| {
                PostgresMemoryError::migration(format!("HNSW index creation failed: {e}"))
            })?;
        }
    }

    Ok(())
}
