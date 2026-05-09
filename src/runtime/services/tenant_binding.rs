use std::collections::HashMap;
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::config::Config;

pub type TenantBindingStore = Arc<Mutex<HashMap<String, String>>>;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PersistedTenantBindingStore {
    #[serde(default)]
    bindings: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantBindingRecord {
    pub principal_hash: String,
    pub tenant_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantBindingInventory {
    pub bindings: Vec<TenantBindingRecord>,
    pub discovered_workspaces: Vec<String>,
    pub binding_count: usize,
    pub workspace_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantContextView {
    pub active_tenant: Option<String>,
    pub tenant_header: Option<String>,
    pub tenant_mode_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantContextUpdate {
    pub tenant_id: Option<String>,
}

fn gateway_admin_state_dir(config: &Config) -> PathBuf {
    config.workspace_dir.join(".asterel").join("gateway")
}

fn tenant_bindings_path(config: &Config) -> PathBuf {
    gateway_admin_state_dir(config).join("tenant-bindings.json")
}

fn load_json_or_default<T>(path: &Path) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    if !path.exists() {
        return Ok(T::default());
    }

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read persisted admin state {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("parse persisted admin state {}", path.display()))
}

fn save_json<T>(path: &Path, value: &T) -> Result<()>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create admin state directory {}", parent.display()))?;
    }

    let serialized = serde_json::to_vec_pretty(value)
        .with_context(|| format!("serialize admin state {}", path.display()))?;
    let tmp_path = atomic_write_tmp_path(path);
    std::fs::write(&tmp_path, serialized)
        .with_context(|| format!("write temp admin state {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "replace persisted admin state {} from {}",
            path.display(),
            tmp_path.display()
        )
    })
}

fn atomic_write_tmp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("tenant-bindings.json");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    path.with_file_name(format!(".{file_name}.{nonce}.tmp"))
}

/// Load persisted tenant bindings from disk.
///
/// # Errors
///
/// Returns an error when the persisted tenant binding file cannot be read or parsed.
fn load_persisted_bindings(config: &Config) -> Result<HashMap<String, String>> {
    Ok(
        load_json_or_default::<PersistedTenantBindingStore>(&tenant_bindings_path(config))?
            .bindings,
    )
}

#[must_use]
pub fn load_persisted_bindings_best_effort(config: &Config) -> HashMap<String, String> {
    load_persisted_bindings(config).unwrap_or_else(|error| {
        tracing::warn!(%error, "failed to load persisted tenant bindings");
        HashMap::new()
    })
}

#[must_use]
pub fn new_tenant_binding_store(config: &Config) -> TenantBindingStore {
    Arc::new(Mutex::new(load_persisted_bindings_best_effort(config)))
}

#[cfg(test)]
#[must_use]
pub fn new_empty_tenant_binding_store() -> TenantBindingStore {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Save tenant bindings to disk.
///
/// # Errors
///
/// Returns an error when the binding store cannot be serialized or written.
fn save_persisted_bindings<S: BuildHasher>(
    config: &Config,
    bindings: &HashMap<String, String, S>,
) -> Result<()> {
    save_json(
        &tenant_bindings_path(config),
        &PersistedTenantBindingStore {
            bindings: bindings
                .iter()
                .map(|(principal, tenant_id)| (principal.clone(), tenant_id.clone()))
                .collect(),
        },
    )
}

/// Load the admin/operator tenant binding inventory, merging persisted and runtime state.
///
/// # Errors
///
/// Returns an error when persisted tenant bindings cannot be read or parsed.
pub async fn list_operator_tenant_inventory<S: BuildHasher>(
    tenant_bindings: &Mutex<HashMap<String, String, S>>,
    config: &Config,
) -> Result<TenantBindingInventory> {
    let mut bindings = load_persisted_bindings(config)?;
    {
        let in_memory = tenant_bindings
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for (principal, tenant_id) in &*in_memory {
            bindings.insert(principal.clone(), tenant_id.clone());
        }
    }

    let mut bound_tenants = bindings
        .into_iter()
        .map(|(principal, tenant_id)| TenantBindingRecord {
            principal_hash: principal[..principal.len().min(12)].to_string(),
            tenant_id,
        })
        .collect::<Vec<_>>();
    bound_tenants.sort_by(|lhs, rhs| lhs.tenant_id.cmp(&rhs.tenant_id));

    let tenants_dir = config.workspace_dir.join("tenants");
    let mut discovered_workspaces = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(&tenants_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Ok(file_type) = entry.file_type().await
                && file_type.is_dir()
                && let Some(name) = entry.file_name().to_str()
            {
                discovered_workspaces.push(name.to_string());
            }
        }
    }
    discovered_workspaces.sort();

    Ok(TenantBindingInventory {
        binding_count: bound_tenants.len(),
        workspace_count: discovered_workspaces.len(),
        bindings: bound_tenants,
        discovered_workspaces,
    })
}

