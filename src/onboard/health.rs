//! Post-onboard health verification.

use std::time::Duration;

use tokio::io::AsyncWriteExt as _;

use crate::config::{Config, MemoryBackend};

/// Result of a single health check step.
pub(crate) enum HealthStatus {
    /// Check passed. Contains a human-readable description.
    Pass(String),
    /// Check failed. Contains the failure reason.
    Fail(String),
    /// Check was not applicable and was skipped.
    Skip(String),
}

/// Aggregated results from all post-onboard health checks.
pub(crate) struct HealthCheckResult {
    /// Whether the configured provider API key is reachable.
    pub api_connectivity: HealthStatus,
    /// Whether the workspace directory is writable.
    pub workspace_writable: HealthStatus,
    /// Whether the memory backend is accessible.
    pub memory_backend: HealthStatus,
}

/// Run all post-onboard health checks against the given configuration.
///
/// This is an async, read-mostly operation. The workspace check writes and
/// immediately removes a `.healthcheck` sentinel file to confirm writability.
pub(crate) async fn run_health_checks(config: &Config) -> HealthCheckResult {
    let api_connectivity = check_api_connectivity(config).await;
    let workspace_writable = check_workspace_writable(config).await;
    let memory_backend = check_memory_backend(config).await;

    HealthCheckResult {
        api_connectivity,
        workspace_writable,
        memory_backend,
    }
}

/// Verify API connectivity using the configured provider and key.
async fn check_api_connectivity(config: &Config) -> HealthStatus {
    let provider = config
        .default_provider
        .as_deref()
        .unwrap_or(crate::config::DEFAULT_PROVIDER);

    let api_key = config.api_key.as_deref().unwrap_or("");

    match super::api_verify::verify_api_key(provider, api_key).await {
        Ok(super::api_verify::VerifyResult::Valid { detail }) => HealthStatus::Pass(detail),
        Ok(super::api_verify::VerifyResult::Invalid { reason }) => HealthStatus::Fail(reason),
        Ok(super::api_verify::VerifyResult::Skipped) => {
            HealthStatus::Skip("no API key configured".to_string())
        }
        Err(e) => HealthStatus::Fail(format!("verification error: {e}")),
    }
}

/// Confirm the workspace directory is writable by creating and removing a
/// temporary sentinel file.
async fn check_workspace_writable(config: &Config) -> HealthStatus {
    let sentinel = config.workspace_dir.join(".healthcheck");

    // Write sentinel file.
    let write_result = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&sentinel)
        .await;

    match write_result {
        Err(e) => {
            return HealthStatus::Fail(format!(
                "workspace not writable ({}): {e}",
                config.workspace_dir.display()
            ));
        }
        Ok(mut f) => {
            // Write a byte so the file is non-empty; ignore write errors here.
            let _ = f.write_all(b"ok").await;
        }
    }

    // Clean up sentinel.
    let _ = tokio::fs::remove_file(&sentinel).await;

    HealthStatus::Pass(format!(
        "workspace writable ({})",
        config.workspace_dir.display()
    ))
}

/// Verify the memory backend is accessible.
async fn check_memory_backend(config: &Config) -> HealthStatus {
    match config.memory.backend {
        MemoryBackend::Postgres => check_postgres_connectivity(config).await,
        MemoryBackend::Markdown => check_markdown_dir(config).await,
        MemoryBackend::None => HealthStatus::Skip("memory backend is none".to_string()),
    }
}

/// Probe the Postgres host:port with a raw TCP connect (avoids pulling in sqlx
/// just for a health check).
async fn check_postgres_connectivity(config: &Config) -> HealthStatus {
    let url = match config.memory.postgres_url.as_deref() {
        Some(u) if !u.is_empty() => u,
        _ => {
            // Try the ASTEREL_POSTGRES_URL env var as a fallback.
            return match std::env::var("ASTEREL_POSTGRES_URL") {
                Ok(env_url) if !env_url.is_empty() => probe_postgres_tcp(&env_url).await,
                _ => HealthStatus::Skip("no postgres_url configured".to_string()),
            };
        }
    };

    probe_postgres_tcp(url).await
}

