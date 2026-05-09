#![warn(clippy::all, clippy::pedantic)]
#![deny(unsafe_code)]
#![cfg_attr(test, allow(clippy::large_stack_arrays))]
//! Public library entry point for `Asterel`.
//!
//! This crate exposes the CLI command model, configuration schema,
//! runtime subsystems, transport adapters, and security primitives used
//! by the binary and integration surfaces.

#[macro_use]
extern crate rust_i18n;

i18n!("locales", fallback = "en");

/// CLI commands and command-line surface types.
pub mod cli;
/// Configuration loading and schema modules.
pub mod config;
/// Shared strong types and boundary-safe domain contracts.
pub mod contracts;
/// Core agent/runtime logic (providers, tools, memory, persona, sessions).
pub mod core;
/// Media and multimodal processing utilities.
pub(crate) mod media;
/// Onboarding flows and first-run assistants.
pub mod onboard;
#[doc(hidden)]
pub mod platform;
/// Plugin and integration extension system.
pub mod plugins;
/// Runtime adapters, diagnostics, and observability.
pub mod runtime;
/// Security policies, approval, and credential handling.
pub mod security;
/// Gateway and channel transports.
pub mod transport;
/// User-interface helpers shared by CLI and desktop-facing surfaces.
pub mod ui;
/// Shared utilities used across subsystems.
pub(crate) mod utils;

/// Common CLI subcommand groups re-exported for consumers.
pub use cli::commands::{
    AuthCommands, ChannelCommands, ConfigCommands, CronCommands, IntegrationCommands,
    ServiceCommands, SkillCommands,
};
