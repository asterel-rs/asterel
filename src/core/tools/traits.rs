//! Core trait and type definitions for the tool system.
//!
//! # Key types
//!
//! | Type | Role |
//! |------|------|
//! | [`Tool`] | Async capability the agent can invoke |
//! | [`ToolSpec`] | Name + schema description sent to the LLM |
//! | [`ToolResult`] | Structured outcome returned to the agent loop |
//! | [`ActionIntent`] | Declared external-action request, pending policy check |
//! | [`ActionOperator`] | Executes (or records) approved action intents |
//! | [`McpToolProvider`] | Factory for MCP-sourced tools |
//!
//! These types are consumed by the middleware pipeline in
//! `crate::core::tools::middleware` and by every concrete tool
//! implementation in this module tree.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::config::schema::McpConfig;
use crate::contracts::strings::verdicts::{
    SECURITY_BLOCK_AUTONOMY_READ_ONLY, SECURITY_POLICY_BLOCK_PREFIX,
};

use crate::core::tools::middleware::ExecutionContext;
use crate::security::{ActionPolicyVerdict, ExternalActionExecution, SecurityPolicy};

pub use crate::contracts::tools::{Capability, ToolSpec};

/// Structured outcome of a single tool invocation.
///
/// Returned by [`Tool::execute`] and transformed by the `after_execute`
/// phase of the middleware chain before being handed back to the agent loop.
///
/// Middleware may mutate `output`, `error`, and `taint_labels` in-place
/// (e.g. truncation, secret scrubbing, taint propagation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Whether the tool invocation succeeded.
    pub success: bool,
    /// Primary text output from the tool, returned verbatim to the model.
    pub output: String,
    /// Human-readable error message when the tool fails; absent on success.
    pub error: Option<String>,
    /// File or URL attachments produced by the tool (e.g. images, reports).
    #[serde(default)]
    pub attachments: Vec<OutputAttachment>,
    /// Taint labels applied by [`TaintMiddleware`] during `after_execute`.
    /// Empty until the middleware chain runs.
    #[serde(default)]
    pub taint_labels: Vec<String>,
    /// Internal-only semantic metadata for output compaction dispatch.
    #[serde(skip, default)]
    pub semantic: ToolResultSemanticMetadata,
}

/// Compaction target inside a [`ToolResult`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ToolResultCompactionTarget {
    /// Compact only the primary `output` field.
    #[default]
    Output,
    /// Compact only the `error` field.
    Error,
    /// Consider both `output` and `error`.
    OutputAndError,
}

/// Text-bearing field inside a [`ToolResult`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ToolResultTextField {
    #[default]
    Output,
    Error,
}

impl ToolResultTextField {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Output => "output",
            Self::Error => "error",
        }
    }
}

/// How semantic compaction should treat multiple text fields in a [`ToolResult`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ToolResultSemanticStreamMode {
    /// Compact each participating field independently.
    #[default]
    PerField,
    /// Merge `output` and `error` into one parse stream before compacting.
    CombinedOutputAndError,
}

/// Size-oriented stats captured from raw tool text before compaction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolResultSemanticStats {
    pub output_bytes: usize,
    pub output_lines: usize,
    pub error_bytes: usize,
    pub error_lines: usize,
}

impl ToolResultSemanticStats {
    #[must_use]
    pub fn from_text(output: &str, error: Option<&str>) -> Self {
        Self {
            output_bytes: output.len(),
            output_lines: text_line_count(output),
            error_bytes: error.map_or(0, str::len),
            error_lines: error.map_or(0, text_line_count),
        }
    }
}

/// Internal metadata describing which tool-result fields contain raw source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResultSemanticSourceFieldMetadata {
    pub field: ToolResultTextField,
}

/// Internal pointer to a persisted raw artifact for semantic compaction recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResultSemanticArtifact {
    pub field: ToolResultTextField,
    pub key: String,
    pub path: PathBuf,
}

/// Internal semantic dispatch metadata carried alongside a [`ToolResult`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolResultSemanticMetadata {
    pub output_kind: Option<String>,
    pub compaction_target: ToolResultCompactionTarget,
    pub stream_mode: ToolResultSemanticStreamMode,
    pub stats: Option<ToolResultSemanticStats>,
    pub raw_command: Option<String>,
    pub source_fields: Vec<ToolResultSemanticSourceFieldMetadata>,
    pub artifacts: Vec<ToolResultSemanticArtifact>,
}

