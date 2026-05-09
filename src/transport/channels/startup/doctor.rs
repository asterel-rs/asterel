//! Channel doctor command: builds all configured channels and runs
//! connectivity health checks with timeout reporting.
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use super::super::factory;
use super::super::health::{ChannelHealthState, classify_health_result};
use crate::config::Config;

/// # Errors
///
/// Returns an error when channel runtime setup or health check execution fails.
pub async fn doctor_channels(config: Arc<Config>) -> Result<()> {
    let security = Arc::new(crate::security::SecurityPolicy::from_config_runtime(
        &config.autonomy,
        &config.runtime,
        &config.workspace_dir,
    ));
    let channels = factory::build_channels(config.channels_config.clone(), &security);

    if channels.is_empty() {
        println!("{}", t!("channels.no_channels_doctor"));
        println!(
            "{}",
            crate::ui::style::note_line(
                "CLI access remains available; this warning only covers real-time listener channels."
            )
        );
        return Ok(());
    }

    println!("◆ {}", t!("channels.doctor_title"));
    println!();

    let mut healthy = 0_u32;
    let mut unhealthy = 0_u32;
    let mut timeout = 0_u32;

    for entry in channels {
        let result =
            tokio::time::timeout(Duration::from_secs(10), entry.channel.health_check()).await;
        let state = classify_health_result(&result);

        match state {
            ChannelHealthState::Healthy => {
                healthy += 1;
                println!("  ✓ {:<9} {}", entry.name, t!("channels.healthy"));
            }
            ChannelHealthState::Unhealthy => {
                unhealthy += 1;
                println!("  ✗ {:<9} {}", entry.name, t!("channels.unhealthy"));
            }
            ChannelHealthState::Timeout => {
                timeout += 1;
                println!("  ! {:<9} {}", entry.name, t!("channels.timed_out"));
            }
        }
    }

    if config.channels_config.webhook.is_some() {
        println!("  › {}", t!("channels.webhook_hint"));
    }

    println!();
    println!(
        "{}",
        t!(
            "channels.doctor_summary",
            healthy = healthy,
            unhealthy = unhealthy,
            timeout = timeout
        )
    );
    Ok(())
}
