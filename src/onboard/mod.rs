//! Onboarding subsystem: guided setup wizard, configuration builder, and health checks.
//!
//! Walks a new operator through initial configuration of channels, provider API keys,
//! database connections, and permissions. The subsystem is structured as a multi-step wizard:
//!
//! - **[`wizard`]** / **[`flow`]** — Top-level wizard entry points: [`run_wizard`] for full
//!   interactive setup, [`run_quick_setup`] for non-interactive defaults, and
//!   [`run_channels_repair_wizard`] for repairing a broken channel configuration.
//! - **[`detect`]** — Auto-detects existing environment variables and partial configs.
//! - **[`config_builder`]** — Assembles a validated `Config` from wizard-collected inputs.
//! - **[`domain`]** — Domain-specific prompts and validation rules per integration type.
//! - **[`health`]** — Post-setup health checks: connectivity, auth round-trips, DB migrations.
//! - **[`api_verify`]** — API-key verification helpers for each provider.
//! - **[`auth_profile`]** — Persists and retrieves saved auth profiles.
//! - **[`postgres`]** — PostgreSQL connection setup and schema migration helpers.
//! - **[`scaffold`]** — Generates skeleton config and skills-directory structure on first run.
//! - **[`prompts`]** / **[`view`]** — TUI prompts and terminal view rendering for the wizard.

pub(crate) mod api_verify;
pub(crate) mod auth_profile;
pub(crate) mod completion;
pub(crate) mod config_builder;
pub(crate) mod detect;
pub(crate) mod domain;
pub(crate) mod flow;
pub(crate) mod health;
pub(crate) mod postgres;
pub(crate) mod prompts;
pub(crate) mod scaffold;
pub(crate) mod view;
pub(crate) mod wizard;

pub use flow::{run_channels_repair_wizard, run_quick_setup, run_wizard};