impl ToolResultSemanticMetadata {
    #[must_use]
    pub fn with_output_kind(mut self, output_kind: impl Into<String>) -> Self {
        self.output_kind = Some(output_kind.into());
        self
    }

    #[must_use]
    pub fn with_compaction_target(mut self, target: ToolResultCompactionTarget) -> Self {
        self.compaction_target = target;
        self
    }

    #[must_use]
    pub fn with_stream_mode(mut self, stream_mode: ToolResultSemanticStreamMode) -> Self {
        self.stream_mode = stream_mode;
        self
    }

    #[must_use]
    pub fn with_raw_command(mut self, raw_command: impl Into<String>) -> Self {
        self.raw_command = Some(raw_command.into());
        self
    }

    #[must_use]
    pub fn with_source_fields<I>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = ToolResultTextField>,
    {
        self.source_fields = fields
            .into_iter()
            .map(|field| ToolResultSemanticSourceFieldMetadata { field })
            .collect();
        self
    }

    pub fn record_artifact(
        &mut self,
        field: ToolResultTextField,
        key: impl Into<String>,
        path: PathBuf,
    ) {
        self.clear_artifact(field);
        self.artifacts.push(ToolResultSemanticArtifact {
            field,
            key: key.into(),
            path,
        });
    }

    pub fn clear_artifact(&mut self, field: ToolResultTextField) {
        self.artifacts.retain(|artifact| artifact.field != field);
    }
}

impl Default for ToolResult {
    fn default() -> Self {
        Self {
            success: true,
            output: String::new(),
            error: None,
            attachments: Vec::new(),
            taint_labels: Vec::new(),
            semantic: ToolResultSemanticMetadata::default(),
        }
    }
}

impl ToolResult {
    #[must_use]
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            ..Self::default()
        }
        .refresh_semantic_stats()
    }

    #[must_use]
    pub fn failure(error: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(error.into()),
            ..Self::default()
        }
        .refresh_semantic_stats()
    }

    #[must_use]
    pub fn with_output_kind(mut self, output_kind: impl Into<String>) -> Self {
        self.semantic.output_kind = Some(output_kind.into());
        self
    }

    #[must_use]
    pub fn with_compaction_target(mut self, target: ToolResultCompactionTarget) -> Self {
        self.semantic.compaction_target = target;
        self
    }

    #[must_use]
    pub fn with_stream_mode(mut self, stream_mode: ToolResultSemanticStreamMode) -> Self {
        self.semantic.stream_mode = stream_mode;
        self
    }

    #[must_use]
    pub fn with_raw_command(mut self, raw_command: impl Into<String>) -> Self {
        self.semantic.raw_command = Some(raw_command.into());
        self
    }

    #[must_use]
    pub fn with_source_fields<I>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = ToolResultTextField>,
    {
        self.semantic.source_fields = fields
            .into_iter()
            .map(|field| ToolResultSemanticSourceFieldMetadata { field })
            .collect();
        self
    }

    #[must_use]
    pub fn with_semantic_stats(mut self, stats: ToolResultSemanticStats) -> Self {
        self.semantic.stats = Some(stats);
        self
    }

    #[must_use]
    pub fn refresh_semantic_stats(mut self) -> Self {
        self.semantic.stats = Some(ToolResultSemanticStats::from_text(
            &self.output,
            self.error.as_deref(),
        ));
        self
    }
}

fn text_line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.bytes().filter(|byte| *byte == b'\n').count() + 1
    }
}

/// Source of an output attachment — either a local file or a remote URL.
///
/// Using an enum instead of two `Option<String>` fields prevents the
/// illegal state where both `path` and `url` are `Some` or both `None`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AttachmentSource {
    /// Local filesystem path.
    File { path: String },
    /// Remote URL.
    Url { url: String },
}

/// A file or URL attachment produced by a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputAttachment {
    /// MIME type of the attachment (e.g. `"image/png"`).
    pub mime_type: String,
    /// Optional human-readable filename.
    pub filename: Option<String>,
    /// Where the attachment data lives.
    pub source: AttachmentSource,
}

