//! `PostgreSQL` persistence for pairwise comparisons and item ratings.
//! Provides the `TasteStore` trait and its `PostgresTasteStore`
//! implementation.

use std::future::Future;
use std::pin::Pin;

use anyhow::Context as _;
use sqlx_core::pool::{Pool, PoolOptions};
use sqlx_core::query::query;
use sqlx_core::row::Row;
use sqlx_postgres::{PgArguments, Postgres};

use super::types::{Domain, PairComparison, TasteContext, TasteOwnerScope, Winner};

/// Rating for an item based on preference comparisons.
#[derive(Clone)]
pub(crate) struct ItemRating {
    /// Owner scope the rating belongs to.
    pub owner: TasteOwnerScope,
    /// Unique identifier of the rated item.
    pub item_id: String,
    /// Domain the rating applies to.
    pub domain: Domain,
    /// Bradley-Terry rating score.
    pub rating: f64,
    /// Number of pairwise comparisons contributing to this rating.
    pub n_comparisons: u32,
    /// ISO 8601 timestamp of the last rating update.
    pub updated_at: String,
}

/// Persistence backend for taste comparisons and item ratings.
pub(crate) trait TasteStore: Send + Sync {
    /// Persist a pairwise comparison record.
    #[allow(dead_code)]
    fn save_comparison<'a>(
        &'a self,
        comparison: &'a PairComparison,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

    /// Retrieve all comparisons involving a given item in a domain.
    fn get_comparisons_for_item<'a>(
        &'a self,
        item_id: &'a str,
        domain: &'a Domain,
        owner: &'a TasteOwnerScope,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<PairComparison>>> + Send + 'a>>;

    /// Look up the current rating for an item in a domain.
    fn get_rating<'a>(
        &'a self,
        item_id: &'a str,
        domain: &'a Domain,
        owner: &'a TasteOwnerScope,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<ItemRating>>> + Send + 'a>>;

