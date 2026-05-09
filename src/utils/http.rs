//! Shared HTTP client factory with default timeout.
//!
//! Provides a single `reqwest::Client` builder used across the
//! codebase to enforce a consistent 30-second request timeout.

use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use anyhow::{Result, bail};
use url::Url;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum RuntimeHttpRoute {
    #[default]
    Direct,
    Proxy(String),
}

fn runtime_http_route_store() -> &'static RwLock<RuntimeHttpRoute> {
    static RUNTIME_HTTP_ROUTE: OnceLock<RwLock<RuntimeHttpRoute>> = OnceLock::new();
    RUNTIME_HTTP_ROUTE.get_or_init(|| RwLock::new(RuntimeHttpRoute::Direct))
}

fn current_runtime_http_route() -> RuntimeHttpRoute {
    runtime_http_route_store()
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn runtime_http_route_from_proxy(proxy: Option<&str>) -> Result<RuntimeHttpRoute> {
    match proxy.map(str::trim).filter(|value| !value.is_empty()) {
        Some(url) => {
            let parsed = Url::parse(url)?;
            if !matches!(parsed.scheme(), "http" | "https") {
                bail!("runtime http proxy must use http:// or https://");
            }
            if parsed.host_str().is_none() {
                bail!("runtime http proxy must include a host");
            }
            reqwest::Proxy::all(url)?;
            Ok(RuntimeHttpRoute::Proxy(url.to_string()))
        }
        None => Ok(RuntimeHttpRoute::Direct),
    }
}

fn apply_runtime_http_route(
    builder: reqwest::ClientBuilder,
    route: &RuntimeHttpRoute,
) -> reqwest::Result<reqwest::ClientBuilder> {
    match route {
        RuntimeHttpRoute::Direct => Ok(builder.no_proxy()),
        RuntimeHttpRoute::Proxy(proxy_url) => Ok(builder
            .no_proxy()
            .proxy(reqwest::Proxy::all(proxy_url.as_str())?)),
    }
}

/// Synchronize the shared HTTP client routing policy with the active runtime
/// network settings.
///
/// The daemon and admin settings surface call this before constructing new
/// outbound clients so persisted `network.proxy` changes become effective for
/// subsequently created integrations without relying on process environment
/// mutation.
///
/// # Errors
///
/// Returns an error when the supplied proxy URL is not valid for reqwest.
pub(crate) fn sync_runtime_http_proxy(proxy: Option<&str>) -> Result<bool> {
    let next = runtime_http_route_from_proxy(proxy)?;

    let mut guard = runtime_http_route_store()
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *guard == next {
        return Ok(false);
    }
    *guard = next;
    Ok(true)
}

/// Apply the current runtime routing policy to a reqwest builder.
///
/// # Errors
///
/// Returns an error when the configured proxy URL cannot be applied.
pub(crate) fn with_runtime_network_policy(
    builder: reqwest::ClientBuilder,
) -> reqwest::Result<reqwest::ClientBuilder> {
    apply_runtime_http_route(builder, &current_runtime_http_route())
}

/// Build a reqwest client from a caller-provided builder after applying the
/// shared runtime network policy.
///
/// # Errors
///
/// Returns an error if proxy configuration or TLS backend initialization
/// fails.
pub(crate) fn try_build_http_client_with(
    builder: reqwest::ClientBuilder,
) -> reqwest::Result<reqwest::Client> {
    with_runtime_network_policy(builder)?.build()
}

/// Build a direct reqwest client from a caller-provided builder, ignoring
/// runtime proxies and process proxy environment variables.
///
/// Use this only for callers that have already validated and pinned the target
/// socket addresses for SSRF-sensitive fetches. Applying a proxy to those
/// clients would delegate resolution/connect decisions to the proxy and bypass
/// the pinning guarantee.
pub(crate) fn try_build_direct_http_client_with(
    builder: reqwest::ClientBuilder,
) -> reqwest::Result<reqwest::Client> {
    builder.no_proxy().build()
}

/// Build a direct client pinned to the currently validated public addresses for
/// `parsed_url`.
///
/// # Errors
///
/// Returns an error if the URL has no host, resolves to a private/internal
/// address, or the reqwest client cannot be built.
pub(crate) async fn try_build_pinned_public_fetch_client_with(
    parsed_url: &url::Url,
    builder: reqwest::ClientBuilder,
) -> anyhow::Result<reqwest::Client> {
    let host = parsed_url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("fetch URL has no host"))?
        .to_string();
    let pinned_addrs = crate::security::resolve_public_fetch_addrs(parsed_url).await?;
    Ok(try_build_direct_http_client_with(
        builder.resolve_to_addrs(&host, &pinned_addrs),
    )?)
}

