#![allow(unsafe_code)]
//! Shared test-only environment variable helpers.
//!
//! Rust 2024 marks process environment mutation as `unsafe`, so tests that
//! touch env vars must serialize access across the whole process.

use std::sync::{LazyLock, Mutex, MutexGuard};
use std::{fs, path::Path};

static TEST_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// Guard that serializes env access and restores the previous value on drop
/// when this guard changed the variable.
pub(crate) struct EnvVarGuard {
    _lock: MutexGuard<'static, ()>,
    key: &'static str,
    previous: Option<String>,
    changed: bool,
}

/// Acquire [`TEST_ENV_LOCK`] recovering from poisoning. A test that panics
/// while holding an env guard will poison this mutex; under normal `std::sync`
/// semantics subsequent `.lock()` calls fail permanently. Tests must keep
/// running across panics (each test is independent), so we unwrap the inner
/// guard and continue.
fn lock_env() -> MutexGuard<'static, ()> {
    match TEST_ENV_LOCK.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl EnvVarGuard {
    /// Temporarily set an environment variable for the duration of the guard.
    pub(crate) fn set(key: &'static str, value: impl AsRef<str>) -> Option<Self> {
        let lock = lock_env();
        let previous = std::env::var(key).ok();
        let normalized = value.as_ref().trim();
        // SAFETY: This helper holds a global mutex that serializes all env
        // mutation across tests.
        unsafe {
            std::env::set_var(key, normalized);
        }
        Some(Self {
            _lock: lock,
            key,
            previous,
            changed: true,
        })
    }

    /// Temporarily unset an environment variable for the duration of the guard.
    pub(crate) fn unset(key: &'static str) -> Option<Self> {
        let lock = lock_env();
        let previous = std::env::var(key).ok();
        // SAFETY: This helper holds a global mutex that serializes all env
        // mutation across tests.
        unsafe {
            std::env::remove_var(key);
        }
        Some(Self {
            _lock: lock,
            key,
            previous,
            changed: true,
        })
    }

    /// Ensure `ASTEREL_POSTGRES_URL` is available for the test by using
    /// `TEST_DATABASE_URL` as fallback when needed.
    pub(crate) fn ensure_postgres_url() -> Option<Self> {
        let lock = lock_env();
        let current = std::env::var("ASTEREL_POSTGRES_URL").ok();
        if current
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Some(Self {
                _lock: lock,
                key: "ASTEREL_POSTGRES_URL",
                previous: current,
                changed: false,
            });
        }

        let fallback = std::env::var("TEST_DATABASE_URL").ok()?;
        let normalized = fallback.trim();
        if normalized.is_empty() {
            return None;
        }

        // SAFETY: This helper holds a global mutex that serializes all env
        // mutation across tests.
        unsafe {
            std::env::set_var("ASTEREL_POSTGRES_URL", normalized);
        }
        Some(Self {
            _lock: lock,
            key: "ASTEREL_POSTGRES_URL",
            previous: current,
            changed: true,
        })
    }

    /// Like [`ensure_postgres_url`] but panics when neither variable is set,
    /// so that `#[ignore]` tests run with `--include-ignored` fail loudly
    /// instead of silently passing when Postgres is unavailable.
    pub(crate) fn require_postgres_url() -> Self {
        Self::ensure_postgres_url().unwrap_or_else(|| {
            panic!("TEST_DATABASE_URL or ASTEREL_POSTGRES_URL must be set to run this test")
        })
    }
}

pub(crate) fn postgres_url() -> Option<String> {
    std::env::var("TEST_DATABASE_URL")
        .ok()
        .or_else(|| std::env::var("ASTEREL_POSTGRES_URL").ok())
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

/// Global mutex that serialises access to the shared test Postgres database.
///
/// Multiple tests in the lib test binary share a single Postgres database
/// (via `ASTEREL_POSTGRES_URL` / `TEST_DATABASE_URL`). Many of them create
/// rows under fixed keys (e.g. `user-1`, `conversation::discord::channel-1`)
/// and would otherwise step on each other. This mutex guarantees that at most
/// one postgres-backed test runs at a time so each test can TRUNCATE the
/// shared database to a clean state during its own setup without racing
/// another test that has already started.
///
/// The returned guard must be held for the entire test body. Drop it at the
/// end of the test (happens automatically when it goes out of scope).
#[cfg(feature = "postgres")]
static TEST_DB_LOCK: std::sync::LazyLock<std::sync::Arc<tokio::sync::Mutex<()>>> =
    std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Mutex::new(())));

/// Guard returned by [`acquire_test_db`] that holds the global postgres test
/// mutex for the duration of the test. Drop to release.
///
/// Note: we deliberately do NOT hold `TEST_ENV_LOCK` across the test body
/// because the sync mutex across async await points caused a runtime
/// slowdown / potential deadlock (tokio workers piling up on the sync
/// lock). Instead, the admin_scheduler test that needs to unset
/// `ASTEREL_POSTGRES_URL` also acquires [`TEST_DB_LOCK`] via
/// [`acquire_test_db_lock_only`] so it can't run concurrently with a
/// postgres test.
#[cfg(feature = "postgres")]
#[must_use]
pub(crate) struct TestDbGuard {
    _inner: tokio::sync::OwnedMutexGuard<()>,
}