    /// Insert or update an item's rating record.
    #[allow(dead_code)]
    fn update_rating(
        &self,
        rating: ItemRating,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>>;

    /// Persist a comparison and all resulting rating updates atomically.
    fn record_comparison_with_ratings<'a>(
        &'a self,
        comparison: &'a PairComparison,
        ratings: Vec<ItemRating>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

    /// Retrieve all ratings for a given domain.
    fn get_all_ratings<'a>(
        &'a self,
        domain: &'a Domain,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<ItemRating>>> + Send + 'a>>;
}

/// PostgreSQL-backed implementation of [`TasteStore`].
pub(crate) struct PostgresTasteStore {
    pool: Pool<Postgres>,
}

impl PostgresTasteStore {
    /// Create a new store, initializing tables if they do not exist.
    ///
    /// # Errors
    ///
    /// Returns an error if `PostgreSQL` connection or table creation fails.
    pub(crate) async fn connect(database_url: &str) -> anyhow::Result<Self> {
        let pool = PoolOptions::<Postgres>::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .context("connect postgres for taste store")?;

        query(
            "CREATE TABLE IF NOT EXISTS taste_comparisons (
                id TEXT PRIMARY KEY,
                owner_scope_key TEXT NOT NULL DEFAULT 'legacy',
                owner_tenant_id TEXT,
                owner_entity_id TEXT,
                owner_session_id TEXT,
                domain TEXT NOT NULL,
                left_id TEXT NOT NULL,
                right_id TEXT NOT NULL,
                winner TEXT NOT NULL,
                rationale TEXT,
                context_json TEXT,
                created_at_ms BIGINT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .context("create taste_comparisons table")?;

        query(
            "CREATE TABLE IF NOT EXISTS taste_ratings (
                item_id TEXT NOT NULL,
                owner_scope_key TEXT NOT NULL DEFAULT 'legacy',
                owner_tenant_id TEXT,
                owner_entity_id TEXT,
                owner_session_id TEXT,
                domain TEXT NOT NULL,
                rating DOUBLE PRECISION NOT NULL,
                n_comparisons BIGINT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY(owner_scope_key, item_id, domain)
            )",
        )
        .execute(&pool)
        .await
        .context("create taste_ratings table")?;

        for statement in [
            "ALTER TABLE taste_comparisons ADD COLUMN IF NOT EXISTS owner_scope_key TEXT NOT NULL DEFAULT 'legacy'",
            "ALTER TABLE taste_comparisons ADD COLUMN IF NOT EXISTS owner_tenant_id TEXT",
            "ALTER TABLE taste_comparisons ADD COLUMN IF NOT EXISTS owner_entity_id TEXT",
            "ALTER TABLE taste_comparisons ADD COLUMN IF NOT EXISTS owner_session_id TEXT",
            "ALTER TABLE taste_ratings ADD COLUMN IF NOT EXISTS owner_scope_key TEXT NOT NULL DEFAULT 'legacy'",
            "ALTER TABLE taste_ratings ADD COLUMN IF NOT EXISTS owner_tenant_id TEXT",
            "ALTER TABLE taste_ratings ADD COLUMN IF NOT EXISTS owner_entity_id TEXT",
            "ALTER TABLE taste_ratings ADD COLUMN IF NOT EXISTS owner_session_id TEXT",
            r"DO $$
            DECLARE
                legacy_constraint RECORD;
            BEGIN
                FOR legacy_constraint IN
                    SELECT c.conname
                    FROM pg_constraint c
                    JOIN pg_class t ON t.oid = c.conrelid
                    JOIN pg_namespace n ON n.oid = t.relnamespace
                    WHERE t.relname = 'taste_ratings'
                      AND n.nspname = current_schema()
                      AND c.contype IN ('p', 'u')
                      AND (
                          SELECT array_agg(a.attname::text ORDER BY keys.ordinality)
                          FROM unnest(c.conkey) WITH ORDINALITY AS keys(attnum, ordinality)
                          JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = keys.attnum
                      ) = ARRAY['item_id', 'domain']
                LOOP
                    EXECUTE format('ALTER TABLE taste_ratings DROP CONSTRAINT %I', legacy_constraint.conname);
                END LOOP;
            END $$",
            "CREATE UNIQUE INDEX IF NOT EXISTS taste_ratings_owner_item_domain_idx ON taste_ratings(owner_scope_key, item_id, domain)",
        ] {
            query(statement)
                .execute(&pool)
                .await
                .with_context(|| format!("ensure taste owner scope schema: {statement}"))?;
        }

        Ok(Self { pool })
    }
}

fn bind_comparison_insert<'q>(
    sql: &'q str,
    id: String,
    comparison: &PairComparison,
    domain: String,
    winner: String,
    context_json: String,
    created_at_ms: i64,
) -> sqlx_core::query::Query<'q, Postgres, PgArguments> {
    query(sql)
        .bind(id)
        .bind(comparison.owner.storage_key())
        .bind(comparison.owner.tenant_id.clone())
        .bind(comparison.owner.entity_id.clone())
        .bind(comparison.owner.session_id.clone())
        .bind(domain)
        .bind(comparison.left_id.clone())
        .bind(comparison.right_id.clone())
        .bind(winner)
        .bind(comparison.rationale.clone())
        .bind(context_json)
        .bind(created_at_ms)
}

fn bind_rating_upsert(
    sql: &str,
    rating: ItemRating,
) -> sqlx_core::query::Query<'_, Postgres, PgArguments> {
    let domain_str = rating.domain.to_string();
    query(sql)
        .bind(rating.item_id)
        .bind(rating.owner.storage_key())
        .bind(rating.owner.tenant_id)
        .bind(rating.owner.entity_id)
        .bind(rating.owner.session_id)
        .bind(domain_str)
        .bind(rating.rating)
        .bind(i64::from(rating.n_comparisons))
        .bind(rating.updated_at)
}

impl TasteStore for PostgresTasteStore {
    fn save_comparison<'a>(
        &'a self,
        comparison: &'a PairComparison,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let id = uuid::Uuid::new_v4().to_string();
            let domain = comparison.domain.to_string();
            let winner = serde_json::to_value(&comparison.winner)?
                .as_str()
                .map(String::from)
                .unwrap_or_default();
            let context_json = serde_json::to_string(&comparison.ctx)?;
            let created_at_ms = i64::try_from(comparison.created_at_ms).unwrap_or(i64::MAX);

