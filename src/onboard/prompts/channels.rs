//! Interactive CLI prompts for configuring messaging channels.
//!
//! Collects credentials and settings for Telegram, Discord, Slack,
//! Matrix, IRC, `WhatsApp`, iMessage, and webhook channels.

use anyhow::Result;

use super::super::view::print_bullet;
use crate::config::schema::{IrcConfig, WhatsAppConfig};
use crate::config::{
    ChannelSecurityPolicy, ChannelsConfig, DiscordConfig, IMessageConfig, MatrixConfig,
    SlackConfig, TelegramConfig, WebhookConfig,
};
use crate::onboard::domain::parse_allowlist;
use crate::ui::style as ui;

/// # Errors
///
/// Returns an error when interactive prompts fail or channel inputs are
/// invalid.
pub(crate) async fn setup_channels() -> Result<ChannelsConfig> {
    print_bullet(&t!("onboard.channels.intro"));
    print_bullet(&t!("onboard.channels.cli_always"));
    println!();

    let mut config = ChannelsConfig {
        cli: true,
        coalescing_window_ms: 0,
        coalescing_max_messages: 4,
        ..ChannelsConfig::default()
    };

    loop {
        let options = build_channel_menu_options(&config);

        let choice: usize = cliclack::select(format!("  {}", t!("onboard.channels.select_prompt")))
            .item(0usize, options[0].as_str(), "")
            .item(1usize, options[1].as_str(), "")
            .item(2usize, options[2].as_str(), "")
            .item(3usize, options[3].as_str(), "")
            .item(4usize, options[4].as_str(), "")
            .item(5usize, options[5].as_str(), "")
            .item(6usize, options[6].as_str(), "")
            .item(7usize, options[7].as_str(), "")
            .item(8usize, options[8].as_str(), "")
            .initial_value(8usize)
            .interact()?;

        match choice {
            0 => setup_telegram(&mut config).await?,
            1 => setup_discord(&mut config).await?,
            2 => setup_slack(&mut config).await?,
            3 => setup_imessage(&mut config)?,
            4 => setup_matrix(&mut config).await?,
            5 => setup_whatsapp(&mut config).await?,
            6 => setup_irc(&mut config)?,
            7 => setup_webhook(&mut config)?,
            _ => break,
        }
        println!();
    }

    let active = config.active_channel_names();
    println!(
        "  {} {}",
        ui::success("✓"),
        t!(
            "onboard.channels.summary",
            channels = ui::value(active.join(", "))
        )
    );

    Ok(config)
}

fn build_channel_menu_options(config: &ChannelsConfig) -> Vec<String> {
    let connected = t!("onboard.channels.connected");
    let configured = t!("onboard.channels.configured");

    vec![
        channel_option_label(
            "Telegram",
            config.telegram.is_some(),
            &connected,
            "connect your bot",
        ),
        channel_option_label(
            "Discord",
            config.discord.is_some(),
            &connected,
            "connect your bot",
        ),
        channel_option_label(
            "Slack",
            config.slack.is_some(),
            &connected,
            "connect your bot",
        ),
        channel_option_label(
            "iMessage",
            config.imessage.is_some(),
            &configured,
            "macOS only",
        ),
        channel_option_label(
            "Matrix",
            config.matrix.is_some(),
            &connected,
            "self-hosted chat",
        ),
        channel_option_label(
            "WhatsApp",
            config.whatsapp.is_some(),
            &connected,
            "Business Cloud API",
        ),
        channel_option_label("IRC", config.irc.is_some(), &configured, "IRC over TLS"),
        channel_option_label(
            "Webhook",
            config.webhook.is_some(),
            &configured,
            "HTTP endpoint",
        ),
        t!("onboard.channels.done").to_string(),
    ]
}

fn channel_option_label(
    name: &str,
    is_configured: bool,
    active_label: &str,
    inactive_hint: &str,
) -> String {
    let status = if is_configured {
        format!("\u{2713} {active_label}")
    } else {
        format!("\u{2014} {inactive_hint}")
    };
    format!("{name:<10} {status}")
}

