use crate::security::policy::TenantPolicyContext;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperatorScope {
    pub principal: String,
    pub tenant_id: Option<String>,
    pub tenant_mode_available: bool,
}

impl OperatorScope {
    #[must_use]
    pub fn from_management_context(principal: String, policy_context: TenantPolicyContext) -> Self {
        Self {
            principal,
            tenant_id: policy_context.tenant_id,
            tenant_mode_available: policy_context.tenant_mode_enabled,
        }
    }

    #[must_use]
    pub fn has_tenant_scope(&self) -> bool {
        self.tenant_id.is_some()
    }

    #[must_use]
    pub fn require_tenant_id(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }
}