/// Build a reqwest client from a caller-provided builder after applying the
/// shared runtime network policy.
#[must_use]
pub(crate) fn build_http_client_with(builder: reqwest::ClientBuilder) -> reqwest::Client {
    try_build_http_client_with(builder).unwrap_or_else(|error| {
        tracing::warn!(
            %error,
            "reqwest client builder failed; falling back to default client"
        );
        reqwest::Client::new()
    })
}

/// Build a `reqwest::Client` with a 30-second timeout.
///
/// # Errors
///
/// Returns an error if the TLS backend fails to initialize (should not
/// happen with the default rustls/native-tls backends).
pub(crate) fn try_build_http_client() -> reqwest::Result<reqwest::Client> {
    try_build_http_client_with(reqwest::Client::builder().timeout(DEFAULT_TIMEOUT))
}

/// Build a `reqwest::Client` with a 30-second timeout.
///
/// Uses the default TLS backend which is infallible in practice.
/// Falls back to the default `reqwest::Client` (no custom timeout)
/// if the builder fails, logging a warning.
#[must_use]
pub(crate) fn build_http_client() -> reqwest::Client {
    build_http_client_with(reqwest::Client::builder().timeout(DEFAULT_TIMEOUT))
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{
        RuntimeHttpRoute, apply_runtime_http_route, runtime_http_route_from_proxy,
        sync_runtime_http_proxy, try_build_direct_http_client_with,
    };

    struct RuntimeProxyGuard;

    impl RuntimeProxyGuard {
        fn set(proxy_url: &str) -> Self {
            sync_runtime_http_proxy(Some(proxy_url)).expect("test proxy route should be accepted");
            Self
        }
    }

    impl Drop for RuntimeProxyGuard {
        fn drop(&mut self) {
            let _ = sync_runtime_http_proxy(None);
        }
    }

    #[test]
    fn runtime_http_route_from_proxy_accepts_http_proxy() {
        assert_eq!(
            runtime_http_route_from_proxy(Some("http://127.0.0.1:8080"))
                .expect("proxy route should parse"),
            RuntimeHttpRoute::Proxy("http://127.0.0.1:8080".to_string())
        );
    }

    #[test]
    fn runtime_http_route_from_proxy_rejects_invalid_proxy_urls() {
        let error = runtime_http_route_from_proxy(Some("socks5://127.0.0.1:1080"))
            .expect_err("unsupported proxy scheme should be rejected");
        let message = error.to_string();
        assert!(message.contains("proxy"));
    }

    #[test]
    fn apply_runtime_http_route_builds_client_for_direct_mode() {
        let builder =
            apply_runtime_http_route(reqwest::Client::builder(), &RuntimeHttpRoute::Direct)
                .expect("direct mode should configure builder");
        builder.build().expect("direct-mode client should build");
    }

    #[tokio::test]
    async fn direct_http_client_ignores_runtime_proxy_for_pinned_resolution() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/direct"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;

        let parsed_server = url::Url::parse(server.uri().as_str()).expect("server URI parses");
        let server_addr = SocketAddr::new(
            parsed_server
                .host_str()
                .and_then(|host| host.parse::<IpAddr>().ok())
                .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            parsed_server.port().expect("wiremock URI includes a port"),
        );

        let _proxy_guard = RuntimeProxyGuard::set("http://127.0.0.1:9");
        let client = try_build_direct_http_client_with(
            reqwest::Client::builder().resolve_to_addrs("example.test", &[server_addr]),
        )
        .expect("direct pinned client should build");

        let body = client
            .get("http://example.test/direct")
            .send()
            .await
            .expect("direct client should bypass runtime proxy")
            .error_for_status()
            .expect("mock response should be successful")
            .text()
            .await
            .expect("mock body should be readable");

        assert_eq!(body, "ok");
    }
}
