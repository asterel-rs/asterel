//! Policy middleware — positions 3–8 in the default middleware chain.
//!
//! Each struct in this module implements one focused policy concern.
//! They all run *after* `SecurityMiddleware` and `HookMiddleware` and are
//! applied in this order:
//!
//! | # | Middleware | Phase | Concern |
//! |---|-----------|-------|---------|
//! | 3 | [`EntityRateLimitMiddleware`] | before | Per-entity, burst, conversation, workspace caps |
//! | 4 | [`AuditMiddleware`] | before + after | `tracing` spans |
//! | 5 | [`ToolOutputCompactionMiddleware`] | after | Head+tail compaction at 8 000 chars |
//! | 6 | [`OutputSizeLimitMiddleware`] | after | Hard ceiling at 256 KB / 4 000 lines |
//! | 7 | [`ToolResultSanitizationMiddleware`] | after | External-content markers (prompt-injection defence) |
//! | 8 | [`SecretScrubMiddleware`] | after | API-key / token redaction |

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use super::{ExecutionContext, MiddlewareDecision, ToolMiddleware};
use crate::config::ExternalKnowledgeTrustConfig;
use crate::contracts::strings::verdicts::{
    SECURITY_BLOCK_GLOBAL_ACTION_LIMIT_EXCEEDED, SECURITY_POLICY_BLOCK_PREFIX,
    TOOL_ERROR_BLOCKED_BY_EXTERNAL_CONTENT_POLICY, TOOL_OUTPUT_BLOCKED_BY_EXTERNAL_CONTENT_POLICY,
};
use crate::core::providers::scrub_secrets;
use crate::core::tools::traits::ToolResult;
use crate::security::external_content::{ExternalAction, prepare_content_with_trust};
use crate::security::policy::RateLimitError;

/// Middleware (position 3) that enforces per-entity and global rate limits.
///
/// Checks are delegated to [`EntityRateLimiter::check_and_record`], which
/// covers entity-level, burst, conversation, and workspace buckets.  Any
/// exhausted bucket results in a `Block` decision with a policy-prefixed
/// message.  The `after_execute` hook is a no-op.
#[derive(Debug)]
pub struct EntityRateLimitMiddleware;

impl ToolMiddleware for EntityRateLimitMiddleware {
    fn before_execute<'a>(
        &'a self,
        _tool_name: &'a str,
        _args: &'a Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<MiddlewareDecision>> + Send + 'a>> {
        Box::pin(async move {
            let conversation_id = ctx
                .session_id
                .as_deref()
                .or(ctx.source_channel_id.as_deref());
            let workspace_id = ctx.workspace_dir.to_string_lossy();
            match ctx.rate_limiter.check_and_record_scoped(
                ctx.entity_id.as_str(),
                conversation_id,
                Some(workspace_id.as_ref()),
            ) {
                Ok(()) => Ok(MiddlewareDecision::Continue),
                Err(RateLimitError::GlobalExhausted) => Ok(MiddlewareDecision::Block(
                    SECURITY_BLOCK_GLOBAL_ACTION_LIMIT_EXCEEDED.to_string(),
                )),
                Err(RateLimitError::EntityExhausted { entity_id }) => {
                    Ok(MiddlewareDecision::Block(format!(
                        "{SECURITY_POLICY_BLOCK_PREFIX}entity action limit exceeded for '{entity_id}'"
                    )))
                }
                Err(RateLimitError::ConversationExhausted { conversation_id }) => {
                    Ok(MiddlewareDecision::Block(format!(
                        "{SECURITY_POLICY_BLOCK_PREFIX}conversation action limit exceeded for '{conversation_id}'"
                    )))
                }
                Err(RateLimitError::WorkspaceExhausted { workspace_id }) => {
                    Ok(MiddlewareDecision::Block(format!(
                        "{SECURITY_POLICY_BLOCK_PREFIX}workspace action limit exceeded for '{workspace_id}'"
                    )))
                }
                Err(RateLimitError::BurstExhausted { entity_id }) => Ok(MiddlewareDecision::Block(
                    format!("{SECURITY_POLICY_BLOCK_PREFIX}burst limit exceeded for '{entity_id}'"),
                )),
            }
        })
    }

    fn after_execute<'a>(
        &'a self,
        _tool_name: &'a str,
        _result: &'a mut ToolResult,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {})
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::EntityRateLimitMiddleware;
    use crate::core::tools::middleware::{ExecutionContext, MiddlewareDecision, ToolMiddleware};
    use crate::security::{EntityRateLimiter, SecurityPolicy};

    #[tokio::test]
    async fn entity_rate_limit_middleware_enforces_conversation_scope() {
        let limiter = Arc::new(EntityRateLimiter::new_with_scopes(100, 100, 1, 100, 0, 0));
        let mut first = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        first.rate_limiter = Arc::clone(&limiter);
        first.session_id = Some("conversation-a".to_string());
        first.entity_id = "entity-a".into();
        let mut second = first.clone();
        second.entity_id = "entity-b".into();

        let middleware = EntityRateLimitMiddleware;
        assert!(matches!(
            middleware
                .before_execute("test", &json!({}), &first)
                .await
                .expect("first action should be evaluated"),
            MiddlewareDecision::Continue
        ));
        assert!(matches!(
            middleware
                .before_execute("test", &json!({}), &second)
                .await
                .expect("second action should be evaluated"),
            MiddlewareDecision::Block(message) if message.contains("conversation action limit")
        ));
    }

