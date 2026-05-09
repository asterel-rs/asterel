//! Taste evaluate tool — scores a single artifact against the taste profile.
//!
//! # What it does
//!
//! `taste_evaluate` parses an `Artifact` (kind `text` or `ui`) and an optional
//! `TasteContext` from the tool arguments, then calls `TasteEngine::evaluate`.
//! The result is a `TasteReport` containing per-axis scores
//! (`Coherence`, `Hierarchy`, `Intentionality`), the evaluated domain, and a
//! list of improvement suggestions with priorities.
//!
//! Parse errors for the artifact or context are surfaced as non-success
//! `ToolResult` values (not hard `Err` returns) so the agent can report them
//! gracefully without losing the current conversation turn.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::json;

use crate::core::taste::engine::TasteEngine;
use crate::core::taste::types::{Artifact, TasteContext, TextFormat};
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult};

/// Tool that evaluates an artifact's aesthetic quality via the taste engine.
///
/// Returns a JSON-serialized `TasteReport` on success, or a non-success
/// `ToolResult` if the artifact cannot be parsed or the engine returns an error.
pub struct TasteEvaluateTool {
    engine: Arc<dyn TasteEngine>,
}

impl TasteEvaluateTool {
    /// Create a new taste-evaluate tool using the given engine.
    pub fn new(engine: Arc<dyn TasteEngine>) -> Self {
        Self { engine }
    }

    fn parse_artifact(artifact: &serde_json::Value) -> anyhow::Result<Artifact> {
        let kind = artifact
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'kind' in artifact"))?;

        let content = artifact
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'content' in artifact"))?;

        match kind {
            "text" => {
                let format = artifact
                    .get("format")
                    .and_then(|v| v.as_str())
                    .map(|f| serde_json::from_value::<TextFormat>(json!(f)))
                    .transpose()?;
                Ok(Artifact::Text {
                    content: content.to_string(),
                    format,
                })
            }
            "ui" => {
                let metadata = artifact.get("metadata").cloned();
                Ok(Artifact::Ui {
                    description: content.to_string(),
                    metadata,
                })
            }
            other => anyhow::bail!("Unsupported artifact kind: {other}"),
        }
    }

    fn parse_context(context: Option<&serde_json::Value>) -> anyhow::Result<TasteContext> {
        match context {
            Some(v) => Ok(serde_json::from_value(v.clone())?),
            None => Ok(TasteContext::default()),
        }
    }
}

