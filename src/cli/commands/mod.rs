//! CLI argument definitions and re-exports.
//!
//! Defines the top-level `Cli` struct, `Commands` enum, and
//! nested subcommand groups.

use clap::{Parser, Subcommand};

mod subcommands;

/// Re-exports conversation command handlers from the canonical location in `core`.
pub mod handlers {
    pub use crate::core::conversation_commands::handle_command;
}

/// Re-exports the conversation command parser from the canonical location in `core`.
pub mod parser {
    pub use crate::core::conversation_commands::parse_command;
}

/// Re-exports conversation command types from the canonical location in `core`.
pub mod types {
    pub use crate::core::conversation_commands::{Command, CommandResult};
}

// Conversation command parsing/execution lives in `core::conversation_commands`.
pub use subcommands::{
    AuthCommands, ChannelCommands, ConfigCommands, CronCommands, EvalCommands, IntegrationCommands,
    ServiceCommands, SkillCommands,
};

/// `Asterel` - Secure, extensible AI assistant built in Rust.
#[derive(Parser, Debug)]
#[command(name = "asterel")]
#[command(author = "Asterel Contributors")]
#[command(version = "0.1.0")]
#[command(about = "A secure, extensible AI assistant.", long_about = None)]
pub struct Cli {
    /// The subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,
}

