//! Persistent and session-scoped permission grant store.
//!
//! Tracks tool-level permission grants (session or permanent) and
//! entity allowlists, persisting permanent grants to disk as TOML.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::contracts::ids::EntityId;
use crate::security::approval::{GrantScope, PermissionGrant};
use crate::security::policy::TenantPolicyContext;

#[derive(Debug, Serialize, Deserialize, Default)]
struct PermissionFile {
    #[serde(default)]
    grants: Vec<StoredGrant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredGrant {
    tool: String,
    pattern: String,
    scope: GrantScope,
    granted_at: String,
    granted_by: String,
    #[serde(default)]
    entity_id: Option<EntityId>,
    #[serde(default)]
    tenant_mode_enabled: Option<bool>,
    #[serde(default)]
    tenant_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GrantSubject {
    entity_id: EntityId,
    tenant_mode_enabled: bool,
    tenant_id: Option<String>,
}

impl GrantSubject {
    fn from_context(entity_id: &str, tenant_context: &TenantPolicyContext) -> Self {
        Self {
            entity_id: EntityId::new(entity_id.trim()),
            tenant_mode_enabled: tenant_context.tenant_mode_enabled,
            tenant_id: normalize_tenant_id(tenant_context.tenant_id.as_deref()),
        }
    }

    fn from_stored(grant: &StoredGrant) -> Option<Self> {
        let entity_id = grant
            .entity_id
            .as_ref()
            .map(EntityId::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(EntityId::new)?;
        let tenant_id = normalize_tenant_id(grant.tenant_id.as_deref());
        let tenant_mode_enabled = grant.tenant_mode_enabled.unwrap_or(tenant_id.is_some());
        Some(Self {
            entity_id,
            tenant_mode_enabled,
            tenant_id,
        })
    }

    fn matches_context(&self, entity_id: &str, tenant_context: &TenantPolicyContext) -> bool {
        self == &Self::from_context(entity_id, tenant_context)
    }
}

#[derive(Debug, Clone)]
struct ScopedGrant {
    grant: PermissionGrant,
    subject: GrantSubject,
}

/// Source-of-truth store for permission records.
/// Persistent and session-scoped permission grant store.
#[derive(Debug)]
pub struct PermissionStore {
    session_grants: Mutex<Vec<ScopedGrant>>,
    permanent_grants: Mutex<Vec<ScopedGrant>>,
    permanent_records: Mutex<Vec<StoredGrant>>,
    entity_allowlists: Mutex<HashMap<EntityId, HashSet<String>>>,
    store_path: PathBuf,
}

impl PermissionStore {
    /// Load or initialize the permission store from the workspace.
    pub fn load(workspace_dir: &Path) -> Self {
        let store_path = workspace_dir.join("permissions.toml");
        let permission_file = match fs::read_to_string(&store_path) {
            Ok(content) => {
                if content.trim().is_empty() {
                    PermissionFile::default()
                } else {
                    toml::from_str(&content).unwrap_or_else(|error| {
                        tracing::warn!(
                            path = %store_path.display(),
                            %error,
                            "failed to parse permissions.toml; starting with empty grants"
                        );
                        PermissionFile::default()
                    })
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let empty = PermissionFile::default();
                if let Err(write_error) = persist_permission_file(&store_path, &empty) {
                    tracing::warn!(
                        path = %store_path.display(),
                        %write_error,
                        "failed to initialize permissions.toml"
                    );
                }
                empty
            }
            Err(error) => {
                tracing::warn!(
                    path = %store_path.display(),
                    %error,
                    "failed to read permissions.toml; starting with empty grants"
                );
                PermissionFile::default()
            }
        };

        let permanent_grants = permission_file
            .grants
            .iter()
            .filter_map(stored_to_scoped_grant)
            .collect();

        Self {
            session_grants: Mutex::new(Vec::new()),
            permanent_grants: Mutex::new(permanent_grants),
            permanent_records: Mutex::new(permission_file.grants),
            entity_allowlists: Mutex::new(HashMap::new()),
            store_path,
        }
    }

    /// Set or clear the tool allowlist for a specific entity.
    pub fn set_entity_allowlist(&self, entity_id: &str, allowlist: Option<HashSet<String>>) {
        let mut allowlists = self
            .entity_allowlists
            .lock()
            .unwrap_or_else(crate::security::poison_recover!());

        match allowlist {
            Some(allowlist) => {
                allowlists.insert(EntityId::new(entity_id), allowlist);
            }
            None => {
                allowlists.remove(&EntityId::new(entity_id));
            }
        }
    }

    /// # Errors
    ///
    /// Returns an error when grant fields are invalid, entity allowlist
    /// constraints are violated, or persistence of permanent grants fails.
    pub fn add_grant(
        &self,
        grant: PermissionGrant,
        entity_id: &str,
        tenant_context: &TenantPolicyContext,
    ) -> Result<()> {
        anyhow::ensure!(
            !grant.tool.trim().is_empty(),
            "grant tool must not be empty"
        );
        anyhow::ensure!(
            !grant.pattern.trim().is_empty(),
            "grant pattern must not be empty"
        );

        let subject = GrantSubject::from_context(entity_id, tenant_context);
        anyhow::ensure!(
            !subject.entity_id.as_str().is_empty(),
            "grant entity id must not be empty"
        );

        if grant.pattern == "*" {
            let risk = crate::security::classify_risk(&grant.tool);
            if matches!(risk, crate::security::RiskLevel::High) {
                tracing::warn!(tool = %grant.tool, "rejecting wildcard grant for high-risk tool");
                bail!(
                    "wildcard grants are not allowed for high-risk tool '{}'",
                    grant.tool
                );
            }
        }

        if let Some(allowed_tools) = self
            .entity_allowlists
            .lock()
            .unwrap_or_else(crate::security::poison_recover!())
            .get(&subject.entity_id)
            .cloned()
            && !allowed_tools.contains(&grant.tool)
        {
            bail!(
                "cannot grant tool '{}' for entity '{}': tool not in allowlist",
                grant.tool,
                subject.entity_id
            );
        }

        match grant.scope {
            GrantScope::Session => {
                self.session_grants
                    .lock()
                    .unwrap_or_else(crate::security::poison_recover!())
                    .push(ScopedGrant { grant, subject });
                Ok(())
            }
            GrantScope::Permanent => {
                let record = StoredGrant {
                    tool: grant.tool.clone(),
                    pattern: grant.pattern.clone(),
                    scope: GrantScope::Permanent,
                    granted_at: Utc::now().to_rfc3339(),
                    granted_by: subject.entity_id.to_string(),
                    entity_id: Some(subject.entity_id.clone()),
                    tenant_mode_enabled: Some(subject.tenant_mode_enabled),
                    tenant_id: subject.tenant_id.clone(),
                };

                let mut records = self
                    .permanent_records
                    .lock()
                    .unwrap_or_else(crate::security::poison_recover!());
                let mut next_records = records.clone();
                next_records.push(record);
                persist_permission_file(
                    &self.store_path,
                    &PermissionFile {
                        grants: next_records.clone(),
                    },
                )?;

                *records = next_records;
                drop(records);

                self.permanent_grants
                    .lock()
                    .unwrap_or_else(crate::security::poison_recover!())
                    .push(ScopedGrant { grant, subject });
                Ok(())
            }
        }
    }

    /// Check whether a tool invocation is covered by an existing grant for
    /// the current entity/tenant scope.
    pub fn is_granted(
        &self,
        tool_name: &str,
        args_summary: &str,
        entity_id: &str,
        tenant_context: &TenantPolicyContext,
    ) -> bool {
        let session_match = self
            .session_grants
            .lock()
            .unwrap_or_else(crate::security::poison_recover!())
            .iter()
            .any(|grant| {
                grant.grant.tool == tool_name
                    && pattern_matches(&grant.grant.pattern, args_summary)
                    && grant.subject.matches_context(entity_id, tenant_context)
            });

        if session_match {
            return true;
        }

        self.permanent_grants
            .lock()
            .unwrap_or_else(crate::security::poison_recover!())
            .iter()
            .any(|grant| {
                grant.grant.tool == tool_name
                    && pattern_matches(&grant.grant.pattern, args_summary)
                    && grant.subject.matches_context(entity_id, tenant_context)
            })
    }

    /// Return all active grants (session and permanent combined).
    pub fn active_grants(&self) -> Vec<PermissionGrant> {
        let mut grants: Vec<PermissionGrant> = self
            .session_grants
            .lock()
            .unwrap_or_else(crate::security::poison_recover!())
            .iter()
            .map(|grant| grant.grant.clone())
            .collect();
        grants.extend(
            self.permanent_grants
                .lock()
                .unwrap_or_else(crate::security::poison_recover!())
                .iter()
                .map(|grant| grant.grant.clone()),
        );
        grants
    }
}

#[must_use]
fn pattern_matches(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(" *") {
        return value.starts_with(prefix)
            && value.len() > prefix.len()
            && value.as_bytes()[prefix.len()] == b' ';
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        // Only allow trailing `*` when the prefix ends with a word boundary
        // character (space, `/`, or is empty). This prevents `"cargo*"` from
        // matching `"cargouploadsecret"` while still allowing `"cargo *"` style
        // patterns (handled above) and exact-prefix patterns like `/usr/bin/*`.
        if prefix.is_empty()
            || prefix.ends_with(' ')
            || prefix.ends_with('/')
            || prefix.ends_with('\\')
        {
            return value.starts_with(prefix);
        }
        // Treat patterns like "cargo*" as exact match (no glob).
        return pattern == value;
    }
    pattern == value
}

fn stored_to_scoped_grant(grant: &StoredGrant) -> Option<ScopedGrant> {
    let subject = GrantSubject::from_stored(grant).or_else(|| {
        tracing::warn!(
            tool = %grant.tool,
            granted_by = %grant.granted_by,
            "skipping permission record without entity_id"
        );
        None
    })?;
    Some(ScopedGrant {
        grant: PermissionGrant {
            tool: grant.tool.clone(),
            pattern: grant.pattern.clone(),
            scope: grant.scope,
        },
        subject,
    })
}

#[must_use]
fn normalize_tenant_id(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

/// Atomic write: serialize → write to temp file → rename to target.
///
/// This prevents partial writes from corrupting the permissions file on
/// crash or power loss.
fn persist_permission_file(path: &Path, data: &PermissionFile) -> Result<()> {
    let content = toml::to_string(data).context("failed to serialize permissions")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create permissions parent directory '{}'",
                parent.display()
            )
        })?;
    }

    // Write to a temp file in the same directory, then atomically rename.
    let tmp_path = path.with_extension("toml.tmp");
    fs::write(&tmp_path, &content)
        .with_context(|| format!("failed to write temp file '{}'", tmp_path.display()))?;

    if let Err(error) = crate::security::private_file_permissions::restrict_private_file(
        &tmp_path,
        "permission grant store",
    ) {
        let _ = fs::remove_file(&tmp_path);
        return Err(error);
    }

    fs::rename(&tmp_path, path)
        .inspect_err(|_error| {
            let _ = fs::remove_file(&tmp_path);
        })
        .with_context(|| {
            format!(
                "failed to rename '{}' → '{}'",
                tmp_path.display(),
                path.display()
            )
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn shell_grant(pattern: &str, scope: GrantScope) -> PermissionGrant {
        PermissionGrant {
            tool: "shell".to_string(),
            pattern: pattern.to_string(),
            scope,
        }
    }

    #[test]
    fn session_grant_works_within_session_and_clears_on_restart() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PermissionStore::load(tmp.path());
        let tenant_context = TenantPolicyContext::disabled();

        store
            .add_grant(
                shell_grant("cargo *", GrantScope::Session),
                "cli:local",
                &tenant_context,
            )
            .expect("add session grant");

        assert!(store.is_granted("shell", "cargo test", "cli:local", &tenant_context));

        let restarted = PermissionStore::load(tmp.path());
        assert!(!restarted.is_granted("shell", "cargo test", "cli:local", &tenant_context));
    }

    #[test]
    fn pattern_matching_prefix_space() {
        assert!(pattern_matches("cargo *", "cargo test"));
        assert!(!pattern_matches("cargo *", "python script.py"));
        assert!(!pattern_matches("cargo *", "cargo"));
    }

    #[test]
    fn pattern_matching_wildcard_everything() {
        assert!(pattern_matches("*", "cargo test"));
        assert!(pattern_matches("*", "anything"));
    }

    #[test]
    fn pattern_matching_exact_only() {
        assert!(pattern_matches("cargo test", "cargo test"));
        assert!(!pattern_matches("cargo test", "cargo test --lib"));
    }

    #[test]
    fn is_granted_false_when_no_grants() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PermissionStore::load(tmp.path());
        assert!(!store.is_granted(
            "shell",
            "cargo test",
            "cli:local",
            &TenantPolicyContext::disabled(),
        ));
    }

    #[test]
    fn permanent_grant_serializes_and_deserializes_toml() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PermissionStore::load(tmp.path());
        let tenant_context = TenantPolicyContext::enabled("tenant-alpha");
        store
            .add_grant(
                shell_grant("cargo *", GrantScope::Permanent),
                "cli:local",
                &tenant_context,
            )
            .expect("add permanent grant");

        let file_content =
            fs::read_to_string(tmp.path().join("permissions.toml")).expect("read permissions");
        let parsed: PermissionFile = toml::from_str(&file_content).expect("parse permissions");

        assert_eq!(parsed.grants.len(), 1);
        assert_eq!(parsed.grants[0].tool, "shell");
        assert_eq!(parsed.grants[0].pattern, "cargo *");
        assert_eq!(parsed.grants[0].scope, GrantScope::Permanent);
        assert_eq!(parsed.grants[0].granted_by, "cli:local");
        assert_eq!(
            parsed.grants[0].entity_id.as_ref().map(EntityId::as_str),
            Some("cli:local")
        );
        assert_eq!(parsed.grants[0].tenant_mode_enabled, Some(true));
        assert_eq!(parsed.grants[0].tenant_id.as_deref(), Some("tenant-alpha"));
        assert!(!parsed.grants[0].granted_at.is_empty());
    }

