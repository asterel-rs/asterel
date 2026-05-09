mod artifacts;
mod classifier;
mod formatters;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

use super::{ExecutionContext, MiddlewareDecision, ToolMiddleware};
use crate::core::tools::traits::{
    ToolResult, ToolResultCompactionTarget, ToolResultSemanticMetadata,
    ToolResultSemanticStreamMode, ToolResultTextField,
};

pub use classifier::classify_shell_command_output_kind;
pub(crate) use formatters::builtin_formatter_registry;

use self::artifacts::{PersistedSemanticArtifact, persist_semantic_artifact};

/// Character count above which semantic compaction is considered.
pub const SEMANTIC_COMPACTION_THRESHOLD_CHARS: usize = 8_000;

/// Minimum formatter confidence required to replace raw output.
pub const SEMANTIC_COMPACTION_CONFIDENCE_FLOOR: f32 = 0.80;

/// Formatter result for registry-driven semantic compaction.
#[derive(Debug, Clone, PartialEq)]
pub enum SemanticCompactionOutcome {
    /// Leave raw text unchanged without treating the formatter as a parse fallback.
    Passthrough,
    /// Replace the raw text with a meaning-preserving compacted form.
    Compacted { content: String, confidence: f32 },
    /// Formatter could not safely preserve meaning and requests raw fallback.
    FallbackRaw,
}

/// Meaning-preserving reducer for a specific normalized output kind.
pub trait SemanticFormatter: Send + Sync + std::fmt::Debug {
    fn compact(
        &self,
        raw: &str,
        metadata: &ToolResultSemanticMetadata,
    ) -> SemanticCompactionOutcome;
}

/// Registry keyed by normalized `ToolResult.semantic.output_kind`.
#[derive(Debug, Clone, Default)]
pub struct SemanticFormatterRegistry {
    formatters: HashMap<String, Arc<dyn SemanticFormatter>>,
}

impl SemanticFormatterRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn from_formatters<I, K>(formatters: I) -> Self
    where
        I: IntoIterator<Item = (K, Arc<dyn SemanticFormatter>)>,
        K: Into<String>,
    {
        let mut registry = Self::new();
        for (output_kind, formatter) in formatters {
            registry.register(output_kind, formatter);
        }
        registry
    }

    pub fn register(
        &mut self,
        output_kind: impl Into<String>,
        formatter: Arc<dyn SemanticFormatter>,
    ) {
        self.formatters.insert(output_kind.into(), formatter);
    }

    fn get(&self, output_kind: &str) -> Option<&dyn SemanticFormatter> {
        self.formatters.get(output_kind).map(Arc::as_ref)
    }
}

/// Registry-driven semantic compaction before generic head/tail compaction.
#[derive(Debug, Clone)]
pub struct SemanticCompactionMiddleware {
    registry: SemanticFormatterRegistry,
}

impl SemanticCompactionMiddleware {
    #[must_use]
    pub fn new(registry: SemanticFormatterRegistry) -> Self {
        Self { registry }
    }
}

impl Default for SemanticCompactionMiddleware {
    fn default() -> Self {
        Self::new(builtin_formatter_registry())
    }
}

impl ToolMiddleware for SemanticCompactionMiddleware {
    fn before_execute<'a>(
        &'a self,
        _tool_name: &'a str,
        _args: &'a Value,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<MiddlewareDecision>> + Send + 'a>> {
        Box::pin(async move { Ok(MiddlewareDecision::Continue) })
    }

    fn after_execute<'a>(
        &'a self,
        tool_name: &'a str,
        result: &'a mut ToolResult,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let Some(output_kind) = result.semantic.output_kind.clone() else {
                return;
            };
            if result.semantic.stats.is_none() {
                return;
            }
            let Some(formatter) = self.registry.get(&output_kind) else {
                return;
            };

            let fields = semantic_compaction_fields(result);
            if fields.is_empty() {
                return;
            }

            if result.semantic.stream_mode == ToolResultSemanticStreamMode::CombinedOutputAndError
                && result.semantic.compaction_target == ToolResultCompactionTarget::OutputAndError
                && fields.len() > 1
            {
                apply_combined_semantic_compaction(
                    &fields,
                    tool_name,
                    &output_kind,
                    formatter,
                    result,
                    ctx,
                )
                .await;
                return;
            }

            for field in fields {
                apply_semantic_compaction(field, tool_name, &output_kind, formatter, result, ctx)
                    .await;
            }
        })
    }
}

