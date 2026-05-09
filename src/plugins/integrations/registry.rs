//! Integration registry: catalog of all integrations with their
//! status-checking functions.

use super::IntegrationEntry;
use crate::config::Config;

mod catalog;
mod status;

/// Returns the full catalog of integrations.
#[must_use]
pub fn all_integrations() -> Vec<IntegrationEntry> {
    catalog::all_integrations()
}

#[must_use]
pub fn runnable_integrations(config: &Config) -> Vec<IntegrationEntry> {
    let _ = config;
    all_integrations()
}

#[cfg(test)]
mod tests;
