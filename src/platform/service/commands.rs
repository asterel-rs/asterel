//! CLI command handler for OS service management.
//!
//! Dispatches `service install`, `service uninstall`, `service start`,
//! `service stop`, and `service status` to platform-specific backends.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use super::platform::{install_linux, install_macos, linux_service_file, macos_service_file};
use super::spawn::{run_checked, run_program_capture, run_program_checked};
use super::{SERVICE_LABEL, SYSTEMD_USER_UNIT};
use super::{ServiceInstallState, detect_service_install_status};
use crate::config::Config;
use crate::security::{ProcessSpawnClass, SecurityPolicy};
use crate::ui::style as ui;

/// Dispatches a service CLI subcommand (install, start, stop,
/// status, uninstall).
///
/// # Errors
///
/// Returns an error if the underlying platform operation fails.
pub fn handle_command(command: &crate::ServiceCommands, config: &Config) -> Result<()> {
    let security = SecurityPolicy::from_config_runtime(
        &config.autonomy,
        &config.runtime,
        &config.workspace_dir,
    );
    let service_security = service_security_policy(&security);
    match command {
        crate::ServiceCommands::Install => install(config, &service_security),
        crate::ServiceCommands::Start => start(config, &service_security),
        crate::ServiceCommands::Stop => stop(config, &service_security),
        crate::ServiceCommands::Status => status(config, &service_security),
        crate::ServiceCommands::Uninstall => uninstall(config, &service_security),
    }
}

fn service_security_policy(base: &SecurityPolicy) -> SecurityPolicy {
    let mut scoped = base.clone();
    allow_command_if_missing(&mut scoped.allowed_commands, "systemctl");
    allow_command_if_missing(&mut scoped.allowed_commands, "launchctl");
    scoped
}

fn allow_command_if_missing(commands: &mut Vec<String>, command: &str) {
    if commands.iter().any(|existing| existing == command) {
        return;
    }
    commands.push(command.to_string());
}

fn install(config: &Config, security: &SecurityPolicy) -> Result<()> {
    if cfg!(target_os = "macos") {
        install_macos(config, security)
    } else if cfg!(target_os = "linux") {
        install_linux(config, security)
    } else {
        anyhow::bail!("Service management is supported on macOS and Linux only");
    }
}

fn start(config: &Config, security: &SecurityPolicy) -> Result<()> {
    if cfg!(target_os = "macos") {
        let plist = macos_service_file()?;
        run_program_checked(
            security,
            "platform_service_start_load",
            ProcessSpawnClass::OperatorPlane,
            "launchctl",
            |command| {
                command.arg("load").arg("-w").arg(&plist);
            },
        )?;
        run_program_checked(
            security,
            "platform_service_start_start",
            ProcessSpawnClass::OperatorPlane,
            "launchctl",
            |command| {
                command.arg("start").arg(SERVICE_LABEL);
            },
        )?;
        println!();
        println!("  {}", ui::section("Service Start"));
        println!("{}", ui::field_line("Result", ui::ok_badge("started")));
        println!(
            "{}",
            ui::field_line("Unit", macos_service_file()?.display())
        );
        Ok(())
    } else if cfg!(target_os = "linux") {
        run_program_checked(
            security,
            "platform_service_start_daemon_reload",
            ProcessSpawnClass::OperatorPlane,
            "systemctl",
            |command| {
                command.args(["--user", "daemon-reload"]);
            },
        )?;
        run_program_checked(
            security,
            "platform_service_start_start",
            ProcessSpawnClass::OperatorPlane,
            "systemctl",
            |command| {
                command.args(["--user", "start", SYSTEMD_USER_UNIT]);
            },
        )?;
        println!();
        println!("  {}", ui::section("Service Start"));
        println!("{}", ui::field_line("Result", ui::ok_badge("started")));
        println!("{}", ui::field_line("Unit", SYSTEMD_USER_UNIT));
        Ok(())
    } else {
        let _ = config;
        anyhow::bail!("Service management is supported on macOS and Linux only")
    }
}

fn stop_macos_with_runner<F>(security: &SecurityPolicy, mut runner: F) -> Result<()>
where
    F: FnMut(&SecurityPolicy, &str, ProcessSpawnClass, &mut Command) -> Result<()>,
{
    let plist = macos_service_file()?;

    let mut stop_command = Command::new("launchctl");
    stop_command.arg("stop").arg(SERVICE_LABEL);
    runner(
        security,
        "platform_service_stop_stop",
        ProcessSpawnClass::OperatorPlane,
        &mut stop_command,
    )?;

    let mut unload_command = Command::new("launchctl");
    unload_command.arg("unload").arg("-w").arg(&plist);
    runner(
        security,
        "platform_service_stop_unload",
        ProcessSpawnClass::OperatorPlane,
        &mut unload_command,
    )?;

    Ok(())
}

