//! Tenant policy context helpers for memory tools.
//!
//! # Purpose
//!
//! Memory tools can receive an optional `policy_context` argument that lets
//! callers declare which tenant scope they intend to operate under. This module
//! merges that declared context with the authoritative context carried on the
//! `ExecutionContext`, applying the following rules:
//!
//! * If the execution context has tenant mode disabled, the declared context
//!   is used as-is (allows tools invoked outside a tenant session to work
//!   normally).
//! * If the execution context has tenant mode enabled, the declared context
//!   cannot disable it — the execution context's mode always wins.
//! * If both contexts have tenant mode enabled and specify different
//!   `tenant_id` values, the request is rejected to prevent cross-tenant
//!   escalation.
//!
//! `enforce_entity_scope` is a convenience wrapper that constructs a minimal
//! `RecallQuery` and calls its policy enforcement, re-using the query's
//! existing validation logic for the scope check.

use crate::core::memory::RecallQuery;
use crate::core::tools::middleware::ExecutionContext;
use crate::security::policy::TenantPolicyContext;

pub(super) fn effective_tenant_policy_context(
    args: &serde_json::Value,
    ctx: &ExecutionContext,
) -> anyhow::Result<TenantPolicyContext> {
    let base = ctx.tenant_context.clone();
    let Some(raw_policy_context) = args.get("policy_context") else {
        return Ok(base);
    };

    let requested: TenantPolicyContext = serde_json::from_value(raw_policy_context.clone())
        .map_err(|error| anyhow::anyhow!("Invalid 'policy_context' parameter: {error}"))?;
    merge_tenant_policy_context(base, requested)
}

fn merge_tenant_policy_context(
    base: TenantPolicyContext,
    requested: TenantPolicyContext,
) -> anyhow::Result<TenantPolicyContext> {
    if !base.tenant_mode_enabled {
        return Ok(requested);
    }

    if !requested.tenant_mode_enabled {
        return Ok(base);
    }

    let base_tenant = normalized_tenant_id(base.tenant_id.as_deref());
    let requested_tenant = normalized_tenant_id(requested.tenant_id.as_deref());

    if let (Some(base_tenant), Some(requested_tenant)) = (&base_tenant, &requested_tenant)
        && base_tenant != requested_tenant
    {
        anyhow::bail!(
            "Invalid 'policy_context' parameter: tenant_id mismatch with execution context"
        );
    }

    Ok(TenantPolicyContext {
        tenant_mode_enabled: true,
        tenant_id: base_tenant.or(requested_tenant),
    })
}

fn normalized_tenant_id(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(std::string::ToString::to_string)
}

pub(super) fn enforce_entity_scope(
    entity_id: &str,
    policy: &TenantPolicyContext,
) -> anyhow::Result<()> {
    RecallQuery::new(entity_id, "", 1)
        .with_policy_context(policy.clone())
        .enforce_policy()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::security::SecurityPolicy;

    #[test]
    fn invalid_policy_context_shape_is_rejected() {
        let ctx = ExecutionContext::from_security(Arc::new(SecurityPolicy::default()));
        let args = serde_json::json!({
            "policy_context": "tenant-alpha"
        });
        let err = effective_tenant_policy_context(&args, &ctx)
            .expect_err("invalid policy context should be rejected")
            .to_string();
        assert!(err.contains("Invalid 'policy_context' parameter"));
    }

    #[test]
    fn execution_context_tenant_mode_cannot_be_disabled_by_args() {
        let mut ctx = ExecutionContext::from_security(Arc::new(SecurityPolicy::default()));
        ctx.tenant_context = TenantPolicyContext::enabled("tenant-alpha");
        let args = serde_json::json!({
            "policy_context": {
                "tenant_mode_enabled": false,
                "tenant_id": null
            }
        });

        let effective = effective_tenant_policy_context(&args, &ctx).unwrap();
        assert!(effective.tenant_mode_enabled);
        assert_eq!(effective.tenant_id.as_deref(), Some("tenant-alpha"));
    }

    #[test]
    fn mismatched_tenant_ids_are_rejected_when_context_is_restricted() {
        let mut ctx = ExecutionContext::from_security(Arc::new(SecurityPolicy::default()));
        ctx.tenant_context = TenantPolicyContext::enabled("tenant-alpha");
        let args = serde_json::json!({
            "policy_context": {
                "tenant_mode_enabled": true,
                "tenant_id": "tenant-beta"
            }
        });

        let err = effective_tenant_policy_context(&args, &ctx)
            .expect_err("mismatch should be rejected")
            .to_string();
        assert!(err.contains("tenant_id mismatch"));
    }

    #[test]
    fn args_context_can_enable_tenant_mode_when_execution_context_is_open() {
        let ctx = ExecutionContext::from_security(Arc::new(SecurityPolicy::default()));
        let args = serde_json::json!({
            "policy_context": {
                "tenant_mode_enabled": true,
                "tenant_id": "tenant-alpha"
            }
        });

        let effective = effective_tenant_policy_context(&args, &ctx).unwrap();
        assert!(effective.tenant_mode_enabled);
        assert_eq!(effective.tenant_id.as_deref(), Some("tenant-alpha"));
    }
}
