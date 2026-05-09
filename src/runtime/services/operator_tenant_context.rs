use anyhow::Result;

use super::operator_scope::OperatorScope;
use super::tenant_binding::{
    TenantBindingInventory, TenantContextUpdate, TenantContextView, list_operator_tenant_inventory,
    load_operator_tenant_context, update_operator_tenant_context,
};
use crate::runtime::diagnostics::control_plane_read_models::{
    TenantContextReadModel, TenantInventoryReadModel, build_tenant_context_read_model,
    build_tenant_inventory_read_model,
};
use crate::runtime::services::TenantBindingStore;

/// # Errors
/// Returns an error if tenant inventory cannot be loaded.
pub async fn list_operator_tenants(
    tenant_bindings: &TenantBindingStore,
    config: &crate::config::Config,
    scope: &OperatorScope,
) -> Result<TenantBindingInventory> {
    let mut inventory = list_operator_tenant_inventory(tenant_bindings, config).await?;
    if let Some(tenant_scope) = scope.tenant_id.as_ref() {
        inventory
            .bindings
            .retain(|binding| &binding.tenant_id == tenant_scope);
        inventory
            .discovered_workspaces
            .retain(|tenant_id| tenant_id == tenant_scope);
        inventory.binding_count = inventory.bindings.len();
        inventory.workspace_count = inventory.discovered_workspaces.len();
    }

    Ok(inventory)
}

#[must_use]
pub fn load_operator_tenant_context_view(
    tenant_bindings: &TenantBindingStore,
    config: &crate::config::Config,
    principal: &str,
    tenant_header: Option<&str>,
) -> TenantContextView {
    load_operator_tenant_context(tenant_bindings, config, principal, tenant_header)
}

/// # Errors
/// Returns an error if tenant inventory cannot be loaded.
pub async fn load_operator_tenant_inventory_read_model(
    tenant_bindings: &TenantBindingStore,
    config: &crate::config::Config,
    scope: &OperatorScope,
) -> Result<TenantInventoryReadModel> {
    let inventory = list_operator_tenants(tenant_bindings, config, scope).await?;
    Ok(build_tenant_inventory_read_model(
        &inventory,
        scope.tenant_id.as_deref(),
    ))
}

#[must_use]
pub fn load_operator_tenant_context_read_model(
    tenant_bindings: &TenantBindingStore,
    config: &crate::config::Config,
    principal: &str,
    tenant_header: Option<&str>,
) -> TenantContextReadModel {
    build_tenant_context_read_model(&load_operator_tenant_context_view(
        tenant_bindings,
        config,
        principal,
        tenant_header,
    ))
}

/// # Errors
/// Returns an error if the operator tenant context cannot be persisted.
pub fn update_operator_tenant_context_view(
    tenant_bindings: &TenantBindingStore,
    config: &crate::config::Config,
    principal: &str,
    tenant_id: Option<&str>,
) -> Result<TenantContextUpdate> {
    update_operator_tenant_context(tenant_bindings, config, principal, tenant_id)
}
