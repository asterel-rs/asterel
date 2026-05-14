//! Top-level onboarding flow orchestration.
//!
//! Drives the interactive CLI setup wizard, channels repair,
//! and quick-setup entry points.

use std::fs;

use anyhow::{Context, Result};

use super::completion::{
    OnboardingPostActionOptions, finalize_plan_with_post_actions, run_post_actions,
};
use super::config_builder::{OnboardingAuthProfileDraft, OnboardingConfigDraft, OnboardingPlan};
use super::detect::DetectedState;
use super::domain::default_model_for_provider;
use super::health::HealthStatus;
use super::postgres::{
    PostgresSetupMode, ensure_postgres_memory_ready, resolve_postgres_setup_mode,
};
use super::prompts::{
    ProjectContext, setup_channels, setup_memory, setup_project_context, setup_provider,
    setup_tool_mode, setup_tunnel, setup_workspace,
};
use super::view::{print_step, print_summary, print_welcome_banner};
use crate::config::{ChannelsConfig, ComposioConfig, Config, MemoryConfig, SecretsConfig};
use crate::ui::style as ui;

/// Run the interactive onboarding wizard.
///
/// # Errors
/// Returns an error if onboarding prompts, scaffolding, or config persistence fails.
pub async fn run_wizard(
    install_daemon_flag: bool,
    postgres_setup_mode: Option<&str>,
) -> Result<(Config, bool)> {
    let setup_mode = resolve_postgres_setup_mode(postgres_setup_mode)?;

    // Detect locale before anything else
    if let Ok(lang) = std::env::var("ASTEREL_LANG")
        && !lang.is_empty()
    {
        rust_i18n::set_locale(&lang);
    }

    run_wizard_cli(install_daemon_flag, setup_mode).await
}

/// CLI-based wizard with `QuickStart` / Advanced mode selection.
async fn run_wizard_cli(
    install_daemon_flag: bool,
    setup_mode: PostgresSetupMode,
) -> Result<(Config, bool)> {
    // Non-terminal fallback
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        eprintln!("Not running in a terminal. Using non-interactive quick setup.");
        return run_quick_setup(None, None, None, None, install_daemon_flag);
    }

    cliclack::intro("Asterel Setup")?;

    // Auto-detection
    let detected = super::detect::detect_existing_setup();
    if detected.config_exists {
        cliclack::log::info("Existing configuration detected.")?;
    }
    if let Some(ref provider) = detected.detected_provider {
        cliclack::log::info(format!("Detected {provider} from existing credentials."))?;
    }

    // QuickStart vs Advanced
    let mode: &str = cliclack::select("How would you like to set up?")
        .item(
            "quick",
            "QuickStart",
            "Provider + API key + Memory (recommended)",
        )
        .item("advanced", "Advanced", "Full configuration (8 steps)")
        .initial_value("quick")
        .interact()?;

    match mode {
        "quick" => run_quickstart_flow(&detected, setup_mode, install_daemon_flag).await,
        _ => run_advanced_flow(&detected, setup_mode, install_daemon_flag).await,
    }
}

