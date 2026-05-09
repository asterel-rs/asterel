//! Taste compare tool — records a pairwise preference comparison between two artifacts.
//!
//! # What it does
//!
//! `taste_compare` builds a `PairComparison` from the supplied `left_id`,
//! `right_id`, `winner` (`left`, `right`, `tie`, or `abstain`), optional
//! `domain`, and optional `rationale`, then calls `TasteEngine::compare` to
//! persist the comparison for future taste-model refinement.
//!
//! The timestamp (`created_at_ms`) is stamped at the moment of tool execution
//! using `now_unix_millis_u64`, which is safe on 32-bit platforms via
//! `crate::utils::truncate_u128_to_u64`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::json;

use crate::core::taste::engine::TasteEngine;
use crate::core::taste::types::{Domain, PairComparison, TasteContext, TasteOwnerScope, Winner};
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult};

/// Tool that records a pairwise artifact preference comparison in the taste engine.
pub struct TasteCompareTool {
    engine: Arc<dyn TasteEngine>,
}

impl TasteCompareTool {
    /// Create a new taste-compare tool using the given engine.
    pub fn new(engine: Arc<dyn TasteEngine>) -> Self {
        Self { engine }
    }

    fn failure(message: impl Into<String>) -> ToolResult {
        ToolResult::failure(message)
    }

    fn required_string<'a>(
        args: &'a serde_json::Value,
        key: &str,
    ) -> Result<&'a str, Box<ToolResult>> {
        args.get(key)
            .and_then(|value| value.as_str())
            .ok_or_else(|| Box::new(Self::failure(format!("Missing '{key}' parameter"))))
    }

    fn parse_winner(args: &serde_json::Value) -> Result<Winner, Box<ToolResult>> {
        let winner = Self::required_string(args, "winner")?;
        serde_json::from_value(serde_json::Value::String(winner.to_string()))
            .map_err(|error| Box::new(Self::failure(error.to_string())))
    }

    fn parse_domain(args: &serde_json::Value) -> Result<Domain, Box<ToolResult>> {
        let domain = args
            .get("domain")
            .and_then(|value| value.as_str())
            .unwrap_or("general");
        serde_json::from_value(serde_json::Value::String(domain.to_string()))
            .map_err(|error| Box::new(Self::failure(error.to_string())))
    }

    fn build_comparison(
        owner: TasteOwnerScope,
        left_id: &str,
        right_id: &str,
        winner: Winner,
        rationale: Option<String>,
    ) -> PairComparison {
        PairComparison {
            owner,
            domain: Domain::General,
            ctx: TasteContext::default(),
            left_id: left_id.to_string(),
            right_id: right_id.to_string(),
            winner,
            rationale,
            created_at_ms: now_unix_millis_u64(),
        }
    }
}

fn now_unix_millis_u64() -> u64 {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    crate::utils::truncate_u128_to_u64(duration.as_millis())
}

