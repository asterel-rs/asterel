//! Shared HTTP client builder for LLM provider requests.
//!
//! Configures timeouts, connection pooling, and TCP keepalive
//! defaults used by all provider implementations.

use std::time::Duration;

use reqwest::Client;

/// Build a shared HTTP client with default 120-second timeout.
#[must_use]
pub fn build_provider_http_client() -> Client {
    build_provider_client_with_timeout(120)
}

/// Build a shared HTTP client with a custom timeout in seconds.
#[must_use]
pub fn build_provider_client_with_timeout(timeout_secs: u64) -> Client {
    crate::utils::http::build_http_client_with(
        Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60)),
    )
}
