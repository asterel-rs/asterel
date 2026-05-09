//! OAuth token recovery wrapper for providers.
//!
//! Intercepts authentication failures, triggers an OAuth token
//! refresh, rebuilds the inner provider, and retries the request
//! with a cooldown to prevent refresh storms.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result as AnyResult;
use tokio::sync::{Mutex, RwLock};

use super::streaming::ProviderStream;
use super::{
    InferenceOpts, Provider, ProviderCallError, ProviderResponse, ProviderResult,
    sanitize_api_error,
};
use crate::core::tools::traits::ToolSpec;

/// Callback that triggers an OAuth token refresh for a named provider.
/// Returns `Ok(true)` if a new token was obtained, `Ok(false)` if recovery
/// was skipped (e.g. no refresh token), or `Err` on hard failure.
type RecoverFn = dyn Fn(&str) -> AnyResult<bool> + Send + Sync;

/// Callback that rebuilds a `Provider` instance after a token refresh.
/// Receives the provider name, reads the refreshed key from config/broker,
/// and returns a new `Arc<dyn Provider>`.
type RebuildFn = dyn Fn(&str) -> AnyResult<Arc<dyn Provider>> + Send + Sync;

/// Tracks the timestamp of the last failed recovery attempt for cooldown enforcement.
struct RecoveryState {
    last_failed_at: Option<Instant>,
}

/// Cached capability profile snapshots — avoids a blocking read on the tokio `RwLock`
/// when the inner provider is being rebuilt during recovery.
struct CachedCaps {
    default: crate::contracts::provider::ProviderCapabilityProfile,
    by_model: HashMap<String, crate::contracts::provider::ProviderCapabilityProfile>,
}

impl CachedCaps {
    fn from_provider(provider: &dyn Provider) -> Self {
        Self {
            default: provider.capability_profile(""),
            by_model: HashMap::new(),
        }
    }

    fn get(&self, model: &str) -> crate::contracts::provider::ProviderCapabilityProfile {
        self.by_model.get(model).copied().unwrap_or(self.default)
    }

    fn update(
        &mut self,
        model: &str,
        profile: crate::contracts::provider::ProviderCapabilityProfile,
    ) {
        if model.is_empty() {
            self.default = profile;
        } else {
            self.by_model.insert(model.to_string(), profile);
        }
    }
}

/// Provider wrapper that intercepts auth errors, refreshes the
/// OAuth token, rebuilds the inner provider, and retries.
pub struct OAuthRecoveryProvider {
    provider_name: String,
    inner: RwLock<Arc<dyn Provider>>,
    recover: Arc<RecoverFn>,
    rebuild: Arc<RebuildFn>,
    state: Mutex<RecoveryState>,
    recovery_gate: Mutex<()>,
    provider_revision: AtomicU64,
    cooldown: Duration,
    cached_caps: std::sync::RwLock<CachedCaps>,
}

impl OAuthRecoveryProvider {
    /// Wrap a provider with OAuth recovery using the given refresh
    /// and rebuild callbacks.
    pub fn new(
        provider_name: &str,
        inner: Arc<dyn Provider>,
        recover: Arc<RecoverFn>,
        rebuild: Arc<RebuildFn>,
    ) -> Self {
        let caps = CachedCaps::from_provider(inner.as_ref());
        Self {
            provider_name: provider_name.to_string(),
            inner: RwLock::new(inner),
            recover,
            rebuild,
            state: Mutex::new(RecoveryState {
                last_failed_at: None,
            }),
            recovery_gate: Mutex::new(()),
            provider_revision: AtomicU64::new(0),
            cooldown: Duration::from_secs(60),
            cached_caps: std::sync::RwLock::new(caps),
        }
    }

    #[cfg(test)]
    fn with_cooldown(
        provider_name: &str,
        inner: Arc<dyn Provider>,
        recover: Arc<RecoverFn>,
        rebuild: Arc<RebuildFn>,
        cooldown: Duration,
    ) -> Self {
        Self {
            provider_name: provider_name.to_string(),
            cached_caps: std::sync::RwLock::new(CachedCaps::from_provider(inner.as_ref())),
            inner: RwLock::new(inner),
            recover,
            rebuild,
            state: Mutex::new(RecoveryState {
                last_failed_at: None,
            }),
            recovery_gate: Mutex::new(()),
            provider_revision: AtomicU64::new(0),
            cooldown,
        }
    }

