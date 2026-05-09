//! Core `Provider` trait defining the LLM inference interface.
//!
//! All provider implementations (Anthropic, OpenAI, OpenRouter,
//! Ollama, etc.) implement this trait for chat and streaming.

use std::future::Future;
use std::pin::Pin;

use futures_util::stream;

use super::response::{ContentBlock, ProviderMessage, ProviderResponse};
use crate::contracts::provider::{ProviderCapabilities, ProviderCapabilityProfile};
use crate::core::providers::streaming::{ProviderStream, resp_to_events};
use crate::core::providers::{InferenceOpts, ProviderResult};
use crate::core::tools::traits::ToolSpec;

/// Concatenate text content from provider messages into a single
/// newline-separated string, skipping tool and image blocks.
#[must_use]
pub fn messages_to_text(messages: &[ProviderMessage]) -> String {
    let mut result = String::new();
    for msg in messages {
        let role_label = match msg.role {
            super::response::MessageRole::User => "User:",
            super::response::MessageRole::Assistant => "Assistant:",
            super::response::MessageRole::System => "System:",
        };
        let mut text_parts = String::new();
        for block in &msg.content {
            if let ContentBlock::Text { text } = block {
                if !text_parts.is_empty() {
                    text_parts.push(' ');
                }
                text_parts.push_str(text);
            }
        }
        if text_parts.is_empty() {
            continue;
        }
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(role_label);
        result.push(' ');
        result.push_str(&text_parts);
    }
    result
}