/// `QuickStart` flow: Provider + API key + Memory, then health checks.
async fn run_quickstart_flow(
    detected: &DetectedState,
    setup_mode: PostgresSetupMode,
    install_daemon_flag: bool,
) -> Result<(Config, bool)> {
    // Step 1: Provider (use detected or ask)
    let (provider, api_key, model, oauth_source) = if let Some(ref p) = detected.detected_provider {
        let reuse = cliclack::confirm(format!("Use detected provider ({p})?"))
            .initial_value(true)
            .interact()?;
        if reuse {
            let key = detected.detected_api_key.clone().unwrap_or_default();
            let model = default_model_for_provider(p);
            (p.clone(), key, model, None)
        } else {
            super::prompts::setup_provider().await?
        }
    } else {
        super::prompts::setup_provider().await?
    };

    // Step 2: Memory
    let mut memory_config = setup_memory()?;
    ensure_postgres_memory_ready(&mut memory_config, setup_mode)?;

    // Minimal defaults for QuickStart
    let asterel_dir = crate::utils::dirs::asterel_home_dir()?;
    let workspace_dir = asterel_dir.join("workspace");
    let config_path = asterel_dir.join("config.toml");

    let project_context = ProjectContext {
        user_name: std::env::var("USER").unwrap_or_else(|_| "User".into()),
        timezone: "UTC".into(),
        agent_name: "Asterel".into(),
        communication_style:
            "Be warm, natural, and clear. Use occasional relevant emojis (1-2 max) and avoid robotic phrasing."
                .into(),
    };

    let (config, autostart) = finalize_plan_with_post_actions(
        OnboardingPlan {
            draft: OnboardingConfigDraft {
                workspace_dir,
                config_path,
                api_key: (!api_key.is_empty()).then_some(api_key.clone()),
                default_provider: provider.clone(),
                default_model: model,
                channels_config: ChannelsConfig::default(),
                memory: memory_config,
                tunnel: crate::config::TunnelConfig::default(),
                composio: ComposioConfig::default(),
                secrets: SecretsConfig::default(),
                locale: String::from("en"),
            },
            project_context,
            auth_profile: OnboardingAuthProfileDraft::from_optional(
                provider,
                api_key,
                oauth_source,
            ),
        },
        &OnboardingPostActionOptions {
            install_daemon_flag,
            interactive_followups: true,
            skip_followup_notice: None,
            allow_channel_launch_prompt: true,
        },
    )?;

    run_health_check_display(&config).await?;

    cliclack::outro("Setup complete! Run: asterel agent")?;
    Ok((config, autostart))
}

/// Advanced flow: the existing 8-step sequence, wrapped with cliclack framing.
async fn run_advanced_flow(
    _detected: &DetectedState,
    setup_mode: PostgresSetupMode,
    install_daemon_flag: bool,
) -> Result<(Config, bool)> {
    cliclack::log::step("Step 1: Workspace")?;
    let (workspace_dir, config_path) = setup_workspace()?;

    cliclack::log::step("Step 2: Provider")?;
    let (provider, api_key, model, oauth_source) = setup_provider().await?;

    cliclack::log::step("Step 3: Channels")?;
    let channels_config = setup_channels(ChannelsConfig::default()).await?;

    cliclack::log::step("Step 4: Tunnel")?;
    let tunnel_config = setup_tunnel()?;

    cliclack::log::step("Step 5: Tool Mode")?;
    let (composio_config, secrets_config) = setup_tool_mode()?;

    cliclack::log::step("Step 6: Memory")?;
    let mut memory_config = setup_memory()?;
    ensure_postgres_memory_ready(&mut memory_config, setup_mode)?;

    cliclack::log::step("Step 7: Project Context")?;
    let project_ctx = setup_project_context()?;

    cliclack::log::step("Step 8: Scaffold")?;
    let (config, autostart) = finalize_plan_with_post_actions(
        OnboardingPlan {
            draft: OnboardingConfigDraft {
                workspace_dir,
                config_path,
                api_key: (!api_key.is_empty()).then_some(api_key.clone()),
                default_provider: provider.clone(),
                default_model: model,
                channels_config,
                memory: memory_config,
                tunnel: tunnel_config,
                composio: composio_config,
                secrets: secrets_config,
                locale: String::from("en"),
            },
            project_context: project_ctx,
            auth_profile: OnboardingAuthProfileDraft::from_optional(
                provider,
                api_key,
                oauth_source,
            ),
        },
        &OnboardingPostActionOptions {
            install_daemon_flag,
            interactive_followups: true,
            skip_followup_notice: None,
            allow_channel_launch_prompt: true,
        },
    )?;

    println!(
        "  {} {}",
        ui::success("✓"),
        t!("onboard.security_confirm", level = "Supervised")
    );
    println!(
        "  {} {}",
        ui::success("✓"),
        t!(
            "onboard.memory_confirm",
            backend = &config.memory.backend,
            auto_save = if config.memory.auto_save { "on" } else { "off" }
        )
    );

    print_summary(&config);

    cliclack::log::step("Step 9: Health Checks")?;
    run_health_check_display(&config).await?;

    cliclack::outro("Setup complete! Run: asterel agent")?;
    Ok((config, autostart))
}