    /// Return `true` if `err` indicates an authentication failure (401/403,
    /// invalid/expired token). Prefers typed `ProviderError::Auth` variants;
    /// falls back to substring matching for untyped errors from providers
    /// that don't produce structured errors.
    fn is_auth_error(err: &ProviderCallError) -> bool {
        match err {
            ProviderCallError::Provider(provider_err) => provider_err.is_auth_error(),
            ProviderCallError::Other(error) => {
                let msg = error.to_string().to_ascii_lowercase();
                msg.contains("401")
                    || msg.contains("403")
                    || msg.contains("unauthorized")
                    || msg.contains("authentication")
                    || msg.contains("invalid api key")
                    || msg.contains("invalid token")
                    || msg.contains("token expired")
            }
        }
    }

    /// Attempt to recover from an auth failure by refreshing the OAuth token
    /// and rebuilding the inner provider.
    ///
    /// Serialized via `recovery_gate` so only one task runs the recovery
    /// sequence at a time. If another task already incremented
    /// `provider_revision` while waiting for the gate, we treat the request
    /// as recovered without re-running the refresh hooks.
    ///
    /// A `cooldown` period prevents hammering the auth backend after a
    /// failed recovery attempt.
    ///
    /// Returns `Ok(true)` if recovery succeeded, `Ok(false)` if skipped
    /// (cooldown active or `recover` returned false), or `Err` on failure.
    async fn attempt_recovery(&self, observed_revision: u64) -> AnyResult<bool> {
        let _recovery_guard = self.recovery_gate.lock().await;

        // Another task has already rebuilt the provider while we waited for
        // the gate; treat this request as recovered without re-running hooks.
        if self.provider_revision.load(Ordering::Acquire) != observed_revision {
            return Ok(true);
        }

        {
            let state = self.state.lock().await;
            if state
                .last_failed_at
                .is_some_and(|failed_at| failed_at.elapsed() < self.cooldown)
            {
                return Ok(false);
            }
        }

        let provider_name = self.provider_name.clone();
        let recover = Arc::clone(&self.recover);
        let recovered = match tokio::task::spawn_blocking(move || (recover)(&provider_name)).await {
            Ok(result) => result?,
            Err(error) => {
                let mut state = self.state.lock().await;
                state.last_failed_at = Some(Instant::now());
                return Err(error.into());
            }
        };
        if !recovered {
            {
                let mut state = self.state.lock().await;
                state.last_failed_at = Some(Instant::now());
            }
            return Ok(false);
        }

        let provider_name = self.provider_name.clone();
        let rebuild_fn = Arc::clone(&self.rebuild);
        let rebuilt_provider =
            match tokio::task::spawn_blocking(move || (rebuild_fn)(&provider_name)).await {
                Ok(result) => result?,
                Err(error) => {
                    let mut state = self.state.lock().await;
                    state.last_failed_at = Some(Instant::now());
                    return Err(error.into());
                }
            };
        let new_caps = CachedCaps::from_provider(rebuilt_provider.as_ref());
        *self.inner.write().await = rebuilt_provider;
        if let Ok(mut caps) = self.cached_caps.write() {
            *caps = new_caps;
        }
        self.provider_revision.fetch_add(1, Ordering::AcqRel);

        {
            let mut state = self.state.lock().await;
            state.last_failed_at = None;
        }
        Ok(true)
    }
}

impl Provider for OAuthRecoveryProvider {
    fn capabilities(&self, model: &str) -> crate::contracts::provider::ProviderCapabilities {
        self.capability_profile(model).effective
    }

    fn capability_profile(
        &self,
        model: &str,
    ) -> crate::contracts::provider::ProviderCapabilityProfile {
        match self.inner.try_read() {
            Ok(provider) => {
                let profile = provider.capability_profile(model);
                if let Ok(mut caps) = self.cached_caps.write() {
                    caps.update(model, profile);
                }
                profile
            }
            Err(_) => self
                .cached_caps
                .read()
                .map(|caps| caps.get(model))
                .unwrap_or_default(),
        }
    }