impl Tool for TasteCompareTool {
    fn name(&self) -> &'static str {
        "taste_compare"
    }

    fn description(&self) -> &'static str {
        "Record a pairwise preference comparison between two artifacts."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "left_id": {
                    "type": "string",
                    "description": "Identifier of the left artifact"
                },
                "right_id": {
                    "type": "string",
                    "description": "Identifier of the right artifact"
                },
                "winner": {
                    "type": "string",
                    "enum": ["left", "right", "tie", "abstain"],
                    "description": "Which artifact won the comparison"
                },
                "domain": {
                    "type": "string",
                    "description": "Domain: text, ui, or general (default: general)"
                },
                "rationale": {
                    "type": "string",
                    "description": "Optional rationale for the preference"
                }
            },
            "required": ["left_id", "right_id", "winner"]
        })
    }

    fn execute<'a>(
        &'a self,
        args: serde_json::Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let left_id = match Self::required_string(&args, "left_id") {
                Ok(value) => value,
                Err(result) => return Ok(*result),
            };
            let right_id = match Self::required_string(&args, "right_id") {
                Ok(value) => value,
                Err(result) => return Ok(*result),
            };
            let winner_str = match Self::required_string(&args, "winner") {
                Ok(value) => value,
                Err(result) => return Ok(*result),
            };
            let winner = match Self::parse_winner(&args) {
                Ok(value) => value,
                Err(result) => return Ok(*result),
            };
            let domain = match Self::parse_domain(&args) {
                Ok(value) => value,
                Err(result) => return Ok(*result),
            };
            let rationale = args
                .get("rationale")
                .and_then(|v| v.as_str())
                .map(String::from);

            let owner = TasteOwnerScope::new(
                ctx.tenant_context.tenant_id.clone(),
                ctx.entity_id.to_string(),
                ctx.session_id.clone(),
            );
            let mut comparison =
                Self::build_comparison(owner, left_id, right_id, winner, rationale);
            comparison.domain = domain;

            match self.engine.compare(&comparison).await {
                Ok(()) => Ok(ToolResult::success(
                    json!({
                        "status": "comparison_recorded",
                        "left_id": left_id,
                        "right_id": right_id,
                        "winner": winner_str
                    })
                    .to_string(),
                )),
                Err(error) => Ok(Self::failure(error.to_string())),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::core::taste::types::{
        Artifact, Axis, Domain, PairComparison, Priority, Suggestion, TasteContext, TasteReport,
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

    struct CapturingTasteEngine {
        comparison: Arc<Mutex<Option<PairComparison>>>,
    }

    impl TasteEngine for CapturingTasteEngine {
        fn evaluate<'a>(
            &'a self,
            _artifact: &'a Artifact,
            _ctx: &'a TasteContext,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<TasteReport>> + Send + 'a>> {
            Box::pin(async move { anyhow::bail!("not used") })
        }

        fn compare<'a>(
            &'a self,
            comparison: &'a PairComparison,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            Box::pin(async move {
                *self.comparison.lock().expect("capture lock") = Some(comparison.clone());
                Ok(())
            })
        }

        fn enabled(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn compare_valid_args() {
        let tool = TasteCompareTool::new(mock_engine());
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({
                    "left_id": "artifact_a",
                    "right_id": "artifact_b",
                    "winner": "left"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["status"], "comparison_recorded");
        assert_eq!(parsed["left_id"], "artifact_a");
        assert_eq!(parsed["right_id"], "artifact_b");
        assert_eq!(parsed["winner"], "left");
    }

    #[tokio::test]
    async fn compare_with_optional_fields() {
        let tool = TasteCompareTool::new(mock_engine());
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({
                    "left_id": "a",
                    "right_id": "b",
                    "winner": "tie",
                    "domain": "text",
                    "rationale": "Both are equally good"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.success);
    }

    #[tokio::test]
    async fn compare_attaches_execution_owner_scope() {
        let captured = Arc::new(Mutex::new(None));
        let tool = TasteCompareTool::new(Arc::new(CapturingTasteEngine {
            comparison: Arc::clone(&captured),
        }));
        let mut ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        ctx.tenant_context = crate::contracts::tenant::TenantPolicyContext::enabled("tenant-a");
        ctx.entity_id = "person-a".into();
        ctx.session_id = Some("session-a".to_string());

        let result = tool
            .execute(
                json!({
                    "left_id": "a",
                    "right_id": "b",
                    "winner": "left"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.success);
        let comparison = captured
            .lock()
            .expect("capture lock")
            .clone()
            .expect("comparison captured");
        assert_eq!(comparison.owner.tenant_id.as_deref(), Some("tenant-a"));
        assert_eq!(comparison.owner.entity_id.as_deref(), Some("person-a"));
        assert_eq!(comparison.owner.session_id.as_deref(), Some("session-a"));
    }

    #[tokio::test]
    async fn compare_missing_winner() {
        let tool = TasteCompareTool::new(mock_engine());
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({
                    "left_id": "a",
                    "right_id": "b"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("winner"));
    }

    #[tokio::test]
    async fn compare_missing_left_id() {
        let tool = TasteCompareTool::new(mock_engine());
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({
                    "right_id": "b",
                    "winner": "left"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("left_id"));
    }

    #[tokio::test]
    async fn compare_invalid_winner() {
        let tool = TasteCompareTool::new(mock_engine());
        let ctx = ExecutionContext::test_default(Arc::new(SecurityPolicy::default()));
        let result = tool
            .execute(
                json!({
                    "left_id": "a",
                    "right_id": "b",
                    "winner": "invalid"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[test]
    fn name_and_schema() {
        let tool = TasteCompareTool::new(mock_engine());
        assert_eq!(tool.name(), "taste_compare");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["left_id"].is_object());
        assert!(schema["properties"]["right_id"].is_object());
        assert!(schema["properties"]["winner"].is_object());
        assert!(schema["properties"]["domain"].is_object());
        assert!(schema["properties"]["rationale"].is_object());
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 3);
    }
}
