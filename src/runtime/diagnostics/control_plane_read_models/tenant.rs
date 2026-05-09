use serde::{Deserialize, Serialize};

use crate::runtime::services::{TenantBindingInventory, TenantContextView};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TenantRegistryRowReadModel {
    pub tenant_id: String,
    pub principal_hashes: Vec<String>,
    pub binding_count: usize,
    pub workspace_present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TenantInventoryReadModel {
    pub rows: Vec<TenantRegistryRowReadModel>,
    pub discovered_workspaces: Vec<String>,
    pub binding_count: usize,
    pub workspace_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TenantContextReadModel {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_tenant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_header: Option<String>,
    pub tenant_mode_available: bool,
}

#[must_use]
pub fn build_tenant_inventory_read_model(
    inventory: &TenantBindingInventory,
    tenant_scope: Option<&str>,
) -> TenantInventoryReadModel {
    let mut rows = std::collections::BTreeMap::<String, Vec<String>>::new();
    for binding in &inventory.bindings {
        rows.entry(binding.tenant_id.clone())
            .or_default()
            .push(binding.principal_hash.clone());
    }

    let workspace_set = inventory
        .discovered_workspaces
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();

    let rows = rows
        .into_iter()
        .map(|(tenant_id, principal_hashes)| TenantRegistryRowReadModel {
            binding_count: principal_hashes.len(),
            workspace_present: workspace_set.contains(&tenant_id),
            tenant_id,
            principal_hashes,
        })
        .collect();

    TenantInventoryReadModel {
        rows,
        discovered_workspaces: inventory.discovered_workspaces.clone(),
        binding_count: inventory.binding_count,
        workspace_count: inventory.workspace_count,
        tenant_scope: tenant_scope.map(ToString::to_string),
    }
}

#[must_use]
pub fn build_tenant_context_read_model(view: &TenantContextView) -> TenantContextReadModel {
    TenantContextReadModel {
        active_tenant: view.active_tenant.clone(),
        tenant_header: view.tenant_header.clone(),
        tenant_mode_available: view.tenant_mode_available,
    }
}

#[cfg(test)]
mod tests {
    use super::{build_tenant_context_read_model, build_tenant_inventory_read_model};
    use crate::runtime::services::{
        TenantBindingInventory, TenantBindingRecord, TenantContextView,
    };

    #[test]
    fn inventory_read_model_groups_bindings_by_tenant() {
        let model = build_tenant_inventory_read_model(
            &TenantBindingInventory {
                bindings: vec![
                    TenantBindingRecord {
                        principal_hash: "abc123".to_string(),
                        tenant_id: "tenant-a".to_string(),
                    },
                    TenantBindingRecord {
                        principal_hash: "def456".to_string(),
                        tenant_id: "tenant-a".to_string(),
                    },
                    TenantBindingRecord {
                        principal_hash: "zzz999".to_string(),
                        tenant_id: "tenant-b".to_string(),
                    },
                ],
                discovered_workspaces: vec!["tenant-a".to_string(), "tenant-c".to_string()],
                binding_count: 3,
                workspace_count: 2,
            },
            Some("tenant-a"),
        );

        assert_eq!(model.rows.len(), 2);
        assert_eq!(model.rows[0].tenant_id, "tenant-a");
        assert_eq!(model.rows[0].binding_count, 2);
        assert!(model.rows[0].workspace_present);
        assert_eq!(model.tenant_scope.as_deref(), Some("tenant-a"));
    }

    #[test]
    fn tenant_context_read_model_round_trips_fields() {
        let model = build_tenant_context_read_model(&TenantContextView {
            active_tenant: Some("tenant-a".to_string()),
            tenant_header: None,
            tenant_mode_available: true,
        });

        assert_eq!(model.active_tenant.as_deref(), Some("tenant-a"));
        assert!(model.tenant_header.is_none());
        assert!(model.tenant_mode_available);
    }
}
