//! Nested CLI subcommand enums.
//!
//! Defines `ServiceCommands`, `ChannelCommands`, `CronCommands`,
//! `AuthCommands`, `SkillCommands`, and other clap subcommand groups.

use clap::Subcommand;
use serde::{Deserialize, Serialize};

/// Service management subcommands
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ServiceCommands {
    /// Install daemon service unit for auto-start and restart
    Install,
    /// Start daemon service
    Start,
    /// Stop daemon service
    Stop,
    /// Check daemon service status
    Status,
    /// Uninstall daemon service unit
    Uninstall,
}

/// Channel management subcommands
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChannelCommands {
    /// List all configured channels
    List,
    /// Start all configured channels (handled in main.rs for async)
    Start,
    /// Run health checks for configured channels (handled in main.rs for async)
    Doctor,
}

/// Config management subcommands
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ConfigCommands {
    /// Validate the effective configuration and print the result
    Validate,
}

/// Skills management subcommands
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SkillCommands {
    /// List all installed skills
    List,
    /// Install a new skill from a URL or local path
    Install {
        /// Source URL or local path
        source: String,
    },
    /// Remove an installed skill
    Remove {
        /// Skill name to remove
        name: String,
    },
}

/// Cron subcommands
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CronCommands {
    /// List all scheduled tasks
    List,
    /// Add a new scheduled task
    Add {
        /// Cron expression
        expression: String,
        /// Command to run
        command: String,
    },
    /// Remove a scheduled task
    Remove {
        /// Task ID
        id: String,
    },
}

/// Auth profile subcommands
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuthCommands {
    /// List configured auth profiles
    List,
    /// Show auth status for a provider
    Status {
        /// Provider to inspect (defaults to configured default provider)
        #[arg(short, long)]
        provider: Option<String>,
    },
    /// Save or update an API-key auth profile
    Login {
        /// Provider name (e.g. openrouter, openai, anthropic)
        #[arg(short, long)]
        provider: String,
        /// Profile id (defaults to <provider>-default)
        #[arg(long)]
        profile: Option<String>,
        /// Human label for the profile
        #[arg(long)]
        label: Option<String>,
        /// API key to store (if omitted, prompt securely)
        #[arg(long)]
        api_key: Option<String>,
        /// Do not set this profile as provider default
        #[arg(long)]
        no_default: bool,
    },
    /// Login using OAuth via provider CLI and store imported token profile
    #[command(name = "oauth-login")]
    OAuthLogin {
        /// OAuth source/provider (openai or claude/anthropic)
        #[arg(short, long)]
        provider: String,
        /// Profile id (defaults to <provider>-oauth-default)
        #[arg(long)]
        profile: Option<String>,
        /// Human label for the profile
        #[arg(long)]
        label: Option<String>,
        /// Do not set this profile as provider default
        #[arg(long)]
        no_default: bool,
        /// Skip launching provider login CLI and import from local credentials only
        #[arg(long)]
        skip_cli_login: bool,
        /// Claude setup token (sk-ant-oat01-...), if already obtained
        #[arg(long)]
        setup_token: Option<String>,
    },
    /// Show OAuth source health (openai/claude)
    #[command(name = "oauth-status")]
    OAuthStatus {
        /// OAuth source/provider to inspect (openai or claude)
        #[arg(short, long)]
        provider: Option<String>,
    },
}

/// Integration subcommands
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum IntegrationCommands {
    /// Show details about a specific integration
    Info {
        /// Integration name
        name: String,
    },
}

/// Eval subcommands
#[derive(Subcommand, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EvalCommands {
    /// Run synthetic deterministic baseline suites
    Baseline {
        /// Deterministic RNG seed
        #[arg(long, default_value_t = 42)]
        seed: u64,

        /// Optional evidence slug (writes workspace `evidence/` files when set)
        #[arg(long)]
        evidence_slug: Option<String>,
    },

    /// Run replay evaluation from a JSONL trace file
    Replay {
        /// Path to the JSONL replay trace file
        #[arg(long)]
        input: String,

        /// Suite name for the replay evaluation
        #[arg(long)]
        suite: String,

        /// Optional evidence slug (writes workspace `evidence/` files when set)
        #[arg(long)]
        evidence_slug: Option<String>,
    },

    /// Run memory quality benchmark
    MemoryBench {
        /// Path to memory bench config JSON
        #[arg(long)]
        config: String,

        /// Optional evidence slug
        #[arg(long)]
        evidence_slug: Option<String>,
    },

    /// Run companion harness OFF/ON ablation over public-safe synthetic fixtures
    Harness {
        /// Path to a JSONL fixture file or directory of JSONL fixtures
        #[arg(long)]
        fixtures: String,

        /// Generate draft responses from the configured model before running OFF/ON scoring
        #[arg(long)]
        model_backed: bool,

        /// Provider override for --model-backed runs
        #[arg(long)]
        provider: Option<String>,

        /// Model override for --model-backed runs
        #[arg(long)]
        model: Option<String>,

        /// Temperature for --model-backed draft generation
        #[arg(long, default_value = "0.4")]
        temperature: String,

        /// Optional output path for the JSON report
        #[arg(long)]
        output: Option<String>,

        /// Optional evidence slug (writes workspace `evidence/` files when set)
        #[arg(long)]
        evidence_slug: Option<String>,
    },
}
