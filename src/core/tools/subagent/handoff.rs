//! Shared helpers for building subagent delegation options and parsing handoff envelopes.
//!
//! # Handoff envelope
//!
//! A handoff envelope carries optional structured intent from the parent agent
//! to the child:
//!
//! * `objective` — high-level goal for the delegated task.
//! * `done_when` — completion condition the child should verify.
//! * `context` — supporting background the child may need.
//! * `constraints` — hard constraints the child must not violate.
//!
//! All fields are optional; an empty envelope (`is_empty()` returns `true`)
//! is collapsed to `None` so the child is not given spurious context.
//!
//! # Delegation options
//!
//! `build_delegation_options` wires the handoff envelope together with the
//! spawn limits, model override, and parent context into a `SubagentRunOptions`
//! struct ready for the subagent runtime. It enforces:
//!
//! 1. The subagent runtime must be configured (process-level or context-injected).
//! 2. `delegation_depth < max_delegation_depth` — prevents unbounded recursion.
//! 3. `child_delegation_quota > 0` — prevents fan-out beyond the configured cap.

use serde_json::Value;

use crate::core::subagents::{
    SubagentDelegationConfig, SubagentHandoffEnvelope, SubagentRunOptions,
};
use crate::core::tools::middleware::ExecutionContext;

pub(crate) fn parse_handoff_envelope(
    args: &Value,
) -> anyhow::Result<Option<SubagentHandoffEnvelope>> {
    let objective = optional_string_field(args, "objective");
    let done_when = optional_string_field(args, "done_when");
    let context = optional_string_field(args, "context");
    let constraints = optional_string_list_field(args, "constraints")?;

    let envelope = SubagentHandoffEnvelope {
        objective,
        done_when,
        context,
        constraints,
    };

    if envelope.is_empty() {
        Ok(None)
    } else {
        Ok(Some(envelope))
    }
}