            bind_comparison_insert(
                "INSERT INTO taste_comparisons
                 (id, owner_scope_key, owner_tenant_id, owner_entity_id, owner_session_id,
                  domain, left_id, right_id, winner, rationale, context_json, created_at_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                id,
                comparison,
                domain,
                winner,
                context_json,
                created_at_ms,
            )
            .execute(&self.pool)
            .await
            .context("insert taste comparison")?;

            Ok(())
        })
    }

    fn get_comparisons_for_item<'a>(
        &'a self,
        item_id: &'a str,
        domain: &'a Domain,
        owner: &'a TasteOwnerScope,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<PairComparison>>> + Send + 'a>> {
        Box::pin(async move {
            let domain_str = domain.to_string();

            let rows = query(
                "SELECT owner_tenant_id, owner_entity_id, owner_session_id,
                         domain, left_id, right_id, winner, rationale, context_json, created_at_ms
                 FROM taste_comparisons
                 WHERE (left_id = $1 OR right_id = $1) AND domain = $2 AND owner_scope_key = $3",
            )
            .bind(item_id)
            .bind(domain_str)
            .bind(owner.storage_key())
            .fetch_all(&self.pool)
            .await
            .context("query comparisons")?;

            let mut comparisons = Vec::with_capacity(rows.len());
            for row in rows {
                let owner = TasteOwnerScope {
                    tenant_id: row.try_get("owner_tenant_id").ok(),
                    entity_id: row.try_get("owner_entity_id").ok(),
                    session_id: row.try_get("owner_session_id").ok(),
                };
                let domain_s: String = row.get("domain");
                let winner_s: String = row.get("winner");
                let left_id: String = row.get("left_id");
                let right_id: String = row.get("right_id");
                let rationale: Option<String> = row.get("rationale");
                let ctx_json: Option<String> = row.get("context_json");
                let created_ms: i64 = row.get("created_at_ms");

                let domain: Domain = serde_json::from_value(serde_json::Value::String(domain_s))
                    .context("deserialize domain")?;
                let winner: Winner = serde_json::from_value(serde_json::Value::String(winner_s))
                    .context("deserialize winner")?;
                let ctx: TasteContext = ctx_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .context("deserialize context")?
                    .unwrap_or_default();

                comparisons.push(PairComparison {
                    owner,
                    domain,
                    ctx,
                    left_id,
                    right_id,
                    winner,
                    rationale,
                    created_at_ms: u64::try_from(created_ms).unwrap_or(0),
                });
            }

            Ok(comparisons)
        })
    }

    fn get_rating<'a>(
        &'a self,
        item_id: &'a str,
        domain: &'a Domain,
        owner: &'a TasteOwnerScope,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Option<ItemRating>>> + Send + 'a>> {
        Box::pin(async move {
            let domain_str = domain.to_string();
            let row = query(
                "SELECT owner_tenant_id, owner_entity_id, owner_session_id,
                         item_id, domain, rating, n_comparisons, updated_at
                 FROM taste_ratings
                 WHERE item_id = $1 AND domain = $2 AND owner_scope_key = $3",
            )
            .bind(item_id)
            .bind(domain_str)
            .bind(owner.storage_key())
            .fetch_optional(&self.pool)
            .await
            .context("query taste rating from database")?;

            let Some(row) = row else {
                return Ok(None);
            };

            let owner = TasteOwnerScope {
                tenant_id: row.try_get("owner_tenant_id").ok(),
                entity_id: row.try_get("owner_entity_id").ok(),
                session_id: row.try_get("owner_session_id").ok(),
            };
            let item_id: String = row.get("item_id");
            let domain_s: String = row.get("domain");
            let rating: f64 = row.get("rating");
            let n_comp: i64 = row.get("n_comparisons");
            let updated_at: String = row.get("updated_at");
            let domain: Domain = serde_json::from_value(serde_json::Value::String(domain_s))
                .context("deserialize domain")?;

            Ok(Some(ItemRating {
                owner,
                item_id,
                domain,
                rating,
                n_comparisons: u32::try_from(n_comp).context("n_comparisons overflow")?,
                updated_at,
            }))
        })
    }

    fn update_rating(
        &self,
        rating: ItemRating,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + '_>> {
        Box::pin(async move {
            bind_rating_upsert(
                "INSERT INTO taste_ratings
                 (item_id, owner_scope_key, owner_tenant_id, owner_entity_id, owner_session_id,
                  domain, rating, n_comparisons, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT (owner_scope_key, item_id, domain)
                  DO UPDATE SET rating = EXCLUDED.rating,
                                n_comparisons = EXCLUDED.n_comparisons,
                                updated_at = EXCLUDED.updated_at,
                                owner_tenant_id = EXCLUDED.owner_tenant_id,
                                owner_entity_id = EXCLUDED.owner_entity_id,
                                owner_session_id = EXCLUDED.owner_session_id",
                rating,
            )
            .execute(&self.pool)
            .await
            .context("upsert taste rating")?;

            Ok(())
        })
    }

    fn record_comparison_with_ratings<'a>(
        &'a self,
        comparison: &'a PairComparison,
        ratings: Vec<ItemRating>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let id = uuid::Uuid::new_v4().to_string();
            let domain = comparison.domain.to_string();
            let winner = serde_json::to_value(&comparison.winner)?
                .as_str()
                .map(String::from)
                .unwrap_or_default();
            let context_json = serde_json::to_string(&comparison.ctx)?;
            let created_at_ms = i64::try_from(comparison.created_at_ms).unwrap_or(i64::MAX);

            let mut tx = self.pool.begin().await.context("begin taste transaction")?;

            bind_comparison_insert(
                "INSERT INTO taste_comparisons
                 (id, owner_scope_key, owner_tenant_id, owner_entity_id, owner_session_id,
                  domain, left_id, right_id, winner, rationale, context_json, created_at_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                id,
                comparison,
                domain,
                winner,
                context_json,
                created_at_ms,
            )
            .execute(&mut *tx)
            .await
            .context("insert taste comparison in transaction")?;

            for rating in ratings {
                bind_rating_upsert(
                    "INSERT INTO taste_ratings
                     (item_id, owner_scope_key, owner_tenant_id, owner_entity_id, owner_session_id,
                      domain, rating, n_comparisons, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                     ON CONFLICT (owner_scope_key, item_id, domain)
                     DO UPDATE SET rating = EXCLUDED.rating,
                                   n_comparisons = EXCLUDED.n_comparisons,
                                   updated_at = EXCLUDED.updated_at,
                                   owner_tenant_id = EXCLUDED.owner_tenant_id,
                                   owner_entity_id = EXCLUDED.owner_entity_id,
                                   owner_session_id = EXCLUDED.owner_session_id",
                    rating,
                )
                .execute(&mut *tx)
                .await
                .context("upsert taste rating in transaction")?;
            }

            tx.commit().await.context("commit taste transaction")?;
            Ok(())
        })
    }

    fn get_all_ratings<'a>(
        &'a self,
        domain: &'a Domain,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<ItemRating>>> + Send + 'a>> {
        Box::pin(async move {
            let domain_str = domain.to_string();
            let rows = query(
                "SELECT owner_tenant_id, owner_entity_id, owner_session_id,
                         item_id, domain, rating, n_comparisons, updated_at
                 FROM taste_ratings
                 WHERE domain = $1",
            )
            .bind(domain_str)
            .fetch_all(&self.pool)
            .await
            .context("query all ratings")?;

            let mut ratings = Vec::with_capacity(rows.len());
            for row in rows {
                let owner = TasteOwnerScope {
                    tenant_id: row.try_get("owner_tenant_id").ok(),
                    entity_id: row.try_get("owner_entity_id").ok(),
                    session_id: row.try_get("owner_session_id").ok(),
                };
                let item_id: String = row.get("item_id");
                let dom_s: String = row.get("domain");
                let rating: f64 = row.get("rating");
                let n_comp: i64 = row.get("n_comparisons");
                let updated_at: String = row.get("updated_at");
                let domain: Domain = serde_json::from_value(serde_json::Value::String(dom_s))
                    .context("deserialize domain")?;
                ratings.push(ItemRating {
                    owner,
                    item_id,
                    domain,
                    rating,
                    n_comparisons: u32::try_from(n_comp).context("n_comparisons overflow")?,
                    updated_at,
                });
            }

            Ok(ratings)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{Domain, PairComparison, TasteContext, TasteOwnerScope, Winner};
    use super::*;

    async fn fresh_store() -> Option<PostgresTasteStore> {
        let url = std::env::var("TEST_DATABASE_URL")
            .ok()
            .or_else(|| std::env::var("ASTEREL_POSTGRES_URL").ok())?;
        PostgresTasteStore::connect(&url).await.ok()
    }

    fn sample_comparison(domain: Domain, left: &str, right: &str) -> PairComparison {
        PairComparison {
            owner: TasteOwnerScope::new(Some("tenant-test"), "person-test", Some("session-test")),
            domain,
            ctx: TasteContext::default(),
            left_id: left.into(),
            right_id: right.into(),
            winner: Winner::Left,
            rationale: Some("better".into()),
            created_at_ms: 1000,
        }
    }

    #[tokio::test]
    async fn append_only_returns_all_comparisons() {
        let Some(store) = fresh_store().await else {
            return;
        };
        let c1 = sample_comparison(Domain::Text, "a", "b");
        let c2 = sample_comparison(Domain::Text, "a", "c");

        store.save_comparison(&c1).await.unwrap();
        store.save_comparison(&c2).await.unwrap();

        let results = store
            .get_comparisons_for_item("a", &Domain::Text, &c1.owner)
            .await
            .unwrap();
        assert!(results.len() >= 2);
    }

    #[tokio::test]
    async fn rating_upsert_keeps_single_row() {
        let Some(store) = fresh_store().await else {
            return;
        };

        store
            .update_rating(ItemRating {
                owner: TasteOwnerScope::new(
                    Some("tenant-test"),
                    "person-test",
                    Some("session-test"),
                ),
                item_id: "x".into(),
                domain: Domain::Text,
                rating: 1500.0,
                n_comparisons: 5,
                updated_at: "2025-01-01".into(),
            })
            .await
            .unwrap();

        store
            .update_rating(ItemRating {
                owner: TasteOwnerScope::new(
                    Some("tenant-test"),
                    "person-test",
                    Some("session-test"),
                ),
                item_id: "x".into(),
                domain: Domain::Text,
                rating: 1600.0,
                n_comparisons: 10,
                updated_at: "2025-01-02".into(),
            })
            .await
            .unwrap();

        let all = store.get_all_ratings(&Domain::Text).await.unwrap();
        let matching: Vec<_> = all.into_iter().filter(|row| row.item_id == "x").collect();
        assert_eq!(matching.len(), 1);
        assert!((matching[0].rating - 1600.0).abs() < f64::EPSILON);
        assert_eq!(matching[0].n_comparisons, 10);
    }

    #[tokio::test]
    async fn domain_scoping_isolates_ratings() {
        let Some(store) = fresh_store().await else {
            return;
        };

        store
            .update_rating(ItemRating {
                owner: TasteOwnerScope::new(
                    Some("tenant-test"),
                    "person-test",
                    Some("session-test"),
                ),
                item_id: "y".into(),
                domain: Domain::Text,
                rating: 1500.0,
                n_comparisons: 3,
                updated_at: "2025-01-01".into(),
            })
            .await
            .unwrap();

        store
            .update_rating(ItemRating {
                owner: TasteOwnerScope::new(
                    Some("tenant-test"),
                    "person-test",
                    Some("session-test"),
                ),
                item_id: "z".into(),
                domain: Domain::Ui,
                rating: 1400.0,
                n_comparisons: 2,
                updated_at: "2025-01-01".into(),
            })
            .await
            .unwrap();

        let text_ratings = store.get_all_ratings(&Domain::Text).await.unwrap();
        assert!(text_ratings.iter().any(|row| row.item_id == "y"));

        let ui_ratings = store.get_all_ratings(&Domain::Ui).await.unwrap();
        assert!(ui_ratings.iter().any(|row| row.item_id == "z"));
    }
}