async fn setup_telegram(config: &mut ChannelsConfig) -> Result<()> {
    println!();
    println!(
        "  {} {}",
        ui::header(t!("onboard.channels.telegram_setup")),
        ui::dim(format!("— {}", t!("onboard.channels.telegram_subtitle")))
    );
    print_bullet(&t!("onboard.channels.telegram_step1"));
    print_bullet(&t!("onboard.channels.telegram_step2"));
    print_bullet(&t!("onboard.channels.telegram_step3"));
    println!();

    let token: String = cliclack::input(format!(
        "  {}",
        t!("onboard.channels.telegram_token_prompt")
    ))
    .required(false)
    .interact()?;

    if token.trim().is_empty() {
        println!("  {} {}", ui::dim("→"), t!("onboard.channels.skipped"));
        return Ok(());
    }

    let client = crate::utils::http::build_http_client();
    let encoded_token: String = token
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' => {
                String::from(b as char)
            }
            _ => format!("%{b:02X}"),
        })
        .collect();
    let url = format!("https://api.telegram.org/bot{encoded_token}/getMe");

    let sp = cliclack::spinner();
    sp.start(t!("onboard.channels.testing").to_string());
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            let data: serde_json::Value = resp.json().await.unwrap_or_default();
            let bot_name = data
                .get("result")
                .and_then(|r| r.get("username"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            sp.stop(format!(
                "✓ {}",
                t!("onboard.channels.test_success", name = bot_name)
            ));
        }
        _ => {
            sp.stop(format!("✗ {}", t!("onboard.channels.test_fail")));
            return Ok(());
        }
    }

    print_bullet(&t!("onboard.channels.telegram_allowlist_hint"));
    print_bullet(&t!("onboard.channels.telegram_allowlist_format"));
    print_bullet(&t!("onboard.channels.telegram_allowlist_star"));

    let users_str: String = cliclack::input(format!(
        "  {}",
        t!("onboard.channels.telegram_users_prompt")
    ))
    .required(false)
    .interact()?;

    let allowed_users = parse_allowlist(&users_str);

    if allowed_users.is_empty() {
        println!("  ! {}", t!("onboard.channels.telegram_no_users"));
    }

    config.telegram = Some(TelegramConfig {
        bot_token: token,
        allowed_users,
        default_account: None,
        default_to: None,
        security: ChannelSecurityPolicy::default(),
    });

    Ok(())
}

async fn setup_discord(config: &mut ChannelsConfig) -> Result<()> {
    println!();
    println!(
        "  {} {}",
        ui::header(t!("onboard.channels.discord_setup")),
        ui::dim(format!("— {}", t!("onboard.channels.discord_subtitle")))
    );
    print_bullet(&t!("onboard.channels.discord_step1"));
    print_bullet(&t!("onboard.channels.discord_step2"));
    print_bullet(&t!("onboard.channels.discord_step3"));
    print_bullet(&t!("onboard.channels.discord_step4"));
    println!();

    let token: String =
        cliclack::input(format!("  {}", t!("onboard.channels.discord_token_prompt")))
            .required(false)
            .interact()?;

    if token.trim().is_empty() {
        println!("  {} {}", ui::dim("→"), t!("onboard.channels.skipped"));
        return Ok(());
    }

    let client = crate::utils::http::build_http_client();
    let sp = cliclack::spinner();
    sp.start(t!("onboard.channels.testing").to_string());
    match client
        .get("https://discord.com/api/v10/users/@me")
        .header("Authorization", format!("Bot {token}"))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let data: serde_json::Value = resp.json().await.unwrap_or_default();
            let bot_name = data
                .get("username")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            sp.stop(format!(
                "✓ {}",
                t!("onboard.channels.test_success", name = bot_name)
            ));
        }
        _ => {
            sp.stop(format!("✗ {}", t!("onboard.channels.test_fail")));
            return Ok(());
        }
    }

    let guild: String =
        cliclack::input(format!("  {}", t!("onboard.channels.discord_guild_prompt")))
            .required(false)
            .interact()?;

    print_bullet(&t!("onboard.channels.discord_allowlist_hint"));
    print_bullet(&t!("onboard.channels.discord_allowlist_format"));
    print_bullet(&t!("onboard.channels.discord_allowlist_star"));

    let allowed_users_str: String =
        cliclack::input(format!("  {}", t!("onboard.channels.discord_users_prompt")))
            .required(false)
            .interact()?;

    let allowed_users = parse_allowlist(&allowed_users_str);

    if allowed_users.is_empty() {
        println!("  ! {}", t!("onboard.channels.discord_no_users"));
    }

    config.discord = Some(DiscordConfig {
        bot_token: token,
        guild_id: if guild.is_empty() { None } else { Some(guild) },
        allowed_users,
        security: ChannelSecurityPolicy::default(),
        application_id: None,
        intents: None,
        status: None,
        default_account: None,
        default_to: None,
        activity_type: None,
        activity_name: None,
        thinking_embed: false,
        thinking_embed_include_preview: false,
        pickup_policy: crate::config::DiscordPickupPolicyConfig::default(),
    });

    Ok(())
}