/// Run health checks and display results via cliclack.
async fn run_health_check_display(config: &Config) -> Result<()> {
    let sp = cliclack::spinner();
    sp.start("Running health checks...");
    let health = super::health::run_health_checks(config).await;
    sp.stop("Health checks complete");

    match health.api_connectivity {
        HealthStatus::Pass(msg) => cliclack::log::success(format!("API: {msg}"))?,
        HealthStatus::Fail(msg) => cliclack::log::warning(format!("API: {msg}"))?,
        HealthStatus::Skip(msg) => cliclack::log::info(format!("API: {msg}"))?,
    }
    match health.workspace_writable {
        HealthStatus::Pass(msg) => cliclack::log::success(format!("Workspace: {msg}"))?,
        HealthStatus::Fail(msg) => cliclack::log::warning(format!("Workspace: {msg}"))?,
        HealthStatus::Skip(msg) => cliclack::log::info(format!("Workspace: {msg}"))?,
    }
    match health.memory_backend {
        HealthStatus::Pass(msg) => cliclack::log::success(format!("Memory: {msg}"))?,
        HealthStatus::Fail(msg) => cliclack::log::warning(format!("Memory: {msg}"))?,
        HealthStatus::Skip(msg) => cliclack::log::info(format!("Memory: {msg}"))?,
    }

    Ok(())
}

/// Interactive repair flow: rerun channel setup only without redoing full onboarding.
///
/// # Errors
/// Returns an error if loading, updating, or saving channel configuration fails.
pub async fn run_channels_repair_wizard() -> Result<(Config, bool)> {
    print_welcome_banner();
    println!("  {}", ui::header(t!("onboard.repair.title")));
    println!();

    let mut config = Config::load_or_init()?;

    print_step(1, 1, &t!("onboard.step.channels"));
    // Pass existing channels through so the wizard preserves any
    // channels the operator does not touch. See Issue #10.
    config.channels_config = setup_channels(config.channels_config.clone()).await?;
    config.save()?;

    println!();
    println!(
        "  {} {}",
        ui::success("✓"),
        t!("onboard.repair.saved", path = config.config_path.display())
    );

    let autostart = run_post_actions(
        &config,
        &OnboardingPostActionOptions {
            install_daemon_flag: false,
            interactive_followups: true,
            skip_followup_notice: None,
            allow_channel_launch_prompt: true,
        },
    )?;

    Ok((config, autostart))
}

// ── Quick setup (zero prompts) ───────────────────────────────────

/// Non-interactive setup: generates a sensible default config instantly.
///
/// # Errors
/// Returns an error if workspace setup, config persistence, or scaffolding fails.
pub fn run_quick_setup(
    api_key: Option<&str>,
    provider: Option<&str>,
    memory_backend: Option<&str>,
    postgres_setup_mode: Option<&str>,
    install_daemon_flag: bool,
) -> Result<(Config, bool)> {
    let setup_mode = resolve_postgres_setup_mode(postgres_setup_mode)?;

    print_welcome_banner();
    println!("  {}", ui::header(t!("onboard.quick.title")));
    println!();

    let asterel_dir = crate::utils::dirs::asterel_home_dir()?;
    let workspace_dir = asterel_dir.join("workspace");
    let config_path = asterel_dir.join("config.toml");

    fs::create_dir_all(&workspace_dir).context("Failed to create workspace directory")?;

    let provider_name = provider
        .unwrap_or(crate::config::DEFAULT_PROVIDER)
        .to_string();
    let model = default_model_for_provider(&provider_name);
    let memory_backend_enum = parse_memory_backend(memory_backend)?;
    let mut memory_config = build_quick_memory_config(memory_backend_enum);
    ensure_postgres_memory_ready(&mut memory_config, setup_mode)?;

    let default_ctx = ProjectContext {
        user_name: std::env::var("USER").unwrap_or_else(|_| "User".into()),
        timezone: "UTC".into(),
        agent_name: "Asterel".into(),
        communication_style:
            "Be warm, natural, and clear. Use occasional relevant emojis (1-2 max) and avoid robotic phrasing."
                .into(),
    };
    let (config, _autostart) = finalize_plan_with_post_actions(
        OnboardingPlan {
            draft: OnboardingConfigDraft {
                workspace_dir: workspace_dir.clone(),
                config_path: config_path.clone(),
                api_key: api_key.map(String::from),
                default_provider: provider_name.clone(),
                default_model: model.clone(),
                channels_config: ChannelsConfig::default(),
                memory: memory_config,
                tunnel: crate::config::TunnelConfig::default(),
                composio: ComposioConfig::default(),
                secrets: SecretsConfig::default(),
                locale: String::from("en"),
            },
            project_context: default_ctx,
            auth_profile: None,
        },
        &OnboardingPostActionOptions {
            install_daemon_flag,
            interactive_followups: false,
            skip_followup_notice: None,
            allow_channel_launch_prompt: false,
        },
    )?;

    print_quick_setup_summary(
        &workspace_dir,
        &config_path,
        &provider_name,
        &model,
        api_key,
        &config,
    );

    Ok((config, false))
}

