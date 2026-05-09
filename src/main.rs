//! `Asterel` CLI entry point.
//!
//! Parses arguments, loads configuration, initializes tracing,
//! and dispatches to the appropriate subcommand handler.

#![warn(clippy::all, clippy::pedantic)]
#![deny(unsafe_code)]

#[macro_use]
extern crate rust_i18n;

i18n!("locales", fallback = "en");

use std::sync::Arc;

use anyhow::{Context, Result};
use asterel::cli::commands::Commands;
use clap::Parser;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::FmtSubscriber;

#[path = "cli/app/mod.rs"]
mod app;
use asterel::cli::commands::Cli;
use asterel::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    // Install default crypto provider for Rustls TLS.
    // This prevents the error: "could not automatically determine the process-level CryptoProvider"
    // when both aws-lc-rs and ring features are available (or neither is explicitly selected).
    if let Err(e) = rustls::crypto::ring::default_provider().install_default() {
        eprintln!("Warning: Failed to install default crypto provider: {e:?}");
    }

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let subscriber = FmtSubscriber::builder()
        .with_env_filter(env_filter)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("setting default subscriber failed")?;

    let cli = Cli::parse();
    let config = Arc::new(load_config_for_command(&cli.command)?);
    app::dispatch::dispatch(cli, config).await
}

fn load_config_for_command(command: &Commands) -> Result<Config> {
    match command {
        Commands::Doctor { .. } | Commands::Config { .. } => Config::load_or_init_unvalidated(),
        _ => Config::load_or_init(),
    }
}