    fn warmup(&self) -> Pin<Box<dyn Future<Output = ProviderResult<()>> + Send + '_>> {
        Box::pin(async move {
            let provider = self.inner.read().await.clone();
            provider.warmup().await
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
            let observed_revision = self.provider_revision.load(Ordering::Acquire);
            let provider = self.inner.read().await.clone();
            let first_attempt = provider
                .chat_with_system(system_prompt, message, model, temperature)
                .await;

            let Err(first_error) = first_attempt else {
                return first_attempt;
            };

            if !Self::is_auth_error(&first_error) {
                return Err(first_error);
            }

            match self.attempt_recovery(observed_revision).await {
                Ok(true) => {
                    let provider = self.inner.read().await.clone();
                    provider
                        .chat_with_system(system_prompt, message, model, temperature)
                        .await
                }
                Ok(false) => Err(first_error),
                Err(recovery_error) => {
                    tracing::warn!(
                        provider = %self.provider_name,
                        "OAuth recovery failed: {}",
                        sanitize_api_error(&recovery_error.to_string())
                    );
                    Err(first_error)
                }
            }
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
            let observed_revision = self.provider_revision.load(Ordering::Acquire);
            let provider = self.inner.read().await.clone();
            let first_attempt = provider
                .chat_with_system_opts(
                    system_prompt,
                    message,
                    model,
                    temperature,
                    inference_options,
                )
                .await;

            let Err(first_error) = first_attempt else {
                return first_attempt;
            };

            if !Self::is_auth_error(&first_error) {
                return Err(first_error);
            }

            match self.attempt_recovery(observed_revision).await {
                Ok(true) => {
                    let provider = self.inner.read().await.clone();
                    provider
                        .chat_with_system_opts(
                            system_prompt,
                            message,
                            model,
                            temperature,
                            inference_options,
                        )
                        .await
                }
                Ok(false) => Err(first_error),
                Err(recovery_error) => {
                    tracing::warn!(
                        provider = %self.provider_name,
                        "OAuth recovery failed: {}",
                        sanitize_api_error(&recovery_error.to_string())
                    );
                    Err(first_error)
                }
            }
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
            let observed_revision = self.provider_revision.load(Ordering::Acquire);
            let provider = self.inner.read().await.clone();
            let first_attempt = provider
                .chat_with_system_full(system_prompt, message, model, temperature)
                .await;

            let Err(first_error) = first_attempt else {
                return first_attempt;
            };

            if !Self::is_auth_error(&first_error) {
                return Err(first_error);
            }

            match self.attempt_recovery(observed_revision).await {
                Ok(true) => {
                    let provider = self.inner.read().await.clone();
                    provider
                        .chat_with_system_full(system_prompt, message, model, temperature)
                        .await
                }
                Ok(false) => Err(first_error),
                Err(recovery_error) => {
                    tracing::warn!(
                        provider = %self.provider_name,
                        "OAuth recovery failed: {}",
                        sanitize_api_error(&recovery_error.to_string())
                    );
                    Err(first_error)
                }
            }
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
            let observed_revision = self.provider_revision.load(Ordering::Acquire);
            let provider = self.inner.read().await.clone();
            let first_attempt = provider
                .chat_with_system_full_opts(
                    system_prompt,
                    message,
                    model,
                    temperature,
                    inference_options,
                )
                .await;

            let Err(first_error) = first_attempt else {
                return first_attempt;
            };

            if !Self::is_auth_error(&first_error) {
                return Err(first_error);
            }

            match self.attempt_recovery(observed_revision).await {
                Ok(true) => {
                    let provider = self.inner.read().await.clone();
                    provider
                        .chat_with_system_full_opts(
                            system_prompt,
                            message,
                            model,
                            temperature,
                            inference_options,
                        )
                        .await
                }
                Ok(false) => Err(first_error),
                Err(recovery_error) => {
                    tracing::warn!(
                        provider = %self.provider_name,
                        "OAuth recovery failed: {}",
                        sanitize_api_error(&recovery_error.to_string())
                    );
                    Err(first_error)
                }
            }
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
            let observed_revision = self.provider_revision.load(Ordering::Acquire);
            let provider = self.inner.read().await.clone();
            let first_attempt = provider
                .chat_with_tools(system_prompt, messages, tools, model, temperature)
                .await;

            let Err(first_error) = first_attempt else {
                return first_attempt;
            };

            if !Self::is_auth_error(&first_error) {
                return Err(first_error);
            }

            match self.attempt_recovery(observed_revision).await {
                Ok(true) => {
                    let provider = self.inner.read().await.clone();
                    provider
                        .chat_with_tools(system_prompt, messages, tools, model, temperature)
                        .await
                }
                Ok(false) => Err(first_error),
                Err(recovery_error) => {
                    tracing::warn!(
                        provider = %self.provider_name,
                        "OAuth recovery failed: {}",
                        sanitize_api_error(&recovery_error.to_string())
                    );
                    Err(first_error)
                }
            }
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
            let observed_revision = self.provider_revision.load(Ordering::Acquire);
            let provider = self.inner.read().await.clone();
            let first_attempt = provider
                .chat_with_tools_opts(
                    system_prompt,
                    messages,
                    tools,
                    model,
                    temperature,
                    inference_options,
                )
                .await;

            let Err(first_error) = first_attempt else {
                return first_attempt;
            };

            if !Self::is_auth_error(&first_error) {
                return Err(first_error);
            }

            match self.attempt_recovery(observed_revision).await {
                Ok(true) => {
                    let provider = self.inner.read().await.clone();
                    provider
                        .chat_with_tools_opts(
                            system_prompt,
                            messages,
                            tools,
                            model,
                            temperature,
                            inference_options,
                        )
                        .await
                }
                Ok(false) => Err(first_error),
                Err(recovery_error) => {
                    tracing::warn!(
                        provider = %self.provider_name,
                        "OAuth recovery failed: {}",
                        sanitize_api_error(&recovery_error.to_string())
                    );
                    Err(first_error)
                }
            }
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
            let observed_revision = self.provider_revision.load(Ordering::Acquire);
            let provider = self.inner.read().await.clone();
            let first_attempt = provider
                .chat_with_tools_stream(system_prompt, messages, tools, model, temperature)
                .await;

            let Err(first_error) = first_attempt else {
                return first_attempt;
            };

            if !Self::is_auth_error(&first_error) {
                return Err(first_error);
            }

            match self.attempt_recovery(observed_revision).await {
                Ok(true) => {
                    let provider = self.inner.read().await.clone();
                    provider
                        .chat_with_tools_stream(system_prompt, messages, tools, model, temperature)
                        .await
                }
                Ok(false) => Err(first_error),
                Err(recovery_error) => {
                    tracing::warn!(
                        provider = %self.provider_name,
                        "OAuth recovery failed: {}",
                        sanitize_api_error(&recovery_error.to_string())
                    );
                    Err(first_error)
                }
            }
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
            let observed_revision = self.provider_revision.load(Ordering::Acquire);
            let provider = self.inner.read().await.clone();
            let first_attempt = provider
                .chat_with_tools_stream_opts(
                    system_prompt,
                    messages,
                    tools,
                    model,
                    temperature,
                    inference_options,
                )
                .await;

            let Err(first_error) = first_attempt else {
                return first_attempt;
            };

            if !Self::is_auth_error(&first_error) {
                return Err(first_error);
            }

            match self.attempt_recovery(observed_revision).await {
                Ok(true) => {
                    let provider = self.inner.read().await.clone();
                    provider
                        .chat_with_tools_stream_opts(
                            system_prompt,
                            messages,
                            tools,
                            model,
                            temperature,
                            inference_options,
                        )
                        .await
                }
                Ok(false) => Err(first_error),
                Err(recovery_error) => {
                    tracing::warn!(
                        provider = %self.provider_name,
                        "OAuth recovery failed: {}",
                        sanitize_api_error(&recovery_error.to_string())
                    );
                    Err(first_error)
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::Barrier;

    use super::*;

    struct FailProvider;

    impl Provider for FailProvider {
        fn chat_with_system<'a>(
            &'a self,
            _system_prompt: Option<&'a str>,
            _message: &'a str,
            _model: &'a str,
            _temperature: f64,
        ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
            Box::pin(async move { Err(anyhow::anyhow!("401 unauthorized").into()) })
        }
    }

    struct OkProvider;

    impl Provider for OkProvider {
        fn chat_with_system<'a>(
            &'a self,
            _system_prompt: Option<&'a str>,
            _message: &'a str,
            _model: &'a str,
            _temperature: f64,
        ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
            Box::pin(async move { Ok("ok".to_string()) })
        }
    }

    struct ToolOkProvider {
        seen_tools: Arc<AtomicUsize>,
    }

    impl Provider for ToolOkProvider {
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
            _messages: &'a [super::super::response::ProviderMessage],
            tools: &'a [ToolSpec],
            _model: &'a str,
            _temperature: f64,
        ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
            Box::pin(async move {
                self.seen_tools.store(tools.len(), Ordering::SeqCst);
                Ok(ProviderResponse::text_only("ok".to_string()))
            })
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

    struct ModelSpecificCapabilityProvider;

    impl Provider for ModelSpecificCapabilityProvider {
        fn chat_with_system<'a>(
            &'a self,
            _system_prompt: Option<&'a str>,
            _message: &'a str,
            _model: &'a str,
            _temperature: f64,
        ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
            Box::pin(async move { Ok(String::new()) })
        }

        fn capability_profile(
            &self,
            model: &str,
        ) -> crate::contracts::provider::ProviderCapabilityProfile {
            crate::contracts::provider::ProviderCapabilityProfile::native_only(
                crate::contracts::provider::ProviderCapabilities {
                    native_tool_calling: model == "tool-model",
                    streaming: true,
                    vision: model == "vision-model",
                },
            )
        }
    }

    struct CoordinatedFailProvider {
        barrier: Arc<Barrier>,
    }

    impl Provider for CoordinatedFailProvider {
        fn chat_with_system<'a>(
            &'a self,
            _system_prompt: Option<&'a str>,
            _message: &'a str,
            _model: &'a str,
            _temperature: f64,
        ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
            Box::pin(async move {
                self.barrier.wait().await;
                Err(anyhow::anyhow!("401 unauthorized").into())
            })
        }
    }

    #[tokio::test]
    async fn retries_once_after_recovery_and_rebuild() {
        let recover_calls = Arc::new(AtomicUsize::new(0));
        let rebuild_calls = Arc::new(AtomicUsize::new(0));

        let recover = {
            let recover_calls = Arc::clone(&recover_calls);
            Arc::new(move |_provider: &str| {
                recover_calls.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            })
        };

        let rebuild = {
            let rebuild_calls = Arc::clone(&rebuild_calls);
            Arc::new(move |_provider: &str| {
                rebuild_calls.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(OkProvider) as Arc<dyn Provider>)
            })
        };

        let provider =
            OAuthRecoveryProvider::new("openai", Arc::new(FailProvider), recover, rebuild);

        let result = provider.chat("hello", "gpt-test", 0.0).await.unwrap();
        assert_eq!(result, "ok");
        assert_eq!(recover_calls.load(Ordering::SeqCst), 1);
        assert_eq!(rebuild_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn tool_calls_retry_once_after_recovery_and_rebuild() {
        let recover_calls = Arc::new(AtomicUsize::new(0));
        let rebuild_calls = Arc::new(AtomicUsize::new(0));
        let seen_tools = Arc::new(AtomicUsize::new(0));

        let recover = {
            let recover_calls = Arc::clone(&recover_calls);
            Arc::new(move |_provider: &str| {
                recover_calls.fetch_add(1, Ordering::SeqCst);
                Ok(true)
            })
        };

        let rebuild = {
            let rebuild_calls = Arc::clone(&rebuild_calls);
            let seen_tools = Arc::clone(&seen_tools);
            Arc::new(move |_provider: &str| {
                rebuild_calls.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(ToolOkProvider {
                    seen_tools: Arc::clone(&seen_tools),
                }) as Arc<dyn Provider>)
            })
        };

        let provider =
            OAuthRecoveryProvider::new("openai", Arc::new(FailProvider), recover, rebuild);
        let tools = vec![test_tool_spec()];

        let response = provider
            .chat_with_tools(None, &[], &tools, "gpt-test", 0.0)
            .await
            .expect("tool call should recover");

        assert_eq!(response.text, "ok");
        assert_eq!(seen_tools.load(Ordering::SeqCst), 1);
        assert_eq!(recover_calls.load(Ordering::SeqCst), 1);
        assert_eq!(rebuild_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cached_capability_fallback_preserves_model_specific_profiles() {
        let recover = Arc::new(move |_provider: &str| Ok(false));
        let rebuild =
            Arc::new(move |_provider: &str| Ok(Arc::new(OkProvider) as Arc<dyn Provider>));
        let provider = OAuthRecoveryProvider::new(
            "openai",
            Arc::new(ModelSpecificCapabilityProvider),
            recover,
            rebuild,
        );

        assert!(
            provider
                .capability_profile("tool-model")
                .native
                .native_tool_calling
        );
        assert!(provider.capability_profile("vision-model").native.vision);

        let _write_guard = provider.inner.write().await;

        assert!(
            provider
                .capability_profile("tool-model")
                .native
                .native_tool_calling
        );
        assert!(provider.capability_profile("vision-model").native.vision);
        assert!(
            !provider
                .capability_profile("text-model")
                .native
                .native_tool_calling
        );
    }

    #[tokio::test]
    async fn cooldown_skips_repeat_recovery_after_failure() {
        let recover_calls = Arc::new(AtomicUsize::new(0));
        let rebuild_calls = Arc::new(AtomicUsize::new(0));

        let recover = {
            let recover_calls = Arc::clone(&recover_calls);
            Arc::new(move |_provider: &str| {
                recover_calls.fetch_add(1, Ordering::SeqCst);
                Ok(false)
            })
        };

        let rebuild = {
            let rebuild_calls = Arc::clone(&rebuild_calls);
            Arc::new(move |_provider: &str| {
                rebuild_calls.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(OkProvider) as Arc<dyn Provider>)
            })
        };

        let provider = OAuthRecoveryProvider::with_cooldown(
            "openai",
            Arc::new(FailProvider),
            recover,
            rebuild,
            Duration::from_secs(60),
        );

        assert!(provider.chat("hello", "gpt-test", 0.0).await.is_err());
        assert!(provider.chat("hello", "gpt-test", 0.0).await.is_err());
        assert_eq!(recover_calls.load(Ordering::SeqCst), 1);
        assert_eq!(rebuild_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_auth_failures_share_single_recovery() {
        let recover_calls = Arc::new(AtomicUsize::new(0));
        let rebuild_calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(2));

        let recover = {
            let recover_calls = Arc::clone(&recover_calls);
            Arc::new(move |_provider: &str| {
                recover_calls.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(50));
                Ok(true)
            })
        };

        let rebuild = {
            let rebuild_calls = Arc::clone(&rebuild_calls);
            Arc::new(move |_provider: &str| {
                rebuild_calls.fetch_add(1, Ordering::SeqCst);
                Ok(Arc::new(OkProvider) as Arc<dyn Provider>)
            })
        };

        let provider = OAuthRecoveryProvider::with_cooldown(
            "openai",
            Arc::new(CoordinatedFailProvider { barrier }),
            recover,
            rebuild,
            Duration::from_secs(60),
        );

        let (left, right) = tokio::join!(
            provider.chat("hello-left", "gpt-test", 0.0),
            provider.chat("hello-right", "gpt-test", 0.0)
        );

        assert_eq!(left.expect("left request should recover"), "ok");
        assert_eq!(right.expect("right request should recover"), "ok");
        assert_eq!(recover_calls.load(Ordering::SeqCst), 1);
        assert_eq!(rebuild_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn auth_detection_prefers_typed_provider_error() {
        let auth_error =
            super::super::ProviderCallError::Provider(super::super::ProviderError::Auth {
                provider: "openai".to_string(),
                status: 401,
                message: "unauthorized".to_string(),
            });
        assert!(OAuthRecoveryProvider::is_auth_error(&auth_error));

        let non_auth_error =
            super::super::ProviderCallError::Provider(super::super::ProviderError::RateLimited {
                provider: "openai".to_string(),
                status: 429,
                message: "rate limited".to_string(),
            });
        assert!(!OAuthRecoveryProvider::is_auth_error(&non_auth_error));
    }

    #[test]
    fn auth_detection_falls_back_to_message_for_untyped_errors() {
        let untyped = super::super::ProviderCallError::Other(anyhow::anyhow!("401 unauthorized"));
        assert!(OAuthRecoveryProvider::is_auth_error(&untyped));

        let non_auth = super::super::ProviderCallError::Other(anyhow::anyhow!("timeout"));
        assert!(!OAuthRecoveryProvider::is_auth_error(&non_auth));
    }
}