fn stop_linux_with_runner<F>(security: &SecurityPolicy, mut runner: F) -> Result<()>
where
    F: FnMut(&SecurityPolicy, &str, ProcessSpawnClass, &mut Command) -> Result<()>,
{
    let mut stop_command = Command::new("systemctl");
    stop_command.args(["--user", "stop", SYSTEMD_USER_UNIT]);
    runner(
        security,
        "platform_service_stop_stop",
        ProcessSpawnClass::OperatorPlane,
        &mut stop_command,
    )?;

    Ok(())
}

fn disable_linux_with_runner<F>(security: &SecurityPolicy, mut runner: F) -> Result<()>
where
    F: FnMut(&SecurityPolicy, &str, ProcessSpawnClass, &mut Command) -> Result<()>,
{
    let mut disable_command = Command::new("systemctl");
    disable_command.args(["--user", "disable", SYSTEMD_USER_UNIT]);
    runner(
        security,
        "platform_service_uninstall_disable",
        ProcessSpawnClass::OperatorPlane,
        &mut disable_command,
    )?;

    Ok(())
}

fn uninstall_macos_with_runner<F>(
    security: &SecurityPolicy,
    file: &Path,
    mut runner: F,
) -> Result<Vec<String>>
where
    F: FnMut(&SecurityPolicy, &str, ProcessSpawnClass, &mut Command) -> Result<()>,
{
    let mut warnings = Vec::new();
    if let Err(error) = stop_macos_with_runner(security, |security, route, class, command| {
        runner(security, route, class, command)
    }) {
        warnings.push(format!("stop failed: {error}"));
    }

    if file.exists() {
        fs::remove_file(file).with_context(|| format!("Failed to remove {}", file.display()))?;
    }

    Ok(warnings)
}

fn stop(config: &Config, security: &SecurityPolicy) -> Result<()> {
    if cfg!(target_os = "macos") {
        stop_macos_with_runner(security, run_checked)?;
        println!();
        println!("  {}", ui::section("Service Stop"));
        println!("{}", ui::field_line("Result", ui::ok_badge("stopped")));
        Ok(())
    } else if cfg!(target_os = "linux") {
        stop_linux_with_runner(security, run_checked)?;
        println!();
        println!("  {}", ui::section("Service Stop"));
        println!("{}", ui::field_line("Result", ui::ok_badge("stopped")));
        Ok(())
    } else {
        let _ = config;
        anyhow::bail!("Service management is supported on macOS and Linux only")
    }
}

fn status(config: &Config, security: &SecurityPolicy) -> Result<()> {
    if cfg!(target_os = "macos") {
        let out = run_program_capture(
            security,
            "platform_service_status_list",
            ProcessSpawnClass::OperatorPlane,
            "launchctl",
            |command| {
                command.arg("list");
            },
        )?;
        let running = out.lines().any(|line| line.contains(SERVICE_LABEL));
        println!();
        println!("  {}", ui::section("Service Status"));
        println!(
            "{}",
            ui::field_line(
                "Service",
                if running {
                    ui::ok_badge("running/loaded")
                } else {
                    ui::warn_badge("not loaded")
                }
            )
        );
        println!(
            "{}",
            ui::field_line("Unit", macos_service_file()?.display())
        );
        return Ok(());
    }

    if cfg!(target_os = "linux") {
        let service_state = detect_service_install_status(config, security)?;
        println!();
        println!("  {}", ui::section("Service Status"));
        match service_state.state {
            ServiceInstallState::Installed => println!(
                "{}",
                ui::field_line(
                    "State",
                    service_state.active_state.as_deref().map_or_else(
                        || ui::ok_badge("installed"),
                        |state| {
                            if state.eq_ignore_ascii_case("active") {
                                ui::ok_badge(state)
                            } else {
                                ui::muted_badge(state)
                            }
                        }
                    )
                )
            ),
            ServiceInstallState::NotInstalled => println!(
                "{}",
                ui::field_line("State", ui::warn_badge("not installed"))
            ),
            ServiceInstallState::PartialArtifact => println!(
                "{}",
                ui::field_line("State", ui::warn_badge("partial install artifact"))
            ),
            ServiceInstallState::ManagerUnavailable => anyhow::bail!(
                "service manager unavailable: {}",
                service_state
                    .detail
                    .as_deref()
                    .unwrap_or("systemd user manager could not be queried")
            ),
        }
        if let Some(detail) = service_state.detail.as_deref()
            && !detail.is_empty()
            && !matches!(service_state.state, ServiceInstallState::ManagerUnavailable)
        {
            println!("{}", ui::field_line("Detail", detail));
        }
        println!(
            "{}",
            ui::field_line("Unit", service_state.unit_path.display())
        );
        return Ok(());
    }

    anyhow::bail!("Service management is supported on macOS and Linux only")
}

