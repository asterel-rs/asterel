//! Provider abstraction and concrete model backends.
//!
//! This module normalizes request/response payloads, streaming events,
//! and tool-call interoperability across supported inference providers.

/// Anthropic provider implementation.
pub mod anthropic;
/// Shared provider metadata used by runtime and onboarding.
pub mod catalog;
/// Codex CLI-backed provider implementation.
pub mod codex_cli;
/// OpenAI-compatible provider implementation.
pub mod compatible;
/// Provider-specific error types.
pub mod error;
/// Provider factory functions.
pub mod factory;
/// Tool fallback logic for providers without native support.
pub mod fallback_tools;
/// Gemini provider implementation.
pub mod gemini;
/// Shared HTTP client builders and defaults.
pub mod http_client;
/// Cross-provider inference options.
pub mod inference;
/// MiniMax provider implementation.
pub mod minimax;
/// OAuth token recovery wrappers.
pub mod oauth_recovery;
/// Ollama provider implementation.
pub mod ollama;
/// `OpenAI` provider implementation.
pub mod openai;
/// `OpenRouter` provider implementation.
pub mod openrouter;
/// Retry/reliability wrappers for providers.
pub mod reliable;
/// Canonical provider response model.
pub mod response;
/// Secret scrubbing helpers for logs/errors.
pub mod scrub;
/// Server-sent events parsing helpers.
pub mod sse;
/// Streaming event model and collectors.
pub mod streaming;
/// Tool schema conversion helpers.
pub mod tool_convert;
/// Core provider trait.
pub mod traits;

pub(crate) use crate::security::scrub::{sanitize_api_error, scrub_secrets};
/// Unified provider error enum.
pub use error::{ProviderCallError, ProviderError, ProviderResult};
#[cfg(test)]
pub(crate) use factory::{
    create_provider, create_resilient_provider, create_resilient_provider_with_resolver,
};
pub(crate) use factory::{
    create_provider_with_oauth_recovery_and_security,
    create_provider_with_oauth_recovery_and_security_for_credential_provider,
    create_resilient_provider_with_oauth_recovery_and_security_for_credential_provider,
};
pub(crate) use http_client::{build_provider_client_with_timeout, build_provider_http_client};
/// Shared per-request inference tuning options.
pub use inference::{InferenceOpts, ThinkingLevel};
/// Canonical provider request/response content model.
pub use response::{
    ContentBlock, ImageSource, MessageRole, ProviderMessage, ProviderResponse, StopReason,
    TokenLogprob,
};
pub(crate) use scrub::api_error;
pub(crate) use streaming::CliStreamSink;
/// Streaming sinks, events, and collectors.
pub use streaming::{ProviderStream, StreamEvent, StreamSink};
/// Provider contract implemented by all model backends.
pub use traits::Provider;

#[cfg(test)]
mod tests;
