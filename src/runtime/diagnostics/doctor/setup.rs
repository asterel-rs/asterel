//! Setup health checks: validates config file, workspace, provider,
//! API key, memory backend, and OS service installation.

use std::fs;

use anyhow::Result;

use crate::config::Config;

/// Run all setup health checks and return pass/fail pairs with descriptions.
pub(crate) fn run_setup_checks(config: &Config) -> Vec<(bool, String)> {
    let mut checks: Vec<(bool, String)> = Vec::new();

    let config_exists = config.config_path.exists();
    checks.push((
        config_exists,
        format!(
            "Config file: {}",
            if config_exists {
                config.config_path.display().to_string()
            } else {
                format!("missing ({})", config.config_path.display())
            }
        ),
    ));

    let (config_valid, config_message) = match config.validate_autonomy_controls() {
        Ok(()) => (true, "Config validation: passed".to_string()),
        Err(error) => (false, format!("Config validation: {error}")),
    };
    checks.push((config_valid, config_message));

    let ws_exists = config.workspace_dir.exists();
    checks.push((
        ws_exists,
        format!(
            "Workspace: {}",
            if ws_exists {
                config.workspace_dir.display().to_string()
            } else {
                format!("missing ({})", config.workspace_dir.display())
            }
        ),
    ));

    let has_provider = config.default_provider.is_some();
    checks.push((
        has_provider,
        format!(
            "Provider: {}",
            config
                .default_provider
                .as_deref()
                .unwrap_or("not configured — run: asterel onboard")
        ),
    ));

    let has_api_key = config.api_key.is_some() || std::env::var("ASTEREL_API_KEY").is_ok();
    checks.push((
        has_api_key,
        if has_api_key {
            "API key: configured".to_string()
        } else {
            "API key: not set — run: asterel onboard".to_string()
        },
    ));

    let memory_ok = config.memory.backend != crate::config::MemoryBackend::None;
    checks.push((
        memory_ok,
        format!(
            "Memory: {} (auto-save: {})",
            config.memory.backend,
            if config.memory.auto_save { "on" } else { "off" }
        ),
    ));

    checks.push(service_installation_check(config));

    checks
}

/// Apply safe, local repairs for missing config/workspace prerequisites.
///
/// # Errors
///
/// Returns an error when filesystem repairs fail.
pub(crate) fn apply_setup_repairs(config: &Config) -> Result<Vec<String>> {
    let mut actions = Vec::new();

    if let Some(parent) = config.config_path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent)?;
        actions.push(format!("created config directory {}", parent.display()));
    }

    if !config.workspace_dir.exists() {
        fs::create_dir_all(&config.workspace_dir)?;
        actions.push(format!(
            "created workspace directory {}",
            config.workspace_dir.display()
        ));
    }

    if !config.config_path.exists() {
        config.save()?;
        actions.push(format!(
            "wrote config file {}",
            config.config_path.display()
        ));
    }

    Ok(actions)
}

fn service_installation_check(config: &Config) -> (bool, String) {
    let security = crate::security::SecurityPolicy::from_config_runtime(
        &config.autonomy,
        &config.runtime,
        &config.workspace_dir,
    );
    let mut service_security = security.clone();
    if !service_security
        .allowed_commands
        .iter()
        .any(|cmd| cmd == "systemctl")
    {
        service_security
            .allowed_commands
            .push("systemctl".to_string());
    }
    if !service_security
        .allowed_commands
        .iter()
        .any(|cmd| cmd == "launchctl")
    {
        service_security
            .allowed_commands
            .push("launchctl".to_string());
    }

    match crate::platform::service::detect_service_install_status(config, &service_security) {
        Ok(status) => match status.state {
            crate::platform::service::ServiceInstallState::Installed => (
                true,
                format!("OS service: installed ({})", status.unit_path.display()),
            ),
            crate::platform::service::ServiceInstallState::NotInstalled => (
                false,
                "OS service: not installed — optional, run: asterel service install".to_string(),
            ),
            crate::platform::service::ServiceInstallState::PartialArtifact => (
                false,
                format!(
                    "OS service: partial install artifact at {} — run: asterel service uninstall",
                    status.unit_path.display()
                ),
            ),
            crate::platform::service::ServiceInstallState::ManagerUnavailable => (
                false,
                format!(
                    "OS service manager unavailable: {}",
                    status
                        .detail
                        .as_deref()
                        .unwrap_or("service manager could not be queried")
                ),
            ),
        },
        Err(error) => (false, format!("OS service: state query failed ({error})")),
    }
}