    #[tokio::test]
    async fn entity_rate_limit_middleware_enforces_workspace_scope() {
        let limiter = Arc::new(EntityRateLimiter::new_with_scopes(100, 100, 100, 1, 0, 0));
        let mut first = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        first.rate_limiter = Arc::clone(&limiter);
        first.workspace_dir = "/tmp/asterel-rate-limit-workspace".into();
        first.entity_id = "entity-a".into();
        let mut second = first.clone();
        second.entity_id = "entity-b".into();
        second.session_id = Some("conversation-b".to_string());

        let middleware = EntityRateLimitMiddleware;
        assert!(matches!(
            middleware
                .before_execute("test", &json!({}), &first)
                .await
                .expect("first action should be evaluated"),
            MiddlewareDecision::Continue
        ));
        assert!(matches!(
            middleware
                .before_execute("test", &json!({}), &second)
                .await
                .expect("second action should be evaluated"),
            MiddlewareDecision::Block(message) if message.contains("workspace action limit")
        ));
    }
}

/// Middleware (position 4) that emits structured `tracing` spans for every
/// tool invocation.
///
/// Logs `INFO` events on `before_execute` ("tool execution started") and
/// `after_execute` ("tool execution finished") with tool name, entity ID,
/// turn number, and outcome.  Does not block or transform results.
#[derive(Debug)]
pub struct AuditMiddleware;

impl ToolMiddleware for AuditMiddleware {
    fn before_execute<'a>(
        &'a self,
        tool_name: &'a str,
        _args: &'a Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<MiddlewareDecision>> + Send + 'a>> {
        Box::pin(async move {
            tracing::info!(
                tool = tool_name,
                entity_id = %ctx.entity_id,
                turn_number = ctx.turn_number,
                "tool execution started"
            );
            Ok(MiddlewareDecision::Continue)
        })
    }

    fn after_execute<'a>(
        &'a self,
        tool_name: &'a str,
        result: &'a mut ToolResult,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            tracing::info!(
                tool = tool_name,
                entity_id = %ctx.entity_id,
                turn_number = ctx.turn_number,
                success = result.success,
                has_error = result.error.is_some(),
                "tool execution finished"
            );
        })
    }
}

/// Middleware (position 6) that applies a hard ceiling on tool output size.
///
/// Applied *after* [`ToolOutputCompactionMiddleware`].  Truncates by line
/// count first, then by byte count, appending a metadata suffix that
/// describes the original size so the model understands truncation occurred.
/// Emits a `WARN` trace event when truncation fires.
#[derive(Debug)]
pub struct OutputSizeLimitMiddleware;

/// Hard ceiling for tool output in bytes (256 KB).
pub const MAX_TOOL_OUTPUT_BYTES: usize = 262_144;
/// Hard ceiling for tool output in lines.
pub const MAX_TOOL_OUTPUT_LINES: usize = 4_000;

impl ToolMiddleware for OutputSizeLimitMiddleware {
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
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let original_bytes = result.output.len();
            let original_lines = result.output.lines().count();

            let mut truncated = false;
            let mut output = result.output.clone();

            // Line-count check first: joining fewer lines can significantly
            // reduce byte size before the byte-count check runs.
            if original_lines > MAX_TOOL_OUTPUT_LINES {
                let mut truncated_output = String::new();
                for (i, line) in output.lines().enumerate() {
                    if i >= MAX_TOOL_OUTPUT_LINES {
                        break;
                    }
                    if i > 0 {
                        truncated_output.push('\n');
                    }
                    truncated_output.push_str(line);
                }
                output = truncated_output;
                truncated = true;
            }

            // Byte-count check: walk back from the limit to the nearest UTF-8
            // character boundary to avoid splitting a multi-byte sequence.
            if output.len() > MAX_TOOL_OUTPUT_BYTES {
                let mut byte_pos = MAX_TOOL_OUTPUT_BYTES;
                while byte_pos > 0 && !output.is_char_boundary(byte_pos) {
                    byte_pos -= 1;
                }
                output.truncate(byte_pos);
                truncated = true;
            }

            if truncated {
                let metadata_suffix = format!(
                    "\n... [output truncated: {original_bytes} bytes/{original_lines} lines \u{2192} {MAX_TOOL_OUTPUT_BYTES} bytes/{MAX_TOOL_OUTPUT_LINES} lines max]"
                );
                output.push_str(&metadata_suffix);

                tracing::warn!(
                    tool = tool_name,
                    original_bytes = original_bytes,
                    original_lines = original_lines,
                    max_bytes = MAX_TOOL_OUTPUT_BYTES,
                    max_lines = MAX_TOOL_OUTPUT_LINES,
                    "tool output truncated due to size limits"
                );

                result.output = output;
            }
        })
    }
}

