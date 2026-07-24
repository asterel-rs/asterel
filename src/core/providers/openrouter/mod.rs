//! OpenRouter provider implementation.
//!
//! Routes requests through the OpenRouter API gateway, reusing the
//! OpenAI Chat Completions wire format with custom auth headers.

use std::future::Future;
use std::pin::Pin;

use reqwest::Client;

use super::openai::compat as openai_compat;
#[cfg(test)]
use super::openai::types::Message;
use super::openai::types::{ChatRequest, ChatResponse};
use crate::core::providers::fallback_tools::{augment_prompt_with_tools, build_fallback_response};
#[cfg(test)]
use crate::core::providers::sse::parse_data_lines_no_done;
use crate::core::providers::streaming::{ProviderStream, resp_to_events};
use crate::core::providers::traits::{Provider, messages_to_text};
use crate::core::providers::{
    InferenceOpts, ProviderMessage, ProviderResponse, ProviderResult, build_provider_http_client,
};
use crate::core::tools::traits::ToolSpec;

/// `OpenRouter` API gateway provider, using the `OpenAI` Chat
/// Completions wire format with custom auth headers.
pub struct OpenRouterProvider {
    /// Pre-computed `"Bearer <key>"` header value (avoids `format!` per request).
    cached_auth_header: Option<String>,
    client: Client,
}

const OPENROUTER_CHAT_COMPLETIONS_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const OPENROUTER_MISSING_API_KEY_MESSAGE: &str =
    "OpenRouter API key not set. Run `asterel onboard` or set OPENROUTER_API_KEY env var.";
const OPENROUTER_EXTRA_HEADERS: [(&str, &str); 2] = [
    ("HTTP-Referer", "https://github.com/asterel-rs/asterel"),
    ("X-Title", "asterel"),
];

/// Model name substrings that indicate vision (image input) support on `OpenRouter`.
const OPENROUTER_VISION_MODEL_MARKERS: &[&str] = &[
    "claude-3",
    "claude-4",
    "gemini",
    "gpt-4.1",
    "gpt-4o",
    "llava",
    "minicpm-v",
    "phi-3.5-vision",
    "pixtral",
    "qwen-vl",
    "qwen2-vl",
];

/// Model name substrings that indicate native tool-calling support on `OpenRouter`.
const OPENROUTER_NATIVE_TOOL_MODEL_MARKERS: &[&str] = &[
    "claude",
    "command-r",
    "deepseek",
    "gemini",
    "gpt-4",
    "gpt-5",
    "gpt-oss",
    "llama-3",
    "llama-4",
    "mistral",
    "mixtral",
    "o1",
    "o3",
    "o4",
    "qwen",
];

impl OpenRouterProvider {
    /// Create a new `OpenRouter` provider with an optional API key.
    #[must_use]
    pub fn new(api_key: Option<&str>) -> Self {
        Self {
            cached_auth_header: api_key.and_then(|key| {
                let trimmed = key.trim();
                (!trimmed.is_empty()).then(|| format!("Bearer {trimmed}"))
            }),
            client: build_provider_http_client(),
        }
    }