async fn setup_slack(config: &mut ChannelsConfig) -> Result<()> {
    println!();
    println!(
        "  {} {}",
        ui::header(t!("onboard.channels.slack_setup")),
        ui::dim(format!("— {}", t!("onboard.channels.slack_subtitle")))
    );
    print_bullet(&t!("onboard.channels.slack_step1"));
    print_bullet(&t!("onboard.channels.slack_step2"));
    print_bullet(&t!("onboard.channels.slack_step3"));
    println!();

    let token: String = cliclack::input(format!("  {}", t!("onboard.channels.slack_token_prompt")))
        .required(false)
        .interact()?;

    if token.trim().is_empty() {
        println!("  {} {}", ui::dim("→"), t!("onboard.channels.skipped"));
        return Ok(());
    }
    crate::onboard::domain::warn_slack_token_prefix(&token, "xoxb-", "Slack bot token");

    if !test_slack_connection(&token).await? {
        return Ok(());
    }

    let details = prompt_slack_details()?;

    config.slack = Some(SlackConfig {
        bot_token: token,
        app_token: if details.app_token.is_empty() {
            None
        } else {
            Some(details.app_token)
        },
        channel_id: if details.channel.is_empty() {
            None
        } else {
            Some(details.channel)
        },
        default_account: None,
        default_to: None,
        allowed_users: details.allowed_users,
        security: ChannelSecurityPolicy::default(),
    });

    Ok(())
}

/// Tests the Slack API connection. Returns `true` if the token is valid and
/// the workspace responded successfully, `false` if the connection failed
/// (with a message already printed).
async fn test_slack_connection(token: &str) -> Result<bool> {
    let client = crate::utils::http::build_http_client();
    let sp = cliclack::spinner();
    sp.start(t!("onboard.channels.testing").to_string());
    match client
        .get("https://slack.com/api/auth.test")
        .bearer_auth(token)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let data: serde_json::Value = resp.json().await.unwrap_or_default();
            let ok = data
                .get("ok")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let team = data
                .get("team")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            if ok {
                sp.stop(format!(
                    "\u{2713} {}",
                    t!("onboard.channels.slack_workspace_connected", team = team)
                ));
                Ok(true)
            } else {
                let err = data
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unknown error");
                sp.stop(format!(
                    "\u{2717} {}",
                    t!("onboard.channels.slack_error", error = err)
                ));
                Ok(false)
            }
        }
        _ => {
            sp.stop(format!("\u{2717} {}", t!("onboard.channels.test_fail")));
            Ok(false)
        }
    }
}

struct SlackDetails {
    app_token: String,
    channel: String,
    allowed_users: Vec<String>,
}

fn prompt_slack_details() -> Result<SlackDetails> {
    let app_token: String = cliclack::input(format!(
        "  {}",
        t!("onboard.channels.slack_app_token_prompt")
    ))
    .required(false)
    .interact()?;
    if !app_token.is_empty() {
        crate::onboard::domain::warn_slack_token_prefix(&app_token, "xapp-", "Slack app token");
    }

    let channel: String =
        cliclack::input(format!("  {}", t!("onboard.channels.slack_channel_prompt")))
            .required(false)
            .interact()?;

    print_bullet(&t!("onboard.channels.slack_allowlist_hint"));
    print_bullet(&t!("onboard.channels.slack_allowlist_format"));
    print_bullet(&t!("onboard.channels.slack_allowlist_star"));

    let allowed_users_str: String =
        cliclack::input(format!("  {}", t!("onboard.channels.slack_users_prompt")))
            .required(false)
            .interact()?;

    let allowed_users = parse_allowlist(&allowed_users_str);

    if allowed_users.is_empty() {
        println!("  ! {}", t!("onboard.channels.slack_no_users"));
    }

    Ok(SlackDetails {
        app_token,
        channel,
        allowed_users,
    })
}

