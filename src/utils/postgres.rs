//! Shared `PostgreSQL` helpers for modules that still expose synchronous APIs.

use std::fs;
use std::future::Future;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct PartialConfig {
    memory: Option<PartialMemoryConfig>,
}

#[derive(Debug, Deserialize)]
struct PartialMemoryConfig {
    postgres_url: Option<String>,
}

/// Resolve `PostgreSQL` URL from explicit config, env, or workspace-local config.toml.
#[must_use]
pub(crate) fn resolve_postgres_url(
    explicit_url: Option<&str>,
    workspace_dir: Option<&Path>,
) -> Option<String> {
    if let Some(url) = explicit_url.map(str::trim).filter(|url| !url.is_empty()) {
        return Some(url.to_string());
    }

    if let Ok(url) = std::env::var("ASTEREL_POSTGRES_URL") {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let workspace_dir = workspace_dir?;
    let config_path = workspace_dir
        .parent()
        .map(|base| base.join("config.toml"))
        .filter(|path| path.exists())?;
    let raw = fs::read_to_string(&config_path).ok()?;
    let parsed: PartialConfig = toml::from_str(&raw).ok()?;
    parsed
        .memory
        .and_then(|memory| memory.postgres_url)
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

/// Resolve `PostgreSQL` URL or return a contextual error.
pub(crate) fn require_postgres_url(
    explicit_url: Option<&str>,
    workspace_dir: Option<&Path>,
    context_label: &str,
) -> Result<String> {
    resolve_postgres_url(explicit_url, workspace_dir).with_context(|| {
        format!(
            "{context_label} requires PostgreSQL URL: set memory.postgres_url or ASTEREL_POSTGRES_URL"
        )
    })
}

/// Execute an async sqlx future from synchronous code safely even when already on a Tokio runtime.
pub(crate) fn block_on_sync<F>(runtime: &tokio::runtime::Runtime, future: F) -> F::Output
where
    F: Future,
{
    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::block_in_place(|| runtime.block_on(future))
    } else {
        runtime.block_on(future)
    }
}