fn uninstall(config: &Config, security: &SecurityPolicy) -> Result<()> {
    if cfg!(target_os = "macos") {
        let file = macos_service_file()?;
        let warnings = uninstall_macos_with_runner(security, &file, run_checked)?;
        println!();
        println!("  {}", ui::section("Service Uninstall"));
        println!("{}", ui::field_line("Result", ui::ok_badge("uninstalled")));
        println!("{}", ui::field_line("Unit", file.display()));
        for warning in warnings {
            println!("{}", ui::note_line(warning));
        }
        return Ok(());
    }

    if cfg!(target_os = "linux") {
        let mut warnings = Vec::new();
        if let Err(error) = stop_linux_with_runner(security, run_checked) {
            warnings.push(format!("stop failed: {error}"));
        }
        if let Err(error) = disable_linux_with_runner(security, run_checked) {
            warnings.push(format!("disable failed: {error}"));
        }

        let file = linux_service_file(config)?;
        if file.exists() {
            fs::remove_file(&file)
                .with_context(|| format!("Failed to remove {}", file.display()))?;
        }
        if let Err(error) = run_program_checked(
            security,
            "platform_service_uninstall_daemon_reload",
            ProcessSpawnClass::OperatorPlane,
            "systemctl",
            |command| {
                command.args(["--user", "daemon-reload"]);
            },
        ) {
            warnings.push(format!("daemon-reload failed: {error}"));
        }
        println!();
        println!("  {}", ui::section("Service Uninstall"));
        println!("{}", ui::field_line("Result", ui::ok_badge("uninstalled")));
        println!("{}", ui::field_line("Unit", file.display()));
        for warning in warnings {
            println!("{}", ui::note_line(warning));
        }
        return Ok(());
    }

    anyhow::bail!("Service management is supported on macOS and Linux only")
}

#[cfg(test)]
mod tests {
    use super::service_security_policy;
    use super::{
        disable_linux_with_runner, stop_linux_with_runner, stop_macos_with_runner,
        uninstall_macos_with_runner,
    };
    use crate::security::{AutonomyLevel, ProcessSpawnClass, SecurityPolicy};
    use tempfile::TempDir;

    #[test]
    fn service_security_policy_includes_platform_service_commands() {
        let base = SecurityPolicy {
            allowed_commands: vec!["git".to_string()],
            ..SecurityPolicy::default()
        };

        let scoped = service_security_policy(&base);
        assert!(scoped.allowed_commands.contains(&"git".to_string()));
        assert!(scoped.allowed_commands.contains(&"systemctl".to_string()));
        assert!(scoped.allowed_commands.contains(&"launchctl".to_string()));
    }

    #[test]
    fn service_security_policy_does_not_duplicate_existing_entries() {
        let base = SecurityPolicy {
            allowed_commands: vec![
                "git".to_string(),
                "systemctl".to_string(),
                "launchctl".to_string(),
            ],
            autonomy: AutonomyLevel::Full,
            ..SecurityPolicy::default()
        };

        let scoped = service_security_policy(&base);
        let systemctl_count = scoped
            .allowed_commands
            .iter()
            .filter(|entry| entry.as_str() == "systemctl")
            .count();
        let launchctl_count = scoped
            .allowed_commands
            .iter()
            .filter(|entry| entry.as_str() == "launchctl")
            .count();
        assert_eq!(systemctl_count, 1);
        assert_eq!(launchctl_count, 1);
        assert_eq!(scoped.autonomy, AutonomyLevel::Full);
    }

    #[test]
    fn stop_linux_with_runner_propagates_backend_error() {
        let security = SecurityPolicy::default();
        let error = stop_linux_with_runner(&security, |_security, route, _class, command| {
            assert_eq!(route, "platform_service_stop_stop");
            assert_eq!(command.get_program().to_string_lossy(), "systemctl");
            Err(anyhow::anyhow!("stop failed"))
        })
        .expect_err("linux stop should propagate runner failure");

        assert!(error.to_string().contains("stop failed"));
    }