/// Character count above which output is compacted instead of passed through.
const COMPACTION_THRESHOLD_CHARS: usize = 8_000;

/// Characters kept from the head of a compacted output (structure / preamble).
const COMPACTION_HEAD_CHARS: usize = 400;

/// Characters kept from the tail of a compacted output (most recent content).
const COMPACTION_TAIL_CHARS: usize = 2_000;

/// Middleware (position 5) that intelligently compacts oversized tool output.
///
/// When output exceeds [`COMPACTION_THRESHOLD_CHARS`], the middle is pruned
/// and replaced with a `[... N chars pruned ...]` marker.  The head
/// (structure / preamble) and hot tail (most recent / relevant data) are
/// preserved because they are typically the most useful parts for the model.
///
/// Runs *before* [`OutputSizeLimitMiddleware`] so the hard byte ceiling sees
/// already-compacted output — reducing the chance that useful tail content is
/// lost to naive byte truncation.
///
/// Emits a `DEBUG` trace event when compaction fires.
#[derive(Debug)]
pub struct ToolOutputCompactionMiddleware;

impl ToolMiddleware for ToolOutputCompactionMiddleware {
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
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            if result.output.len() <= COMPACTION_THRESHOLD_CHARS {
                return;
            }
            let char_count = result.output.chars().count();
            if char_count <= COMPACTION_THRESHOLD_CHARS {
                return;
            }

            // Find safe byte boundaries for head and tail slices.
            let head_end = result
                .output
                .char_indices()
                .nth(COMPACTION_HEAD_CHARS)
                .map_or(result.output.len(), |(idx, _)| idx);

            let tail_start = result
                .output
                .char_indices()
                .rev()
                .nth(COMPACTION_TAIL_CHARS.saturating_sub(1))
                .map_or(0, |(idx, _)| idx);

            // Safety guard: if head and tail overlap (shouldn't happen given
            // current thresholds, but defensive) keep everything unchanged.
            if head_end >= tail_start {
                return;
            }

            let pruned_chars = char_count - COMPACTION_HEAD_CHARS - COMPACTION_TAIL_CHARS;
            let compacted = format!(
                "{}\n\n[... {} chars pruned for context efficiency ...]\n\n{}",
                &result.output[..head_end],
                pruned_chars,
                &result.output[tail_start..],
            );

            tracing::debug!(
                tool = tool_name,
                original_chars = char_count,
                head = COMPACTION_HEAD_CHARS,
                tail = COMPACTION_TAIL_CHARS,
                pruned = pruned_chars,
                "tool output compacted (head + tail)"
            );

            result.output = compacted;
        })
    }
}

/// Middleware (position 7) that wraps tool output in external-content markers.
///
/// Passes both `output` and `error` through `prepare_content_with_trust`,
/// which wraps the content in delimiters that signal to the model that the
/// text originates from an untrusted external source.  If the trust policy
/// classifies the content as `Block`, the result is overwritten with a policy
/// violation message.
///
/// This is the primary prompt-injection defence for tool outputs.
#[derive(Debug)]
pub struct ToolResultSanitizationMiddleware;

impl ToolMiddleware for ToolResultSanitizationMiddleware {
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
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let trust = ExternalKnowledgeTrustConfig::default();

            if !result.output.is_empty() {
                let prepared = prepare_content_with_trust(
                    &format!("tool:{tool_name}:output"),
                    &result.output,
                    &trust,
                );
                result.output = prepared.model_input;

                if prepared.action == ExternalAction::Block {
                    result.success = false;
                    result.error = Some(TOOL_OUTPUT_BLOCKED_BY_EXTERNAL_CONTENT_POLICY.to_string());
                }
            }

            if let Some(existing_error) = result.error.take() {
                let prepared = prepare_content_with_trust(
                    &format!("tool:{tool_name}:error"),
                    &existing_error,
                    &trust,
                );
                if prepared.action == ExternalAction::Block {
                    result.success = false;
                    result.error = Some(TOOL_ERROR_BLOCKED_BY_EXTERNAL_CONTENT_POLICY.to_string());
                } else {
                    result.error = Some(prepared.model_input);
                }
            }
        })
    }
}

/// Middleware (position 8) that redacts secrets from tool output and error text.
///
/// Calls `scrub_secrets` on both `output` and `error` fields.  The scrubber
/// applies regex patterns matching common API key and token formats (e.g.
/// `sk-`, `ghp_`, bearer tokens) and replaces matches with a redaction marker.
///
/// Runs last in the output-shaping sequence so it sees the fully compacted
/// and sanitised content.
#[derive(Debug)]
pub struct SecretScrubMiddleware;

impl ToolMiddleware for SecretScrubMiddleware {
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
        _tool_name: &'a str,
        result: &'a mut ToolResult,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            result.output = scrub_secrets(&result.output).into_owned();
            result.error = result
                .error
                .as_deref()
                .map(scrub_secrets)
                .map(std::borrow::Cow::into_owned);
        })
    }
}
