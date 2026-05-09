use std::sync::Arc;

use asterel::cli::commands::{Cli, Commands, ConfigCommands, EvalCommands};
use asterel::config::Config;
use clap::Parser;
use tempfile::TempDir;

use super::dispatch;

#[tokio::test]
async fn dispatch_eval_with_evidence_writes_baseline_files() {
    let tmp = TempDir::new().expect("temp dir");
    let config = Config {
        workspace_dir: tmp.path().to_path_buf(),
        ..Config::default()
    };

    let cli = Cli {
        command: Commands::Eval {
            eval_command: EvalCommands::Baseline {
                seed: 123,
                evidence_slug: Some("dispatch-eval".to_string()),
            },
        },
    };

    dispatch(cli, Arc::new(config))
        .await
        .expect("eval dispatch should succeed");

    let evidence_dir = tmp.path().join("evidence");
    assert!(
        evidence_dir.join("dispatch-eval.txt").exists(),
        "text evidence should be written"
    );
    assert!(
        evidence_dir
            .join("dispatch-eval-baseline-report.csv")
            .exists(),
        "csv evidence should be written"
    );
    assert!(
        evidence_dir
            .join("dispatch-eval-baseline-report.json")
            .exists(),
        "json evidence should be written"
    );
}

#[tokio::test]
async fn dispatch_eval_with_unsafe_slug_writes_sanitized_paths() {
    let tmp = TempDir::new().expect("temp dir");
    let config = Config {
        workspace_dir: tmp.path().to_path_buf(),
        ..Config::default()
    };

    let cli = Cli {
        command: Commands::Eval {
            eval_command: EvalCommands::Baseline {
                seed: 456,
                evidence_slug: Some(" ../A/B C?* ".to_string()),
            },
        },
    };

    dispatch(cli, Arc::new(config))
        .await
        .expect("eval dispatch should succeed with unsafe slug");

    let evidence_dir = tmp.path().join("evidence");
    assert!(
        evidence_dir.join("a-b-c.txt").exists(),
        "sanitized text evidence should be written"
    );
    assert!(
        evidence_dir.join("a-b-c-baseline-report.csv").exists(),
        "sanitized csv evidence should be written"
    );
    assert!(
        evidence_dir.join("a-b-c-baseline-report.json").exists(),
        "sanitized json evidence should be written"
    );
}

#[tokio::test]
async fn dispatch_eval_with_blank_slug_falls_back_to_default_slug() {
    let tmp = TempDir::new().expect("temp dir");
    let config = Config {
        workspace_dir: tmp.path().to_path_buf(),
        ..Config::default()
    };

    let cli = Cli {
        command: Commands::Eval {
            eval_command: EvalCommands::Baseline {
                seed: 789,
                evidence_slug: Some("   ".to_string()),
            },
        },
    };

    dispatch(cli, Arc::new(config))
        .await
        .expect("eval dispatch should succeed with blank slug");

    let evidence_dir = tmp.path().join("evidence");
    assert!(
        evidence_dir.join("eval.txt").exists(),
        "default slug text evidence should be written"
    );
    assert!(
        evidence_dir.join("eval-baseline-report.csv").exists(),
        "default slug csv evidence should be written"
    );
    assert!(
        evidence_dir.join("eval-baseline-report.json").exists(),
        "default slug json evidence should be written"
    );
}

#[tokio::test]
async fn dispatch_status_command_succeeds() {
    let cli = Cli {
        command: Commands::Status,
    };

    dispatch(cli, Arc::new(Config::default()))
        .await
        .expect("status dispatch should succeed");
}

#[tokio::test]
async fn dispatch_gateway_rejects_runtime_boot_without_onboarding() {
    let cli = Cli {
        command: Commands::Gateway {
            port: 3000,
            host: "127.0.0.1".to_string(),
        },
    };

    let error = dispatch(cli, Arc::new(Config::default()))
        .await
        .expect_err("gateway should require explicit onboarding");
    assert!(
        error
            .to_string()
            .contains("Run `asterel onboard` before starting `gateway`")
    );
}

