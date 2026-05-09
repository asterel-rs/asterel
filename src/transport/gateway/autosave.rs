//! Gateway autosave helpers: tenant-scoped workspace resolution, entity ID
//! derivation, and memory ingestion for webhook and API interactions.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result, anyhow};

use crate::contracts::ids::EntityId;
use crate::contracts::strings::data_model::SLOT_EXTERNAL_GATEWAY_WEBHOOK;
use crate::core::memory::{
    MemoryEventInput, MemoryEventType, MemoryLayer, MemoryProvenance, MemorySource, PrivacyLevel,
    SourceKind,
};
use crate::core::persona::person_identity::channel_entity_id;
use crate::security::policy::TenantPolicyContext;
use crate::security::{RootBoundPathKind, canonicalize_path_within_root};

const MAX_TENANT_ID_LEN: usize = 64;

/// Validates and normalizes a tenant ID, rejecting empty, overlong,
/// or non-alphanumeric values.
pub(crate) fn sanitize_tenant_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_TENANT_ID_LEN {
        return None;
    }
    if trimmed == "." || trimmed == ".." {
        return None;
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn resolve_runtime_tenant_id(tenant_id: Option<&str>) -> Option<String> {
    tenant_id.and_then(sanitize_tenant_id).or_else(|| {
        std::env::var("ASTEREL_TENANT_ID")
            .ok()
            .or_else(|| std::env::var("TENANT_ID").ok())
            .and_then(|value| sanitize_tenant_id(&value))
    })
}

/// Derives a person entity ID for a gateway autosave source.
pub(super) fn gateway_autosave_entity_id(source: &str) -> EntityId {
    EntityId::new(channel_entity_id("gateway", source))
}

/// Builds a tenant policy context from an optional tenant ID,
/// falling back to environment variables.
pub(super) fn gateway_runtime_policy_context(tenant_id: Option<&str>) -> TenantPolicyContext {
    let resolved = resolve_runtime_tenant_id(tenant_id);

    resolved.map_or_else(TenantPolicyContext::disabled, TenantPolicyContext::enabled)
}

/// Resolves the workspace directory for a tenant, creating it if
/// needed and validating it stays within the workspace root.
pub(super) async fn tenant_workspace_dir(
    workspace_dir: &Path,
    policy_context: &TenantPolicyContext,
    scope: &'static str,
) -> Result<PathBuf> {
    if policy_context.tenant_mode_enabled
        && let Some(tenant_id) = policy_context.tenant_id.as_deref()
    {
        let workspace_root = canonical_workspace_root(workspace_dir).await?;
        let scoped = workspace_dir.join("tenants").join(tenant_id);
        if tokio::fs::metadata(&scoped).await.is_err()
            && let Err(error) = tokio::fs::create_dir_all(&scoped).await
        {
            tracing::warn!(
                error = %error,
                tenant_id,
                scope,
                "failed to create tenant scoped workspace"
            );
            return Err(error).with_context(|| {
                format!("failed to create tenant scoped workspace for tenant {tenant_id}")
            });
        }

        match canonicalize_path_within_root(&scoped, &workspace_root, RootBoundPathKind::Directory)
        {
            Ok(resolved_scoped) => {
                return Ok(resolved_scoped);
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    tenant_id,
                    scope,
                    scoped_path = %scoped.display(),
                    workspace_root = %workspace_root.display(),
                    "failed to resolve tenant workspace inside root"
                );
                return Err(anyhow!(error)).with_context(|| {
                    format!("failed to resolve tenant workspace for tenant {tenant_id}")
                });
            }
        }
    }

    Ok(cached_workspace_root(workspace_dir).await)
}

fn workspace_root_cache() -> &'static Mutex<HashMap<PathBuf, PathBuf>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, PathBuf>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn cached_workspace_root(workspace_dir: &Path) -> PathBuf {
    if let Some(cached) = workspace_root_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(workspace_dir)
        .cloned()
    {
        return cached;
    }

    let resolved = tokio::fs::canonicalize(workspace_dir)
        .await
        .unwrap_or_else(|_| workspace_dir.to_path_buf());
    let mut cache = workspace_root_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache
        .entry(workspace_dir.to_path_buf())
        .or_insert_with(|| resolved.clone())
        .clone()
}

async fn canonical_workspace_root(workspace_dir: &Path) -> Result<PathBuf> {
    if let Some(cached) = workspace_root_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(workspace_dir)
        .cloned()
    {
        return Ok(cached);
    }

    let resolved = tokio::fs::canonicalize(workspace_dir)
        .await
        .with_context(|| {
            format!(
                "failed to canonicalize workspace root {}",
                workspace_dir.display()
            )
        })?;
    let mut cache = workspace_root_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache
        .entry(workspace_dir.to_path_buf())
        .or_insert_with(|| resolved.clone());
    Ok(resolved)
}

/// Prefixes an entity ID with the tenant ID when tenant mode is
/// active.
pub(super) fn tenant_scoped_entity_id(
    base_entity_id: EntityId,
    policy_context: &TenantPolicyContext,
) -> EntityId {
    if policy_context.tenant_mode_enabled
        && let Some(tenant_id) = policy_context.tenant_id.as_deref()
    {
        return EntityId::new(format!("{tenant_id}:{base_entity_id}"));
    }
    base_entity_id
}

/// Creates a memory event for persisting a webhook interaction.
pub(super) fn gateway_webhook_autosave_event(entity_id: &str, summary: String) -> MemoryEventInput {
    MemoryEventInput::new(
        entity_id,
        SLOT_EXTERNAL_GATEWAY_WEBHOOK,
        MemoryEventType::FactAdded,
        summary,
        MemorySource::ExplicitUser,
        PrivacyLevel::Private,
    )
    .with_layer(MemoryLayer::Working)
    .with_confidence(0.95)
    .with_importance(0.5)
    .with_source_kind(SourceKind::Api)
    .with_source_ref("gateway:webhook")
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::ExplicitUser,
        "gateway.autosave.webhook",
    ))
}

/// Creates a memory event for persisting a `WhatsApp` interaction.
#[cfg(feature = "whatsapp")]
pub(super) fn gateway_whatsapp_autosave_event(
    entity_id: &str,
    sender: &str,
    summary: String,
) -> MemoryEventInput {
    MemoryEventInput::new(
        entity_id,
        format!("external.whatsapp.{sender}"),
        MemoryEventType::FactAdded,
        summary,
        MemorySource::ExplicitUser,
        PrivacyLevel::Private,
    )
    .with_layer(MemoryLayer::Working)
    .with_confidence(0.95)
    .with_importance(0.6)
    .with_source_kind(SourceKind::Api)
    .with_source_ref(format!("gateway:whatsapp:{sender}"))
    .with_provenance(MemoryProvenance::source_reference(
        MemorySource::ExplicitUser,
        "gateway.autosave.whatsapp",
    ))
}
