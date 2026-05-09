//! Integration registry: catalog, runnable-status checks, CLI display,
//! and scope-lock inventory for third-party service integrations.

pub mod cli;
pub mod inventory;
pub mod registry;
pub mod types;

pub use cli::handle_command;
pub use types::{IntegrationCategory, IntegrationEntry, IntegrationStatus};