fn semantic_compaction_fields(result: &ToolResult) -> Vec<ToolResultTextField> {
    const OUTPUT_ONLY: &[ToolResultTextField] = &[ToolResultTextField::Output];
    const ERROR_ONLY: &[ToolResultTextField] = &[ToolResultTextField::Error];
    const OUTPUT_AND_ERROR: &[ToolResultTextField] =
        &[ToolResultTextField::Output, ToolResultTextField::Error];

    let fields = match result.semantic.compaction_target {
        ToolResultCompactionTarget::Output => OUTPUT_ONLY,
        ToolResultCompactionTarget::Error => ERROR_ONLY,
        ToolResultCompactionTarget::OutputAndError => OUTPUT_AND_ERROR,
    };

    fields
        .iter()
        .copied()
        .filter(|field| semantic_metadata_allows_field(&result.semantic, *field))
        .filter(|field| semantic_field_text(result, *field).is_some())
        .collect()
}

fn semantic_metadata_allows_field(
    metadata: &ToolResultSemanticMetadata,
    field: ToolResultTextField,
) -> bool {
    metadata.source_fields.is_empty()
        || metadata
            .source_fields
            .iter()
            .any(|source_field| source_field.field == field)
}

fn semantic_field_text(result: &ToolResult, field: ToolResultTextField) -> Option<&str> {
    match field {
        ToolResultTextField::Output => {
            (!result.output.is_empty()).then_some(result.output.as_str())
        }
        ToolResultTextField::Error => result.error.as_deref().filter(|error| !error.is_empty()),
    }
}

async fn apply_semantic_compaction(
    field: ToolResultTextField,
    tool_name: &str,
    output_kind: &str,
    formatter: &dyn SemanticFormatter,
    result: &mut ToolResult,
    ctx: &ExecutionContext,
) {
    let Some(raw) = semantic_field_text(result, field).map(ToOwned::to_owned) else {
        return;
    };
    let Some((measured_chars, original_bytes)) = semantic_measurement(&raw) else {
        return;
    };

    log_semantic_attempt(
        tool_name,
        output_kind,
        field.as_str(),
        original_bytes,
        measured_chars,
    );

    handle_field_compaction_outcome(
        field,
        tool_name,
        output_kind,
        formatter.compact(&raw, &result.semantic),
        result,
        ctx,
        &raw,
        original_bytes,
    )
    .await;
}

async fn apply_combined_semantic_compaction(
    fields: &[ToolResultTextField],
    tool_name: &str,
    output_kind: &str,
    formatter: &dyn SemanticFormatter,
    result: &mut ToolResult,
    ctx: &ExecutionContext,
) {
    let raw_fields = fields
        .iter()
        .filter_map(|field| {
            semantic_field_text(result, *field).map(|text| (*field, text.to_string()))
        })
        .collect::<Vec<_>>();
    if raw_fields.len() < 2 {
        return;
    }

    let raw = raw_fields
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let Some((measured_chars, _)) = semantic_measurement(&raw) else {
        return;
    };

    let original_bytes = raw_fields.iter().map(|(_, text)| text.len()).sum::<usize>();
    log_semantic_attempt(
        tool_name,
        output_kind,
        "output+error",
        original_bytes,
        measured_chars,
    );

    handle_combined_compaction_outcome(
        fields,
        tool_name,
        output_kind,
        formatter.compact(&raw, &result.semantic),
        result,
        ctx,
        &raw_fields,
        original_bytes,
    )
    .await;
}

fn combined_compaction_destination(
    result: &ToolResult,
    fields: &[ToolResultTextField],
) -> ToolResultTextField {
    if !result.success && fields.contains(&ToolResultTextField::Error) {
        ToolResultTextField::Error
    } else if fields.contains(&ToolResultTextField::Output) {
        ToolResultTextField::Output
    } else {
        ToolResultTextField::Error
    }
}