fn parse_memory_backend(backend: Option<&str>) -> Result<crate::config::MemoryBackend> {
    let normalized = backend
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("postgres")
        .to_ascii_lowercase();

    match normalized.as_str() {
        "none" => Ok(crate::config::MemoryBackend::None),
        "markdown" => Ok(crate::config::MemoryBackend::Markdown),
        "postgres" => Ok(crate::config::MemoryBackend::Postgres),
        _ => anyhow::bail!(
            "Unsupported memory backend '{normalized}'. Use one of: postgres, markdown, none"
        ),
    }
}

fn build_quick_memory_config(backend: crate::config::MemoryBackend) -> MemoryConfig {
    MemoryConfig {
        backend,
        auto_save: backend != crate::config::MemoryBackend::None,
        ..MemoryConfig::default()
    }
}

fn print_quick_setup_summary(
    workspace_dir: &std::path::Path,
    config_path: &std::path::Path,
    provider_name: &str,
    model: &str,
    api_key: Option<&str>,
    config: &Config,
) {
    let (memory_backend, memory_auto_save) = quick_setup_memory_summary(config);

    println!(
        "  {} {} {}",
        ui::success("✓"),
        t!("onboard.quick.workspace"),
        ui::value(workspace_dir.display())
    );
    println!(
        "  {} {} {}",
        ui::success("✓"),
        t!("onboard.quick.provider"),
        ui::value(provider_name)
    );
    println!(
        "  {} {} {}",
        ui::success("✓"),
        t!("onboard.quick.model"),
        ui::value(model)
    );
    println!(
        "  {} {} {}",
        ui::success("✓"),
        t!("onboard.quick.api_key"),
        if api_key.is_some() {
            ui::value(t!("onboard.quick.api_key_set"))
        } else {
            ui::yellow(t!("onboard.quick.api_key_not_set"))
        }
    );
    println!(
        "  {} {} {}",
        ui::success("✓"),
        t!("onboard.quick.security"),
        ui::value(t!("onboard.quick.security_value"))
    );
    println!(
        "  {} {} {} (auto-save: {})",
        ui::success("✓"),
        t!("onboard.quick.memory"),
        ui::value(memory_backend),
        if memory_auto_save { "on" } else { "off" }
    );
    println!(
        "  {} {} {}",
        ui::success("✓"),
        t!("onboard.quick.secrets"),
        ui::value(t!("onboard.quick.secrets_value"))
    );
    println!(
        "  {} {} {}",
        ui::success("✓"),
        t!("onboard.quick.gateway"),
        ui::value(t!("onboard.quick.gateway_value"))
    );
    println!(
        "  {} {} {}",
        ui::success("✓"),
        t!("onboard.quick.tunnel"),
        ui::dim(t!("onboard.quick.tunnel_value"))
    );
    println!(
        "  {} {} {}",
        ui::success("✓"),
        t!("onboard.quick.composio"),
        ui::dim(t!("onboard.quick.composio_value"))
    );
    println!();
    println!(
        "  {} {}",
        ui::header(t!("onboard.quick.config_saved")),
        ui::value(config_path.display())
    );
    println!();
    println!("  {}", ui::header(t!("onboard.summary.next_steps")));
    if api_key.is_none() {
        println!("    1. Set your API key:  export OPENROUTER_API_KEY=\"sk-...\"");
        println!("    2. Or edit:           ~/.asterel/config.toml");
        println!("    3. Chat:              asterel agent -m \"Hello!\"");
        println!("    4. Gateway:           asterel gateway");
    } else {
        println!("    1. Chat:     asterel agent -m \"Hello!\"");
        println!("    2. Gateway:  asterel gateway");
        println!("    3. Status:   asterel status");
    }
    println!();
}

