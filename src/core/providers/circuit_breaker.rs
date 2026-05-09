//! Circuit breaker for LLM providers.
//!
//! Wraps any `Provider` with a state machine that trips open after
//! consecutive transient failures, preventing request storms against a
//! degraded backend. Automatically probes for recovery via half-open state.
//!
//! ```text
//!   Closed ──(failures >= threshold)──► Open
//!     ▲                                   │
//!     │                          (recovery timeout)
//!     │                                   ▼
//!     └──(probe succeeds)──── HalfOpen ──(probe fails)──► Open
//! ```

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use super::response::ProviderResponse;
use super::streaming::ProviderStream;
use super::traits::Provider;
use super::{InferenceOpts, ProviderCallError, ProviderError, ProviderResult};
use crate::core::tools::traits::ToolSpec;

// ── Configuration ────────────────────────────────────────────────

/// Configuration for the provider circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Consecutive transient failures before the circuit opens.
    pub failure_threshold: u32,
    /// How long the circuit stays open before allowing a probe.
    pub recovery_timeout: Duration,
    /// Successful probes needed in half-open state to close the circuit.
    pub half_open_max_calls: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_timeout: Duration::from_secs(30),
            half_open_max_calls: 2,
        }
    }
}

// ── State ────────────────────────────────────────────────────────

/// Circuit breaker states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation; tracking consecutive failures.
    Closed,
    /// Rejecting all calls; waiting for recovery timeout to elapse.
    Open,
    /// Allowing probe calls to test whether the backend recovered.
    HalfOpen,
}

/// Internal mutable state protected by `CircuitBreakerProvider::state`.
struct BreakerState {
    state: CircuitState,
    consecutive_failures: u32,
    opened_at: Option<Instant>,
    half_open_successes: u32,
}

impl BreakerState {
    fn new() -> Self {
        Self {
            state: CircuitState::Closed,
            consecutive_failures: 0,
            opened_at: None,
            half_open_successes: 0,
        }
    }
}

// ── Error classification ─────────────────────────────────────────

/// Returns `true` for errors that indicate the provider is degraded
/// (server errors, rate limits, network failures).
///
/// Excludes client errors that are the caller's fault, not backend
/// trouble: auth, client errors, quota, missing creds, parse errors,
/// empty responses.
fn is_transient(err: &ProviderCallError) -> bool {
    match err {
        ProviderCallError::Provider(provider_err) => matches!(
            provider_err,
            ProviderError::RateLimited { .. }
                | ProviderError::ServerError { .. }
                | ProviderError::Network { .. }
        ),
        ProviderCallError::Other(_) => {
            // Untyped errors are assumed transient (network, timeout, etc.)
            true
        }
    }
}

// ── Provider wrapper ─────────────────────────────────────────────

/// Wraps a `Provider` with circuit breaker protection.
///
/// Tracks consecutive transient failures. After `failure_threshold`
/// failures the circuit opens and all requests are rejected for
/// `recovery_timeout`. After that timeout a probe call is allowed
/// through (half-open); if it succeeds the circuit closes, otherwise
/// it reopens.
pub struct CircuitBreakerProvider {
    inner: Box<dyn Provider>,
    state: Mutex<BreakerState>,
    config: CircuitBreakerConfig,
    /// Provider name for diagnostics/logging.
    provider_name: String,
}

impl CircuitBreakerProvider {
    /// Create a circuit breaker wrapping the given provider.
    #[must_use]
    pub fn new(
        inner: Box<dyn Provider>,
        config: CircuitBreakerConfig,
        provider_name: String,
    ) -> Self {
        Self {
            inner,
            state: Mutex::new(BreakerState::new()),
            config,
            provider_name,
        }
    }

    /// Current circuit state (for observability / health checks).
    pub async fn circuit_state(&self) -> CircuitState {
        self.state.lock().await.state
    }

    /// Number of consecutive transient failures recorded so far.
    pub async fn consecutive_failures(&self) -> u32 {
        self.state.lock().await.consecutive_failures
    }