impl OutputAttachment {
    /// Create an attachment backed by a local file path.
    pub fn from_path(
        mime_type: impl Into<String>,
        path: impl Into<String>,
        filename: Option<String>,
    ) -> Self {
        Self {
            mime_type: mime_type.into(),
            filename,
            source: AttachmentSource::File { path: path.into() },
        }
    }

    /// Create an attachment backed by a remote URL.
    pub fn from_url(
        mime_type: impl Into<String>,
        url: impl Into<String>,
        filename: Option<String>,
    ) -> Self {
        Self {
            mime_type: mime_type.into(),
            filename,
            source: AttachmentSource::Url { url: url.into() },
        }
    }
}

/// Core tool trait — implement this to expose a capability to the agent.
///
/// # Contract
///
/// Implementations must be `Send + Sync` and stateless (or use interior
/// mutability) because a single tool instance may be called concurrently
/// from multiple sessions.
///
/// `execute` must **not** bypass the middleware chain.  Security enforcement,
/// rate limiting, and output sanitization are all handled by the middleware
/// pipeline that wraps every call coming through [`ToolRegistry`].
///
/// # Error handling
///
/// Return `Err(...)` only for unrecoverable infrastructure failures (e.g. a
/// library panic, serialisation error).  User-visible failures (bad args,
/// file not found, permission denied) should be returned as a
/// `ToolResult { success: false, error: Some(...) }`.
pub trait Tool: Send + Sync {
    /// Stable tool identifier used in LLM function-calling schemas.
    fn name(&self) -> &str;

    /// One-sentence description shown to the model in the tool schema.
    fn description(&self) -> &str;

    /// JSON Schema object describing the tool's parameters.
    fn parameters_schema(&self) -> serde_json::Value;

    /// Execute the tool with the provided arguments inside the given context.
    ///
    /// # Errors
    ///
    /// Returns `Err` only for infrastructure failures.  Use a failed
    /// [`ToolResult`] for expected error conditions.
    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>>;

    /// Build the full [`ToolSpec`] for LLM registration.
    ///
    /// The default implementation derives the spec from `name`, `description`,
    /// `parameters_schema`, and the tool's classified [`ToolEffect`].
    fn spec(&self) -> ToolSpec {
        let name = self.name().to_string();
        let effect = crate::contracts::tools::ToolEffect::classify(&name);
        ToolSpec {
            name,
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
            required_capabilities: Vec::new(),
            effect,
        }
    }
}

/// A declared intent for an external action, pending policy check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionIntent {
    /// Unique identifier for this intent.
    pub intent_id: String,
    /// Category of external action (e.g. `"composio"`, `"game"`).
    pub action_kind: String,
    /// Name of the operator that will execute the action.
    pub operator: String,
    /// Arbitrary payload describing the action details.
    pub payload: serde_json::Value,
    /// RFC 3339 timestamp when the intent was created.
    pub requested_at: String,
}

impl ActionIntent {
    #[must_use]
    /// Build a new intent with a fresh UUID and current timestamp.
    pub fn new(action_kind: &str, operator: &str, payload: serde_json::Value) -> Self {
        Self {
            intent_id: uuid::Uuid::new_v4().to_string(),
            action_kind: action_kind.to_string(),
            operator: operator.to_string(),
            payload,
            requested_at: Utc::now().to_rfc3339(),
        }
    }

    #[allow(clippy::unused_self)] // Method semantics preferred for ActionIntent API ergonomics
    #[must_use]
    /// Evaluate whether the security policy allows this intent.
    pub fn policy_verdict(&self, policy: &SecurityPolicy) -> ActionPolicyVerdict {
        if !policy.can_act() {
            return ActionPolicyVerdict::deny(SECURITY_BLOCK_AUTONOMY_READ_ONLY);
        }

        if policy.external_action_execution == ExternalActionExecution::Disabled {
            return ActionPolicyVerdict::deny(format!(
                "{SECURITY_POLICY_BLOCK_PREFIX}external_action_execution is disabled"
            ));
        }

        ActionPolicyVerdict::allow("allowed by security policy")
    }
}

