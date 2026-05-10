//! Introspection tools exposed to the agent runtime.
//!
//! These tools read the current cognitive context, relationship state,
//! self-model, principles, experience memory, and consistency scores without
//! mutating durable state. They are intentionally separate from writeback tools
//! so prompt/tool planning can ask "what do I know about this turn?" before any
//! memory or persona update is allowed.

use std::future::Future;
use std::pin::Pin;

use anyhow::anyhow;
use serde_json::{Value, json};

use crate::contracts::strings::data_model::PREFIX_PRINCIPLE_SLOT;
use crate::core::eval::persona_consistency::score_prompt_to_line;
use crate::core::experience::memory_rl::{MemoryRL, retrieve_principles_with_q};
use crate::core::experience::retrieve_relevant_experiences;
use crate::core::memory::recall_helpers::recall_typed;
use crate::core::persona::relationship::load_relationship;
use crate::core::persona::self_model::build_self_model_shadow;
use crate::core::tools::cognitive_context::CognitiveContext;
use crate::core::tools::middleware::ExecutionContext;
use crate::core::tools::traits::{Tool, ToolResult, ToolSpec};
use crate::security::capability::Capability;

const MAX_QUERY_LIMIT: usize = 5;
#[derive(Default)]
pub struct IntrospectAffectTool;
#[derive(Default)]
pub struct IntrospectRelationshipTool;
#[derive(Default)]
pub struct IntrospectSelfModelTool;
#[derive(Default)]
pub struct IntrospectPrinciplesTool;
#[derive(Default)]
pub struct IntrospectExperienceTool;
#[derive(Default)]
pub struct AdjustReasoningTool;
#[derive(Default)]
pub struct FlagUncertaintyTool;
#[derive(Default)]
pub struct AnnotateTurnTool;

#[derive(Default)]
pub struct EvaluateConsistencyTool;

fn require_cognitive_context(ctx: &ExecutionContext) -> anyhow::Result<&CognitiveContext> {
    ctx.cognitive_context
        .as_deref()
        .ok_or_else(|| anyhow!("cognitive context not available"))
}

fn rate_limited_result() -> ToolResult {
    ToolResult {
        success: false,
        output: "Introspection rate limit reached for this turn.".to_string(),
        error: Some("rate_limited".to_string()),
        attachments: Vec::new(),
        taint_labels: Vec::new(),
        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
    }
}

fn json_result(payload: &Value) -> anyhow::Result<ToolResult> {
    Ok(ToolResult {
        success: true,
        output: serde_json::to_string_pretty(&payload)?,
        error: None,
        attachments: Vec::new(),
        taint_labels: Vec::new(),
        semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
    })
}

fn required_string_arg<'a>(args: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Missing '{key}' parameter"))
}

fn optional_string_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn bounded_limit(args: &Value, key: &str, default: usize, max: usize) -> usize {
    args.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .map_or(default, |value| value.clamp(1, max))
}

fn cognitive_spec(tool: &dyn Tool, capability: Capability) -> ToolSpec {
    let name = tool.name().to_string();
    let effect = crate::contracts::tools::ToolEffect::classify(&name);
    ToolSpec {
        name,
        description: tool.description().to_string(),
        parameters: tool.parameters_schema(),
        required_capabilities: vec![capability],
        effect,
    }
}

fn relationship_depth(interaction_count: u32) -> &'static str {
    match interaction_count {
        0..=4 => "new",
        5..=20 => "developing",
        21..=100 => "established",
        _ => "deep",
    }
}

fn validate_reasoning_strategy(strategy: &str) -> anyhow::Result<&str> {
    match strategy {
        "standard" | "verify_first" | "ask_clarify" | "stepwise" => Ok(strategy),
        _ => Err(anyhow!(
            "Invalid 'strategy' parameter; expected one of: standard, verify_first, ask_clarify, stepwise"
        )),
    }
}

