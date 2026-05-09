//! Platform-specific service installation (macOS and Linux).
//!
//! Generates and installs launchd plist (macOS) or systemd
//! unit files (Linux) for running the daemon as an OS service.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::spawn::{run_program_checked, xml_escape};
use super::{SERVICE_LABEL, SYSTEMD_USER_UNIT};
use crate::config::Config;
use crate::security::{ProcessSpawnClass, SecurityPolicy};
use crate::ui::style as ui;

/// Generates and installs a macOS launchd plist service file.
///
/// # Errors
///
/// Returns an error if file creation or write fails.
pub(super) fn install_macos(config: &Config, _security: &SecurityPolicy) -> Result<()> {
    let file = macos_service_file()?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }

    let exe = std::env::current_exe().context("Failed to resolve current executable")?;
    let logs_dir = config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("logs");
    fs::create_dir_all(&logs_dir)?;

    let stdout = logs_dir.join("daemon.stdout.log");
    let stderr = logs_dir.join("daemon.stderr.log");

    let plist = format!(
        r#"<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>daemon</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{stdout}</string>
  <key>StandardErrorPath</key>
  <string>{stderr}</string>
</dict>
</plist>
"#,
        label = SERVICE_LABEL,
        exe = xml_escape(&exe.display().to_string()),
        stdout = xml_escape(&stdout.display().to_string()),
        stderr = xml_escape(&stderr.display().to_string())
    );

    fs::write(&file, plist)?;
    println!();
    println!("  {}", ui::section("Service Install"));
    println!("{}", ui::field_line("Platform", "launchd"));
    println!("{}", ui::field_line("Result", ui::ok_badge("installed")));
    println!("{}", ui::field_line("Unit", file.display()));
    println!("{}", ui::note_line("Start with:"));
    println!("{}", ui::command_line("asterel service start"));
    Ok(())
}

/// Generates and installs a Linux systemd user service unit.
///
/// # Errors
///
/// Returns an error if file creation or systemctl commands fail.
pub(super) fn install_linux(config: &Config, security: &SecurityPolicy) -> Result<()> {
    let file = linux_service_file(config)?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }

    let exe = std::env::current_exe().context("Failed to resolve current executable")?;
    let unit = format!(
        "[Unit]\nDescription=Asterel daemon\nAfter=network.target\n\n[Service]\nType=simple\nExecStart={} daemon\nRestart=always\nRestartSec=3\n\n[Install]\nWantedBy=default.target\n",
        exe.display()
    );

    fs::write(&file, unit)?;
    if let Err(error) = run_program_checked(
        security,
        "platform_service_install_daemon_reload",
        ProcessSpawnClass::OperatorPlane,
        "systemctl",
        |command| {
            command.args(["--user", "daemon-reload"]);
        },
    ) {
        rollback_linux_install(&file);
        return Err(error).context("reload systemd user daemon after service install");
    }
    if let Err(error) = run_program_checked(
        security,
        "platform_service_install_enable",
        ProcessSpawnClass::OperatorPlane,
        "systemctl",
        |command| {
            command.args(["--user", "enable", SYSTEMD_USER_UNIT]);
        },
    ) {
        rollback_linux_install(&file);
        return Err(error).context("enable systemd user service after install");
    }
    println!();
    println!("  {}", ui::section("Service Install"));
    println!("{}", ui::field_line("Platform", "systemd-user"));
    println!("{}", ui::field_line("Result", ui::ok_badge("installed")));
    println!("{}", ui::field_line("Unit", file.display()));
    println!("{}", ui::note_line("Start with:"));
    println!("{}", ui::command_line("asterel service start"));
    Ok(())
}

fn rollback_linux_install(file: &std::path::Path) {
    if let Err(error) = fs::remove_file(file) {
        tracing::warn!(%error, unit = %file.display(), "failed to remove partially installed unit file");
    }
}

#[cfg(test)]
mod tests {
    use super::rollback_linux_install;
    use tempfile::TempDir;

    #[test]
    fn rollback_linux_install_removes_partial_unit_file() {
        let tmp = TempDir::new().expect("tempdir");
        let file = tmp.path().join("asterel.service");
        std::fs::write(&file, "[Unit]\nDescription=test").expect("write file");

        rollback_linux_install(&file);

        assert!(!file.exists());
    }
}

/// Returns the path to the macOS launchd plist file.
///
/// # Errors
///
/// Returns an error if the home directory cannot be resolved.
pub(super) fn macos_service_file() -> Result<PathBuf> {
    let home = directories::UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .context("Could not find home directory")?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{SERVICE_LABEL}.plist")))
}

/// Returns the path to the Linux systemd user service file.
///
/// # Errors
///
/// Returns an error if the home directory cannot be resolved.
pub(super) fn linux_service_file(config: &Config) -> Result<PathBuf> {
    let home = directories::UserDirs::new()
        .map(|u| u.home_dir().to_path_buf())
        .context("Could not find home directory")?;
    let _ = config;
    Ok(home
        .join(".config")
        .join("systemd")
        .join("user")
        .join(SYSTEMD_USER_UNIT))
}
