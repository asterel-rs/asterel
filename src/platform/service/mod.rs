//! Re-exports for the OS service management subsystem.

mod commands;
mod platform;
mod spawn;
mod state;

#[cfg(test)]
mod tests;

pub use commands::handle_command;
pub(crate) use state::{ServiceInstallState, detect_service_install_status};

/// Reverse-DNS label used for launchd / systemd service
/// registration.
pub(super) const SERVICE_LABEL: &str = "com.asterel.daemon";

/// Canonical systemd user unit name for the daemon service.
pub(super) const SYSTEMD_USER_UNIT: &str = "asterel.service";
