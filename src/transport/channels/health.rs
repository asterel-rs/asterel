//! Channel health classification: maps connection-check results to
//! `Healthy`, `Unhealthy`, or `Timeout` states for the doctor command.
/// Tri-state health classification for a channel connection check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChannelHealthState {
    Healthy,
    Unhealthy,
    Timeout,
}

/// Maps a health-check result (success, failure, or timeout) to a
/// `ChannelHealthState`.
pub(crate) fn classify_health_result(
    result: &std::result::Result<bool, tokio::time::error::Elapsed>,
) -> ChannelHealthState {
    match result {
        Ok(true) => ChannelHealthState::Healthy,
        Ok(false) => ChannelHealthState::Unhealthy,
        Err(_) => ChannelHealthState::Timeout,
    }
}