fn optional_string_field(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn optional_string_list_field(args: &Value, key: &str) -> anyhow::Result<Vec<String>> {
    let Some(raw) = args.get(key) else {
        return Ok(Vec::new());
    };
    let Some(items) = raw.as_array() else {
        anyhow::bail!("'{key}' must be an array of strings");
    };

    items
        .iter()
        .map(|item| {
            let Some(value) = item.as_str() else {
                anyhow::bail!("'{key}' must contain only strings");
            };
            Ok(value.trim().to_string())
        })
        .filter(|result: &anyhow::Result<String>| {
            result
                .as_ref()
                .map_or(true, |value| !value.trim().is_empty())
        })
        .collect()
}

pub(crate) fn build_delegation_options(
    ctx: &ExecutionContext,
    label: Option<String>,
    model_override: Option<String>,
    handoff: Option<SubagentHandoffEnvelope>,
) -> anyhow::Result<SubagentRunOptions> {
    if ctx.subagent_manager.is_none() && !crate::core::subagents::is_configured() {
        anyhow::bail!("subagent runtime is not configured");
    }
    if ctx.delegation_depth >= ctx.max_delegation_depth {
        anyhow::bail!(
            "delegation depth limit reached ({}/{})",
            ctx.delegation_depth,
            ctx.max_delegation_depth
        );
    }
    if !ctx.try_consume_child_delegation_slot() {
        anyhow::bail!("child delegation quota exhausted for {}", ctx.entity_id);
    }

    Ok(SubagentRunOptions {
        label,
        system_prompt_override: ctx.delegation_system_prompt.clone(),
        model_override,
        handoff,
        parent_context: Some(ctx.clone()),
        delegation: Some(SubagentDelegationConfig {
            depth: ctx.delegation_depth.saturating_add(1),
            max_depth: ctx.max_delegation_depth,
            child_quota: ctx.child_delegation_quota,
        }),
        ..SubagentRunOptions::default()
    })
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;

    use serde_json::json;

    use super::{build_delegation_options, parse_handoff_envelope};
    use crate::config::SkillsRuntimeConfig;
    use crate::core::providers::{Provider, ProviderResult};
    use crate::core::subagents::{SubagentConfig, TEST_RUNTIME_LOCK, configure_runtime};
    use crate::core::tools::middleware::ExecutionContext;
    use crate::security::SecurityPolicy;

    struct NoopProvider;

    impl Provider for NoopProvider {
        fn chat_with_system<'a>(
            &'a self,
            _system_prompt: Option<&'a str>,
            message: &'a str,
            _model: &'a str,
            _temperature: f64,
        ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
            Box::pin(async move { Ok(message.to_string()) })
        }
    }

    fn configure_test_runtime() {
        configure_runtime(SubagentConfig {
            provider: Arc::new(NoopProvider),
            system_prompt: "sys".to_string(),
            default_model: "test-model".to_string(),
            default_temperature: 0.0,
            tool_registry: None,
            workspace_dir: std::path::PathBuf::from("."),
            skill_loading_security: SecurityPolicy::default(),
            skills: SkillsRuntimeConfig::default(),
            max_delegation_depth: crate::core::tools::DEFAULT_MAX_DELEGATION_DEPTH,
            child_delegation_quota: crate::core::tools::DEFAULT_SUBAGENT_CHILD_DELEGATION_QUOTA,
            agent_extensions: Vec::new(),
            extension_loader: None,
            skill_metadata_provider: Arc::new(
                crate::core::subagents::NoopSkillMetadataProvider::new(),
            ),
        })
        .expect("runtime config should succeed");
    }

    #[test]
    fn parse_handoff_envelope_returns_none_when_fields_absent() {
        let parsed = parse_handoff_envelope(&json!({})).expect("parse should succeed");
        assert!(parsed.is_none());
    }

    #[test]
    fn parse_handoff_envelope_collects_string_fields() {
        let parsed = parse_handoff_envelope(&json!({
            "objective": "Investigate failure",
            "done_when": "Root cause is identified",
            "context": "CI started failing after a refactor",
            "constraints": ["Do not edit migrations", "Keep output short"]
        }))
        .expect("parse should succeed")
        .expect("envelope should exist");

        assert_eq!(parsed.objective.as_deref(), Some("Investigate failure"));
        assert_eq!(
            parsed.done_when.as_deref(),
            Some("Root cause is identified")
        );
        assert_eq!(
            parsed.context.as_deref(),
            Some("CI started failing after a refactor")
        );
        assert_eq!(
            parsed.constraints,
            vec!["Do not edit migrations", "Keep output short"]
        );
    }

    #[test]
    fn parse_handoff_envelope_rejects_non_string_constraints() {
        let error = parse_handoff_envelope(&json!({
            "constraints": ["Keep changes small", 7]
        }))
        .expect_err("parse should fail");

        assert!(error.to_string().contains("constraints"));
    }

    #[test]
    fn build_delegation_options_increments_depth_and_consumes_quota() {
        let _guard = TEST_RUNTIME_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        configure_test_runtime();

        let security = Arc::new(SecurityPolicy::default());
        let ctx = ExecutionContext::test_default(security).with_delegation_limits(0, 2, 2, 1);

        let options = build_delegation_options(&ctx, Some("planner".to_string()), None, None)
            .expect("options should build");

        assert_eq!(ctx.remaining_child_delegations(), 0);
        let delegation = options.delegation.expect("delegation config should exist");
        assert_eq!(delegation.depth, 1);
        assert_eq!(delegation.max_depth, 2);
        assert_eq!(delegation.child_quota, 2);
        assert_eq!(
            options
                .parent_context
                .as_ref()
                .map(|parent| parent.entity_id.as_str()),
            Some("test:default")
        );
    }

    #[test]
    fn build_delegation_options_copies_delegation_prompt_override() {
        let _guard = TEST_RUNTIME_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        configure_test_runtime();

        let security = Arc::new(SecurityPolicy::default());
        let mut ctx = ExecutionContext::test_default(security).with_delegation_limits(0, 2, 2, 1);
        ctx.delegation_system_prompt = Some("live turn prompt".to_string());

        let options = build_delegation_options(&ctx, Some("planner".to_string()), None, None)
            .expect("options should build");

        assert_eq!(
            options.system_prompt_override.as_deref(),
            Some("live turn prompt")
        );
    }

    #[test]
    fn build_delegation_options_rejects_when_depth_limit_reached() {
        let _guard = TEST_RUNTIME_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        configure_test_runtime();

        let security = Arc::new(SecurityPolicy::default());
        let ctx = ExecutionContext::test_default(security).with_delegation_limits(2, 2, 2, 1);

        let error = build_delegation_options(&ctx, None, None, None).expect_err("must fail");
        assert!(error.to_string().contains("depth limit"));
    }

    #[test]
    fn build_delegation_options_rejects_when_child_quota_exhausted() {
        let _guard = TEST_RUNTIME_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        configure_test_runtime();

        let security = Arc::new(SecurityPolicy::default());
        let ctx = ExecutionContext::test_default(security).with_delegation_limits(1, 2, 2, 0);

        let error = build_delegation_options(&ctx, None, None, None).expect_err("must fail");
        assert!(error.to_string().contains("quota exhausted"));
    }
}