fn quick_setup_memory_summary(config: &Config) -> (crate::config::MemoryBackend, bool) {
    (config.memory.backend, config.memory.auto_save)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::config::{ChannelSecurityPolicy, Config, TelegramConfig};
    use crate::onboard::prompts::ProjectContext;
    use crate::onboard::scaffold::scaffold_workspace;

    fn resume_context() -> ProjectContext {
        ProjectContext {
            user_name: "Resume User".to_string(),
            timezone: "UTC".to_string(),
            agent_name: "Resume Agent".to_string(),
            communication_style: "Be clear and direct.".to_string(),
        }
    }

    #[test]
    fn happy_path_completion_launch_offer_is_skipped_without_channels() {
        let config = Config {
            api_key: Some("sk-test".to_string()),
            ..Config::default()
        };

        assert!(
            !run_post_actions(
                &config,
                &OnboardingPostActionOptions {
                    interactive_followups: true,
                    allow_channel_launch_prompt: true,
                    ..OnboardingPostActionOptions::default()
                }
            )
            .unwrap()
        );
    }

    #[test]
    fn abort_cancel_mid_flow_launch_offer_is_skipped_without_api_key() {
        let mut config = Config::default();
        config.channels_config.telegram = Some(TelegramConfig {
            bot_token: "bot-token".to_string(),
            allowed_users: Vec::new(),
            default_account: None,
            default_to: None,
            security: ChannelSecurityPolicy::default(),
        });

        assert!(
            !run_post_actions(
                &config,
                &OnboardingPostActionOptions {
                    interactive_followups: true,
                    allow_channel_launch_prompt: true,
                    ..OnboardingPostActionOptions::default()
                }
            )
            .unwrap()
        );
    }

    #[test]
    fn resume_after_interruption_keeps_existing_voice_file() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();

        let sentinel = "resume-marker";
        std::fs::write(workspace.join("CHARACTER.md"), sentinel).unwrap();

        scaffold_workspace(&workspace, &resume_context()).unwrap();

        let voice = std::fs::read_to_string(workspace.join("CHARACTER.md")).unwrap();
        assert_eq!(voice, sentinel);
    }

    #[test]
    fn resume_after_interruption_creates_missing_templates() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();

        std::fs::write(workspace.join("USER.md"), "keep-me").unwrap();

        scaffold_workspace(&workspace, &resume_context()).unwrap();

        let bootstrap = workspace.join("BOOTSTRAP.md");
        assert!(bootstrap.exists());

        let existing_user = std::fs::read_to_string(workspace.join("USER.md")).unwrap();
        assert_eq!(existing_user, "keep-me");
    }

    #[test]
    fn quick_setup_memory_default_maps_to_postgres_backend() {
        assert_eq!(
            parse_memory_backend(None).unwrap(),
            crate::config::MemoryBackend::Postgres
        );
    }

    #[test]
    fn quick_setup_memory_unknown_backend_is_rejected() {
        let err = parse_memory_backend(Some("legacy")).expect_err("unknown backend should fail");
        assert!(
            err.to_string().contains("Unsupported memory backend"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn quick_setup_memory_accepts_postgres_backend() {
        assert_eq!(
            parse_memory_backend(Some("postgres")).unwrap(),
            crate::config::MemoryBackend::Postgres
        );
    }

    #[test]
    fn quick_setup_summary_uses_effective_config_memory_state() {
        let mut config = Config::default();
        config.memory.backend = crate::config::MemoryBackend::None;
        config.memory.auto_save = false;

        assert_eq!(
            quick_setup_memory_summary(&config),
            (crate::config::MemoryBackend::None, false)
        );
    }
}
