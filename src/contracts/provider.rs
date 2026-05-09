//! Provider capability contracts shared across provider implementations
//! and the tool loop.

/// Declares what a provider/model combination supports.
///
/// The tool loop reads these flags before every dispatch cycle to decide how
/// to format requests and how to interpret responses. Incorrect values here
/// will cause either malformed requests (if a capability is claimed but not
/// actually available) or degraded behaviour (if a real capability is not
/// declared and the loop falls back unnecessarily).
#[derive(Debug, Clone, Copy, Default)]
pub struct ProviderCapabilities {
    /// Provider returns structured `ContentBlock::ToolUse` for tool calls.
    ///
    /// When `true`, the tool loop serialises tool definitions into the API
    /// request's `tools` array and reads back typed `ToolUse`/`ToolResult`
    /// content blocks. When `false`, the loop must instead inject tool
    /// definitions as plain text into the system prompt and parse tool calls
    /// from the model's free-text response using a regex/grammar extractor.
    /// Setting this incorrectly to `true` on a provider that does not support
    /// it will produce API errors or silent misparses.
    pub native_tool_calling: bool,
    /// Provider supports SSE/streaming responses.
    ///
    /// When `true`, the tool loop opens a streaming request and processes
    /// `delta` events incrementally, enabling lower time-to-first-token and
    /// progress callbacks. When `false`, the loop falls back to a single
    /// blocking request and waits for the full response body before
    /// processing.
    pub streaming: bool,
    /// Provider can process image/vision input.
    ///
    /// When `true`, the tool loop may include `ContentBlock::Image` items in
    /// the message payload, enabling the model to reason about screenshots,
    /// diagrams, and other visual context. When `false`, image blocks are
    /// stripped or converted to a textual placeholder before dispatch to
    /// avoid API errors.
    pub vision: bool,
}

/// Capability truth split by meaning.
///
/// `native` describes the currently selected provider/model itself. `effective`
/// describes the behavior available after wrappers, adapters, or fallback chains
/// are considered. Callers that decide request formatting must use `native`;
/// callers that only report user-visible availability may use `effective`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProviderCapabilityProfile {
    pub native: ProviderCapabilities,
    pub effective: ProviderCapabilities,
}

impl ProviderCapabilityProfile {
    #[must_use]
    pub fn native_only(native: ProviderCapabilities) -> Self {
        Self {
            native,
            effective: native,
        }
    }
}
