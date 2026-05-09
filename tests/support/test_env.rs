#![allow(unsafe_code)]
//! Shared integration-test environment variable helpers.
//!
//! Rust 2024 marks process-wide env mutation as unsafe. This module
//! centralizes those calls behind a global lock to avoid concurrent
//! mutation races across integration tests.

use std::sync::{LazyLock, Mutex, MutexGuard};
use std::{fs, path::Path};

static TEST_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// RAII guard that serializes env mutation and restores previous values.
pub struct ScopedEnvVar {
    _lock: MutexGuard<'static, ()>,
    key: &'static str,
    previous: Option<String>,
    changed: bool,
}

impl ScopedEnvVar {
    /// Set `key=value` for the rest of the guard lifetime.
    #[allow(dead_code)]
    pub fn set(key: &'static str, value: &str) -> Self {
        let lock = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = std::env::var(key).ok();
        // SAFETY: Serialized by TEST_ENV_LOCK for the whole guard lifetime.
        unsafe {
            std::env::set_var(key, value);
        }
        Self {
            _lock: lock,
            key,
            previous,
            changed: true,
        }
    }

    /// Ensure `primary` is present, falling back to `fallback` when absent.
    /// Returns `None` when neither variable contains a non-empty value.
    #[allow(dead_code)]
    pub fn ensure_primary_from_fallback(
        primary: &'static str,
        fallback: &'static str,
    ) -> Option<Self> {
        let lock = TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let previous = std::env::var(primary).ok();
        if previous
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Some(Self {
                _lock: lock,
                key: primary,
                previous,
                changed: false,
            });
        }

        let fallback_value = std::env::var(fallback).ok()?;
        let normalized = fallback_value.trim();
        if normalized.is_empty() {
            return None;
        }

        // SAFETY: Serialized by TEST_ENV_LOCK for the whole guard lifetime.
        unsafe {
            std::env::set_var(primary, normalized);
        }
        Some(Self {
            _lock: lock,
            key: primary,
            previous,
            changed: true,
        })
    }

    /// Like [`ensure_primary_from_fallback`] for Postgres, but panics when
    /// neither variable is set so that `#[ignore]` tests run with
    /// `--include-ignored` fail loudly instead of silently passing.
    #[allow(dead_code)]
    pub fn require_postgres_url() -> Self {
        Self::ensure_primary_from_fallback("ASTEREL_POSTGRES_URL", "TEST_DATABASE_URL")
            .unwrap_or_else(|| {
                panic!("TEST_DATABASE_URL or ASTEREL_POSTGRES_URL must be set to run this test")
            })
    }
}

#[allow(dead_code)]
pub fn postgres_url() -> Option<String> {
    std::env::var("TEST_DATABASE_URL")
        .ok()
        .or_else(|| std::env::var("ASTEREL_POSTGRES_URL").ok())
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

#[allow(dead_code)]
pub fn write_workspace_postgres_config(
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

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        if !self.changed {
            return;
        }

        if let Some(previous) = &self.previous {
            // SAFETY: Serialized by TEST_ENV_LOCK for the whole guard lifetime.
            unsafe {
                std::env::set_var(self.key, previous);
            }
        } else {
            // SAFETY: Serialized by TEST_ENV_LOCK for the whole guard lifetime.
            unsafe {
                std::env::remove_var(self.key);
            }
        }
    }
}