fn setup_imessage(config: &mut ChannelsConfig) -> Result<()> {
    println!();
    println!(
        "  {} {}",
        ui::header(t!("onboard.channels.imessage_setup")),
        ui::dim(format!("— {}", t!("onboard.channels.imessage_subtitle")))
    );

    if !cfg!(target_os = "macos") {
        println!("  ! {}", t!("onboard.channels.imessage_macos_only"));
        return Ok(());
    }

    print_bullet(&t!("onboard.channels.imessage_desc"));
    print_bullet(&t!("onboard.channels.imessage_disk_access"));
    println!();

    let contacts_str: String = cliclack::input(format!(
        "  {}",
        t!("onboard.channels.imessage_contacts_prompt")
    ))
    .default_input("*")
    .interact()?;

    let allowed_contacts = parse_allowlist(&contacts_str);

    config.imessage = Some(IMessageConfig {
        allowed_contacts: if allowed_contacts.is_empty() {
            vec!["*".into()]
        } else {
            allowed_contacts
        },
        security: ChannelSecurityPolicy::default(),
    });
    println!(
        "  ✓ {}",
        t!(
            "onboard.channels.imessage_confirm",
            contacts = ui::cyan(&contacts_str)
        )
    );

    Ok(())
}

async fn setup_matrix(config: &mut ChannelsConfig) -> Result<()> {
    println!();
    println!(
        "  {} {}",
        ui::header(t!("onboard.channels.matrix_setup")),
        ui::dim(format!("— {}", t!("onboard.channels.matrix_subtitle")))
    );
    print_bullet(&t!("onboard.channels.matrix_desc"));
    print_bullet(&t!("onboard.channels.matrix_token_hint"));
    println!();

    let homeserver: String = cliclack::input(format!(
        "  {}",
        t!("onboard.channels.matrix_homeserver_prompt")
    ))
    .required(false)
    .interact()?;

    if homeserver.trim().is_empty() {
        println!("  {} {}", ui::dim("→"), t!("onboard.channels.skipped"));
        return Ok(());
    }
    let hs = crate::onboard::domain::validate_base_url(&homeserver)?;

    let access_token: String =
        cliclack::input(format!("  {}", t!("onboard.channels.matrix_token_prompt")))
            .required(false)
            .interact()?;

    if access_token.trim().is_empty() {
        println!(
            "  {} {}",
            ui::dim("→"),
            t!("onboard.channels.matrix_token_required")
        );
        return Ok(());
    }

    let client = crate::utils::http::build_http_client();
    let sp = cliclack::spinner();
    sp.start(t!("onboard.channels.testing").to_string());
    match client
        .get(format!("{hs}/_matrix/client/v3/account/whoami"))
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let data: serde_json::Value = resp.json().await.unwrap_or_default();
            let user_id = data
                .get("user_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            sp.stop(format!(
                "✓ {}",
                t!("onboard.channels.test_success", name = user_id)
            ));
        }
        _ => {
            sp.stop(format!("✗ {}", t!("onboard.channels.matrix_test_fail")));
            return Ok(());
        }
    }

    let room_id: String =
        cliclack::input(format!("  {}", t!("onboard.channels.matrix_room_prompt"))).interact()?;

    let users_str: String =
        cliclack::input(format!("  {}", t!("onboard.channels.matrix_users_prompt")))
            .default_input("*")
            .interact()?;

    let allowed_users = parse_allowlist(&users_str);

    config.matrix = Some(MatrixConfig {
        homeserver: hs,
        access_token,
        room_id,
        allowed_users: if allowed_users.is_empty() {
            vec!["*".into()]
        } else {
            allowed_users
        },
        security: ChannelSecurityPolicy::default(),
    });

    Ok(())
}

