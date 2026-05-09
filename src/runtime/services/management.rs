//! Shared control-plane management for config-backed runtime surfaces.
//!
//! This module is the canonical owner for persisted `channels` / `skills`
//! operator mutations so HTTP handlers stay thin and the daemon can apply
//! changes through its existing live-reload loop.

mod channel_runtime;
mod channels;
mod config_store;
mod skills;

use anyhow::Result;
use serde_json::Value;

use crate::config::Config;
use crate::plugins::skills::SkillTool;
use crate::security::SecurityPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeApplyMode {
    DaemonLiveReload,
    RestartRequired,
}

impl RuntimeApplyMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DaemonLiveReload => "daemon_live_reload",
            Self::RestartRequired => "restart_required",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedRuntimeOwner {
    CliSurface,
    GatewaySurface,
    ChannelsSurface,
}

impl ManagedRuntimeOwner {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CliSurface => "cli_surface",
            Self::GatewaySurface => "gateway_surface",
            Self::ChannelsSurface => "channels_surface",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedChannelRecord {
    pub id: String,
    pub display_name: String,
    pub configured: bool,
    pub enabled: bool,
    pub supported: bool,
    pub owner: ManagedRuntimeOwner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedChannelInventory {
    pub items: Vec<ManagedChannelRecord>,
    pub active_names: Vec<String>,
    pub high_freedom: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelMutationResult {
    pub record: ManagedChannelRecord,
    pub changes: Vec<String>,
    pub apply_mode: RuntimeApplyMode,
    pub reload_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelActionResult {
    pub record: ManagedChannelRecord,
    pub action: String,
    pub status: String,
    pub detail: Option<String>,
    pub apply_mode: Option<RuntimeApplyMode>,
    pub reload_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedSkillRecord {
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: Option<String>,
    pub tags: Vec<String>,
    pub tools: Vec<SkillTool>,
    pub enabled: bool,
    pub location: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillMutationResult {
    pub skill_id: String,
    pub enabled: bool,
    pub changes: Vec<String>,
    pub apply_mode: RuntimeApplyMode,
}

/// Load channel inventory for admin/operator surfaces.
///
/// # Errors
///
/// Returns an error if the persisted config snapshot cannot be loaded.
pub fn list_admin_channels(current: &Config) -> Result<ManagedChannelInventory> {
    channels::list_admin_channels(current)
}

/// Create a config-backed channel through the runtime-owned admin command surface.
///
/// # Errors
///
/// Returns an error if the channel type is unknown, unsupported, already
/// configured, the config payload is missing or invalid, or persistence fails.
pub fn create_admin_channel(
    current: &Config,
    channel_type: &str,
    raw_config: Option<Value>,
) -> Result<ChannelMutationResult> {
    channels::create_admin_channel(current, channel_type, raw_config)
}

/// Update persisted channel state through the runtime-owned admin command surface.
///
/// # Errors
///
/// Returns an error if the channel type is unknown, unsupported, not
/// configured, the config payload is invalid, or persistence fails.
pub fn update_admin_channel(
    current: &Config,
    channel_id: &str,
    enabled: Option<bool>,
    raw_config: Option<Value>,
) -> Result<ChannelMutationResult> {
    channels::update_channel(current, channel_id, enabled, raw_config)
}

/// Execute an admin/operator channel action through the runtime-owned command surface.
///
/// # Errors
///
/// Returns an error if the channel or action is unsupported, the channel is
/// not configured, or the persisted config snapshot cannot be loaded.
pub async fn run_admin_channel_action(
    current: &Config,
    channel_id: &str,
    action: &str,
) -> Result<ChannelActionResult> {
    channel_runtime::run_admin_channel_action(current, channel_id, action).await
}

/// Load installed skills for admin/operator surfaces, including disabled entries.
///
/// # Errors
///
/// Returns an error if the persisted config snapshot cannot be loaded.
pub fn list_admin_skills(
    current: &Config,
    security: &SecurityPolicy,
) -> Result<Vec<ManagedSkillRecord>> {
    skills::list_admin_skills(current, security)
}

/// Install a skill from a reviewed local source path through the runtime-owned admin surface.
///
/// # Errors
///
/// Returns an error if the persisted config snapshot cannot be loaded or the
/// install command fails.
pub fn install_admin_skill(
    current: &Config,
    security: &SecurityPolicy,
    source: &str,
) -> Result<()> {
    skills::install_admin_skill(current, security, source)
}

/// Remove an installed skill through the runtime-owned admin command surface.
///
/// # Errors
///
/// Returns an error if the persisted config snapshot cannot be loaded, skill
/// removal fails, or the updated config cannot be persisted.
pub fn remove_admin_skill(
    current: &Config,
    security: &SecurityPolicy,
    skill_id: &str,
) -> Result<()> {
    skills::remove_admin_skill(current, security, skill_id)
}

/// Toggle persisted enabled state for an installed skill through the runtime-owned admin surface.
///
/// # Errors
///
/// Returns an error if the persisted config snapshot cannot be loaded, the
/// skill is missing, or the updated config cannot be persisted.
pub fn update_admin_skill(
    current: &Config,
    security: &SecurityPolicy,
    skill_id: &str,
    enabled: bool,
) -> Result<SkillMutationResult> {
    skills::update_admin_skill(current, security, skill_id, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChannelSecurityPolicy, SkillSource};
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        Config {
            workspace_dir: tmp.path().to_path_buf(),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        }
    }

    fn write_skill(workspace: &TempDir, name: &str) {
        let skill_dir = workspace.path().join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        std::fs::write(
            skill_dir.join("extension.toml"),
            format!(
                "[extension]\nid = \"{name}\"\nkind = \"skill\"\ndescription = \"{name}\"\n\n[skill]\nprompt_bodies = [\"SKILL.md\"]\n"
            ),
        )
        .expect("write manifest");
        std::fs::write(skill_dir.join("SKILL.md"), format!("# {name}\n")).expect("write prompt");
    }

    #[test]
    fn managed_channel_inventory_marks_disabled_channels() {
        let tmp = TempDir::new().expect("tempdir");
        let mut config = test_config(&tmp);
        config.channels_config.discord = Some(crate::config::DiscordConfig {
            bot_token: "token".to_string(),
            application_id: None,
            guild_id: None,
            allowed_users: Vec::new(),
            intents: None,
            status: None,
            default_account: None,
            default_to: None,
            activity_type: None,
            activity_name: None,
            thinking_embed: false,
            thinking_embed_include_preview: false,
            pickup_policy: crate::config::DiscordPickupPolicyConfig::default(),
            security: ChannelSecurityPolicy::default(),
        });
        config.channels_config.disabled_channels = vec!["discord".to_string()];
        config.save().expect("save config");

        let inventory = list_admin_channels(&config).expect("inventory");
        let discord = inventory
            .items
            .iter()
            .find(|item| item.id == "discord")
            .expect("discord record");
        assert!(discord.configured);
        assert!(!discord.enabled);
        assert_eq!(discord.owner, ManagedRuntimeOwner::ChannelsSurface);
    }

    #[test]
    fn managed_skill_listing_includes_disabled_skills() {
        let tmp = TempDir::new().expect("tempdir");
        write_skill(&tmp, "ops-review");
        let mut config = test_config(&tmp);
        config.skills.source_priority = vec![SkillSource::Workspace];
        config.skills.disabled_skills = vec!["ops-review".to_string()];
        config.save().expect("save config");

        let items = list_admin_skills(&config, &SecurityPolicy::default()).expect("skills");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "ops-review");
        assert!(!items[0].enabled);
    }

    #[test]
    fn update_managed_skill_persists_disabled_state() {
        let tmp = TempDir::new().expect("tempdir");
        write_skill(&tmp, "ops-review");
        let mut config = test_config(&tmp);
        config.skills.source_priority = vec![SkillSource::Workspace];
        config.save().expect("save config");

        let result = update_admin_skill(&config, &SecurityPolicy::default(), "ops-review", false)
            .expect("disable skill");
        assert_eq!(result.skill_id, "ops-review");
        assert!(!result.enabled);

        let reloaded = config_store::load_persisted_runtime_config(&config).expect("reload config");
        assert_eq!(
            reloaded.skills.disabled_skills,
            vec!["ops-review".to_string()]
        );
    }
}