    /// Pre-flight: is a call allowed right now?
    async fn check_allowed(&self) -> ProviderResult<()> {
        let mut state = self.state.lock().await;
        match state.state {
            CircuitState::Closed | CircuitState::HalfOpen => Ok(()),
            CircuitState::Open => {
                if let Some(opened_at) = state.opened_at {
                    if opened_at.elapsed() >= self.config.recovery_timeout {
                        state.state = CircuitState::HalfOpen;
                        state.half_open_successes = 0;
                        tracing::info!(
                            provider = %self.provider_name,
                            "Circuit breaker: Open -> HalfOpen, allowing probe"
                        );
                        Ok(())
                    } else {
                        let remaining = self
                            .config
                            .recovery_timeout
                            .checked_sub(opened_at.elapsed())
                            .unwrap_or(Duration::ZERO);
                        Err(ProviderError::ServerError {
                            provider: self.provider_name.clone(),
                            status: 503,
                            message: format!(
                                "Circuit breaker open ({} consecutive failures, \
                                 recovery in {:.0}s)",
                                state.consecutive_failures,
                                remaining.as_secs_f64()
                            ),
                        }
                        .into())
                    }
                } else {
                    // opened_at should always be Some when Open; recover gracefully.
                    state.state = CircuitState::Closed;
                    Ok(())
                }
            }
        }
    }

    /// Record a successful call.
    async fn record_success(&self) {
        let mut state = self.state.lock().await;
        match state.state {
            CircuitState::Closed => {
                state.consecutive_failures = 0;
            }
            CircuitState::HalfOpen => {
                state.half_open_successes += 1;
                if state.half_open_successes >= self.config.half_open_max_calls {
                    state.state = CircuitState::Closed;
                    state.consecutive_failures = 0;
                    state.opened_at = None;
                    tracing::info!(
                        provider = %self.provider_name,
                        "Circuit breaker: HalfOpen -> Closed (recovered)"
                    );
                }
            }
            CircuitState::Open => {
                // Shouldn't reach here (check_allowed blocks Open), but recover.
                state.state = CircuitState::Closed;
                state.consecutive_failures = 0;
                state.opened_at = None;
            }
        }
    }

    /// Record a failed call; only transient errors count toward the threshold.
    async fn record_failure(&self, err: &ProviderCallError) {
        if !is_transient(err) {
            return;
        }

        let mut state = self.state.lock().await;
        match state.state {
            CircuitState::Closed => {
                state.consecutive_failures += 1;
                if state.consecutive_failures >= self.config.failure_threshold {
                    state.state = CircuitState::Open;
                    state.opened_at = Some(Instant::now());
                    tracing::warn!(
                        provider = %self.provider_name,
                        failures = state.consecutive_failures,
                        "Circuit breaker: Closed -> Open"
                    );
                }
            }
            CircuitState::HalfOpen => {
                state.state = CircuitState::Open;
                state.opened_at = Some(Instant::now());
                state.half_open_successes = 0;
                tracing::warn!(
                    provider = %self.provider_name,
                    "Circuit breaker: HalfOpen -> Open (probe failed)"
                );
            }
            CircuitState::Open => {}
        }
    }

    /// Run `call` through the circuit breaker gate.
    ///
    /// 1. Calls `check_allowed` — rejects immediately if the circuit is open.
    /// 2. Awaits the inner future.
    /// 3. Records success or failure, updating the circuit state accordingly.
    async fn guarded_call<T, Fut>(&self, call: Fut) -> ProviderResult<T>
    where
        Fut: Future<Output = ProviderResult<T>>,
    {
        self.check_allowed().await?;
        match call.await {
            Ok(resp) => {
                self.record_success().await;
                Ok(resp)
            }
            Err(err) => {
                self.record_failure(&err).await;
                Err(err)
            }
        }
    }
}

impl Provider for CircuitBreakerProvider {
    fn capabilities(&self, model: &str) -> crate::contracts::provider::ProviderCapabilities {
        self.inner.capabilities(model)
    }

    fn capability_profile(
        &self,
        model: &str,
    ) -> crate::contracts::provider::ProviderCapabilityProfile {
        self.inner.capability_profile(model)
    }