/// Core LLM inference trait implemented by all provider backends.
pub trait Provider: Send + Sync {
    /// Send a single user message and return the text response.
    fn chat<'a>(
        &'a self,
        message: &'a str,
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move {
            self.chat_with_system(None, message, model, temperature)
                .await
        })
    }

    /// Send a user message with an optional system prompt.
    fn chat_with_system<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>>;

    /// Send a user message with system prompt and inference options.
    fn chat_with_system_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
        _inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
        Box::pin(async move {
            self.chat_with_system(system_prompt, message, model, temperature)
                .await
        })
    }

    /// Warm up the HTTP connection pool (TLS handshake, DNS, HTTP/2 setup).
    /// Default implementation is a no-op; providers with HTTP clients should override.
    fn warmup(&self) -> Pin<Box<dyn Future<Output = ProviderResult<()>> + Send + '_>> {
        Box::pin(async move { Ok(()) })
    }

    /// Send a user message and return the full provider response
    /// including usage metadata.
    fn chat_with_system_full<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            let text = self
                .chat_with_system_opts(system_prompt, message, model, temperature, None)
                .await?;
            Ok(ProviderResponse::text_only(text))
        })
    }

    /// Full response with inference options (thinking level, etc.).
    fn chat_with_system_full_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        message: &'a str,
        model: &'a str,
        temperature: f64,
        _inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            self.chat_with_system_full(system_prompt, message, model, temperature)
                .await
        })
    }

    /// Chat with structured tool support.
    /// Default: concatenates messages into text, ignores tools, falls back to text-only chat.
    fn chat_with_tools<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            if !tools.is_empty() && self.capability_profile(model).native.native_tool_calling {
                return Err(super::ProviderError::ClientError {
                    provider: "Provider".to_string(),
                    status: 400,
                    message: "provider advertises native tool calling but uses the text-only trait default"
                        .to_string(),
                }
                .into());
            }
            let text = messages_to_text(messages);
            self.chat_with_system_full_opts(system_prompt, &text, model, temperature, None)
                .await
        })
    }

    /// Chat with tools and inference options.
    fn chat_with_tools_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
        _inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderResponse>> + Send + 'a>> {
        Box::pin(async move {
            // Default behavior keeps custom tool handling in `chat_with_tools`.
            // Providers that need options can override this method.
            self.chat_with_tools(system_prompt, messages, tools, model, temperature)
                .await
        })
    }

    /// Declare what this provider supports for the given model.
    /// Default: no native tools, no streaming, no vision.
    fn capabilities(&self, _model: &str) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }

    /// Declare capability truth split by meaning for the given model.
    ///
    /// Plain providers have the same native and effective capabilities. Wrapper
    /// providers can override this when their effective behavior differs from
    /// the selected provider/model's native request-formatting capability.
    fn capability_profile(&self, model: &str) -> ProviderCapabilityProfile {
        ProviderCapabilityProfile::native_only(self.capabilities(model))
    }

    // Deprecated: use capabilities() instead.
    /// Whether this provider supports native structured tool calling.
    fn supports_tools(&self) -> bool {
        self.capabilities("").native_tool_calling
    }

    // Deprecated: use capabilities() instead.
    /// Whether the specific model supports native structured tool calling.
    ///
    /// Default behavior reuses provider-wide capability. Pass-through
    /// providers should override this when model families differ.
    fn supports_tools_model(&self, model: &str) -> bool {
        self.capabilities(model).native_tool_calling
    }

    // Deprecated: use capabilities() instead.
    /// Whether this provider supports streaming responses.
    fn supports_streaming(&self) -> bool {
        self.capabilities("").streaming
    }

    // Deprecated: use capabilities() instead.
    /// Whether this provider supports vision (image) inputs.
    fn supports_vision(&self) -> bool {
        self.capabilities("").vision
    }

    // Deprecated: use capabilities() instead.
    /// Whether the specific model supports vision (image) inputs.
    ///
    /// Default behavior reuses provider-wide capability. Pass-through
    /// providers should override this when model families differ.
    fn supports_vision_model(&self, model: &str) -> bool {
        self.capabilities(model).vision
    }

    /// Chat with tools and return a stream of events.
    /// Default: converts response to events and returns as a stream.
    fn chat_with_tools_stream<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderStream>> + Send + 'a>> {
        Box::pin(async move {
            let resp = self
                .chat_with_tools_opts(system_prompt, messages, tools, model, temperature, None)
                .await?;
            Ok(Box::pin(stream::iter(resp_to_events(resp))) as ProviderStream)
        })
    }

    /// Streaming chat with tools and inference options.
    fn chat_with_tools_stream_opts<'a>(
        &'a self,
        system_prompt: Option<&'a str>,
        messages: &'a [ProviderMessage],
        tools: &'a [ToolSpec],
        model: &'a str,
        temperature: f64,
        _inference_options: Option<&'a InferenceOpts>,
    ) -> Pin<Box<dyn Future<Output = ProviderResult<ProviderStream>> + Send + 'a>> {
        Box::pin(async move {
            // Default behavior uses `chat_with_tools_stream` unless overridden.
            self.chat_with_tools_stream(system_prompt, messages, tools, model, temperature)
                .await
        })
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use super::*;
    use crate::core::providers::response::MessageRole;

    #[test]
    fn messages_to_text_concatenates_text_blocks() {
        let messages = vec![
            ProviderMessage {
                role: MessageRole::User,
                content: vec![ContentBlock::Text {
                    text: "Hello".to_string(),
                }],
            },
            ProviderMessage {
                role: MessageRole::Assistant,
                content: vec![ContentBlock::Text {
                    text: "Hi there".to_string(),
                }],
            },
        ];

        let result = messages_to_text(&messages);

        assert_eq!(result, "User: Hello\nAssistant: Hi there");
    }

    #[test]
    fn messages_to_text_skips_tool_use_blocks() {
        let messages = vec![ProviderMessage {
            role: MessageRole::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: "I'll search".to_string(),
                },
                ContentBlock::ToolUse {
                    id: "toolu_1".to_string(),
                    name: "search".to_string(),
                    input: serde_json::json!({"q": "rust"}),
                },
            ],
        }];

        let result = messages_to_text(&messages);

        assert_eq!(result, "Assistant: I'll search");
    }

    #[test]
    fn messages_to_text_skips_tool_result_blocks() {
        let messages = vec![ProviderMessage {
            role: MessageRole::User,
            content: vec![
                ContentBlock::ToolResult {
                    tool_use_id: "toolu_1".to_string(),
                    content: "result".to_string(),
                    is_error: false,
                },
                ContentBlock::Text {
                    text: "Got it".to_string(),
                },
            ],
        }];

        let result = messages_to_text(&messages);

        assert_eq!(result, "User: Got it");
    }

    #[test]
    fn messages_to_text_handles_empty_messages() {
        let messages: Vec<ProviderMessage> = vec![];

        let result = messages_to_text(&messages);

        assert_eq!(result, "");
    }

    #[test]
    fn messages_to_text_skips_messages_with_only_tool_blocks() {
        let messages = vec![ProviderMessage {
            role: MessageRole::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "toolu_1".to_string(),
                name: "search".to_string(),
                input: serde_json::json!({"q": "rust"}),
            }],
        }];

        let result = messages_to_text(&messages);

        assert_eq!(result, "");
    }

    #[test]
    fn messages_to_text_handles_multiple_text_blocks_in_one_message() {
        let messages = vec![ProviderMessage {
            role: MessageRole::User,
            content: vec![
                ContentBlock::Text {
                    text: "Part 1".to_string(),
                },
                ContentBlock::Text {
                    text: "Part 2".to_string(),
                },
            ],
        }];

        let result = messages_to_text(&messages);

        assert_eq!(result, "User: Part 1 Part 2");
    }

    #[test]
    fn messages_to_text_skips_image_blocks() {
        let messages = vec![ProviderMessage {
            role: MessageRole::User,
            content: vec![
                ContentBlock::Text {
                    text: "Describe this".to_string(),
                },
                ContentBlock::Image {
                    source: crate::core::providers::response::ImageSource::base64(
                        "image/png",
                        "data",
                    ),
                },
            ],
        }];

        let result = messages_to_text(&messages);
        assert_eq!(result, "User: Describe this");
    }

    #[test]
    fn default_supports_tool_calling_returns_false() {
        // Create a minimal mock provider to test the default implementation
        struct MockProvider;

        impl Provider for MockProvider {
            fn chat_with_system<'a>(
                &'a self,
                _system_prompt: Option<&'a str>,
                _message: &'a str,
                _model: &'a str,
                _temperature: f64,
            ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
                Box::pin(async move { Ok("response".to_string()) })
            }
        }

        let provider = MockProvider;
        assert!(!provider.supports_tools());
        assert!(!provider.supports_tools_model("any-model"));
        assert!(!provider.supports_vision_model("any-model"));
    }

    #[tokio::test]
    async fn default_chat_with_tools_errors_when_native_tools_are_advertised() {
        struct MockProvider;

        impl Provider for MockProvider {
            fn chat_with_system<'a>(
                &'a self,
                _system_prompt: Option<&'a str>,
                _message: &'a str,
                _model: &'a str,
                _temperature: f64,
            ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
                Box::pin(async move { Ok("response".to_string()) })
            }

            fn capabilities(&self, _model: &str) -> ProviderCapabilities {
                ProviderCapabilities {
                    native_tool_calling: true,
                    streaming: false,
                    vision: false,
                }
            }
        }

        let provider = MockProvider;
        let messages = vec![ProviderMessage {
            role: MessageRole::User,
            content: vec![ContentBlock::Text {
                text: "use a tool".to_string(),
            }],
        }];
        let tools = vec![ToolSpec {
            name: "shell".to_string(),
            description: "run shell".to_string(),
            parameters: serde_json::json!({"type":"object"}),
            required_capabilities: Vec::new(),
            effect: crate::contracts::tools::ToolEffect::ReadOnly,
        }];

        let error = provider
            .chat_with_tools(None, &messages, &tools, "native-model", 0.0)
            .await
            .expect_err("native tool providers must override chat_with_tools");

        assert!(error.to_string().contains("advertises native tool calling"));
    }
}
