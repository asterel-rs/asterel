//! PostgreSQL-backed usage tracker: records per-request token counts,
//! costs, and latency, with summary queries by time range.

use anyhow::{Context, Result, anyhow};
use sqlx_core::pool::{Pool, PoolOptions};
use sqlx_core::query::query;
use sqlx_core::row::Row;
use sqlx_postgres::Postgres;

use super::types::{UsageRecord, UsageSummary};
use crate::contracts::ids::SessionId;

const USAGE_TRACKER_SCHEMA_LOCK: i64 = 4_159_013;

/// Trait for recording and summarizing token usage and cost data.
pub trait UsageTracker {
    /// # Errors
    /// Returns an error if persisting the usage record fails.
    fn record(&self, record: &UsageRecord) -> Result<()>;
    /// # Errors
    /// Returns an error if summary query or value conversion fails.
    fn summarize(&self, since: Option<&str>) -> Result<UsageSummary>;
}

/// PostgreSQL-backed implementation of [`UsageTracker`].
pub struct PostgresUsageTracker {
    pool: Pool<Postgres>,
    runtime: tokio::runtime::Runtime,
}

impl PostgresUsageTracker {
    /// # Errors
    /// Returns an error if opening `PostgreSQL` or schema initialization fails.
    pub fn new(database_url: &str) -> Result<Self> {
        let runtime = tokio::runtime::Runtime::new().context("create usage runtime")?;
        let pool = runtime.block_on(async {
            PoolOptions::<Postgres>::new()
                .max_connections(5)
                .connect(database_url)
                .await
        })?;

        runtime.block_on(async {
            let mut tx = pool.begin().await?;

            query("SELECT pg_advisory_xact_lock($1)")
                .bind(USAGE_TRACKER_SCHEMA_LOCK)
                .execute(&mut *tx)
                .await
                .context("acquire usage tracker schema advisory lock")?;

            query(
                "CREATE TABLE IF NOT EXISTS usage_records (
                    id TEXT PRIMARY KEY,
                    session_id TEXT,
                    provider TEXT NOT NULL,
                    model TEXT NOT NULL,
                    input_tokens BIGINT,
                    output_tokens BIGINT,
                    estimated_cost_micros BIGINT,
                    created_at TEXT NOT NULL
                )",
            )
            .execute(&mut *tx)
            .await
            .context("ensure usage_records table exists")?;

            tx.commit()
                .await
                .context("commit usage tracker schema init")
        })?;

        Ok(Self { pool, runtime })
    }

    fn i64_to_u64(value: i64, column_name: &'static str) -> Result<u64> {
        u64::try_from(value).map_err(|error| anyhow!("{column_name} conversion failed: {error}"))
    }
}

impl UsageTracker for PostgresUsageTracker {
    fn record(&self, record: &UsageRecord) -> Result<()> {
        let input_tokens = record.input_tokens.map(i64::try_from).transpose()?;
        let output_tokens = record.output_tokens.map(i64::try_from).transpose()?;

        self.runtime.block_on(async {
            query(
                "INSERT INTO usage_records (id, session_id, provider, model, input_tokens, output_tokens, estimated_cost_micros, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(&record.id)
            .bind(record.session_id.as_ref().map(SessionId::as_str))
            .bind(&record.provider)
            .bind(&record.model)
            .bind(input_tokens)
            .bind(output_tokens)
            .bind(record.estimated_cost_micros)
            .bind(&record.created_at)
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(Into::into)
        })
    }

    fn summarize(&self, since: Option<&str>) -> Result<UsageSummary> {
        self.runtime.block_on(async {
            // SUM(bigint) returns NUMERIC in PostgreSQL; cast back to bigint
            // so the row decoder can bind to i64.
            let row = if let Some(since_ts) = since {
                query(
                    "SELECT
                        COALESCE(SUM(input_tokens)::bigint, 0),
                        COALESCE(SUM(output_tokens)::bigint, 0),
                        COALESCE(SUM(estimated_cost_micros)::bigint, 0),
                        COUNT(*)
                     FROM usage_records
                     WHERE created_at >= $1",
                )
                .bind(since_ts)
                .fetch_one(&self.pool)
                .await?
            } else {
                query(
                    "SELECT
                        COALESCE(SUM(input_tokens)::bigint, 0),
                        COALESCE(SUM(output_tokens)::bigint, 0),
                        COALESCE(SUM(estimated_cost_micros)::bigint, 0),
                        COUNT(*)
                     FROM usage_records",
                )
                .fetch_one(&self.pool)
                .await?
            };

            let total_input_tokens = Self::i64_to_u64(row.get::<i64, _>(0), "total_input_tokens")?;
            let total_output_tokens =
                Self::i64_to_u64(row.get::<i64, _>(1), "total_output_tokens")?;
            let record_count = Self::i64_to_u64(row.get::<i64, _>(3), "record_count")?;

            Ok(UsageSummary {
                total_input_tokens,
                total_output_tokens,
                total_estimated_cost_micros: row.get::<i64, _>(2),
                record_count,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{PostgresUsageTracker, UsageTracker};
    use crate::contracts::ids::SessionId;
    use crate::runtime::usage::types::{ModelPricing, UsageRecord};

    fn sample_record(
        id: &str,
        created_at: &str,
        input: u64,
        output: u64,
        cost: i64,
    ) -> UsageRecord {
        UsageRecord {
            id: id.to_string(),
            session_id: Some(SessionId::new("session-1")),
            provider: "openrouter".to_string(),
            model: "anthropic/claude-sonnet-4.6".to_string(),
            input_tokens: Some(input),
            output_tokens: Some(output),
            estimated_cost_micros: Some(cost),
            created_at: created_at.to_string(),
        }
    }

    fn test_db_url() -> Option<String> {
        std::env::var("TEST_DATABASE_URL").ok()
    }

    #[test]
    fn create_tracker_with_postgres_succeeds() {
        let Some(url) = test_db_url() else { return };
        let tracker = PostgresUsageTracker::new(&url);
        assert!(tracker.is_ok());
    }

    #[test]
    fn record_usage_entry_and_summarize() {
        let Some(url) = test_db_url() else { return };
        let tracker = PostgresUsageTracker::new(&url).unwrap();

        tracker
            .record(&sample_record(
                "id-1",
                "2026-02-20T10:00:00Z",
                120,
                80,
                1_000,
            ))
            .unwrap();

        let summary = tracker.summarize(None).unwrap();
        assert!(summary.total_input_tokens >= 120);
        assert!(summary.total_output_tokens >= 80);
        assert!(summary.total_estimated_cost_micros >= 1_000);
        assert!(summary.record_count >= 1);
    }

    #[test]
    fn pricing_estimation_returns_expected_micros() {
        let pricing = ModelPricing {
            model_pattern: "test".to_string(),
            input_cost_per_million: 2.5,
            output_cost_per_million: 10.0,
        };

        let estimated = pricing.estimate_cost_micros(100_000, 50_000);
        assert_eq!(estimated, 750_000);
    }
}