#[tokio::test]
async fn dispatch_agent_rejects_runtime_boot_without_onboarding() {
    let cli = Cli {
        command: Commands::Agent {
            message: Some("hello".to_string()),
            provider: None,
            model: None,
            temperature: 0.7,
        },
    };

    let error = dispatch(cli, Arc::new(Config::default()))
        .await
        .expect_err("agent should require explicit onboarding");
    assert!(
        error
            .to_string()
            .contains("Run `asterel onboard` before starting `agent`")
    );
}

#[tokio::test]
async fn dispatch_daemon_rejects_runtime_boot_without_onboarding() {
    let cli = Cli {
        command: Commands::Daemon {
            port: 3000,
            host: "127.0.0.1".to_string(),
        },
    };

    let error = dispatch(cli, Arc::new(Config::default()))
        .await
        .expect_err("daemon should require explicit onboarding");
    assert!(
        error
            .to_string()
            .contains("Run `asterel onboard` before starting `daemon`")
    );
}

#[tokio::test]
async fn dispatch_agent_allows_provider_override_when_auth_profile_exists() {
    let tmp = TempDir::new().expect("tempdir");
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };

    dispatch(
        Cli {
            command: Commands::Auth {
                auth_command: asterel::AuthCommands::Login {
                    provider: "openai".to_string(),
                    profile: Some("openai-default".to_string()),
                    label: Some("OpenAI".to_string()),
                    api_key: Some("sk-test".to_string()),
                    no_default: false,
                },
            },
        },
        Arc::new(config.clone()),
    )
    .await
    .expect("auth login should seed the profile store");

    let cli = Cli {
        command: Commands::Agent {
            message: Some("hello".to_string()),
            provider: Some("openai".to_string()),
            model: Some("gpt-test".to_string()),
            temperature: 0.7,
        },
    };

    let error = dispatch(cli, Arc::new(config))
        .await
        .expect_err("provider override should pass onboarding gate but fail provider auth");
    assert!(!error.to_string().contains("Run `asterel onboard`"));
}

#[tokio::test]
async fn dispatch_model_command_rejects_blank_provider() {
    let tmp = TempDir::new().expect("temp dir");
    let config = Config {
        workspace_dir: tmp.path().to_path_buf(),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    let cli = Cli {
        command: Commands::Model {
            set: "gpt-5.2".to_string(),
            provider: Some("   ".to_string()),
        },
    };

    let error = dispatch(cli, Arc::new(config))
        .await
        .expect_err("model command should fail when provider is blank");
    assert!(error.to_string().contains("--provider cannot be empty"));
}

#[tokio::test]
async fn dispatch_config_validate_command_succeeds() {
    let cli = Cli {
        command: Commands::Config {
            config_command: ConfigCommands::Validate,
        },
    };

    dispatch(cli, Arc::new(Config::default()))
        .await
        .expect("config validate dispatch should succeed");
}

#[tokio::test]
async fn dispatch_onboard_rejects_interactive_and_channels_only_combination() {
    let cli = Cli {
        command: Commands::Onboard {
            interactive: true,
            channels_only: true,
            api_key: None,
            provider: None,
            memory: None,
            postgres_setup: None,
            install_daemon: false,
        },
    };

    let error = dispatch(cli, Arc::new(Config::default()))
        .await
        .expect_err("onboard should reject interactive with channels-only");
    assert!(
        error
            .to_string()
            .contains("Use either --interactive or --channels-only")
    );
}

#[tokio::test]
async fn dispatch_onboard_rejects_channels_only_with_provider_flags() {
    let cli = Cli {
        command: Commands::Onboard {
            interactive: false,
            channels_only: true,
            api_key: Some("sk-test".to_string()),
            provider: None,
            memory: None,
            postgres_setup: None,
            install_daemon: false,
        },
    };

    let error = dispatch(cli, Arc::new(Config::default()))
        .await
        .expect_err("onboard should reject provider flags in channels-only mode");
    assert!(
        error
            .to_string()
            .contains("--channels-only does not accept --api-key")
    );
}

#[test]
fn dispatch_path_reports_unknown_command_parse_error() {
    let err = Cli::try_parse_from(["asterel", "unknown-command"])
        .expect_err("unknown command should fail before dispatch");
    assert!(err.to_string().contains("unrecognized subcommand"));
}