async fn setup_whatsapp(config: &mut ChannelsConfig) -> Result<()> {
    println!();
    println!(
        "  {} {}",
        ui::header(t!("onboard.channels.whatsapp_setup")),
        ui::dim(format!("— {}", t!("onboard.channels.whatsapp_subtitle")))
    );
    print_bullet(&t!("onboard.channels.whatsapp_step1"));
    print_bullet(&t!("onboard.channels.whatsapp_step2"));
    print_bullet(&t!("onboard.channels.whatsapp_step3"));
    print_bullet(&t!("onboard.channels.whatsapp_step4"));
    println!();

    let access_token: String = cliclack::input(format!(
        "  {}",
        t!("onboard.channels.whatsapp_token_prompt")
    ))
    .required(false)
    .interact()?;

    if access_token.trim().is_empty() {
        println!("  {} {}", ui::dim("→"), t!("onboard.channels.skipped"));
        return Ok(());
    }

    let phone_number_id: String = cliclack::input(format!(
        "  {}",
        t!("onboard.channels.whatsapp_phone_prompt")
    ))
    .required(false)
    .interact()?;

    if phone_number_id.trim().is_empty() {
        println!(
            "  {} {}",
            ui::dim("→"),
            t!("onboard.channels.whatsapp_phone_required")
        );
        return Ok(());
    }

    let verify_token: String = cliclack::input(format!(
        "  {}",
        t!("onboard.channels.whatsapp_verify_prompt")
    ))
    .default_input("asterel-whatsapp-verify")
    .interact()?;

    let client = crate::utils::http::build_http_client();
    let url = format!(
        "https://graph.facebook.com/v21.0/{}",
        phone_number_id.trim()
    );
    let sp = cliclack::spinner();
    sp.start(t!("onboard.channels.testing").to_string());
    match client
        .get(&url)
        .header("Authorization", format!("Bearer {}", access_token.trim()))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            sp.stop(format!(
                "✓ {}",
                t!("onboard.channels.whatsapp_test_success")
            ));
        }
        _ => {
            sp.stop(format!("✗ {}", t!("onboard.channels.whatsapp_test_fail")));
            return Ok(());
        }
    }

    let users_str: String = cliclack::input(format!(
        "  {}",
        t!("onboard.channels.whatsapp_numbers_prompt")
    ))
    .default_input("*")
    .interact()?;

    let allowed_numbers = parse_allowlist(&users_str);

    config.whatsapp = Some(WhatsAppConfig {
        access_token: access_token.trim().to_string(),
        phone_number_id: phone_number_id.trim().to_string(),
        verify_token: verify_token.trim().to_string(),
        allowed_numbers: if allowed_numbers.is_empty() {
            vec!["*".into()]
        } else {
            allowed_numbers
        },
        app_secret: None,
        security: ChannelSecurityPolicy::default(),
    });

    Ok(())
}

fn setup_irc(config: &mut ChannelsConfig) -> Result<()> {
    println!();
    println!(
        "  {} {}",
        ui::header(t!("onboard.channels.irc_setup")),
        ui::dim(format!("— {}", t!("onboard.channels.irc_subtitle")))
    );
    print_bullet(&t!("onboard.channels.irc_desc"));
    print_bullet(&t!("onboard.channels.irc_sasl"));
    println!();

    let (server, port, nickname) = prompt_irc_server()?;

    let Some(server) = server else {
        return Ok(());
    };
    let Some(nickname) = nickname else {
        return Ok(());
    };

    let (channels, allowed_users) = prompt_irc_channels_and_users()?;
    let auth = prompt_irc_auth()?;

    println!(
        "  \u{2713} {}",
        t!(
            "onboard.channels.irc_confirm",
            nick = ui::cyan(&nickname),
            server = ui::cyan(&server),
            port = ui::cyan(port)
        )
    );

    config.irc = Some(IrcConfig {
        server: server.trim().to_string(),
        port,
        nickname: nickname.trim().to_string(),
        username: None,
        channels,
        allowed_users: if allowed_users.is_empty() {
            vec!["*".into()]
        } else {
            allowed_users
        },
        server_password: auth.server_password,
        nickserv_password: auth.nickserv_password,
        sasl_password: auth.sasl_password,
        verify_tls: Some(auth.verify_tls),
        security: ChannelSecurityPolicy::default(),
    });

    Ok(())
}

