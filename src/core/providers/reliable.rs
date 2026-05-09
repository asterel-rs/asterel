//! Reliability wrapper for providers.
//!
//! Adds exponential-backoff retry with jitter for transient errors,
//! skipping retries on non-retryable failures (auth, quota, 4xx).

use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use super::streaming::ProviderStream;
use super::{
    InferenceOpts, Provider, ProviderCallError, ProviderError, ProviderResponse, ProviderResult,
    sanitize_api_error,
};
use crate::core::tools::traits::ToolSpec;

/// Return `true` for errors that cannot be resolved by retrying.
///
/// Non-retryable conditions: HTTP 4xx client errors (excluding 429 and 408),
/// and billing/quota exhaustion messages. Retryable conditions include
/// rate limits (429), request timeouts (408), server errors (5xx), and
/// network failures.
fn is_non_retryable(err: &ProviderCallError) -> bool {
    match err {
        ProviderCallError::Provider(provider_err) => provider_err.is_non_retryable(),
        ProviderCallError::Other(other) => {
            let msg = other.to_string();
            if is_quota_exhausted(&msg) {
                return true;
            }

            for word in msg.split(|c: char| !c.is_ascii_digit()) {
                if let Ok(code) = word.parse::<u16>()
                    && (400..500).contains(&code)
                {
                    return code != 429 && code != 408;
                }
            }
            false
        }
    }
}

/// Return `true` if the error message indicates quota or billing exhaustion.
fn is_quota_exhausted(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("insufficient_quota")
        || lower.contains("exceeded your current quota")
        || lower.contains("billing")
}

/// Provider wrapper with retry + fallback behavior.
pub struct ReliableProvider {
    providers: Vec<(String, Box<dyn Provider>)>,
    max_retries: u32,
    base_backoff_ms: u64,
}

impl ReliableProvider {
    /// Create a reliable provider with the given ordered list of
    /// `(name, provider)` pairs, retry count, and base backoff.
    #[must_use]
    pub fn new(
        providers: Vec<(String, Box<dyn Provider>)>,
        max_retries: u32,
        base_backoff_ms: u64,
    ) -> Self {
        Self {
            providers,
            max_retries,
            base_backoff_ms: base_backoff_ms.max(50),
        }
    }

    /// Execute `call` against each provider in order, retrying transient
    /// errors with exponential backoff. Advances to the next provider when
    /// retries are exhausted or a non-retryable error is encountered.
    ///
    /// Returns `AllProvidersFailed` when every provider in the list has
    /// been tried and failed.
    async fn execute_with_fallback<'a, T, F>(&'a self, mut call: F) -> ProviderResult<T>
    where
        F: FnMut(&'a dyn Provider) -> Pin<Box<dyn Future<Output = ProviderResult<T>> + Send + 'a>>,
    {
        let mut failures = String::new();

        for (provider_name, provider) in &self.providers {
            if let Some(resp) = self
                .call_provider_with_retries(
                    provider_name,
                    provider.as_ref(),
                    &mut failures,
                    &mut call,
                )
                .await
            {
                return Ok(resp);
            }
        }

        Err(ProviderError::AllProvidersFailed { summary: failures }.into())
    }

    /// Attempt a single provider up to `max_retries + 1` times with
    /// exponential backoff (capped at 10 s). Backoff is doubled after each
    /// attempt. Non-retryable errors short-circuit immediately.
    ///
    /// Returns `Some(value)` on success, `None` when all attempts fail.
    /// Appends a human-readable failure summary to `failures` for each
    /// attempt (used by `AllProvidersFailed` later).
    async fn call_provider_with_retries<'a, T, F>(
        &'a self,
        provider_name: &str,
        provider: &'a dyn Provider,
        failures: &mut String,
        call: &mut F,
    ) -> Option<T>
    where
        F: FnMut(&'a dyn Provider) -> Pin<Box<dyn Future<Output = ProviderResult<T>> + Send + 'a>>,
    {
        let mut backoff_ms = self.base_backoff_ms;

        for attempt in 0..=self.max_retries {
            match call(provider).await {
                Ok(resp) => {
                    if attempt > 0 {
                        tracing::info!(
                            provider = provider_name,
                            attempt,
                            "Provider recovered after retries"
                        );
                    }
                    return Some(resp);
                }
                Err(error) => {
                    let non_retryable = is_non_retryable(&error);
                    let sanitized = sanitize_api_error(&error.to_string());
                    if !failures.is_empty() {
                        failures.push('\n');
                    }
                    let _ = write!(
                        failures,
                        "{provider_name} attempt {}/{}: {sanitized}",
                        attempt + 1,
                        self.max_retries + 1
                    );

                    if non_retryable {
                        tracing::warn!(
                            provider = provider_name,
                            "Non-retryable error, switching provider"
                        );
                        break;
                    }

                    if attempt < self.max_retries {
                        tracing::warn!(
                            provider = provider_name,
                            attempt = attempt + 1,
                            max_retries = self.max_retries,
                            "Provider call failed, retrying"
                        );
                        tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                        backoff_ms = (backoff_ms.saturating_mul(2)).min(10_000);
                    }
                }
            }
        }

        tracing::warn!(provider = provider_name, "Switching to fallback provider");
        None
    }

    fn primary_capability_profile(
        &self,
        model: &str,
    ) -> crate::contracts::provider::ProviderCapabilityProfile {
        self.providers.first().map_or_else(
            crate::contracts::provider::ProviderCapabilityProfile::default,
            |(_, provider)| provider.capability_profile(model),
        )
    }

    fn effective_capabilities(
        &self,
        model: &str,
    ) -> crate::contracts::provider::ProviderCapabilities {
        self.providers.iter().fold(
            crate::contracts::provider::ProviderCapabilities::default(),
            |mut capabilities, (_, provider)| {
                let inner = provider.capability_profile(model).effective;
                capabilities.native_tool_calling |= inner.native_tool_calling;
                capabilities.streaming |= inner.streaming;
                capabilities.vision |= inner.vision;
                capabilities
            },
        )
    }
}