/// Parse host:port from a postgres URL and attempt a 3-second TCP connect.
async fn probe_postgres_tcp(url: &str) -> HealthStatus {
    let Some(addr) = extract_postgres_host_port(url) else {
        return HealthStatus::Fail("could not parse host:port from postgres URL".to_string());
    };

    let connect_future = tokio::net::TcpStream::connect(&addr);
    match tokio::time::timeout(Duration::from_secs(3), connect_future).await {
        Ok(Ok(_)) => HealthStatus::Pass(format!("postgres reachable at {addr}")),
        Ok(Err(e)) => HealthStatus::Fail(format!("postgres TCP connect failed ({addr}): {e}")),
        Err(_) => HealthStatus::Fail(format!("postgres TCP connect timed out after 3s ({addr})")),
    }
}

/// Extract `"host:port"` from a postgres URL string.
///
/// Handles `postgres://user:pass@host:port/db` and
/// `postgresql://user:pass@host:port/db`.
fn extract_postgres_host_port(url: &str) -> Option<String> {
    // Strip scheme prefix.
    let rest = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))?;

    // Drop everything after the first `/` (the database path).
    let authority = rest.split('/').next()?;

    // Drop user:pass@ prefix if present.
    let host_port = if let Some(at_pos) = authority.rfind('@') {
        &authority[at_pos + 1..]
    } else {
        authority
    };

    // If no explicit port, default to 5432.
    if host_port.is_empty() {
        return None;
    }

    if host_port.contains(':') {
        Some(host_port.to_string())
    } else {
        Some(format!("{host_port}:5432"))
    }
}

/// Verify the markdown memory directory exists and is writable.
async fn check_markdown_dir(config: &Config) -> HealthStatus {
    let memory_dir = config.workspace_dir.join("memory");

    match tokio::fs::metadata(&memory_dir).await {
        Ok(meta) if meta.is_dir() => {
            // Confirm writability with a sentinel file.
            let sentinel = memory_dir.join(".healthcheck");
            match tokio::fs::write(&sentinel, b"ok").await {
                Ok(()) => {
                    let _ = tokio::fs::remove_file(&sentinel).await;
                    HealthStatus::Pass(format!(
                        "markdown memory dir writable ({})",
                        memory_dir.display()
                    ))
                }
                Err(e) => HealthStatus::Fail(format!(
                    "markdown memory dir not writable ({}): {e}",
                    memory_dir.display()
                )),
            }
        }
        Ok(_) => HealthStatus::Fail(format!(
            "markdown memory path exists but is not a directory ({})",
            memory_dir.display()
        )),
        Err(_) => {
            // Directory does not yet exist — attempt to create it.
            match tokio::fs::create_dir_all(&memory_dir).await {
                Ok(()) => HealthStatus::Pass(format!(
                    "markdown memory dir created ({})",
                    memory_dir.display()
                )),
                Err(e) => HealthStatus::Fail(format!(
                    "could not create markdown memory dir ({}): {e}",
                    memory_dir.display()
                )),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_postgres_host_port_standard_url() {
        assert_eq!(
            extract_postgres_host_port("postgres://user:pass@localhost:5432/mydb"),
            Some("localhost:5432".to_string())
        );
    }

    #[test]
    fn extract_postgres_host_port_no_credentials() {
        assert_eq!(
            extract_postgres_host_port("postgres://localhost:5433/mydb"),
            Some("localhost:5433".to_string())
        );
    }

    #[test]
    fn extract_postgres_host_port_defaults_to_5432() {
        assert_eq!(
            extract_postgres_host_port("postgres://localhost/mydb"),
            Some("localhost:5432".to_string())
        );
    }

    #[test]
    fn extract_postgres_host_port_postgresql_scheme() {
        assert_eq!(
            extract_postgres_host_port("postgresql://db.example.com:5432/prod"),
            Some("db.example.com:5432".to_string())
        );
    }

    #[test]
    fn extract_postgres_host_port_rejects_non_postgres_url() {
        assert_eq!(extract_postgres_host_port("mysql://localhost/db"), None);
        assert_eq!(extract_postgres_host_port("not-a-url"), None);
    }

    #[tokio::test]
    async fn run_health_checks_does_not_panic_with_default_config() {
        // Use a temp dir as workspace to avoid touching the real filesystem.
        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            workspace_dir: tmp.path().to_path_buf(),
            memory: crate::config::MemoryConfig {
                backend: MemoryBackend::None,
                ..Default::default()
            },
            // api_key is None → api_connectivity should be Skipped.
            api_key: None,
            ..Default::default()
        };

        let result = run_health_checks(&config).await;
        assert!(matches!(result.memory_backend, HealthStatus::Skip(_)));
    }
}
