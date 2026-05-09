//! Interactive CLI prompts for tunnel configuration.
//!
//! Lets the user choose and configure a tunnel provider
//! (Cloudflare, Tailscale, or ngrok) for external access.

use anyhow::Result;

use super::super::view::print_bullet;
use crate::ui::style as ui;

/// # Errors
///
/// Returns an error when interactive prompt input fails.
pub(crate) fn setup_tunnel() -> Result<crate::config::TunnelConfig> {
    use crate::config::schema::TunnelConfig;

    print_bullet(&t!("onboard.tunnel.intro"));
    print_bullet(&t!("onboard.tunnel.skip_hint"));
    println!();

    let choice: usize = cliclack::select(format!("  {}", t!("onboard.tunnel.select_prompt")))
        .item(0usize, t!("onboard.tunnel.skip").to_string(), "")
        .item(1usize, t!("onboard.tunnel.cloudflare").to_string(), "")
        .item(2usize, t!("onboard.tunnel.tailscale").to_string(), "")
        .item(3usize, t!("onboard.tunnel.ngrok").to_string(), "")
        .item(4usize, t!("onboard.tunnel.custom").to_string(), "")
        .initial_value(0usize)
        .interact()?;

    let config = match choice {
        1 => setup_cloudflare_tunnel()?,
        2 => setup_tailscale_tunnel()?,
        3 => setup_ngrok_tunnel()?,
        4 => setup_custom_tunnel()?,
        _ => {
            println!(
                "  {} Tunnel: {}",
                ui::success("✓"),
                ui::dim(t!("onboard.tunnel.confirm_none"))
            );
            TunnelConfig::default()
        }
    };

    Ok(config)
}

fn setup_cloudflare_tunnel() -> Result<crate::config::TunnelConfig> {
    use crate::config::schema::{CloudflareTunnelConfig, TunnelConfig};

    println!();
    print_bullet(&t!("onboard.tunnel.cloudflare_token_hint"));
    let token: String = cliclack::input(format!(
        "  {}",
        t!("onboard.tunnel.cloudflare_token_prompt")
    ))
    .required(false)
    .interact()?;
    if token.trim().is_empty() {
        println!("  {} {}", ui::dim("→"), t!("onboard.channels.skipped"));
        return Ok(TunnelConfig::default());
    }
    println!("  {} Tunnel: {}", ui::success("✓"), ui::value("Cloudflare"));
    Ok(TunnelConfig {
        provider: crate::config::TunnelProvider::Cloudflare,
        cloudflare: Some(CloudflareTunnelConfig { token }),
        ..TunnelConfig::default()
    })
}

fn setup_tailscale_tunnel() -> Result<crate::config::TunnelConfig> {
    use crate::config::schema::{TailscaleTunnelConfig, TunnelConfig};

    println!();
    print_bullet(&t!("onboard.tunnel.tailscale_hint"));
    let funnel: bool = cliclack::confirm(format!(
        "  {}",
        t!("onboard.tunnel.tailscale_funnel_prompt")
    ))
    .initial_value(false)
    .interact()?;
    println!(
        "  {} Tunnel: {} ({})",
        ui::success("✓"),
        ui::value("Tailscale"),
        if funnel {
            t!("onboard.tunnel.tailscale_funnel_public")
        } else {
            t!("onboard.tunnel.tailscale_serve_tailnet")
        }
    );
    Ok(TunnelConfig {
        provider: crate::config::TunnelProvider::Tailscale,
        tailscale: Some(TailscaleTunnelConfig {
            funnel,
            hostname: None,
        }),
        ..TunnelConfig::default()
    })
}

fn setup_ngrok_tunnel() -> Result<crate::config::TunnelConfig> {
    use crate::config::schema::{NgrokTunnelConfig, TunnelConfig};

    println!();
    print_bullet(&t!("onboard.tunnel.ngrok_hint"));
    let auth_token: String =
        cliclack::input(format!("  {}", t!("onboard.tunnel.ngrok_token_prompt")))
            .required(false)
            .interact()?;
    if auth_token.trim().is_empty() {
        println!("  {} {}", ui::dim("→"), t!("onboard.channels.skipped"));
        return Ok(TunnelConfig::default());
    }
    let domain: String = cliclack::input(format!("  {}", t!("onboard.tunnel.ngrok_domain_prompt")))
        .required(false)
        .interact()?;
    println!("  {} Tunnel: {}", ui::success("✓"), ui::value("ngrok"));
    Ok(TunnelConfig {
        provider: crate::config::TunnelProvider::Ngrok,
        ngrok: Some(NgrokTunnelConfig {
            auth_token,
            domain: if domain.is_empty() {
                None
            } else {
                Some(domain)
            },
        }),
        ..TunnelConfig::default()
    })
}

fn setup_custom_tunnel() -> Result<crate::config::TunnelConfig> {
    use crate::config::schema::{CustomTunnelConfig, TunnelConfig};

    println!();
    print_bullet(&t!("onboard.tunnel.custom_hint"));
    print_bullet(&t!("onboard.tunnel.custom_placeholder_hint"));
    print_bullet(&t!("onboard.tunnel.custom_example"));
    let cmd: String = cliclack::input(format!("  {}", t!("onboard.tunnel.custom_prompt")))
        .required(false)
        .interact()?;
    if cmd.trim().is_empty() {
        println!("  {} {}", ui::dim("→"), t!("onboard.channels.skipped"));
        return Ok(TunnelConfig::default());
    }
    if !cmd.contains("{port}") {
        anyhow::bail!("custom tunnel command must include the {{port}} placeholder");
    }
    println!(
        "  {} Tunnel: {} ({})",
        ui::success("✓"),
        ui::value(t!("onboard.tunnel.confirm_custom")),
        ui::dim(&cmd)
    );
    Ok(TunnelConfig {
        provider: crate::config::TunnelProvider::Custom,
        custom: Some(CustomTunnelConfig {
            start_command: cmd,
            health_url: None,
            url_pattern: Some("https://".to_string()),
        }),
        ..TunnelConfig::default()
    })
}