/// Outcome of applying an action intent through an operator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResult {
    /// Whether the action was actually executed.
    pub executed: bool,
    /// Human-readable outcome or denial reason.
    pub message: String,
    /// Filesystem path to the JSONL audit record, if written.
    pub audit_record_path: Option<String>,
}

/// Operator that executes or records an approved external action intent.
///
/// Two concrete implementations exist:
/// - [`NoopOperator`]: audit-only — records the intent to JSONL but never
///   executes.  This is the safe default when no external connector is wired.
/// - `ComposioOperator` (in `crate::core::tools::composio`): forwards the
///   intent to the `Composio` API if the policy verdict allows it.
pub trait ActionOperator: Send + Sync {
    /// Returns the operator's name (e.g. `"noop"`, `"composio"`).
    fn name(&self) -> &str;

    /// Apply the intent, using the supplied policy verdict as the gate.
    ///
    /// `verdict` should always be `Some`; passing `None` is an error for most
    /// implementations because the verdict is required for audit records.
    ///
    /// # Errors
    ///
    /// Returns an error if the operator cannot process the intent (e.g. the
    /// audit file is not writable, or the connector returns an error).
    fn apply<'a>(
        &'a self,
        intent: &'a ActionIntent,
        verdict: Option<&'a ActionPolicyVerdict>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ActionResult>> + Send + 'a>>;
}

/// Audit-only operator that records every intent to a daily JSONL file but
/// never executes the underlying action.
///
/// Used as the safe default when no real external connector is configured.
/// The audit file lives at `{workspace_dir}/action_intents/{YYYY-MM-DD}.jsonl`.
pub struct NoopOperator {
    security: Arc<SecurityPolicy>,
}

impl NoopOperator {
    #[must_use]
    /// Create a new no-op operator backed by the given policy.
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    fn audit_path(&self) -> PathBuf {
        let date = Utc::now().format("%Y-%m-%d").to_string();
        self.security
            .workspace_dir
            .join("action_intents")
            .join(format!("{date}.jsonl"))
    }

    async fn append_audit_record(
        &self,
        intent: &ActionIntent,
        verdict: &ActionPolicyVerdict,
        message: &str,
    ) -> anyhow::Result<String> {
        let path = self.audit_path();

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?;

        let record = serde_json::json!({
            "recorded_at": Utc::now().to_rfc3339(),
            "operator": self.name(),
            "intent": intent,
            "policy_verdict": verdict,
            "executed": false,
            "message": message,
        });

        file.write_all(record.to_string().as_bytes()).await?;
        file.write_all(b"\n").await?;
        Ok(path.to_string_lossy().into_owned())
    }
}

impl ActionOperator for NoopOperator {
    fn name(&self) -> &'static str {
        "noop"
    }

    fn apply<'a>(
        &'a self,
        intent: &'a ActionIntent,
        verdict: Option<&'a ActionPolicyVerdict>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ActionResult>> + Send + 'a>> {
        Box::pin(async move {
            let verdict = verdict.ok_or_else(|| anyhow::anyhow!("policy verdict required"))?;

            let message = if verdict.allowed {
                "external action execution is disabled by default"
            } else {
                verdict.reason.as_str()
            };

            let audit_record_path = Some(self.append_audit_record(intent, verdict, message).await?);

            Ok(ActionResult {
                executed: false,
                message: message.to_string(),
                audit_record_path,
            })
        })
    }
}

/// Factory for creating tools sourced from an MCP server configuration.
///
/// Implementations inspect [`McpConfig`] and instantiate the appropriate
/// `mcp_*`-prefixed tool wrappers.  The `NoopMcpToolProvider` is used when
/// no MCP integration is configured.
pub trait McpToolProvider: Send + Sync {
    /// Instantiate all MCP-backed tools from the given config and security policy.
    fn create_mcp_tools(&self, config: &McpConfig, security: &SecurityPolicy)
    -> Vec<Box<dyn Tool>>;
}

/// No-op [`McpToolProvider`] that always returns an empty tool list.
///
/// Used as the default when no MCP integration is wired into the runtime.
#[derive(Debug, Default)]
pub struct NoopMcpToolProvider;