impl Tool for TasteEvaluateTool {
    fn name(&self) -> &'static str {
        "taste_evaluate"
    }

    fn description(&self) -> &'static str {
        "Evaluate an artifact's aesthetic quality using the taste engine."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "artifact": {
                    "type": "object",
                    "description": "The artifact to evaluate",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "The artifact content (text content or UI description)"
                        },
                        "format": {
                            "type": "string",
                            "description": "Text format: plain, markdown, or html (text kind only)"
                        },
                        "kind": {
                            "type": "string",
                            "enum": ["text", "ui"],
                            "description": "Artifact kind"
                        }
                    },
                    "required": ["content", "kind"]
                },
                "context": {
                    "type": "object",
                    "description": "Evaluation context",
                    "properties": {
                        "domain": {
                            "type": "string",
                            "description": "Domain: text, ui, or general"
                        },
                        "genre": {
                            "type": "string",
                            "description": "Genre of the artifact"
                        },
                        "purpose": {
                            "type": "string",
                            "description": "Purpose of the artifact"
                        }
                    }
                }
            },
            "required": ["artifact"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let Some(artifact_value) = args.get("artifact") else {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some("Missing 'artifact' parameter".to_string()),
                    attachments: vec![],
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                });
            };

            let artifact = match Self::parse_artifact(artifact_value) {
                Ok(a) => a,
                Err(e) => {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(e.to_string()),
                        attachments: vec![],
                        taint_labels: Vec::new(),
                        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                    });
                }
            };

            let taste_ctx = match Self::parse_context(args.get("context")) {
                Ok(c) => c,
                Err(e) => {
                    return Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(e.to_string()),
                        attachments: vec![],
                        taint_labels: Vec::new(),
                        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                    });
                }
            };

            match self.engine.evaluate(&artifact, &taste_ctx).await {
                Ok(report) => match serde_json::to_string(&report) {
                    Ok(json_string) => Ok(ToolResult {
                        success: true,
                        output: json_string,
                        error: None,
                        attachments: vec![],
                        taint_labels: Vec::new(),
                        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                    }),
                    Err(e) => Ok(ToolResult {
                        success: false,
                        output: String::new(),
                        error: Some(format!("Failed to serialize report: {e}")),
                        attachments: vec![],
                        taint_labels: Vec::new(),
                        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                    }),
                },
                Err(e) => Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(e.to_string()),
                    attachments: vec![],
                    taint_labels: Vec::new(),
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                }),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;

    use super::*;
    use crate::core::taste::types::{
        Axis, Domain, PairComparison, Priority, Suggestion, TasteReport,
    };
    use crate::security::SecurityPolicy;

    struct MockTasteEngine;

    impl TasteEngine for MockTasteEngine {
        fn evaluate<'a>(
            &'a self,
            _artifact: &'a Artifact,
            _ctx: &'a TasteContext,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<TasteReport>> + Send + 'a>> {
            Box::pin(async move {
                let mut axis = BTreeMap::new();
                axis.insert(Axis::Coherence, 0.8);
                axis.insert(Axis::Hierarchy, 0.7);
                axis.insert(Axis::Intentionality, 0.9);
                Ok(TasteReport {
                    axis,
                    domain: Domain::Text,
                    suggestions: vec![Suggestion::General {
                        title: "Improve structure".into(),
                        rationale: "Would benefit from clearer sections".into(),
                        priority: Priority::Medium,
                    }],
                    raw_critique: None,
                })
            })
        }

        fn compare<'a>(
            &'a self,
            _comparison: &'a PairComparison,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            Box::pin(async move { Ok(()) })
        }

        fn enabled(&self) -> bool {
            true
        }
    }

    fn mock_engine() -> Arc<dyn TasteEngine> {
        Arc::new(MockTasteEngine)
    }

    #[tokio::test]
    async fn evaluate_text_artifact() {
        let tool = TasteEvaluateTool::new(mock_engine());
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({
                    "artifact": {
                        "kind": "text",
                        "content": "Hello world"
                    }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert!(parsed.get("axis").is_some());
        assert!(parsed.get("suggestions").is_some());
    }

    #[tokio::test]
    async fn evaluate_ui_artifact() {
        let tool = TasteEvaluateTool::new(mock_engine());
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({
                    "artifact": {
                        "kind": "ui",
                        "content": "A dashboard with sidebar navigation"
                    }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert!(parsed.get("axis").is_some());
        assert!(parsed.get("suggestions").is_some());
    }

    #[tokio::test]
    async fn evaluate_with_context() {
        let tool = TasteEvaluateTool::new(mock_engine());
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({
                    "artifact": {
                        "kind": "text",
                        "content": "Test content",
                        "format": "markdown"
                    },
                    "context": {
                        "domain": "text",
                        "genre": "technical",
                        "purpose": "documentation"
                    }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
    }

    #[tokio::test]
    async fn evaluate_missing_artifact() {
        let tool = TasteEvaluateTool::new(mock_engine());
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        let result = tool.execute(json!({}), &ctx).await.unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .contains("Missing 'artifact'")
        );
    }

    #[tokio::test]
    async fn evaluate_missing_kind() {
        let tool = TasteEvaluateTool::new(mock_engine());
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({
                    "artifact": {
                        "content": "hello"
                    }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("kind"));
    }

    #[tokio::test]
    async fn evaluate_unsupported_kind() {
        let tool = TasteEvaluateTool::new(mock_engine());
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({
                    "artifact": {
                        "kind": "audio",
                        "content": "something"
                    }
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert!(
            result
                .error
                .as_ref()
                .unwrap()
                .contains("Unsupported artifact kind")
        );
    }

    #[test]
    fn name_and_schema() {
        let tool = TasteEvaluateTool::new(mock_engine());
        assert_eq!(tool.name(), "taste_evaluate");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["artifact"].is_object());
        assert!(schema["properties"]["context"].is_object());
    }
}
