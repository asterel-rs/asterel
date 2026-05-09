//! MiniMax provider using the Anthropic-compatible API surface.

use std::future::Future;
use std::pin::Pin;

use crate::contracts::provider::ProviderCapabilities;
use crate::core::providers::anthropic::{AnthropicAuthMode, AnthropicProvider};
use crate::core::providers::traits::Provider;
use crate::core::providers::{
    InferenceOpts, ProviderMessage, ProviderResponse, ProviderResult, ProviderStream,
};
use crate::core::tools::traits::ToolSpec;

const MINIMAX_ANTHROPIC_BASE_URL: &str = "https://api.minimax.io/anthropic";

/// `MiniMax` text provider routed through its Anthropic-compatible surface.
pub struct MiniMaxProvider {
    inner: AnthropicProvider,
}

impl MiniMaxProvider {
    #[must_use]
    pub fn new(api_key: Option<&str>) -> Self {
        Self {
            inner: AnthropicProvider::with_base_url_and_auth_mode(
                api_key,
                Some(MINIMAX_ANTHROPIC_BASE_URL),
                AnthropicAuthMode::Bearer,
            ),
        }
    }
}

impl Provider for MiniMaxProvider {
    fn capabilities(&self, _model: &str) -> ProviderCapabilities {
        ProviderCapabilities {
            native_tool_calling: true,
            streaming: true,
            vision: false,
        }
    }

    fn chat_with_system<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        self.inner
            .chat_with_system(system_prompt, message, model, temperature)
    }

    fn chat_with_system_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        self.inner.chat_with_system_opts(
            system_prompt,
            message,
            model,
            temperature,
            inference_options,
        )
    }

    fn chat_with_system_full<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        self.inner
            .chat_with_system_full(system_prompt, message, model, temperature)
    }

    fn chat_with_system_full_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        self.inner.chat_with_system_full_opts(
            system_prompt,
            message,
            model,
            temperature,
            inference_options,
        )
    }

    fn chat_with_tools<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        self.inner
            .chat_with_tools(system_prompt, messages, tools, model, temperature)
    }

    fn chat_with_tools_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        self.inner.chat_with_tools_opts(
            system_prompt,
            messages,
            tools,
            model,
            temperature,
            inference_options,
        )
    }

    fn chat_with_tools_stream<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderStream>> + Send + 'a>> {
        self.inner
            .chat_with_tools_stream(system_prompt, messages, tools, model, temperature)
    }

    fn chat_with_tools_stream_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
        inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderStream>> + Send + 'a>> {
        self.inner.chat_with_tools_stream_opts(
            system_prompt,
            messages,
            tools,
            model,
            temperature,
            inference_options,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::MiniMaxProvider;
    use crate::core::providers::Provider;

    #[test]
    fn minimax_provider_disables_vision_capability() {
        let provider = MiniMaxProvider::new(Some("test-key"));
        let caps = provider.capabilities("MiniMax-M2.7");
        assert!(caps.native_tool_calling);
        assert!(caps.streaming);
        assert!(!caps.vision);
    }
}