fn validate_uncertainty_level(level: &str) -> anyhow::Result<&str> {
    match level {
        "low" | "medium" | "high" | "critical" => Ok(level),
        _ => Err(anyhow!(
            "Invalid 'level' parameter; expected one of: low, medium, high, critical"
        )),
    }
}

fn build_consistency_concerns(score: f64, draft: &str) -> Vec<String> {
    let mut concerns = Vec::new();
    let trimmed = draft.trim();

    if trimmed.is_empty() {
        concerns.push("Draft is empty, so persona alignment cannot be evaluated.".to_string());
        return concerns;
    }

    if score < 0.5 {
        concerns
            .push("Draft appears weakly aligned with persona directives or keywords.".to_string());
    }
    if score < 0.3 {
        concerns.push(
            "Draft may be drifting from the intended persona voice and priorities.".to_string(),
        );
    }
    if trimmed.len() < 40 {
        concerns
            .push("Draft is terse, which may leave persona traits under-expressed.".to_string());
    }

    concerns
}

impl Tool for IntrospectAffectTool {
    fn name(&self) -> &'static str {
        "introspect_affect"
    }

    fn description(&self) -> &'static str {
        "Query your affect detection for this turn."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn execute<'a>(
        &'a self,
        _args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let cog = require_cognitive_context(ctx)?;
            if !cog.try_tier1_call() {
                return Ok(rate_limited_result());
            }

            json_result(&json!({
                "label": cog.affect_reading.label,
                "valence": cog.affect_reading.valence,
                "arousal": cog.affect_reading.arousal,
                "dominance": cog.affect_reading.dominance,
                "confidence": cog.affect_reading.confidence.get(),
                "desire": {
                    "primary": cog.desire_state.primary,
                    "intensity": cog.desire_state.intensity,
                    "objective_prefix": cog.desire_state.objective_prefix,
                }
            }))
        })
    }

    fn spec(&self) -> ToolSpec {
        cognitive_spec(self, Capability::CognitiveRead)
    }
}

impl Tool for IntrospectRelationshipTool {
    fn name(&self) -> &'static str {
        "introspect_relationship"
    }

    fn description(&self) -> &'static str {
        "Inspect the current relationship state for this person."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn execute<'a>(
        &'a self,
        _args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let cog = require_cognitive_context(ctx)?;
            if !cog.try_tier1_call() {
                return Ok(rate_limited_result());
            }

            let state = load_relationship(cog.memory.as_ref(), cog.person_id.as_str())
                .await?
                .unwrap_or_default();

            json_result(&json!({
                "trust": state.trust_level,
                "rapport": state.rapport,
                "interaction_count": state.interaction_count,
                "depth": relationship_depth(state.interaction_count),
                "notable_events": state.notable_events,
            }))
        })
    }

    fn spec(&self) -> ToolSpec {
        cognitive_spec(self, Capability::CognitiveRead)
    }
}

impl Tool for IntrospectSelfModelTool {
    fn name(&self) -> &'static str {
        "introspect_self_model"
    }

    fn description(&self) -> &'static str {
        "Inspect the current self-model shadow."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "domain": {
                    "type": "string",
                    "description": "Optional domain filter for capability estimates"
                }
            },
            "additionalProperties": false
        })
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let cog = require_cognitive_context(ctx)?;
            if !cog.try_tier1_call() {
                return Ok(rate_limited_result());
            }

            let domain_filter = optional_string_arg(&args, "domain").map(str::to_ascii_lowercase);
            let shadow =
                build_self_model_shadow(cog.memory.as_ref(), cog.person_id.as_str(), "").await?;

            let capability_estimates = shadow
                .capability_estimates
                .iter()
                .filter(|estimate| {
                    domain_filter
                        .as_ref()
                        .is_none_or(|domain| estimate.domain.eq_ignore_ascii_case(domain))
                })
                .collect::<Vec<_>>();

            let uncertainty_register = shadow
                .uncertainty_register
                .iter()
                .filter(|entry| {
                    domain_filter.as_ref().is_none_or(|domain| {
                        entry.topic.eq_ignore_ascii_case(domain)
                            || entry.source.eq_ignore_ascii_case(domain)
                            || entry.topic.to_ascii_lowercase().contains(domain)
                            || entry.source.to_ascii_lowercase().contains(domain)
                    })
                })
                .collect::<Vec<_>>();

            json_result(&json!({
                "capability_estimates": capability_estimates,
                "uncertainty_register": uncertainty_register,
                "continuity_score": shadow.continuity_score,
            }))
        })
    }

    fn spec(&self) -> ToolSpec {
        cognitive_spec(self, Capability::CognitiveRead)
    }
}