    #[test]
    fn cannot_grant_tool_not_in_entity_allowlist() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PermissionStore::load(tmp.path());
        store.set_entity_allowlist("entity:1", Some(HashSet::from(["file_read".to_string()])));

        let grant = PermissionGrant {
            tool: "shell".to_string(),
            pattern: "cargo *".to_string(),
            scope: GrantScope::Session,
        };

        assert!(
            store
                .add_grant(grant, "entity:1", &TenantPolicyContext::disabled())
                .is_err()
        );
    }

    #[test]
    fn active_grants_returns_session_and_permanent() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PermissionStore::load(tmp.path());
        let tenant_context = TenantPolicyContext::disabled();

        store
            .add_grant(
                shell_grant("cargo test", GrantScope::Session),
                "cli:local",
                &tenant_context,
            )
            .expect("add session grant");
        store
            .add_grant(
                shell_grant("cargo *", GrantScope::Permanent),
                "cli:local",
                &tenant_context,
            )
            .expect("add permanent grant");

        let grants = store.active_grants();
        assert_eq!(grants.len(), 2);
        assert!(
            grants
                .iter()
                .any(|grant| grant.scope == GrantScope::Session && grant.pattern == "cargo test")
        );
        assert!(
            grants
                .iter()
                .any(|grant| grant.scope == GrantScope::Permanent && grant.pattern == "cargo *")
        );
    }

    #[test]
    fn grant_is_scoped_to_entity() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PermissionStore::load(tmp.path());
        let tenant_context = TenantPolicyContext::disabled();

        store
            .add_grant(
                shell_grant("cargo *", GrantScope::Session),
                "entity:one",
                &tenant_context,
            )
            .expect("add session grant");

        assert!(store.is_granted("shell", "cargo test", "entity:one", &tenant_context));
        assert!(!store.is_granted("shell", "cargo test", "entity:two", &tenant_context));
    }

    #[test]
    fn grant_is_scoped_to_tenant_context() {
        let tmp = TempDir::new().expect("tempdir");
        let store = PermissionStore::load(tmp.path());
        let tenant_alpha = TenantPolicyContext::enabled("tenant-alpha");
        let tenant_beta = TenantPolicyContext::enabled("tenant-beta");

        store
            .add_grant(
                shell_grant("cargo *", GrantScope::Session),
                "gateway:user-1",
                &tenant_alpha,
            )
            .expect("add tenant-scoped grant");

        assert!(store.is_granted("shell", "cargo test", "gateway:user-1", &tenant_alpha,));
        assert!(!store.is_granted("shell", "cargo test", "gateway:user-1", &tenant_beta,));
    }

    #[test]
    fn legacy_records_without_entity_id_are_skipped() {
        let tmp = TempDir::new().expect("tempdir");
        fs::write(
            tmp.path().join("permissions.toml"),
            r#"
[[grants]]
tool = "shell"
pattern = "cargo *"
scope = "permanent"
granted_at = "2026-01-01T00:00:00Z"
granted_by = "legacy:entity"
"#,
        )
        .expect("write legacy permissions file");

        let store = PermissionStore::load(tmp.path());
        let tenant_context = TenantPolicyContext::disabled();

        assert!(
            !store.is_granted("shell", "cargo test", "legacy:entity", &tenant_context),
            "records without entity_id should be skipped"
        );
    }
}