    fn build_request(
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> ChatRequest {
        openai_compat::build_request(
            system_prompt,
            message,
            model,
            temperature,
            inference_options,
        )
    }

    fn extract_text(chat_response: &ChatResponse) -> anyhow::Result<String> {
        openai_compat::extract_text(chat_response, "OpenRouter")
    }

    #[cfg(test)]
    fn map_provider_message(provider_message: &ProviderMessage) -> Vec<Message> {
        openai_compat::map_provider_message(provider_message)
    }

    fn build_tools_request(
        system_prompt: Option<&str>,
        messages: &[ProviderMessage],
        tools: &[ToolSpec],
        model: &str,
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> ChatRequest {
        openai_compat::build_tools_request(
            system_prompt,
            messages,
            tools,
            model,
            temperature,
            inference_options,
        )
    }

    async fn call_api_req(&self, request: &ChatRequest) -> anyhow::Result<ChatResponse> {
        openai_compat::send_chat_completions_json(
            &self.client,
            self.cached_auth_header.as_ref(),
            request,
            openai_compat::ChatCompletionsEndpoint {
                provider_name: "OpenRouter",
                url: OPENROUTER_CHAT_COMPLETIONS_URL,
                missing_api_key_message: OPENROUTER_MISSING_API_KEY_MESSAGE,
                extra_headers: &OPENROUTER_EXTRA_HEADERS,
            },
        )
        .await
    }

    async fn call_api_streaming(&self, request: &ChatRequest) -> anyhow::Result<reqwest::Response> {
        openai_compat::send_chat_completions_raw(
            &self.client,
            self.cached_auth_header.as_ref(),
            request,
            openai_compat::ChatCompletionsEndpoint {
                provider_name: "OpenRouter",
                url: OPENROUTER_CHAT_COMPLETIONS_URL,
                missing_api_key_message: OPENROUTER_MISSING_API_KEY_MESSAGE,
                extra_headers: &OPENROUTER_EXTRA_HEADERS,
            },
        )
        .await
    }

    async fn chat_with_tools_stream_inner(
        &self,
        system_prompt: Option<&str>,
        messages: &[ProviderMessage],
        tools: &[ToolSpec],
        model: &str,
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> anyhow::Result<ProviderStream> {
        let request = openai_compat::build_stream_request(
            system_prompt,
            messages,
            tools,
            model,
            temperature,
            inference_options,
        );
        let response = self.call_api_streaming(&request).await?;
        Ok(openai_compat::sse_response_to_provider_stream(response))
    }

    async fn call_api(
        &self,
        system_prompt: Option<&str>,
        message: &str,
        model: &str,
        temperature: f64,
        inference_options: Option<&InferenceOpts>,
    ) -> anyhow::Result<ChatResponse> {
        let request = Self::build_request(
            system_prompt,
            message,
            model,
            temperature,
            inference_options,
        );
        self.call_api_req(&request).await
    }

    /// Build the system prompt and message text for XML fallback tool calling.
    /// Returns `(augmented_system_prompt, concatenated_message_text)`.
    fn prepare_fallback_input(
        system_prompt: Option<&str>,
        messages: &[ProviderMessage],
        tools: &[ToolSpec],
    ) -> (String, String) {
        let augmented_prompt = augment_prompt_with_tools(system_prompt.unwrap_or(""), tools);
        let text = messages_to_text(messages);
        (augmented_prompt, text)
    }

    /// Return `true` if the model name indicates vision support, using a
    /// substring match against known `OpenRouter` vision model identifiers.
    fn model_supports_vision(model: &str) -> bool {
        let normalized = model.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return false;
        }

        OPENROUTER_VISION_MODEL_MARKERS
            .iter()
            .any(|marker| normalized.contains(marker))
            || normalized.contains("vision")
            || normalized.contains("multimodal")
            || normalized.contains("omni")
    }

    /// Return `true` if the model name indicates native tool-calling support,
    /// using a substring match against known `OpenRouter` tool-capable identifiers.
    /// Falls back to XML tool calling via `fallback_tools` when this returns `false`.
    fn model_supports_tools(model: &str) -> bool {
        let normalized = model.trim().to_ascii_lowercase();
        if normalized.is_empty() {
            return false;
        }

        OPENROUTER_NATIVE_TOOL_MODEL_MARKERS
            .iter()
            .any(|marker| normalized.contains(marker))
            || normalized.contains("tool")
            || normalized.contains("function")
    }
}

impl Provider for OpenRouterProvider {
    fn capabilities(&self, model: &str) -> crate::contracts::provider::ProviderCapabilities {
        let model = model.trim();
        crate::contracts::provider::ProviderCapabilities {
            native_tool_calling: model.is_empty() || Self::model_supports_tools(model),
            streaming: true,
            vision: model.is_empty() || Self::model_supports_vision(model),
        }
    }

    fn warmup(&self) -> Pin<Box<dyn Future<Output = ProviderResult<()>> + Send + '_>> {
        Box::pin(async move {
            // Hit a lightweight endpoint to establish TLS + HTTP/2 connection pool.
            // This prevents the first real chat request from timing out on cold start.
            if let Some(auth_header) = self.cached_auth_header.as_ref() {
                self.client
                    .get("https://openrouter.ai/api/v1/auth/key")
                    .header("Authorization", auth_header)
                    .send()
                    .await
                    .map_err(anyhow::Error::from)?
                    .error_for_status()
                    .map_err(anyhow::Error::from)?;
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
            let chat_response = self
                .call_api(system_prompt, message, model, temperature, None)
                .await?;
            Self::extract_text(&chat_response).map_err(Into::into)
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
            let chat_response = self
                .call_api(
                    system_prompt,
                    message,
                    model,
                    temperature,
                    inference_options,
                )
                .await?;
            Self::extract_text(&chat_response).map_err(Into::into)
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
            let chat_response = self
                .call_api(system_prompt, message, model, temperature, None)
                .await?;
            openai_compat::build_text_provider_response(chat_response, "OpenRouter")
                .map_err(Into::into)
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
            let chat_response = self
                .call_api(
                    system_prompt,
                    message,
                    model,
                    temperature,
                    inference_options,
                )
                .await?;
            openai_compat::build_text_provider_response(chat_response, "OpenRouter")
                .map_err(Into::into)
        })
    }

    fn chat_with_tools<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            if !self.supports_tools_model(model) {
                let (augmented_prompt, text) =
                    Self::prepare_fallback_input(system_prompt, messages, tools);
                let response = self
                    .chat_with_system_full(Some(&augmented_prompt), &text, model, temperature)
                    .await?;
                return Ok(build_fallback_response(response, tools));
            }

            let request =
                Self::build_tools_request(system_prompt, messages, tools, model, temperature, None);
            let chat_response = self.call_api_req(&request).await?;
            openai_compat::build_tool_provider_response(chat_response, "OpenRouter")
                .map_err(Into::into)
        })
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
        Box::pin(async move {
            if !self.supports_tools_model(model) {
                let (augmented_prompt, text) =
                    Self::prepare_fallback_input(system_prompt, messages, tools);
                let response = self
                    .chat_with_system_full_opts(
                        Some(&augmented_prompt),
                        &text,
                        model,
                        temperature,
                        inference_options,
                    )
                    .await?;
                return Ok(build_fallback_response(response, tools));
            }

            let request = Self::build_tools_request(
                system_prompt,
                messages,
                tools,
                model,
                temperature,
                inference_options,
            );
            let chat_response = self.call_api_req(&request).await?;
            openai_compat::build_tool_provider_response(chat_response, "OpenRouter")
                .map_err(Into::into)
        })
    }

    fn supports_tools_model(&self, model: &str) -> bool {
        Self::model_supports_tools(model)
    }

    fn supports_vision_model(&self, model: &str) -> bool {
        Self::model_supports_vision(model)
    }

    fn chat_with_tools_stream<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderStream>> + Send + 'a>> {
        Box::pin(async move {
            if !self.supports_tools_model(model) {
                let response = self
                    .chat_with_tools(system_prompt, messages, tools, model, temperature)
                    .await?;
                return Ok(
                    Box::pin(futures_util::stream::iter(resp_to_events(response)))
                        as ProviderStream,
                );
            }

            self.chat_with_tools_stream_inner(
                system_prompt,
                messages,
                tools,
                model,
                temperature,
                None,
            )
            .await
            .map_err(Into::into)
        })
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
        Box::pin(async move {
            if !self.supports_tools_model(model) {
                let response = self
                    .chat_with_tools_opts(
                        system_prompt,
                        messages,
                        tools,
                        model,
                        temperature,
                        inference_options,
                    )
                    .await?;
                return Ok(
                    Box::pin(futures_util::stream::iter(resp_to_events(response)))
                        as ProviderStream,
                );
            }

            self.chat_with_tools_stream_inner(
                system_prompt,
                messages,
                tools,
                model,
                temperature,
                inference_options,
            )
            .await
            .map_err(Into::into)
        })
    }
}

#[cfg(test)]
mod tests;