fn apply_combined_compacted_content(
    result: &mut ToolResult,
    fields: &[ToolResultTextField],
    destination: ToolResultTextField,
    content: &str,
) {
    for field in fields {
        match (*field, destination) {
            (ToolResultTextField::Output, ToolResultTextField::Output) => {
                result.output.clear();
                result.output.push_str(content);
            }
            (ToolResultTextField::Error, ToolResultTextField::Error) => {
                result.error = Some(content.to_owned());
            }
            (ToolResultTextField::Output, ToolResultTextField::Error) => {
                result.output.clear();
            }
            (ToolResultTextField::Error, ToolResultTextField::Output) => {
                result.error = None;
            }
        }
    }
}

fn semantic_measurement(raw: &str) -> Option<(usize, usize)> {
    let measured_chars = raw.chars().count();
    (measured_chars > SEMANTIC_COMPACTION_THRESHOLD_CHARS).then_some((measured_chars, raw.len()))
}

fn log_semantic_attempt(
    tool_name: &str,
    output_kind: &str,
    field: &str,
    original_bytes: usize,
    original_chars: usize,
) {
    tracing::debug!(
        tool = tool_name,
        output_kind,
        field,
        original_bytes,
        original_chars,
        outcome = "attempt",
        "tool output semantic compaction attempted"
    );
}

fn log_semantic_declined(
    tool_name: &str,
    output_kind: &str,
    field: &str,
    outcome: &str,
    bytes: usize,
) {
    tracing::debug!(
        tool = tool_name,
        output_kind,
        field,
        original_bytes = bytes,
        compacted_bytes = 0usize,
        saved_bytes = 0usize,
        artifact_write_status = "not_written",
        outcome,
        "tool output semantic compaction declined"
    );
}

fn log_low_confidence(
    tool_name: &str,
    output_kind: &str,
    field: &str,
    original_bytes: usize,
    compacted_bytes: usize,
    confidence: f32,
) {
    tracing::debug!(
        tool = tool_name,
        output_kind,
        field,
        original_bytes,
        compacted_bytes,
        saved_bytes = 0usize,
        confidence,
        artifact_write_status = "not_written",
        outcome = "fallback_low_confidence",
        "tool output semantic compaction rejected due to low confidence"
    );
}

fn log_compaction_success(
    tool_name: &str,
    output_kind: &str,
    field: &str,
    original_bytes: usize,
    compacted_bytes: usize,
    saved_bytes: usize,
    confidence: f32,
    artifact_write_status: &str,
) {
    tracing::debug!(
        tool = tool_name,
        output_kind,
        field,
        original_bytes,
        compacted_bytes,
        saved_bytes,
        confidence,
        artifact_write_status,
        outcome = "compacted",
        "tool output semantically compacted"
    );
}

async fn handle_field_compaction_outcome(
    field: ToolResultTextField,
    tool_name: &str,
    output_kind: &str,
    outcome: SemanticCompactionOutcome,
    result: &mut ToolResult,
    ctx: &ExecutionContext,
    raw: &str,
    original_bytes: usize,
) {
    match outcome {
        SemanticCompactionOutcome::Passthrough => {
            log_semantic_declined(
                tool_name,
                output_kind,
                field.as_str(),
                "passthrough",
                original_bytes,
            );
        }
        SemanticCompactionOutcome::FallbackRaw => {
            log_semantic_declined(
                tool_name,
                output_kind,
                field.as_str(),
                "fallback_raw",
                original_bytes,
            );
        }
        SemanticCompactionOutcome::Compacted {
            content,
            confidence,
        } => {
            if !confidence.is_finite()
                || confidence < SEMANTIC_COMPACTION_CONFIDENCE_FLOOR
                || content.trim().is_empty()
            {
                log_low_confidence(
                    tool_name,
                    output_kind,
                    field.as_str(),
                    original_bytes,
                    content.len(),
                    confidence,
                );
                return;
            }

            match persist_semantic_artifact(&ctx.workspace_dir, tool_name, output_kind, field, raw)
                .await
            {
                Ok(artifact) => {
                    let compacted_bytes = content.len();
                    let saved_bytes = original_bytes.saturating_sub(compacted_bytes);
                    match field {
                        ToolResultTextField::Output => result.output = content,
                        ToolResultTextField::Error => result.error = Some(content),
                    }
                    result
                        .semantic
                        .record_artifact(field, artifact.key, artifact.path);
                    log_compaction_success(
                        tool_name,
                        output_kind,
                        field.as_str(),
                        original_bytes,
                        compacted_bytes,
                        saved_bytes,
                        confidence,
                        "written",
                    );
                }
                Err(error) => {
                    result.semantic.clear_artifact(field);
                    tracing::warn!(
                        tool = tool_name,
                        output_kind,
                        field = field.as_str(),
                        original_bytes,
                        compacted_bytes = content.len(),
                        saved_bytes = original_bytes.saturating_sub(content.len()),
                        confidence,
                        artifact_write_status = "failed",
                        outcome = "fallback_raw",
                        error = %error,
                        "failed to persist semantic compaction raw artifact; keeping raw output"
                    );
                }
            }
        }
    }
}