    fn warmup(&self) -> Pin<Box<dyn Future<Output = ProviderResult<()>> + Send + '_>> {
        // Warmup bypasses the circuit breaker — it's best-effort.
        self.inner.warmup()
    }

    fn chat_with_system<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move {
            self.guarded_call(self.inner.chat_with_system(
                system_prompt,
                message,
                model,
                temperature,
            ))
            .await
        })
    }

    fn chat_with_system_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move {
            self.guarded_call(self.inner.chat_with_system_opts(
                system_prompt,
                message,
                model,
                temperature,
                inference_options,
            ))
            .await
        })
    }

    fn chat_with_system_full<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            self.guarded_call(self.inner.chat_with_system_full(
                system_prompt,
                message,
                model,
                temperature,
            ))
            .await
        })
    }

    fn chat_with_system_full_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            self.guarded_call(self.inner.chat_with_system_full_opts(
                system_prompt,
                message,
                model,
                temperature,
                inference_options,
            ))
            .await
        })
    }

    fn chat_with_tools<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [super::response::ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            self.guarded_call(self.inner.chat_with_tools(
                system_prompt,
                messages,
                tools,
                model,
                temperature,
            ))
            .await
        })
    }

    fn chat_with_tools_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [super::response::ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            self.guarded_call(self.inner.chat_with_tools_opts(
                system_prompt,
                messages,
                tools,
                model,
                temperature,
                inference_options,
            ))
            .await
        })
    }

    fn supports_tools_model(&self, model: &str) -> bool {
        self.capability_profile(model).native.native_tool_calling
    }

    fn supports_vision_model(&self, model: &str) -> bool {
        self.capability_profile(model).native.vision
    }

    fn chat_with_tools_stream<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [super::response::ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderStream>> + Send + 'a>> {
        Box::pin(async move {
            self.guarded_call(self.inner.chat_with_tools_stream(
                system_prompt,
                messages,
                tools,
                model,
                temperature,
            ))
            .await
        })
    }

    fn chat_with_tools_stream_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [super::response::ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderStream>> + Send + 'a>> {
        Box::pin(async move {
            self.guarded_call(self.inner.chat_with_tools_stream_opts(
                system_prompt,
                messages,
                tools,
                model,
                temperature,
                inference_options,
            ))
            .await
        })
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;

    /// Mock provider that can be toggled between success and failure.
    struct MockProvider {
        calls: Arc<AtomicUsize>,
        failing: Arc<AtomicBool>,
        /// When true, failures produce non-transient errors (client 400).
        non_transient: bool,
    }

    impl MockProvider {
        fn succeeding() -> (Self, Arc<AtomicUsize>, Arc<AtomicBool>) {
            let calls = Arc::new(AtomicUsize::new(0));
            let failing = Arc::new(AtomicBool::new(false));
            (
                Self {
                    calls: Arc::clone(&calls),
                    failing: Arc::clone(&failing),
                    non_transient: false,
                },
                calls,
                failing,
            )
        }

        fn always_failing() -> (Self, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    calls: Arc::clone(&calls),
                    failing: Arc::new(AtomicBool::new(true)),
                    non_transient: false,
                },
                calls,
            )
        }

        fn non_transient_failing() -> Self {
            Self {
                calls: Arc::new(AtomicUsize::new(0)),
                failing: Arc::new(AtomicBool::new(true)),
                non_transient: true,
            }
        }
    }

    impl Provider for MockProvider {
        fn chat_with_system<'a>(
            &'a self,
            _system_prompt: Option<&'a str>,
            _message: &'a str,
            _model: &'a str,
            _temperature: f64,
        ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                if self.failing.load(Ordering::SeqCst) {
                    if self.non_transient {
                        Err(ProviderError::ClientError {
                            provider: "mock".into(),
                            status: 400,
                            message: "bad request".into(),
                        }
                        .into())
                    } else {
                        Err(ProviderError::ServerError {
                            provider: "mock".into(),
                            status: 500,
                            message: "internal server error".into(),
                        }
                        .into())
                    }
                } else {
                    Ok("ok".to_string())
                }
            })
        }
    }

    fn fast_config(threshold: u32) -> CircuitBreakerConfig {
        CircuitBreakerConfig {
            failure_threshold: threshold,
            recovery_timeout: Duration::from_millis(50),
            half_open_max_calls: 1,
        }
    }

    fn make_cb(provider: MockProvider, config: CircuitBreakerConfig) -> CircuitBreakerProvider {
        CircuitBreakerProvider::new(Box::new(provider), config, "test".into())
    }

    async fn chat(cb: &CircuitBreakerProvider) -> ProviderResult<String> {
        cb.chat("hello", "test-model", 0.0).await
    }

    // -- State machine tests --

    #[tokio::test]
    async fn closed_allows_calls_and_resets_on_success() {
        let (provider, calls, _failing) = MockProvider::succeeding();
        let cb = make_cb(provider, fast_config(3));

        let resp = chat(&cb).await;
        assert!(resp.is_ok());
        assert_eq!(resp.unwrap(), "ok");
        assert_eq!(cb.circuit_state().await, CircuitState::Closed);
        assert_eq!(cb.consecutive_failures().await, 0);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failures_accumulate_then_trip_to_open() {
        let (provider, _calls) = MockProvider::always_failing();
        let cb = make_cb(provider, fast_config(3));

        // First 2 failures: still closed.
        for i in 0..2 {
            let _ = chat(&cb).await;
            assert_eq!(cb.circuit_state().await, CircuitState::Closed);
            assert_eq!(cb.consecutive_failures().await, i + 1);
        }

        // 3rd failure: trips to open.
        let _ = chat(&cb).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);
    }

    #[tokio::test]
    async fn open_rejects_immediately() {
        let (provider, calls) = MockProvider::always_failing();
        let cb = make_cb(
            provider,
            CircuitBreakerConfig {
                failure_threshold: 1,
                recovery_timeout: Duration::from_secs(60),
                half_open_max_calls: 1,
            },
        );

        // Trip the breaker.
        let _ = chat(&cb).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Next call rejected without reaching inner provider.
        let err = chat(&cb).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Circuit breaker open"), "got: {msg}");
        // Inner provider NOT called again.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn recovery_timeout_transitions_to_half_open() {
        let (provider, _calls) = MockProvider::always_failing();
        let cb = make_cb(provider, fast_config(1));

        // Trip to open.
        let _ = chat(&cb).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);

        // Wait for recovery timeout.
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Next call transitions to half-open (fails, since stub always fails).
        let _ = chat(&cb).await;
        // Failed probe sends it back to Open.
        assert_eq!(cb.circuit_state().await, CircuitState::Open);
    }

    #[tokio::test]
    async fn half_open_success_closes_circuit() {
        let (provider, _calls, failing) = MockProvider::succeeding();
        failing.store(true, Ordering::SeqCst);
        let cb = make_cb(provider, fast_config(1));

        // Trip to open.
        let _ = chat(&cb).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);

        // Wait for recovery, then make the provider succeed.
        tokio::time::sleep(Duration::from_millis(60)).await;
        failing.store(false, Ordering::SeqCst);

        // Probe succeeds, closing the circuit.
        let resp = chat(&cb).await;
        assert!(resp.is_ok());
        assert_eq!(cb.circuit_state().await, CircuitState::Closed);
        assert_eq!(cb.consecutive_failures().await, 0);
    }

    #[tokio::test]
    async fn non_transient_errors_do_not_trip_breaker() {
        let provider = MockProvider::non_transient_failing();
        let cb = make_cb(provider, fast_config(1));

        // Client errors should never trip the breaker.
        for _ in 0..5 {
            let _ = chat(&cb).await;
        }
        assert_eq!(cb.circuit_state().await, CircuitState::Closed);
        assert_eq!(cb.consecutive_failures().await, 0);
    }

    #[tokio::test]
    async fn success_resets_failure_count() {
        let (provider, _calls, failing) = MockProvider::succeeding();
        failing.store(true, Ordering::SeqCst);
        let cb = make_cb(provider, fast_config(3));

        // Accumulate 2 failures.
        let _ = chat(&cb).await;
        let _ = chat(&cb).await;
        assert_eq!(cb.consecutive_failures().await, 2);

        // One success resets the counter.
        failing.store(false, Ordering::SeqCst);
        let resp = chat(&cb).await;
        assert!(resp.is_ok());
        assert_eq!(cb.consecutive_failures().await, 0);
    }

    #[tokio::test]
    async fn multiple_half_open_successes_needed() {
        let (provider, _calls, failing) = MockProvider::succeeding();
        failing.store(true, Ordering::SeqCst);
        let cb = make_cb(
            provider,
            CircuitBreakerConfig {
                failure_threshold: 1,
                recovery_timeout: Duration::from_millis(50),
                half_open_max_calls: 3,
            },
        );

        // Trip to open.
        let _ = chat(&cb).await;

        // Wait and flip to succeed.
        tokio::time::sleep(Duration::from_millis(60)).await;
        failing.store(false, Ordering::SeqCst);

        // First probe: half-open, success but not enough yet.
        let _ = chat(&cb).await;
        assert_eq!(cb.circuit_state().await, CircuitState::HalfOpen);

        // Second probe: still half-open.
        let _ = chat(&cb).await;
        assert_eq!(cb.circuit_state().await, CircuitState::HalfOpen);

        // Third probe: closes.
        let _ = chat(&cb).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Closed);
    }

    #[tokio::test]
    async fn half_open_failure_reopens_and_resets_successes() {
        let (provider, _calls, failing) = MockProvider::succeeding();
        failing.store(true, Ordering::SeqCst);
        let cb = make_cb(
            provider,
            CircuitBreakerConfig {
                failure_threshold: 1,
                recovery_timeout: Duration::from_millis(20),
                half_open_max_calls: 3,
            },
        );

        // Trip the breaker.
        let _ = chat(&cb).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);

        // Wait, then succeed once to accumulate 1 half-open success.
        tokio::time::sleep(Duration::from_millis(30)).await;
        failing.store(false, Ordering::SeqCst);
        let _ = chat(&cb).await;
        assert_eq!(cb.circuit_state().await, CircuitState::HalfOpen);

        // Now fail: should immediately re-open.
        failing.store(true, Ordering::SeqCst);
        let _ = chat(&cb).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);

        // After re-opening, need 3 fresh successes (not 2).
        tokio::time::sleep(Duration::from_millis(30)).await;
        failing.store(false, Ordering::SeqCst);

        let _ = chat(&cb).await;
        assert_eq!(cb.circuit_state().await, CircuitState::HalfOpen);
        let _ = chat(&cb).await;
        assert_eq!(cb.circuit_state().await, CircuitState::HalfOpen);
        let _ = chat(&cb).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Closed);
    }

    #[tokio::test]
    async fn passthrough_capability_flags() {
        let (provider, _calls, _failing) = MockProvider::succeeding();
        let cb = make_cb(provider, fast_config(3));

        // Default MockProvider returns false for all capability checks.
        assert!(!cb.supports_tools());
        assert!(!cb.supports_streaming());
        assert!(!cb.supports_vision());
    }

    // -- Error classification tests --

    #[test]
    fn transient_classification() {
        // Transient.
        assert!(is_transient(&ProviderCallError::Provider(
            ProviderError::ServerError {
                provider: "p".into(),
                status: 500,
                message: "err".into(),
            }
        )));
        assert!(is_transient(&ProviderCallError::Provider(
            ProviderError::RateLimited {
                provider: "p".into(),
                status: 429,
                message: "slow down".into(),
            }
        )));
        assert!(is_transient(&ProviderCallError::Other(anyhow::anyhow!(
            "connection reset"
        ))));

        // NOT transient.
        assert!(!is_transient(&ProviderCallError::Provider(
            ProviderError::Auth {
                provider: "p".into(),
                status: 401,
                message: "bad key".into(),
            }
        )));
        assert!(!is_transient(&ProviderCallError::Provider(
            ProviderError::ClientError {
                provider: "p".into(),
                status: 400,
                message: "bad request".into(),
            }
        )));
        assert!(!is_transient(&ProviderCallError::Provider(
            ProviderError::QuotaExhausted {
                provider: "p".into(),
                message: "exceeded".into(),
            }
        )));
        assert!(!is_transient(&ProviderCallError::Provider(
            ProviderError::MissingCredentials {
                provider: "p".into(),
                message: "no key".into(),
            }
        )));
        assert!(!is_transient(&ProviderCallError::Provider(
            ProviderError::ResponseParse {
                provider: "p".into(),
                message: "bad json".into(),
            }
        )));
        assert!(!is_transient(&ProviderCallError::Provider(
            ProviderError::EmptyResponse {
                provider: "p".into(),
            }
        )));
    }

    #[tokio::test]
    async fn zero_recovery_timeout_allows_immediate_probe() {
        let (provider, _calls, failing) = MockProvider::succeeding();
        failing.store(true, Ordering::SeqCst);
        let cb = make_cb(
            provider,
            CircuitBreakerConfig {
                failure_threshold: 1,
                recovery_timeout: Duration::ZERO,
                half_open_max_calls: 1,
            },
        );

        // Trip the breaker.
        let _ = chat(&cb).await;
        assert_eq!(cb.circuit_state().await, CircuitState::Open);

        // With recovery_timeout = 0, next call should probe immediately.
        failing.store(false, Ordering::SeqCst);
        let result = chat(&cb).await;
        assert!(
            result.is_ok(),
            "zero recovery_timeout should allow immediate probe"
        );
        assert_eq!(cb.circuit_state().await, CircuitState::Closed);
    }
}