impl Tool for IntrospectPrinciplesTool {
    fn name(&self) -> &'static str {
        "introspect_principles"
    }

    fn description(&self) -> &'static str {
        "Search distilled principles relevant to the current query."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query for relevant principles"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 5,
                    "description": "Maximum principles to return"
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let cog = require_cognitive_context(ctx)?;
            if !cog.try_tier1_call() {
                return Ok(rate_limited_result());
            }

            let query = required_string_arg(&args, "query")?;
            let limit = bounded_limit(&args, "limit", MAX_QUERY_LIMIT, MAX_QUERY_LIMIT);
            let memory_rl = MemoryRL::new(0.4);

            let principles = match retrieve_principles_with_q(
                cog.memory.as_ref(),
                cog.entity_id.as_str(),
                query,
                &memory_rl,
                limit,
            )
            .await
            {
                Ok(principles) => principles,
                Err(_) => recall_typed::<crate::core::experience::distill_types::Principle>(
                    cog.memory.as_ref(),
                    cog.entity_id.as_str(),
                    PREFIX_PRINCIPLE_SLOT,
                    limit,
                )
                .await
                .unwrap_or_default(),
            };

            let principles = principles
                .into_iter()
                .take(limit)
                .map(|principle| {
                    json!({
                        "text": principle.statement,
                        "quality_score": principle.confidence.get(),
                    })
                })
                .collect::<Vec<_>>();

            json_result(&json!(principles))
        })
    }

    fn spec(&self) -> ToolSpec {
        cognitive_spec(self, Capability::CognitiveRead)
    }
}

impl Tool for IntrospectExperienceTool {
    fn name(&self) -> &'static str {
        "introspect_experience"
    }

    fn description(&self) -> &'static str {
        "Search relevant past experiences for the current query."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query for relevant experiences"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 5,
                    "description": "Maximum experiences to return"
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let cog = require_cognitive_context(ctx)?;
            if !cog.try_tier1_call() {
                return Ok(rate_limited_result());
            }

            let query = required_string_arg(&args, "query")?;
            let limit = bounded_limit(&args, "limit", MAX_QUERY_LIMIT, MAX_QUERY_LIMIT);
            let experiences = retrieve_relevant_experiences(
                cog.memory.as_ref(),
                cog.entity_id.as_str(),
                query,
                limit,
            )
            .await
            .unwrap_or_default();

            json_result(&json!(
                experiences.into_iter().take(limit).collect::<Vec<_>>()
            ))
        })
    }

    fn spec(&self) -> ToolSpec {
        cognitive_spec(self, Capability::CognitiveRead)
    }
}