#[must_use]
pub fn load_operator_tenant_context<S: BuildHasher>(
    tenant_bindings: &Mutex<HashMap<String, String, S>>,
    config: &Config,
    principal: &str,
    tenant_header: Option<&str>,
) -> TenantContextView {
    let tenant_header = tenant_header
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let bound_tenant = resolve_selected_tenant_for_principal(tenant_bindings, config, principal);

    TenantContextView {
        active_tenant: tenant_header.clone().or(bound_tenant),
        tenant_header,
        tenant_mode_available: true,
    }
}

/// Persist or clear the admin/operator tenant context for a principal.
///
/// # Errors
///
/// Returns an error when the persisted tenant binding file cannot be read, written, or parsed.
pub fn update_operator_tenant_context<S: BuildHasher>(
    tenant_bindings: &Mutex<HashMap<String, String, S>>,
    config: &Config,
    principal: &str,
    tenant_id: Option<&str>,
) -> Result<TenantContextUpdate> {
    let tenant_id = tenant_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    let mut runtime_bindings = tenant_bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let principal_key = principal.to_string();
    if let Some(tenant_id) = tenant_id.as_deref() {
        let previous = runtime_bindings.insert(principal_key.clone(), tenant_id.to_string());
        if let Err(error) = save_persisted_bindings(config, &*runtime_bindings) {
            match previous {
                Some(previous_tenant) => {
                    runtime_bindings.insert(principal_key, previous_tenant);
                }
                None => {
                    runtime_bindings.remove(principal);
                }
            }
            return Err(error);
        }
    } else {
        let previous = runtime_bindings.remove(principal);
        if let Err(error) = save_persisted_bindings(config, &*runtime_bindings) {
            if let Some(previous_tenant) = previous {
                runtime_bindings.insert(principal_key, previous_tenant);
            }
            return Err(error);
        }
    }

    Ok(TenantContextUpdate { tenant_id })
}

#[must_use]
pub fn resolve_selected_tenant_for_principal<S: BuildHasher>(
    tenant_bindings: &Mutex<HashMap<String, String, S>>,
    config: &Config,
    principal: &str,
) -> Option<String> {
    {
        let bindings = tenant_bindings
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(bound) = bindings.get(principal) {
            return Some(bound.clone());
        }
    }

    let persisted = load_persisted_bindings_best_effort(config);
    let mut bindings = tenant_bindings
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for (persisted_principal, tenant_id) in persisted {
        bindings.entry(persisted_principal).or_insert(tenant_id);
    }
    bindings.get(principal).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(workspace_dir: &Path) -> Config {
        Config {
            workspace_dir: workspace_dir.to_path_buf(),
            ..Config::default()
        }
    }

    #[test]
    fn update_operator_tenant_context_persists_existing_runtime_bindings() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let config = test_config(temp_dir.path());
        let store = new_empty_tenant_binding_store();

        {
            let mut bindings = store.lock().unwrap_or_else(PoisonError::into_inner);
            bindings.insert("principal-a".to_string(), "tenant-a".to_string());
        }

        update_operator_tenant_context(&store, &config, "principal-b", Some("tenant-b"))
            .expect("tenant update should persist");

        let persisted = load_persisted_bindings(&config).expect("bindings should load");
        assert_eq!(
            persisted.get("principal-a").map(String::as_str),
            Some("tenant-a")
        );
        assert_eq!(
            persisted.get("principal-b").map(String::as_str),
            Some("tenant-b")
        );
    }

    #[test]
    fn update_operator_tenant_context_clear_preserves_other_runtime_bindings() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let config = test_config(temp_dir.path());
        let store = new_empty_tenant_binding_store();

        {
            let mut bindings = store.lock().unwrap_or_else(PoisonError::into_inner);
            bindings.insert("principal-a".to_string(), "tenant-a".to_string());
            bindings.insert("principal-b".to_string(), "tenant-b".to_string());
        }

        update_operator_tenant_context(&store, &config, "principal-a", None)
            .expect("tenant clear should persist");

        let persisted = load_persisted_bindings(&config).expect("bindings should load");
        assert!(!persisted.contains_key("principal-a"));
        assert_eq!(
            persisted.get("principal-b").map(String::as_str),
            Some("tenant-b")
        );
    }
}
