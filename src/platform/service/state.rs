use std::process::Command;

use anyhow::Result;

use super::SYSTEMD_USER_UNIT;
use super::platform::{linux_service_file, macos_service_file};
use super::spawn::{ObservedCommandOutput, run_observed};
use crate::config::Config;
use crate::security::{ProcessSpawnClass, SecurityPolicy};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceInstallState {
    Installed,
    NotInstalled,
    PartialArtifact,
    ManagerUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInstallStatus {
    pub state: ServiceInstallState,
    pub unit_path: std::path::PathBuf,
    pub active_state: Option<String>,
    pub detail: Option<String>,
}

pub(crate) fn detect_service_install_status(
    config: &Config,
    security: &SecurityPolicy,
) -> Result<ServiceInstallStatus> {
    if cfg!(target_os = "macos") {
        let unit_path = macos_service_file()?;
        return Ok(ServiceInstallStatus {
            state: if unit_path.exists() {
                ServiceInstallState::Installed
            } else {
                ServiceInstallState::NotInstalled
            },
            unit_path,
            active_state: None,
            detail: None,
        });
    }

    if cfg!(target_os = "linux") {
        return detect_linux_service_install_status(config, security);
    }

    anyhow::bail!("Service management is supported on macOS and Linux only")
}

fn detect_linux_service_install_status(
    config: &Config,
    security: &SecurityPolicy,
) -> Result<ServiceInstallStatus> {
    let unit_path = linux_service_file(config)?;
    let file_exists = unit_path.exists();

    let enabled = observe_systemctl_user(
        security,
        "platform_service_status_is_enabled",
        ["--user", "is-enabled", SYSTEMD_USER_UNIT],
    )?;
    let active = observe_systemctl_user(
        security,
        "platform_service_status_is_active",
        ["--user", "is-active", SYSTEMD_USER_UNIT],
    )?;

    let enabled_state = enabled.primary_text().trim().to_ascii_lowercase();
    let active_state = active.primary_text().trim().to_ascii_lowercase();
    let combined = format!("{}\n{}", enabled.combined(), active.combined()).to_ascii_lowercase();

    Ok(classify_linux_service_install_status(
        unit_path,
        file_exists,
        &enabled_state,
        &active_state,
        &combined,
    ))
}

fn classify_linux_service_install_status(
    unit_path: std::path::PathBuf,
    file_exists: bool,
    enabled_state: &str,
    active_state: &str,
    combined: &str,
) -> ServiceInstallStatus {
    if looks_like_manager_unavailable(combined) {
        return ServiceInstallStatus {
            state: if file_exists {
                ServiceInstallState::PartialArtifact
            } else {
                ServiceInstallState::ManagerUnavailable
            },
            unit_path,
            active_state: (!active_state.is_empty()).then_some(active_state.to_string()),
            detail: Some(combined.trim().to_string()),
        };
    }

    if enabled_state == "not-found" {
        return ServiceInstallStatus {
            state: if file_exists {
                ServiceInstallState::PartialArtifact
            } else {
                ServiceInstallState::NotInstalled
            },
            unit_path,
            active_state: (!active_state.is_empty()).then_some(active_state.to_string()),
            detail: file_exists.then_some(
                "unit file exists locally, but systemd user manager does not know about it"
                    .to_string(),
            ),
        };
    }

    ServiceInstallStatus {
        state: ServiceInstallState::Installed,
        unit_path,
        active_state: (!active_state.is_empty()).then_some(active_state.to_string()),
        detail: Some(enabled_state.to_string()),
    }
}

fn observe_systemctl_user<const N: usize>(
    security: &SecurityPolicy,
    route_marker: &str,
    args: [&str; N],
) -> Result<ObservedCommandOutput> {
    let mut command = Command::new("systemctl");
    command.args(args);
    run_observed(
        security,
        route_marker,
        ProcessSpawnClass::OperatorPlane,
        &mut command,
    )
}

fn looks_like_manager_unavailable(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    [
        "failed to connect to bus",
        "no medium found",
        "dbus",
        "failed to connect to user scope bus",
        "transport endpoint is not connected",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::{
        ServiceInstallState, classify_linux_service_install_status, looks_like_manager_unavailable,
    };
    use std::path::PathBuf;

    #[test]
    fn manager_unavailable_detector_covers_dbus_errors() {
        assert!(looks_like_manager_unavailable(
            "Failed to connect to bus: No medium found"
        ));
        assert!(looks_like_manager_unavailable(
            "Failed to connect to user scope bus via local transport"
        ));
        assert!(!looks_like_manager_unavailable("inactive"));
    }

    #[test]
    fn classify_linux_service_install_status_marks_partial_artifact_for_not_found_unit() {
        let status = classify_linux_service_install_status(
            PathBuf::from("/tmp/asterel.service"),
            true,
            "not-found",
            "inactive",
            "not-found\ninactive",
        );

        assert_eq!(status.state, ServiceInstallState::PartialArtifact);
        assert_eq!(status.active_state.as_deref(), Some("inactive"));
    }

    #[test]
    fn classify_linux_service_install_status_marks_manager_unavailable_without_artifact() {
        let status = classify_linux_service_install_status(
            PathBuf::from("/tmp/asterel.service"),
            false,
            "failed",
            "unknown",
            "Failed to connect to bus: No medium found",
        );

        assert_eq!(status.state, ServiceInstallState::ManagerUnavailable);
        assert!(status.detail.as_deref().is_some_and(|detail| {
            detail
                .to_ascii_lowercase()
                .contains("failed to connect to bus")
        }));
    }
}