    #[test]
    fn stop_linux_with_runner_uses_expected_route_class_and_arguments() {
        let security = SecurityPolicy::default();
        let mut calls = Vec::new();

        stop_linux_with_runner(&security, |_security, route, class, command| {
            calls.push((
                route.to_string(),
                class,
                command.get_program().to_string_lossy().to_string(),
                command
                    .get_args()
                    .map(|arg| arg.to_string_lossy().to_string())
                    .collect::<Vec<_>>(),
            ));
            Ok(())
        })
        .expect("linux stop should succeed");

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "platform_service_stop_stop");
        assert_eq!(calls[0].1, ProcessSpawnClass::OperatorPlane);
        assert_eq!(calls[0].2, "systemctl");
        assert_eq!(calls[0].3, vec!["--user", "stop", super::SYSTEMD_USER_UNIT]);
    }

    #[test]
    fn stop_macos_with_runner_propagates_unload_error() {
        let security = SecurityPolicy::default();
        let mut seen_routes = Vec::new();
        let error = stop_macos_with_runner(&security, |_security, route, _class, _command| {
            seen_routes.push(route.to_string());
            if route == "platform_service_stop_unload" {
                Err(anyhow::anyhow!("unload failed"))
            } else {
                Ok(())
            }
        })
        .expect_err("macos stop should propagate unload failure");

        assert_eq!(
            seen_routes,
            vec![
                "platform_service_stop_stop".to_string(),
                "platform_service_stop_unload".to_string()
            ]
        );
        assert!(error.to_string().contains("unload failed"));
    }

    #[test]
    fn stop_macos_with_runner_issues_stop_then_unload_with_expected_arguments() {
        let security = SecurityPolicy::default();
        let mut calls = Vec::new();

        stop_macos_with_runner(&security, |_security, route, class, command| {
            calls.push((
                route.to_string(),
                class,
                command.get_program().to_string_lossy().to_string(),
                command
                    .get_args()
                    .map(|arg| arg.to_string_lossy().to_string())
                    .collect::<Vec<_>>(),
            ));
            Ok(())
        })
        .expect("macos stop should succeed");

        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "platform_service_stop_stop");
        assert_eq!(calls[0].1, ProcessSpawnClass::OperatorPlane);
        assert_eq!(calls[0].2, "launchctl");
        assert_eq!(calls[0].3, vec!["stop", super::SERVICE_LABEL]);
        assert_eq!(calls[1].0, "platform_service_stop_unload");
        assert_eq!(calls[1].1, ProcessSpawnClass::OperatorPlane);
        assert_eq!(calls[1].2, "launchctl");
        assert_eq!(
            calls[1].3,
            vec![
                "unload".to_string(),
                "-w".to_string(),
                super::macos_service_file()
                    .expect("macos service file")
                    .display()
                    .to_string(),
            ]
        );
    }

    #[test]
    fn uninstall_macos_with_runner_removes_file_even_when_stop_fails() {
        let security = SecurityPolicy::default();
        let tmp = TempDir::new().expect("tempdir");
        let file = tmp.path().join("com.example.asterel.plist");
        std::fs::write(&file, "plist").expect("write plist");

        let warnings =
            uninstall_macos_with_runner(&security, &file, |_security, _route, _class, _command| {
                Err(anyhow::anyhow!("job not loaded"))
            })
            .expect("macos uninstall should still remove the file");

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("stop failed: job not loaded"));
        assert!(!file.exists());
    }

    #[test]
    fn disable_linux_with_runner_propagates_backend_error() {
        let security = SecurityPolicy::default();
        let error = disable_linux_with_runner(&security, |_security, route, _class, command| {
            assert_eq!(route, "platform_service_uninstall_disable");
            assert_eq!(command.get_program().to_string_lossy(), "systemctl");
            Err(anyhow::anyhow!("disable failed"))
        })
        .expect_err("linux disable should propagate runner failure");

        assert!(error.to_string().contains("disable failed"));
    }

    #[test]
    fn disable_linux_with_runner_uses_expected_route_class_and_arguments() {
        let security = SecurityPolicy::default();
        let mut calls = Vec::new();

        disable_linux_with_runner(&security, |_security, route, class, command| {
            calls.push((
                route.to_string(),
                class,
                command.get_program().to_string_lossy().to_string(),
                command
                    .get_args()
                    .map(|arg| arg.to_string_lossy().to_string())
                    .collect::<Vec<_>>(),
            ));
            Ok(())
        })
        .expect("linux disable should succeed");

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "platform_service_uninstall_disable");
        assert_eq!(calls[0].1, ProcessSpawnClass::OperatorPlane);
        assert_eq!(calls[0].2, "systemctl");
        assert_eq!(
            calls[0].3,
            vec!["--user", "disable", super::SYSTEMD_USER_UNIT]
        );
    }
}