impl Tool for AdjustReasoningTool {
    fn name(&self) -> &'static str {
        "adjust_reasoning"
    }

    fn description(&self) -> &'static str {
        "Record a reasoning strategy adjustment intent."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "strategy": {
                    "type": "string",
                    "enum": ["standard", "verify_first", "ask_clarify", "stepwise"],
                    "description": "Requested reasoning strategy"
                },
                "reason": {
                    "type": "string",
                    "description": "Why the reasoning strategy should change"
                }
            },
            "required": ["strategy", "reason"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let cog = require_cognitive_context(ctx)?;
            if !cog.try_tier2_call() {
                return Ok(rate_limited_result());
            }

            let strategy = validate_reasoning_strategy(required_string_arg(&args, "strategy")?)?;
            let reason = required_string_arg(&args, "reason")?;

            json_result(&json!({
                "previous": cog.scaffolding.reasoning_mode,
                "current": strategy,
                "reason": reason,
                "recorded": true,
            }))
        })
    }

    fn spec(&self) -> ToolSpec {
        cognitive_spec(self, Capability::CognitiveWrite)
    }
}

impl Tool for FlagUncertaintyTool {
    fn name(&self) -> &'static str {
        "flag_uncertainty"
    }

    fn description(&self) -> &'static str {
        "Record an uncertainty flag for the current turn."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "domain": {
                    "type": "string",
                    "description": "Domain or topic where uncertainty is present"
                },
                "level": {
                    "type": "string",
                    "enum": ["low", "medium", "high", "critical"],
                    "description": "Severity of the uncertainty signal"
                }
            },
            "required": ["domain", "level"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let cog = require_cognitive_context(ctx)?;
            if !cog.try_tier2_call() {
                return Ok(rate_limited_result());
            }

            let domain = required_string_arg(&args, "domain")?;
            let level = validate_uncertainty_level(required_string_arg(&args, "level")?)?;

            json_result(&json!({
                "domain": domain,
                "level": level,
                "registered": true,
            }))
        })
    }

    fn spec(&self) -> ToolSpec {
        cognitive_spec(self, Capability::CognitiveWrite)
    }
}

impl Tool for AnnotateTurnTool {
    fn name(&self) -> &'static str {
        "annotate_turn"
    }

    fn description(&self) -> &'static str {
        "Record a turn-level annotation for downstream cognitive updates."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "note": {
                    "type": "string",
                    "description": "Freeform annotation text"
                },
                "salience": {
                    "type": "number",
                    "description": "How salient the note is on a 0-1 scale"
                },
                "affect_shift": {
                    "type": "string",
                    "description": "Optional affect shift summary"
                }
            },
            "required": ["note", "salience"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let cog = require_cognitive_context(ctx)?;
            if !cog.try_tier2_call() {
                return Ok(rate_limited_result());
            }

            let note = required_string_arg(&args, "note")?;
            let salience = args
                .get("salience")
                .and_then(Value::as_f64)
                .ok_or_else(|| anyhow!("Missing 'salience' parameter"))?
                .clamp(0.0, 1.0);
            let affect_shift = optional_string_arg(&args, "affect_shift");

            json_result(&json!({
                "turn_number": ctx.turn_number,
                "note": note,
                "salience": salience,
                "affect_shift": affect_shift,
                "recorded": true,
            }))
        })
    }

    fn spec(&self) -> ToolSpec {
        cognitive_spec(self, Capability::CognitiveWrite)
    }
}

impl Tool for EvaluateConsistencyTool {
    fn name(&self) -> &'static str {
        "evaluate_consistency"
    }

    fn description(&self) -> &'static str {
        "Evaluate a draft against the active persona specification."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "draft": {
                    "type": "string",
                    "description": "Draft response text to evaluate"
                }
            },
            "required": ["draft"],
            "additionalProperties": false
        })
    }

    fn execute<'a>(
        &'a self,
        args: Value,
        ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            let cog = require_cognitive_context(ctx)?;
            if !cog.try_tier3_call() {
                return Ok(rate_limited_result());
            }

            let draft = required_string_arg(&args, "draft")?;
            let score = score_prompt_to_line(&cog.persona_spec, draft);
            let concerns = build_consistency_concerns(score, draft);

            json_result(&json!({
                "prompt_to_line_score": score,
                "concerns": concerns,
            }))
        })
    }

    fn spec(&self) -> ToolSpec {
        cognitive_spec(self, Capability::CognitiveRead)
    }
}