/// Top-level CLI subcommands for `Asterel`.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialize your workspace and configuration
    Onboard {
        /// Run the full interactive wizard (default is quick setup)
        #[arg(long)]
        interactive: bool,

        /// Reconfigure channels only (fast repair flow)
        #[arg(long)]
        channels_only: bool,

        /// API key (used in quick mode, ignored with `--interactive`)
        #[arg(long)]
        api_key: Option<String>,

        /// Provider name (used in quick mode, default: `openrouter`)
        #[arg(long)]
        provider: Option<String>,

        /// Memory backend (`postgres`, `markdown`, `none`) used in quick mode, default: `postgres`
        #[arg(long)]
        memory: Option<String>,

        /// Postgres setup mode (`auto`, `native`, `docker`) used with `--memory postgres`
        #[arg(long)]
        postgres_setup: Option<String>,

        /// Also install the daemon as an OS service (`launchd`/`systemd`)
        #[arg(long)]
        install_daemon: bool,
    },

    /// Start the AI agent loop
    Agent {
        /// Single message mode (don't enter interactive mode)
        #[arg(short, long)]
        message: Option<String>,

        /// Provider to use (openrouter, anthropic, openai)
        #[arg(short, long)]
        provider: Option<String>,

        /// Model to use
        #[arg(long)]
        model: Option<String>,

        /// Temperature (0.0 - 2.0)
        #[arg(short, long, default_value = "0.7")]
        temperature: f64,
    },

    /// Start the gateway server (`webhooks`, `websockets`)
    Gateway {
        /// Port to listen on (use 0 for random available port)
        #[arg(short, long, default_value = "3000")]
        port: u16,

        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },

    /// Start long-running autonomous runtime (gateway + channels + heartbeat + scheduler)
    Daemon {
        /// Port to listen on (use 0 for random available port)
        #[arg(short, long, default_value = "3000")]
        port: u16,

        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },

    /// Manage OS service lifecycle (`launchd`/`systemd` user service)
    Service {
        #[command(subcommand)]
        service_command: ServiceCommands,
    },

    /// Run diagnostics for `daemon`/`scheduler`/`channel` freshness
    Doctor {
        /// Apply safe local repairs before running diagnostics
        #[arg(long)]
        repair: bool,
    },

    /// Validate the current configuration
    Config {
        #[command(subcommand)]
        config_command: ConfigCommands,
    },

    /// Show system status (full details)
    Status,

    /// Run evaluation suites (baseline synthetic or replay from traces)
    Eval {
        #[command(subcommand)]
        eval_command: EvalCommands,
    },

    /// Update the default model and optional provider
    Model {
        /// Model identifier to set as default
        #[arg(long)]
        set: String,

        /// Provider to associate with the model
        #[arg(long)]
        provider: Option<String>,
    },

    /// Configure and manage scheduled tasks
    Cron {
        #[command(subcommand)]
        cron_command: CronCommands,
    },

    /// Manage configured channels and channel health
    Channel {
        #[command(subcommand)]
        channel_command: ChannelCommands,
    },

    /// Browse the integration catalog (runnable and planned)
    Integrations {
        #[command(subcommand)]
        integration_command: IntegrationCommands,
    },

    /// Manage auth profiles and credentials
    Auth {
        #[command(subcommand)]
        auth_command: AuthCommands,
    },

    /// Manage skills (`user-defined` capabilities)
    Skills {
        #[command(subcommand)]
        skill_command: SkillCommands,
    },
}

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;
    use clap::{CommandFactory, Parser};

    use super::{
        AuthCommands, ChannelCommands, Cli, Commands, ConfigCommands, CronCommands, EvalCommands,
        IntegrationCommands, ServiceCommands, SkillCommands,
    };

    const fn command_tag(command: &Commands) -> &'static str {
        match command {
            Commands::Onboard { .. } => "onboard",
            Commands::Agent { .. } => "agent",
            Commands::Gateway { .. } => "gateway",
            Commands::Daemon { .. } => "daemon",
            Commands::Service { .. } => "service",
            Commands::Doctor { .. } => "doctor",
            Commands::Config { .. } => "config",
            Commands::Status => "status",
            Commands::Eval { .. } => "eval",
            Commands::Model { .. } => "model",
            Commands::Cron { .. } => "cron",
            Commands::Channel { .. } => "channel",
            Commands::Integrations { .. } => "integrations",
            Commands::Auth { .. } => "auth",
            Commands::Skills { .. } => "skills",
        }
    }

    #[test]
    fn cli_definition_has_no_flag_conflicts() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parse_eval_baseline_command_with_seed_and_slug() {
        let cli = Cli::parse_from([
            "asterel",
            "eval",
            "baseline",
            "--seed",
            "99",
            "--evidence-slug",
            "baseline",
        ]);

        match cli.command {
            Commands::Eval { eval_command } => match eval_command {
                EvalCommands::Baseline {
                    seed,
                    evidence_slug,
                } => {
                    assert_eq!(seed, 99);
                    assert_eq!(evidence_slug.as_deref(), Some("baseline"));
                }
                other => panic!("expected baseline subcommand, got {other:?}"),
            },
            other => panic!("expected eval command, got {other:?}"),
        }
    }

    #[test]
    fn parse_eval_replay_command() {
        let cli = Cli::parse_from([
            "asterel",
            "eval",
            "replay",
            "--input",
            "trace.jsonl",
            "--suite",
            "nightly",
        ]);

        match cli.command {
            Commands::Eval { eval_command } => match eval_command {
                EvalCommands::Replay {
                    input,
                    suite,
                    evidence_slug,
                } => {
                    assert_eq!(input, "trace.jsonl");
                    assert_eq!(suite, "nightly");
                    assert!(evidence_slug.is_none());
                }
                other => panic!("expected replay subcommand, got {other:?}"),
            },
            other => panic!("expected eval command, got {other:?}"),
        }
    }

    #[test]
    fn parse_doctor_repair_command() {
        let cli = Cli::parse_from(["asterel", "doctor", "--repair"]);
        match cli.command {
            Commands::Doctor { repair } => assert!(repair),
            other => panic!("expected doctor command, got {other:?}"),
        }
    }

    #[test]
    fn parse_config_validate_command() {
        let cli = Cli::parse_from(["asterel", "config", "validate"]);
        match cli.command {
            Commands::Config { config_command } => {
                assert_eq!(config_command, ConfigCommands::Validate);
            }
            other => panic!("expected config command, got {other:?}"),
        }
    }

    #[test]
    fn parse_all_top_level_commands_table_driven() {
        let cases = [
            (["asterel", "onboard"].as_slice(), "onboard"),
            (["asterel", "agent"].as_slice(), "agent"),
            (["asterel", "gateway"].as_slice(), "gateway"),
            (["asterel", "daemon"].as_slice(), "daemon"),
            (["asterel", "service", "status"].as_slice(), "service"),
            (["asterel", "doctor"].as_slice(), "doctor"),
            (["asterel", "config", "validate"].as_slice(), "config"),
            (["asterel", "status"].as_slice(), "status"),
            (["asterel", "eval", "baseline"].as_slice(), "eval"),
            (["asterel", "model", "--set", "gpt-4o"].as_slice(), "model"),
            (["asterel", "cron", "list"].as_slice(), "cron"),
            (["asterel", "channel", "list"].as_slice(), "channel"),
            (
                ["asterel", "integrations", "info", "slack"].as_slice(),
                "integrations",
            ),
            (["asterel", "auth", "list"].as_slice(), "auth"),
            (["asterel", "skills", "list"].as_slice(), "skills"),
        ];

        for (args, expected) in cases {
            let cli = Cli::try_parse_from(args)
                .unwrap_or_else(|err| panic!("failed to parse {args:?}: {err}"));
            assert_eq!(command_tag(&cli.command), expected, "args: {args:?}");
        }
    }

    #[test]
    fn parse_onboard_flag_matrix_table_driven() {
        let cases = [
            (
                [
                    "asterel",
                    "onboard",
                    "--interactive",
                    "--channels-only",
                    "--api-key",
                    "k",
                    "--provider",
                    "openai",
                    "--memory",
                    "markdown",
                    "--postgres-setup",
                    "native",
                    "--install-daemon",
                ]
                .as_slice(),
                (
                    true,
                    true,
                    Some("k"),
                    Some("openai"),
                    Some("markdown"),
                    Some("native"),
                    true,
                ),
            ),
            (
                ["asterel", "onboard"].as_slice(),
                (false, false, None, None, None, None, false),
            ),
        ];

        for (args, expected) in cases {
            let cli = Cli::try_parse_from(args)
                .unwrap_or_else(|err| panic!("failed to parse {args:?}: {err}"));
            match cli.command {
                Commands::Onboard {
                    interactive,
                    channels_only,
                    api_key,
                    provider,
                    memory,
                    postgres_setup,
                    install_daemon,
                } => {
                    assert_eq!(interactive, expected.0);
                    assert_eq!(channels_only, expected.1);
                    assert_eq!(api_key.as_deref(), expected.2);
                    assert_eq!(provider.as_deref(), expected.3);
                    assert_eq!(memory.as_deref(), expected.4);
                    assert_eq!(postgres_setup.as_deref(), expected.5);
                    assert_eq!(install_daemon, expected.6);
                }
                other => panic!("expected onboard command, got {other:?}"),
            }
        }
    }

    #[test]
    fn parse_agent_matrix_table_driven() {
        let cases = [
            (
                [
                    "asterel",
                    "agent",
                    "--message",
                    "hello",
                    "--provider",
                    "openrouter",
                    "--model",
                    "gpt-4o-mini",
                    "--temperature",
                    "1.2",
                ]
                .as_slice(),
                (Some("hello"), Some("openrouter"), Some("gpt-4o-mini"), 1.2),
            ),
            (["asterel", "agent"].as_slice(), (None, None, None, 0.7)),
        ];

        for (args, expected) in cases {
            let cli = Cli::try_parse_from(args)
                .unwrap_or_else(|err| panic!("failed to parse {args:?}: {err}"));
            match cli.command {
                Commands::Agent {
                    message,
                    provider,
                    model,
                    temperature,
                } => {
                    assert_eq!(message.as_deref(), expected.0);
                    assert_eq!(provider.as_deref(), expected.1);
                    assert_eq!(model.as_deref(), expected.2);
                    assert!((temperature - expected.3).abs() < f64::EPSILON);
                }
                other => panic!("expected agent command, got {other:?}"),
            }
        }
    }

    #[test]
    fn parse_gateway_and_daemon_matrix_table_driven() {
        let cases = [
            (
                ["asterel", "gateway", "--port", "9090", "--host", "0.0.0.0"].as_slice(),
                "gateway",
                9090_u16,
                "0.0.0.0",
            ),
            (
                ["asterel", "gateway"].as_slice(),
                "gateway",
                3000_u16,
                "127.0.0.1",
            ),
            (
                ["asterel", "daemon", "--port", "7000", "--host", "127.0.0.2"].as_slice(),
                "daemon",
                7000_u16,
                "127.0.0.2",
            ),
            (
                ["asterel", "daemon"].as_slice(),
                "daemon",
                3000_u16,
                "127.0.0.1",
            ),
        ];

        for (args, expected_kind, expected_port, expected_host) in cases {
            let cli = Cli::try_parse_from(args)
                .unwrap_or_else(|err| panic!("failed to parse {args:?}: {err}"));
            match cli.command {
                Commands::Gateway { port, host } => {
                    assert_eq!(expected_kind, "gateway");
                    assert_eq!(port, expected_port);
                    assert_eq!(host, expected_host);
                }
                Commands::Daemon { port, host } => {
                    assert_eq!(expected_kind, "daemon");
                    assert_eq!(port, expected_port);
                    assert_eq!(host, expected_host);
                }
                other => panic!("expected gateway/daemon command, got {other:?}"),
            }
        }
    }

    #[test]
    fn parse_eval_matrix_table_driven() {
        type EvalEvidenceSeed<'a> = Option<(u64, Option<&'a str>)>;

        let cases: Vec<(&[&str], EvalEvidenceSeed<'_>)> = vec![
            (
                [
                    "asterel",
                    "eval",
                    "baseline",
                    "--seed",
                    "7",
                    "--evidence-slug",
                    "nightly",
                ]
                .as_slice(),
                Some((7_u64, Some("nightly"))),
            ),
            (
                ["asterel", "eval", "baseline"].as_slice(),
                Some((42_u64, None)),
            ),
        ];

        for (args, eval_expected) in cases {
            let cli = Cli::try_parse_from(args)
                .unwrap_or_else(|err| panic!("failed to parse {args:?}: {err}"));
            match (cli.command, eval_expected) {
                (
                    Commands::Eval {
                        eval_command:
                            EvalCommands::Baseline {
                                seed,
                                evidence_slug,
                            },
                    },
                    Some((expected_seed, expected_slug)),
                ) => {
                    assert_eq!(seed, expected_seed);
                    assert_eq!(evidence_slug.as_deref(), expected_slug);
                }
                (other, _) => panic!("unexpected command shape for {args:?}: {other:?}"),
            }
        }
    }

    #[test]
    fn parse_nested_subcommands_table_driven() {
        let cases = [
            (["asterel", "service", "start"].as_slice(), "service:start"),
            (
                ["asterel", "cron", "remove", "job1"].as_slice(),
                "cron:remove",
            ),
            (["asterel", "channel", "list"].as_slice(), "channel:list"),
            (
                ["asterel", "integrations", "info", "discord"].as_slice(),
                "integrations:info",
            ),
            (
                ["asterel", "auth", "status", "--provider", "openai"].as_slice(),
                "auth:status",
            ),
            (
                ["asterel", "skills", "install", "https://example.com/skill"].as_slice(),
                "skills:install",
            ),
        ];

        for (args, expected) in cases {
            let cli = Cli::try_parse_from(args)
                .unwrap_or_else(|err| panic!("failed to parse {args:?}: {err}"));
            let actual = match cli.command {
                Commands::Service { service_command } => match service_command {
                    ServiceCommands::Start => "service:start",
                    _ => "service:other",
                },
                Commands::Cron { cron_command } => match cron_command {
                    CronCommands::Remove { .. } => "cron:remove",
                    _ => "cron:other",
                },
                Commands::Channel { channel_command } => match channel_command {
                    ChannelCommands::List => "channel:list",
                    _ => "channel:other",
                },
                Commands::Integrations {
                    integration_command,
                } => match integration_command {
                    IntegrationCommands::Info { .. } => "integrations:info",
                },
                Commands::Auth { auth_command } => match auth_command {
                    AuthCommands::Status { .. } => "auth:status",
                    _ => "auth:other",
                },
                Commands::Skills { skill_command } => match skill_command {
                    SkillCommands::Install { .. } => "skills:install",
                    _ => "skills:other",
                },
                _ => "unexpected",
            };

            assert_eq!(actual, expected, "args: {args:?}");
        }
    }

    #[test]
    fn parse_unknown_commands_table_driven() {
        let cases = [
            ["asterel", "unknown"].as_slice(),
            ["asterel", "banana"].as_slice(),
            ["asterel", "modelz"].as_slice(),
            ["asterel", "/status"].as_slice(),
        ];

        for args in cases {
            let err = Cli::try_parse_from(args)
                .err()
                .unwrap_or_else(|| panic!("expected parse failure for {args:?}"));
            assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
        }
    }

    #[test]
    fn parse_partial_command_names_table_driven() {
        let cases = [
            ["asterel", "sta"].as_slice(),
            ["asterel", "doct"].as_slice(),
            ["asterel", "integ"].as_slice(),
            ["asterel", "cha"].as_slice(),
        ];

        for args in cases {
            let err = Cli::try_parse_from(args)
                .err()
                .unwrap_or_else(|| panic!("expected parse failure for {args:?}"));
            assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
        }
    }

    #[test]
    fn parse_edge_case_empty_and_whitespace_inputs_table_driven() {
        let cases = [
            Vec::<&str>::new(),
            vec!["asterel"],
            vec!["asterel", ""],
            vec!["asterel", "   "],
        ];

        for args in cases {
            let err = Cli::try_parse_from(args)
                .err()
                .unwrap_or_else(|| panic!("expected parse failure"));
            assert!(
                matches!(
                    err.kind(),
                    ErrorKind::InvalidSubcommand
                        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                        | ErrorKind::MissingRequiredArgument
                ),
                "unexpected error kind: {:?}",
                err.kind()
            );
        }
    }

    #[test]
    fn parse_missing_required_values_table_driven() {
        let cases = [
            ["asterel", "model"].as_slice(),
            ["asterel", "model", "--set"].as_slice(),
            ["asterel", "cron", "add", "* * * * *"].as_slice(),
            ["asterel", "auth", "login", "--provider"].as_slice(),
        ];

        for args in cases {
            let err = Cli::try_parse_from(args)
                .err()
                .unwrap_or_else(|| panic!("expected parse failure for {args:?}"));
            assert!(
                matches!(
                    err.kind(),
                    ErrorKind::MissingRequiredArgument | ErrorKind::InvalidValue
                ),
                "unexpected error kind for {args:?}: {:?}",
                err.kind()
            );
        }
    }

    #[test]
    fn parse_channel_add_now_fails_as_unknown_subcommand() {
        let err = Cli::try_parse_from(["asterel", "channel", "add", "telegram", "{}"])
            .err()
            .unwrap_or_else(|| panic!("expected parse failure for deleted channel add surface"));
        assert_eq!(err.kind(), ErrorKind::InvalidSubcommand);
    }
}