impl Provider for ReliableProvider {
    fn capabilities(&self, model: &str) -> crate::contracts::provider::ProviderCapabilities {
        self.effective_capabilities(model)
    }

    fn capability_profile(
        &self,
        model: &str,
    ) -> crate::contracts::provider::ProviderCapabilityProfile {
        let primary = self.primary_capability_profile(model);
        crate::contracts::provider::ProviderCapabilityProfile {
            native: primary.native,
            effective: self.effective_capabilities(model),
        }
    }

    fn warmup(&self) -> Pin<Box<dyn Future<Output = ProviderResult<()>> + Send + '_>> {
        Box::pin(async move {
            for (name, provider) in &self.providers {
                tracing::info!(provider = name, "Warming up provider connection pool");
                if let Err(e) = provider.warmup().await {
                    tracing::warn!(provider = name, "Warmup failed (non-fatal): {e}");
                }
            }
            Ok(())
        })
    }

    fn chat_with_system<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move {
            self.execute_with_fallback(|provider| {
                provider.chat_with_system(system_prompt, message, model, temperature)
            })
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
            self.execute_with_fallback(|provider| {
                provider.chat_with_system_opts(
                    system_prompt,
                    message,
                    model,
                    temperature,
                    inference_options,
                )
            })
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
            self.execute_with_fallback(|provider| {
                provider.chat_with_system_full(system_prompt, message, model, temperature)
            })
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
            self.execute_with_fallback(|provider| {
                provider.chat_with_system_full_opts(
                    system_prompt,
                    message,
                    model,
                    temperature,
                    inference_options,
                )
            })
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
            self.execute_with_fallback(|provider| {
                provider.chat_with_tools(system_prompt, messages, tools, model, temperature)
            })
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
            self.execute_with_fallback(|provider| {
                provider.chat_with_tools_opts(
                    system_prompt,
                    messages,
                    tools,
                    model,
                    temperature,
                    inference_options,
                )
            })
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
            self.execute_with_fallback(|provider| {
                provider.chat_with_tools_stream(system_prompt, messages, tools, model, temperature)
            })
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
            self.execute_with_fallback(|provider| {
                provider.chat_with_tools_stream_opts(
                    system_prompt,
                    messages,
                    tools,
                    model,
                    temperature,
                    inference_options,
                )
            })
            .await
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct MockProvider {
        calls: Arc<AtomicUsize>,
        fail_until_attempt: usize,
        response: &'static str,
        error: &'static str,
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
                let attempt = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
                if attempt <= self.fail_until_attempt {
                    return Err(anyhow::anyhow!(self.error).into());
                }
                Ok(self.response.to_string())
            })
        }
    }

    struct CapabilityProvider {
        capabilities: crate::contracts::provider::ProviderCapabilities,
    }

    impl Provider for CapabilityProvider {
        fn chat_with_system<'a>(
            &'a self,
            _system_prompt: Option<&'a str>,
            _message: &'a str,
            _model: &'a str,
            _temperature: f64,
        ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
            Box::pin(async move { Ok(String::new()) })
        }

        fn capabilities(&self, _model: &str) -> crate::contracts::provider::ProviderCapabilities {
            self.capabilities
        }
    }

    fn provider_with_capabilities(
        native_tool_calling: bool,
        streaming: bool,
        vision: bool,
    ) -> Box<dyn Provider> {
        Box::new(CapabilityProvider {
            capabilities: crate::contracts::provider::ProviderCapabilities {
                native_tool_calling,
                streaming,
                vision,
            },
        })
    }

    struct ToolForwardingProvider {
        seen_tools: Arc<AtomicUsize>,
    }

    impl Provider for ToolForwardingProvider {
        fn chat_with_system<'a>(
            &'a self,
            _system_prompt: Option<&'a str>,
            _message: &'a str,
            _model: &'a str,
            _temperature: f64,
        ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
            Box::pin(async move { Ok(String::new()) })
        }

        fn chat_with_tools<'a>(
            &'a self,
            _system_prompt: Option<&'a str>,
            _messages: &'a [crate::core::providers::response::ProviderMessage],
            tools: &'a [ToolSpec],
            _model: &'a str,
            _temperature: f64,
        ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
            Box::pin(async move {
                self.seen_tools.store(tools.len(), Ordering::SeqCst);
                Ok(ProviderResponse::text_only("ok".to_string()))
            })
        }

        fn capabilities(&self, _model: &str) -> crate::contracts::provider::ProviderCapabilities {
            crate::contracts::provider::ProviderCapabilities {
                native_tool_calling: true,
                streaming: false,
                vision: false,
            }
        }
    }

    fn test_tool_spec() -> ToolSpec {
        ToolSpec::with_auto_effect(
            "test_tool".to_string(),
            "Test tool".to_string(),
            serde_json::json!({"type": "object"}),
            Vec::new(),
        )
    }

    #[tokio::test]
    async fn succeeds_without_retry() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = ReliableProvider::new(
            vec![(
                "primary".into(),
                Box::new(MockProvider {
                    calls: Arc::clone(&calls),
                    fail_until_attempt: 0,
                    response: "ok",
                    error: "boom",
                }),
            )],
            2,
            1,
        );

        let result = provider.chat("hello", "test", 0.0).await.unwrap();
        assert_eq!(result, "ok");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retries_then_recovers() {
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = ReliableProvider::new(
            vec![(
                "primary".into(),
                Box::new(MockProvider {
                    calls: Arc::clone(&calls),
                    fail_until_attempt: 1,
                    response: "recovered",
                    error: "temporary",
                }),
            )],
            2,
            1,
        );

        let result = provider.chat("hello", "test", 0.0).await.unwrap();
        assert_eq!(result, "recovered");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn falls_back_after_retries_exhausted() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));

        let provider = ReliableProvider::new(
            vec![
                (
                    "primary".into(),
                    Box::new(MockProvider {
                        calls: Arc::clone(&primary_calls),
                        fail_until_attempt: usize::MAX,
                        response: "never",
                        error: "primary down",
                    }),
                ),
                (
                    "fallback".into(),
                    Box::new(MockProvider {
                        calls: Arc::clone(&fallback_calls),
                        fail_until_attempt: 0,
                        response: "from fallback",
                        error: "fallback down",
                    }),
                ),
            ],
            1,
            1,
        );

        let result = provider.chat("hello", "test", 0.0).await.unwrap();
        assert_eq!(result, "from fallback");
        assert_eq!(primary_calls.load(Ordering::SeqCst), 2);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn returns_aggregated_error_when_all_providers_fail() {
        let provider = ReliableProvider::new(
            vec![
                (
                    "p1".into(),
                    Box::new(MockProvider {
                        calls: Arc::new(AtomicUsize::new(0)),
                        fail_until_attempt: usize::MAX,
                        response: "never",
                        error: "p1 error",
                    }),
                ),
                (
                    "p2".into(),
                    Box::new(MockProvider {
                        calls: Arc::new(AtomicUsize::new(0)),
                        fail_until_attempt: usize::MAX,
                        response: "never",
                        error: "p2 error",
                    }),
                ),
            ],
            0,
            1,
        );

        let err = provider
            .chat("hello", "test", 0.0)
            .await
            .expect_err("all providers should fail");
        let msg = err.to_string();
        assert!(msg.contains("All providers failed"));
        assert!(msg.contains("p1 attempt 1/1"));
        assert!(msg.contains("p2 attempt 1/1"));
    }

    #[tokio::test]
    async fn aggregated_error_scrubs_secret_like_values() {
        let provider = ReliableProvider::new(
            vec![(
                "p1".into(),
                Box::new(MockProvider {
                    calls: Arc::new(AtomicUsize::new(0)),
                    fail_until_attempt: usize::MAX,
                    response: "never",
                    error: "api_key=raw-secret-123",
                }),
            )],
            0,
            1,
        );

        let err = provider
            .chat("hello", "test", 0.0)
            .await
            .expect_err("provider should fail");
        let msg = err.to_string();
        assert!(msg.contains("All providers failed"));
        assert!(!msg.contains("raw-secret-123"));
        assert!(msg.contains("[REDACTED]"));
    }

    #[test]
    fn non_retryable_detects_common_patterns() {
        // Non-retryable 4xx errors
        assert!(is_non_retryable(&ProviderCallError::Other(
            anyhow::anyhow!("400 Bad Request")
        )));
        assert!(is_non_retryable(&ProviderCallError::Other(
            anyhow::anyhow!("401 Unauthorized")
        )));
        assert!(is_non_retryable(&ProviderCallError::Other(
            anyhow::anyhow!("403 Forbidden")
        )));
        assert!(is_non_retryable(&ProviderCallError::Other(
            anyhow::anyhow!("404 Not Found")
        )));
        assert!(is_non_retryable(&ProviderCallError::Other(
            anyhow::anyhow!("API error with 400 Bad Request")
        )));
        // Retryable: 429 Too Many Requests
        assert!(!is_non_retryable(&ProviderCallError::Other(
            anyhow::anyhow!("429 Too Many Requests")
        )));
        // Retryable: 408 Request Timeout
        assert!(!is_non_retryable(&ProviderCallError::Other(
            anyhow::anyhow!("408 Request Timeout")
        )));
        // Retryable: 5xx server errors
        assert!(!is_non_retryable(&ProviderCallError::Other(
            anyhow::anyhow!("500 Internal Server Error")
        )));
        assert!(!is_non_retryable(&ProviderCallError::Other(
            anyhow::anyhow!("502 Bad Gateway")
        )));
        // Retryable: transient errors
        assert!(!is_non_retryable(&ProviderCallError::Other(
            anyhow::anyhow!("timeout")
        )));
        assert!(!is_non_retryable(&ProviderCallError::Other(
            anyhow::anyhow!("connection reset")
        )));

        assert!(is_non_retryable(&ProviderCallError::Other(
            anyhow::anyhow!(
                "{}",
                "OpenAI API error (429 Too Many Requests): {\"error\":{\"message\":\"You exceeded your current quota\",\"type\":\"insufficient_quota\"}}"
            )
        )));
    }

    #[test]
    fn non_retryable_prefers_typed_provider_error() {
        let quota = ProviderCallError::Provider(ProviderError::QuotaExhausted {
            provider: "test".to_string(),
            message: "quota exceeded".to_string(),
        });
        assert!(is_non_retryable(&quota));

        let rate_limited = ProviderCallError::Provider(ProviderError::RateLimited {
            provider: "test".to_string(),
            status: 429,
            message: "slow down".to_string(),
        });
        assert!(!is_non_retryable(&rate_limited));
    }

    #[test]
    fn capability_profile_splits_primary_native_from_fallback_effective() {
        let provider = ReliableProvider::new(
            vec![
                (
                    "primary".into(),
                    provider_with_capabilities(false, false, false),
                ),
                (
                    "fallback".into(),
                    provider_with_capabilities(true, true, true),
                ),
            ],
            0,
            1,
        );

        let profile = provider.capability_profile("test-model");

        assert!(!profile.native.native_tool_calling);
        assert!(!profile.native.streaming);
        assert!(!profile.native.vision);
        assert!(profile.effective.native_tool_calling);
        assert!(profile.effective.streaming);
        assert!(profile.effective.vision);
        assert!(provider.capabilities("test-model").native_tool_calling);
        assert!(!provider.supports_tools_model("test-model"));
    }

    #[tokio::test]
    async fn forwards_native_tool_calls_to_inner_provider() {
        let seen_tools = Arc::new(AtomicUsize::new(0));
        let provider = ReliableProvider::new(
            vec![(
                "primary".into(),
                Box::new(ToolForwardingProvider {
                    seen_tools: Arc::clone(&seen_tools),
                }) as Box<dyn Provider>,
            )],
            0,
            1,
        );
        let tools = vec![test_tool_spec()];

        let response = provider
            .chat_with_tools(None, &[], &tools, "test-model", 0.0)
            .await
            .expect("tool call should forward");

        assert_eq!(response.text, "ok");
        assert_eq!(seen_tools.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn skips_retries_on_non_retryable_error() {
        let primary_calls = Arc::new(AtomicUsize::new(0));
        let fallback_calls = Arc::new(AtomicUsize::new(0));

        let provider = ReliableProvider::new(
            vec![
                (
                    "primary".into(),
                    Box::new(MockProvider {
                        calls: Arc::clone(&primary_calls),
                        fail_until_attempt: usize::MAX,
                        response: "never",
                        error: "401 Unauthorized",
                    }),
                ),
                (
                    "fallback".into(),
                    Box::new(MockProvider {
                        calls: Arc::clone(&fallback_calls),
                        fail_until_attempt: 0,
                        response: "from fallback",
                        error: "fallback err",
                    }),
                ),
            ],
            3, // 3 retries allowed, but should skip them
            1,
        );

        let result = provider.chat("hello", "test", 0.0).await.unwrap();
        assert_eq!(result, "from fallback");
        // Primary should have been called only once (no retries)
        assert_eq!(primary_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 1);
    }
}