/// Acquire exclusive access to the shared test Postgres database and
/// truncate every table that stores test-visible data. Returns a guard that
/// must be held for the entire test body.
///
/// # Panics
///
/// Panics if `ASTEREL_POSTGRES_URL` / `TEST_DATABASE_URL` is not set or
/// the truncate SQL fails. Tests that call this helper are ignored by default
/// so CI only runs them when the env var is explicitly provided.
/// Shared multi-thread runtime used by [`acquire_test_db_blocking`]. Built
/// once for the test binary process so that tokio `Mutex` wakers registered
/// via this runtime remain valid across all sync test invocations.
#[cfg(feature = "postgres")]
static TEST_BLOCKING_RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> =
    std::sync::LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build shared test blocking runtime")
    });

/// Blocking variant of [`acquire_test_db`] for `#[test]` (non-async) tests.
/// Uses a shared multi-thread runtime so tokio `Mutex` state stays consistent
/// across parallel sync-test invocations.
///
/// # Panics
///
/// See [`acquire_test_db`].
#[cfg(feature = "postgres")]
pub(crate) fn acquire_test_db_blocking() -> TestDbGuard {
    TEST_BLOCKING_RUNTIME.block_on(acquire_test_db())
}

#[cfg(feature = "postgres")]
pub(crate) async fn acquire_test_db() -> TestDbGuard {
    use sqlx_core::executor::Executor;
    use sqlx_core::pool::PoolOptions;
    use sqlx_core::query::query;
    use sqlx_postgres::Postgres;

    let guard = TEST_DB_LOCK.clone().lock_owned().await;

    let url = postgres_url().expect(
        "ASTEREL_POSTGRES_URL or TEST_DATABASE_URL must be set to run postgres-gated tests",
    );

    let pool = PoolOptions::<Postgres>::new()
        .max_connections(1)
        .connect(&url)
        .await
        .expect("connect to postgres for test reset");

    // TRUNCATE every table in the public schema EXCEPT schema_version.
    // The migration runner consults schema_version to decide whether
    // migrations have already been applied — clearing it would force
    // migrations to re-run on the next connect, and v1 (which uses plain
    // `CREATE INDEX` without IF NOT EXISTS) would fail because the
    // objects still exist. Preserving schema_version keeps the DB in a
    // consistent "already migrated" state across tests while still
    // giving each test fresh data tables.
    let truncate_sql = r"
        DO $$
        DECLARE
            t TEXT;
        BEGIN
            FOR t IN
                SELECT tablename FROM pg_tables
                WHERE schemaname = 'public'
                  AND tablename <> 'schema_version'
            LOOP
                EXECUTE format('TRUNCATE TABLE %I RESTART IDENTITY CASCADE', t);
            END LOOP;
        END
        $$;
    ";
    (&pool)
        .execute(query(truncate_sql))
        .await
        .expect("truncate test DB");
    pool.close().await;

    TestDbGuard { _inner: guard }
}

/// Acquire the global test postgres mutex WITHOUT connecting to postgres.
/// Used by tests that want to be mutually exclusive with postgres-backed
/// tests but don't themselves talk to postgres (e.g. the
/// `admin_scheduler_mutations_are_truthful_without_postgres` test which
/// unsets `ASTEREL_POSTGRES_URL`).
#[cfg(feature = "postgres")]
#[allow(dead_code)]
pub(crate) async fn acquire_test_db_lock_only() -> TestDbGuard {
    let guard = TEST_DB_LOCK.clone().lock_owned().await;
    TestDbGuard { _inner: guard }
}

/// Blocking variant of [`acquire_test_db_lock_only`] for `#[test]` tests.
#[cfg(feature = "postgres")]
#[allow(dead_code)]
pub(crate) fn acquire_test_db_lock_only_blocking() -> TestDbGuard {
    let guard = TEST_BLOCKING_RUNTIME.block_on(TEST_DB_LOCK.clone().lock_owned());
    TestDbGuard { _inner: guard }
}

/// Drop a value that owns a nested `tokio::runtime::Runtime` from within an
/// async test context.
///
/// Several of our legacy store types (`MediaStore`, etc.) wrap a dedicated
/// `Runtime` so their sync methods can run async sqlx calls via
/// `block_on_sync`. When such a store is constructed and dropped inside a
/// `#[tokio::test]` body, the nested runtime drop panics with
/// "Cannot drop a runtime in a context where blocking is not allowed"
/// because async code is not allowed to block the current worker.
///
/// Wrapping the drop in `tokio::task::block_in_place` releases the current
/// worker before the drop runs, making blocking (and therefore the nested
/// runtime teardown) legal. Requires a multi-thread tokio runtime; the
/// `TestDbGuard` path ensures that.
#[cfg(feature = "postgres")]
#[allow(dead_code)]
pub(crate) fn drop_in_blocking<T>(value: T) {
    tokio::task::block_in_place(move || drop(value));
}

pub(crate) fn write_workspace_postgres_config(
    workspace_dir: &Path,
    postgres_url: &str,
) -> std::io::Result<()> {
    fs::create_dir_all(workspace_dir)?;
    let config_path = workspace_dir
        .parent()
        .unwrap_or(workspace_dir)
        .join("config.toml");
    fs::write(
        config_path,
        format!("[memory]\npostgres_url = {postgres_url:?}\n"),
    )
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if !self.changed {
            return;
        }

        if let Some(previous) = &self.previous {
            // SAFETY: Serialized by TEST_ENV_LOCK held for the whole guard lifetime.
            unsafe {
                std::env::set_var(self.key, previous);
            }
        } else {
            // SAFETY: Serialized by TEST_ENV_LOCK held for the whole guard lifetime.
            unsafe {
                std::env::remove_var(self.key);
            }
        }
    }
}