async fn handle_combined_compaction_outcome(
    fields: &[ToolResultTextField],
    tool_name: &str,
    output_kind: &str,
    outcome: SemanticCompactionOutcome,
    result: &mut ToolResult,
    ctx: &ExecutionContext,
    raw_fields: &[(ToolResultTextField, String)],
    original_bytes: usize,
) {
    match outcome {
        SemanticCompactionOutcome::Passthrough => {
            log_semantic_declined(
                tool_name,
                output_kind,
                "output+error",
                "passthrough",
                original_bytes,
            );
        }
        SemanticCompactionOutcome::FallbackRaw => {
            log_semantic_declined(
                tool_name,
                output_kind,
                "output+error",
                "fallback_raw",
                original_bytes,
            );
        }
        SemanticCompactionOutcome::Compacted {
            content,
            confidence,
        } => {
            if confidence < SEMANTIC_COMPACTION_CONFIDENCE_FLOOR {
                log_low_confidence(
                    tool_name,
                    output_kind,
                    "output+error",
                    original_bytes,
                    content.len(),
                    confidence,
                );
                return;
            }

            let compacted_bytes = content.len();
            let saved_bytes = original_bytes.saturating_sub(compacted_bytes);
            let artifacts = persist_combined_artifacts(
                tool_name,
                output_kind,
                ctx,
                raw_fields,
                compacted_bytes,
                saved_bytes,
                confidence,
            )
            .await;
            let Ok(artifacts) = artifacts else {
                tracing::warn!(
                    tool = tool_name,
                    output_kind,
                    field = "output+error",
                    original_bytes,
                    compacted_bytes,
                    saved_bytes,
                    confidence,
                    artifact_write_status = "failed",
                    outcome = "fallback_raw",
                    "failed to persist all semantic compaction raw artifacts; keeping raw output"
                );
                return;
            };

            let destination = combined_compaction_destination(result, fields);
            apply_combined_compacted_content(result, fields, destination, &content);
            for (field, artifact) in artifacts {
                result
                    .semantic
                    .record_artifact(field, artifact.key, artifact.path);
            }
            log_compaction_success(
                tool_name,
                output_kind,
                "output+error",
                original_bytes,
                compacted_bytes,
                saved_bytes,
                confidence,
                "written",
            );
        }
    }
}

async fn persist_combined_artifacts(
    tool_name: &str,
    output_kind: &str,
    ctx: &ExecutionContext,
    raw_fields: &[(ToolResultTextField, String)],
    compacted_bytes: usize,
    saved_bytes: usize,
    confidence: f32,
) -> anyhow::Result<Vec<(ToolResultTextField, PersistedSemanticArtifact)>> {
    let mut artifacts = Vec::with_capacity(raw_fields.len());
    for (field, source_text) in raw_fields {
        match persist_semantic_artifact(
            &ctx.workspace_dir,
            tool_name,
            output_kind,
            *field,
            source_text,
        )
        .await
        {
            Ok(artifact) => {
                artifacts.push((*field, artifact));
            }
            Err(error) => {
                tracing::warn!(
                    tool = tool_name,
                    output_kind,
                    field = field.as_str(),
                    original_bytes = source_text.len(),
                    compacted_bytes,
                    saved_bytes,
                    confidence,
                    artifact_write_status = "failed",
                    outcome = "compacted",
                    error = %error,
                    "failed to persist semantic compaction raw artifact"
                );
                return Err(error);
            }
        }
    }
    Ok(artifacts)
}