impl NoopMcpToolProvider {
    /// Create a new no-op MCP tool provider.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl McpToolProvider for NoopMcpToolProvider {
    fn create_mcp_tools(
        &self,
        _config: &McpConfig,
        _security: &SecurityPolicy,
    ) -> Vec<Box<dyn Tool>> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{
        AttachmentSource, OutputAttachment, ToolResult, ToolResultCompactionTarget,
        ToolResultSemanticStats, ToolResultSemanticStreamMode, ToolResultTextField,
    };

    #[test]
    fn tool_result_serde_defaults_attachments_when_missing() {
        let raw = json!({
            "success": true,
            "output": "ok",
            "error": null
        });

        let parsed: ToolResult = serde_json::from_value(raw).unwrap();
        assert!(parsed.attachments.is_empty());
        assert!(parsed.semantic.output_kind.is_none());
        assert_eq!(
            parsed.semantic.compaction_target,
            ToolResultCompactionTarget::Output
        );
        assert_eq!(
            parsed.semantic.stream_mode,
            ToolResultSemanticStreamMode::PerField
        );
        assert!(parsed.semantic.stats.is_none());
        assert!(parsed.semantic.raw_command.is_none());
        assert!(parsed.semantic.source_fields.is_empty());
        assert!(parsed.semantic.artifacts.is_empty());
    }

    #[test]
    fn tool_result_serde_roundtrip_with_empty_attachments() {
        let result = ToolResult {
            success: true,
            output: "ok".to_string(),
            error: None,
            attachments: Vec::new(),
            taint_labels: Vec::new(),
            semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
        };

        let json = serde_json::to_value(&result).unwrap();
        let parsed: ToolResult = serde_json::from_value(json).unwrap();
        assert!(parsed.success);
        assert!(parsed.attachments.is_empty());
    }

    #[test]
    fn tool_result_serde_does_not_serialize_internal_semantic_metadata() {
        let result = ToolResult::success("ok")
            .with_output_kind("shell_command")
            .with_compaction_target(ToolResultCompactionTarget::OutputAndError);

        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("semantic").is_none());
        assert!(json.get("semantic_metadata").is_none());
        assert!(json.get("semantic_stats").is_none());