/// Prompts for IRC server, port, and nickname. Returns `(Some(server), port,
/// Some(nickname))` on success, or `(None, _, _)` / `(_, _, None)` when the
/// user skips.
fn prompt_irc_server() -> Result<(Option<String>, u16, Option<String>)> {
    let server: String = cliclack::input(format!("  {}", t!("onboard.channels.irc_server_prompt")))
        .required(false)
        .interact()?;

    if server.trim().is_empty() {
        println!(
            "  {} {}",
            ui::dim("\u{2192}"),
            t!("onboard.channels.skipped")
        );
        return Ok((None, 6697, None));
    }

    let port_str: String = cliclack::input(format!("  {}", t!("onboard.channels.irc_port_prompt")))
        .default_input("6697")
        .interact()?;

    let port: u16 = if let Ok(p) = port_str.trim().parse() {
        p
    } else {
        println!(
            "  {} {}",
            ui::dim("\u{2192}"),
            t!("onboard.channels.irc_port_invalid")
        );
        6697
    };

    let nickname: String = cliclack::input(format!("  {}", t!("onboard.channels.irc_nick_prompt")))
        .required(false)
        .interact()?;

    if nickname.trim().is_empty() {
        println!(
            "  {} {}",
            ui::dim("\u{2192}"),
            t!("onboard.channels.irc_nick_required")
        );
        return Ok((Some(server), port, None));
    }

    Ok((Some(server), port, Some(nickname)))
}

fn prompt_irc_channels_and_users() -> Result<(Vec<String>, Vec<String>)> {
    let channels_str: String =
        cliclack::input(format!("  {}", t!("onboard.channels.irc_channels_prompt")))
            .required(false)
            .interact()?;

    let channels = if channels_str.trim().is_empty() {
        vec![]
    } else {
        let raw: Vec<String> = channels_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        crate::onboard::domain::normalize_irc_channels(&raw)
    };

    print_bullet(&t!("onboard.channels.irc_allowlist_hint"));
    print_bullet(&t!("onboard.channels.irc_allowlist_star"));

    let users_str: String =
        cliclack::input(format!("  {}", t!("onboard.channels.irc_users_prompt")))
            .required(false)
            .interact()?;

    let allowed_users = parse_allowlist(&users_str);

    if allowed_users.is_empty() {
        print_bullet(&format!("! {}", t!("onboard.channels.irc_empty_allowlist")));
    }

    Ok((channels, allowed_users))
}

struct IrcAuth {
    server_password: Option<String>,
    nickserv_password: Option<String>,
    sasl_password: Option<String>,
    verify_tls: bool,
}

fn prompt_irc_auth() -> Result<IrcAuth> {
    println!();
    print_bullet(&t!("onboard.channels.irc_auth_header"));

    let server_password: String =
        cliclack::input(format!("  {}", t!("onboard.channels.irc_server_password")))
            .required(false)
            .interact()?;

    let nickserv_password: String = cliclack::input(format!(
        "  {}",
        t!("onboard.channels.irc_nickserv_password")
    ))
    .required(false)
    .interact()?;

    let sasl_password: String =
        cliclack::input(format!("  {}", t!("onboard.channels.irc_sasl_password")))
            .required(false)
            .interact()?;

    let verify_tls: bool =
        cliclack::confirm(format!("  {}", t!("onboard.channels.irc_verify_tls")))
            .initial_value(true)
            .interact()?;

    Ok(IrcAuth {
        server_password: nonempty_trimmed(&server_password),
        nickserv_password: nonempty_trimmed(&nickserv_password),
        sasl_password: nonempty_trimmed(&sasl_password),
        verify_tls,
    })
}

/// Returns `Some(trimmed)` if the string is non-empty after trimming,
/// `None` otherwise.
fn nonempty_trimmed(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn setup_webhook(config: &mut ChannelsConfig) -> Result<()> {
    println!();
    println!(
        "  {} {}",
        ui::header(t!("onboard.channels.webhook_setup")),
        ui::dim(format!("— {}", t!("onboard.channels.webhook_subtitle")))
    );

    let port: String = cliclack::input(format!("  {}", t!("onboard.channels.webhook_port_prompt")))
        .default_input("8080")
        .interact()?;

    let secret: String = cliclack::input(format!(
        "  {}",
        t!("onboard.channels.webhook_secret_prompt")
    ))
    .required(false)
    .interact()?;

    let Some(secret) = nonempty_trimmed(&secret) else {
        anyhow::bail!("webhook setup requires a non-empty secret");
    };

    let parsed_port = crate::onboard::domain::validate_port(&port, 8080)?;
    config.webhook = Some(WebhookConfig {
        port: parsed_port,
        secret: Some(secret),
        security: ChannelSecurityPolicy::default(),
    });
    println!(
        "  ✓ {}",
        t!("onboard.channels.webhook_confirm", port = ui::cyan(&port))
    );

    Ok(())
}
