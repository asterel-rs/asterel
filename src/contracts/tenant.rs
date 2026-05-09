//! Tenant isolation policy for multi-tenant memory access.
//!
//! Enforces that memory recall and store operations respect tenant
//! boundaries, preventing cross-tenant data leakage.

use serde::{Deserialize, Serialize};

/// Error message returned when tenant mode forbids default scope fallback.
pub const TENANT_DEFAULT_SCOPE_FALLBACK_DENIED_ERROR: &str =
    crate::contracts::strings::verdicts::TENANT_DEFAULT_RECALL_FORBIDDEN;
/// Error message returned when a recall targets a different tenant scope.
pub const TENANT_RECALL_CROSS_SCOPE_DENIED_ERROR: &str =
    crate::contracts::strings::verdicts::TENANT_RECALL_SCOPE_MISMATCH;

/// Tenant isolation context for multi-tenant memory operations.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantPolicyContext {
    /// Whether tenant isolation is active.
    pub tenant_mode_enabled: bool,
    /// The active tenant identifier, if any.
    pub tenant_id: Option<String>,
}

impl TenantPolicyContext {
    /// Create a context with tenant isolation disabled.
    #[must_use]
    pub fn disabled() -> Self {
        Self::default()
    }

    /// Create a context with tenant isolation enabled for the given id.
    pub fn enabled(tenant_id: impl Into<String>) -> Self {
        Self {
            tenant_mode_enabled: true,
            tenant_id: Some(tenant_id.into()),
        }
    }

    /// Prefix an entity identifier with the active tenant scope when tenant
    /// isolation is enabled.
    #[must_use]
    pub fn scope_entity_id(&self, entity_id: &str) -> String {
        let requested = entity_id.trim();
        if !self.tenant_mode_enabled || requested.is_empty() {
            return requested.to_string();
        }

        let Some(tenant_id) = self.tenant_id.as_deref().filter(|id| !id.is_empty()) else {
            return requested.to_string();
        };

        if requested == tenant_id
            || requested
                .strip_prefix(tenant_id)
                .is_some_and(|suffix| suffix.starts_with(':') || suffix.starts_with('/'))
        {
            requested.to_string()
        } else {
            format!("{tenant_id}:{requested}")
        }
    }

    /// # Errors
    ///
    /// Returns an error when tenant mode is enabled and the requested recall
    /// entity id is empty, default-scoped, or outside the tenant scope.
    pub fn enforce_recall_scope(&self, entity_id: &str) -> Result<(), &'static str> {
        if !self.tenant_mode_enabled {
            return Ok(());
        }

        let requested = entity_id.trim();
        if requested.is_empty() || requested == "default" {
            return Err(TENANT_DEFAULT_SCOPE_FALLBACK_DENIED_ERROR);
        }

        let Some(tenant_id) = self.tenant_id.as_deref().filter(|id| !id.is_empty()) else {
            return Err(TENANT_RECALL_CROSS_SCOPE_DENIED_ERROR);
        };

        let in_scope = requested == tenant_id
            || requested
                .strip_prefix(tenant_id)
                .is_some_and(|suffix| suffix.starts_with(':') || suffix.starts_with('/'));

        if in_scope {
            Ok(())
        } else {
            Err(TENANT_RECALL_CROSS_SCOPE_DENIED_ERROR)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scope_is_rejected_in_tenant_mode() {
        let context = TenantPolicyContext::enabled("tenant-alpha");

        assert_eq!(
            context.enforce_recall_scope("default"),
            Err(TENANT_DEFAULT_SCOPE_FALLBACK_DENIED_ERROR)
        );
    }

    #[test]
    fn empty_entity_id_is_rejected_in_tenant_mode() {
        let context = TenantPolicyContext::enabled("tenant-alpha");

        assert_eq!(
            context.enforce_recall_scope("   "),
            Err(TENANT_DEFAULT_SCOPE_FALLBACK_DENIED_ERROR)
        );
    }

    #[test]
    fn matching_tenant_id_is_allowed() {
        let context = TenantPolicyContext::enabled("tenant-alpha");

        assert!(context.enforce_recall_scope("tenant-alpha").is_ok());
    }

    #[test]
    fn mismatched_tenant_id_is_rejected() {
        let context = TenantPolicyContext::enabled("tenant-alpha");

        assert_eq!(
            context.enforce_recall_scope("tenant-beta"),
            Err(TENANT_RECALL_CROSS_SCOPE_DENIED_ERROR)
        );
    }

    #[test]
    fn hierarchical_colon_scope_is_allowed_for_same_tenant() {
        let context = TenantPolicyContext::enabled("tenant-alpha");

        assert!(
            context
                .enforce_recall_scope("tenant-alpha:subtenant:user-1")
                .is_ok()
        );
    }

    #[test]
    fn hierarchical_slash_scope_is_allowed_for_same_tenant() {
        let context = TenantPolicyContext::enabled("tenant-alpha");

        assert!(
            context
                .enforce_recall_scope("tenant-alpha/subtenant/session")
                .is_ok()
        );
    }

    #[test]
    fn empty_tenant_id_in_context_rejects_requests() {
        let context = TenantPolicyContext {
            tenant_mode_enabled: true,
            tenant_id: Some(String::new()),
        };

        assert_eq!(
            context.enforce_recall_scope("tenant-alpha"),
            Err(TENANT_RECALL_CROSS_SCOPE_DENIED_ERROR)
        );
    }

    #[test]
    fn missing_tenant_id_in_context_rejects_requests() {
        let context = TenantPolicyContext {
            tenant_mode_enabled: true,
            tenant_id: None,
        };

        assert_eq!(
            context.enforce_recall_scope("tenant-alpha"),
            Err(TENANT_RECALL_CROSS_SCOPE_DENIED_ERROR)
        );
    }

    #[test]
    fn non_tenant_mode_always_allows() {
        let context = TenantPolicyContext::disabled();

        assert!(context.enforce_recall_scope("").is_ok());
        assert!(context.enforce_recall_scope("default").is_ok());
        assert!(context.enforce_recall_scope("tenant-beta").is_ok());
    }

    #[test]
    fn scope_entity_id_prefixes_active_tenant() {
        let context = TenantPolicyContext::enabled("tenant-alpha");
        assert_eq!(
            context.scope_entity_id("person:discord.user-1"),
            "tenant-alpha:person:discord.user-1"
        );
    }

    #[test]
    fn scope_entity_id_keeps_already_scoped_entity() {
        let context = TenantPolicyContext::enabled("tenant-alpha");
        assert_eq!(
            context.scope_entity_id("tenant-alpha:person:discord.user-1"),
            "tenant-alpha:person:discord.user-1"
        );
    }
}