        let parsed: ToolResult = serde_json::from_value(json).unwrap();
        assert!(parsed.semantic.output_kind.is_none());
        assert_eq!(
            parsed.semantic.compaction_target,
            ToolResultCompactionTarget::Output
        );
        assert_eq!(
            parsed.semantic.stream_mode,
            ToolResultSemanticStreamMode::PerField
        );
        assert!(parsed.semantic.stats.is_none());
        assert!(parsed.semantic.raw_command.is_none());
        assert!(parsed.semantic.source_fields.is_empty());
        assert!(parsed.semantic.artifacts.is_empty());
    }

    #[test]
    fn tool_result_helpers_populate_internal_semantic_stats() {
        let result = ToolResult::failure("boom\nmore")
            .with_output_kind("shell_command")
            .with_compaction_target(ToolResultCompactionTarget::OutputAndError);

        assert_eq!(
            result.semantic.output_kind.as_deref(),
            Some("shell_command")
        );
        assert_eq!(
            result.semantic.compaction_target,
            ToolResultCompactionTarget::OutputAndError
        );
        let stats = result.semantic.stats.as_ref().unwrap();
        assert_eq!(stats.output_bytes, 0);
        assert_eq!(stats.output_lines, 0);
        assert_eq!(stats.error_bytes, "boom\nmore".len());
        assert_eq!(stats.error_lines, 2);
    }

    #[test]
    fn tool_result_builder_order_does_not_clobber_explicit_stats() {
        let explicit = ToolResultSemanticStats {
            output_bytes: 7,
            output_lines: 3,
            error_bytes: 11,
            error_lines: 5,
        };

        let result = ToolResult::default()
            .with_semantic_stats(explicit.clone())
            .with_output_kind("shell_command")
            .with_compaction_target(ToolResultCompactionTarget::OutputAndError);

        assert_eq!(
            result.semantic.output_kind.as_deref(),
            Some("shell_command")
        );
        assert_eq!(
            result.semantic.compaction_target,
            ToolResultCompactionTarget::OutputAndError
        );
        assert_eq!(result.semantic.stats, Some(explicit));
    }

    #[test]
    fn tool_result_success_helper_populates_stats_without_target_override() {
        let result = ToolResult::success("ok")
            .with_output_kind("shell_command")
            .with_compaction_target(ToolResultCompactionTarget::OutputAndError);

        assert_eq!(
            result.semantic.output_kind.as_deref(),
            Some("shell_command")
        );
        assert_eq!(
            result.semantic.compaction_target,
            ToolResultCompactionTarget::OutputAndError
        );
        let stats = result.semantic.stats.as_ref().unwrap();
        assert_eq!(stats.output_bytes, 2);
        assert_eq!(stats.output_lines, 1);
        assert_eq!(stats.error_bytes, 0);
        assert_eq!(stats.error_lines, 0);
    }

    #[test]
    fn semantic_metadata_helpers_track_internal_source_fields_and_artifacts() {
        let mut result = ToolResult::success("ok");
        result.semantic = result
            .semantic
            .clone()
            .with_raw_command("git status")
            .with_source_fields([ToolResultTextField::Output, ToolResultTextField::Error]);
        result.semantic.record_artifact(
            ToolResultTextField::Output,
            "shell/output.txt",
            PathBuf::from("/tmp/output.txt"),
        );

        assert_eq!(result.semantic.raw_command.as_deref(), Some("git status"));
        assert_eq!(
            result
                .semantic
                .source_fields
                .iter()
                .map(|field| field.field.as_str())
                .collect::<Vec<_>>(),
            vec!["output", "error"]
        );
        assert_eq!(result.semantic.artifacts.len(), 1);

        result.semantic.clear_artifact(ToolResultTextField::Output);
        assert!(result.semantic.artifacts.is_empty());
    }

    #[test]
    fn tool_result_serde_roundtrip_with_attachments() {
        let result = ToolResult {
            success: true,
            output: "done".to_string(),
            error: None,
            attachments: vec![OutputAttachment::from_path(
                "image/png",
                "/tmp/chart.png",
                Some("chart.png".to_string()),
            )],
            taint_labels: Vec::new(),
            semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
        };

        let json = serde_json::to_value(&result).unwrap();
        let parsed: ToolResult = serde_json::from_value(json).unwrap();
        assert_eq!(parsed.attachments.len(), 1);
        assert!(matches!(
            &parsed.attachments[0].source,
            AttachmentSource::File { path } if path == "/tmp/chart.png"
        ));
    }

    #[test]
    fn output_attachment_from_path_sets_path_only() {
        let attachment = OutputAttachment::from_path(
            "image/png",
            "/tmp/image.png",
            Some("image.png".to_string()),
        );

        assert_eq!(attachment.mime_type, "image/png");
        assert!(matches!(
            &attachment.source,
            AttachmentSource::File { path } if path == "/tmp/image.png"
        ));
    }

    #[test]
    fn output_attachment_from_url_sets_url_only() {
        let attachment = OutputAttachment::from_url(
            "image/png",
            "https://example.com/image.png",
            Some("image.png".to_string()),
        );

        assert_eq!(attachment.mime_type, "image/png");
        assert!(matches!(
            &attachment.source,
            AttachmentSource::Url { url } if url == "https://example.com/image.png"
        ));
    }

    #[test]
    fn output_attachment_serde_roundtrip_path_variant() {
        let attachment =
            OutputAttachment::from_path("text/plain", "/tmp/out.txt", Some("out.txt".to_string()));

        let json = serde_json::to_value(&attachment).unwrap();
        let parsed: OutputAttachment = serde_json::from_value(json).unwrap();
        assert!(matches!(
            &parsed.source,
            AttachmentSource::File { path } if path == "/tmp/out.txt"
        ));
    }

    #[test]
    fn output_attachment_serde_roundtrip_url_variant() {
        let attachment =
            OutputAttachment::from_url("application/pdf", "https://example.com/report.pdf", None);

        let json = serde_json::to_value(&attachment).unwrap();
        let parsed: OutputAttachment = serde_json::from_value(json).unwrap();
        assert!(matches!(
            &parsed.source,
            AttachmentSource::Url { url } if url == "https://example.com/report.pdf"
        ));
    }
}
