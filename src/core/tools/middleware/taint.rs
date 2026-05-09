//! Taint-tracking middleware (position 9, last in the chain).
//!
//! # Purpose
//!
//! Taint tracking records the provenance of data flowing through tool
//! executions.  When a tool produces output, this middleware annotates the
//! [`ToolResult`] with taint labels derived from:
//!
//! 1. The **input taint context** (`ctx.taint_context`) — labels already
//!    carried by the session at the time of the call (e.g. `user_input`,
//!    `pii` from a previous turn).
//! 2. The **tool's inherent taint** — determined by propagation rules in
//!    `crate::security::taint::propagation` (e.g. network tools like
//!    `web_fetch` and `browser` add `external_network`).
//!
//! The final label set is the union of both sources.
//!
//! # Placement
//!
//! Running last means taint labels are applied *after* all output
//! transformations (compaction, truncation, sanitisation, scrubbing), so
//! they accurately reflect the final content the model will see.
//!
//! Labels are stored in `ToolResult::taint_labels` and propagated into the
//! next session turn by the agent loop when it updates `ctx.taint_context`.

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use super::{ExecutionContext, MiddlewareDecision, ToolMiddleware};
use crate::core::tools::traits::ToolResult;
use crate::security::taint::propagation;

/// Last middleware in the chain: propagates taint labels to tool results.
///
/// `before_execute` logs the incoming taint context at `DEBUG` level and
/// always returns [`MiddlewareDecision::Continue`] (taint never blocks).
///
/// `after_execute` calls the propagation engine and writes the resulting
/// label set into `result.taint_labels`.  Empty sets are not written to
/// avoid unnecessary allocations.
#[derive(Debug)]
pub struct TaintMiddleware;

impl ToolMiddleware for TaintMiddleware {
    fn before_execute<'a>(
        &'a self,
        tool_name: &'a str,
        _args: &'a Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<MiddlewareDecision>> + Send + 'a>> {
        Box::pin(async move {
            let input_taints = ctx
                .taint_context
                .as_ref()
                .map_or_else(|| "(none)".to_string(), ToString::to_string);

            tracing::debug!(
                tool = tool_name,
                entity_id = %ctx.entity_id,
                input_taints = %input_taints,
                "taint context before tool execution"
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
            let input_taints = ctx.taint_context.clone().unwrap_or_default();

            let output_taints = propagation::propagate(&input_taints, tool_name);

            if !output_taints.is_empty() {
                for label in output_taints.to_string_vec() {
                    if !result.taint_labels.contains(&label) {
                        result.taint_labels.push(label);
                    }
                }
                tracing::debug!(
                    tool = tool_name,
                    taint_labels = ?result.taint_labels,
                    "taint labels applied to tool result"
                );
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::security::SecurityPolicy;
    use crate::security::taint::label::{TaintLabel, TaintSet};

    fn test_ctx() -> ExecutionContext {
        let security = Arc::new(SecurityPolicy::default());
        ExecutionContext::test_default(security)
    }

    #[tokio::test]
    async fn taint_middleware_continues_on_before() {
        let mw = TaintMiddleware;
        let ctx = test_ctx();
        let decision = mw
            .before_execute("shell", &serde_json::json!({}), &ctx)
            .await
            .unwrap();
        assert!(matches!(decision, MiddlewareDecision::Continue));
    }

    #[tokio::test]
    async fn taint_middleware_no_labels_for_non_network_tool() {
        let mw = TaintMiddleware;
        let ctx = test_ctx();
        let mut result = ToolResult {
            success: true,
            output: "ok".to_string(),
            error: None,
            attachments: Vec::new(),
            taint_labels: Vec::new(),
            semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
        };

        mw.after_execute("file_read", &mut result, &ctx).await;
        assert!(result.taint_labels.is_empty());
    }

    #[tokio::test]
    async fn taint_middleware_adds_external_network_for_web_fetch() {
        let mw = TaintMiddleware;
        let ctx = test_ctx();
        let mut result = ToolResult {
            success: true,
            output: "fetched".to_string(),
            error: None,
            attachments: Vec::new(),
            taint_labels: Vec::new(),
            semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
        };

        mw.after_execute("web_fetch", &mut result, &ctx).await;
        assert!(
            result
                .taint_labels
                .contains(&"external_network".to_string())
        );
    }

    #[tokio::test]
    async fn taint_middleware_propagates_input_context() {
        let mw = TaintMiddleware;
        let mut ctx = test_ctx();
        ctx.taint_context = Some(TaintSet::from_labels([TaintLabel::UserInput]));

        let mut result = ToolResult {
            success: true,
            output: "done".to_string(),
            error: None,
            attachments: Vec::new(),
            taint_labels: Vec::new(),
            semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
        };

        mw.after_execute("file_write", &mut result, &ctx).await;
        assert!(result.taint_labels.contains(&"user_input".to_string()));
    }

    #[tokio::test]
    async fn taint_middleware_merges_input_and_tool_taints() {
        let mw = TaintMiddleware;
        let mut ctx = test_ctx();
        ctx.taint_context = Some(TaintSet::from_labels([TaintLabel::Pii]));

        let mut result = ToolResult {
            success: true,
            output: "data".to_string(),
            error: None,
            attachments: Vec::new(),
            taint_labels: Vec::new(),
            semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
        };

        mw.after_execute("browser", &mut result, &ctx).await;
        assert!(result.taint_labels.contains(&"pii".to_string()));
        assert!(
            result
                .taint_labels
                .contains(&"external_network".to_string())
        );
    }

    #[tokio::test]
    async fn taint_middleware_preserves_existing_boundary_labels() {
        let mw = TaintMiddleware;
        let mut ctx = test_ctx();
        ctx.taint_context = Some(TaintSet::from_labels([TaintLabel::UserInput]));

        let mut result = ToolResult {
            success: true,
            output: "remote output".to_string(),
            error: None,
            attachments: Vec::new(),
            taint_labels: vec!["external:mcp".to_string()],
            semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
        };

        mw.after_execute("mcp_filesystem_search", &mut result, &ctx)
            .await;
        assert!(result.taint_labels.contains(&"external:mcp".to_string()));
        assert!(result.taint_labels.contains(&"user_input".to_string()));
        assert!(
            result
                .taint_labels
                .contains(&"external_network".to_string())
        );
    }
}
