use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, bail};

use super::channels::{ManagedChannelKind, set_channel_enabled, update_channel};
use super::config_store::load_persisted_runtime_config;
use super::{ChannelActionResult, ChannelMutationResult, ManagedRuntimeOwner, RuntimeApplyMode};
use crate::config::Config;
use crate::security::SecurityPolicy;
use crate::transport::channels::factory;
use crate::transport::channels::{ChannelHealthState, classify_health_result};

fn channel_action_detail(result: &ChannelMutationResult, verb: &str) -> String {
    match (
        result.record.owner,
        result.reload_requested,
        result.apply_mode,
    ) {
        (ManagedRuntimeOwner::ChannelsSurface, true, _) => {
            format!("channel {verb}d in persisted config; channel surface reload requested")
        }
        (ManagedRuntimeOwner::ChannelsSurface, false, RuntimeApplyMode::DaemonLiveReload) => {
            format!(
                "channel {verb}d in persisted config; daemon config reload will refresh listeners"
            )
        }
        _ => format!("channel {verb}d in persisted config"),
    }
}

async fn run_channel_health_check(
    config: &Config,
    kind: ManagedChannelKind,
) -> Result<(String, Option<String>)> {
    match kind {
        ManagedChannelKind::Cli => {
            Ok((
                "skipped".to_string(),
                Some(
                    "CLI is interactive and does not expose a supervised listener health check."
                        .to_string(),
                ),
            ))
        }
        ManagedChannelKind::Webhook => {
            Ok((
                "skipped".to_string(),
                Some(
                    "Webhook ingress is owned by the gateway surface and does not run a separate channel listener."
                        .to_string(),
                ),
            ))
        }
        _ => {
            kind.ensure_mutable()?;
            let mut probe = config.clone();
            set_channel_enabled(&mut probe.channels_config, kind.id(), true);
            let security = Arc::new(SecurityPolicy::from_config_runtime(
                &probe.autonomy,
                &probe.runtime,
                &probe.workspace_dir,
            ));
            let Some(entry) = factory::build_channels(probe.channels_config.clone(), &security)
                .into_iter()
                .find(|entry| entry.channel.name() == kind.id())
            else {
                bail!("channel '{}' is not configured", kind.display_name());
            };

            let result =
                tokio::time::timeout(Duration::from_secs(10), entry.channel.health_check()).await;
            let (status, detail) = match classify_health_result(&result) {
                ChannelHealthState::Healthy => ("healthy", Some("connection check passed")),
                ChannelHealthState::Unhealthy => ("unhealthy", Some("connection check failed")),
                ChannelHealthState::Timeout => ("timeout", Some("connection check timed out")),
            };
            Ok((status.to_string(), detail.map(ToString::to_string)))
        }
    }
}

pub(super) async fn run_admin_channel_action(
    current: &Config,
    channel_id: &str,
    action: &str,
) -> Result<ChannelActionResult> {
    let kind = ManagedChannelKind::parse(channel_id)?;
    let action = action.trim().to_ascii_lowercase();

    match action.as_str() {
        "start" => {
            let result = update_channel(current, kind.id(), Some(true), None)?;
            let detail = channel_action_detail(&result, "enable");
            Ok(ChannelActionResult {
                record: result.record,
                action,
                status: "updated".to_string(),
                detail: Some(detail),
                apply_mode: Some(result.apply_mode),
                reload_requested: result.reload_requested,
            })
        }
        "stop" => {
            let result = update_channel(current, kind.id(), Some(false), None)?;
            let detail = channel_action_detail(&result, "disable");
            Ok(ChannelActionResult {
                record: result.record,
                action,
                status: "updated".to_string(),
                detail: Some(detail),
                apply_mode: Some(result.apply_mode),
                reload_requested: result.reload_requested,
            })
        }
        "doctor" | "test" => {
            let config = load_persisted_runtime_config(current)?;
            let record = kind.record(&config.channels_config);
            let (status, detail) = run_channel_health_check(&config, kind).await?;
            Ok(ChannelActionResult {
                record,
                action,
                status,
                detail,
                apply_mode: None,
                reload_requested: false,
            })
        }
        _ => bail!("unsupported channel action '{action}'"),
    }
}
