use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[path = "support/test_env.rs"]
mod test_env;

use tempfile::TempDir;

#[derive(Debug)]
struct CmdResult {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl CmdResult {
    fn combined(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

fn bin_path() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_asterel") {
        return PathBuf::from(path);
    }

    // Fallback for environments where cargo does not inject CARGO_BIN_EXE_*.
    let mut exe = std::env::current_exe().expect("resolve current test executable");
    exe.pop(); // test binary filename
    if exe.file_name().and_then(|s| s.to_str()) == Some("deps") {
        exe.pop(); // target/{profile}
    }
    let candidate = exe.join(if cfg!(windows) {
        "asterel.exe"
    } else {
        "asterel"
    });
    assert!(
        candidate.exists(),
        "could not locate asterel binary: {}",
        candidate.display()
    );
    candidate
}

fn run_cli(home: &Path, args: &[&str], stdin_text: Option<&str>) -> CmdResult {
    let mut command = Command::new(bin_path());
    command
        .args(args)
        .env("HOME", home)
        .stdin(if stdin_text.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().expect("spawn cli");
    if let Some(text) = stdin_text {
        use std::io::Write;
        let mut stdin = child.stdin.take().expect("stdin pipe");
        stdin.write_all(text.as_bytes()).expect("write stdin");
    }

    let output = child.wait_with_output().expect("wait output");
    CmdResult {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn assert_success(result: &CmdResult, context: &str) {
    assert_eq!(
        result.code,
        Some(0),
        "{context} should succeed, got code={:?}\nstdout:\n{}\nstderr:\n{}",
        result.code,
        result.stdout,
        result.stderr
    );
}

fn assert_failure(result: &CmdResult, context: &str) {
    assert_ne!(
        result.code,
        Some(0),
        "{context} should fail, got success\nstdout:\n{}\nstderr:\n{}",
        result.stdout,
        result.stderr
    );
}

fn assert_not_security_policy_block(result: &CmdResult, context: &str) {
    let combined = result.combined();
    assert!(
        !combined.contains("blocked by security policy"),
        "{context} should not be blocked by security policy\nstdout:\n{}\nstderr:\n{}",
        result.stdout,
        result.stderr
    );
    assert!(
        !combined.contains("not allowlisted"),
        "{context} should not fail due allowlist denial\nstdout:\n{}\nstderr:\n{}",
        result.stdout,
        result.stderr
    );
}

fn bootstrap_quick_setup(home: &Path) {
    let result = run_cli(
        home,
        &[
            "onboard",
            "--api-key",
            "sk-test",
            "--provider",
            "openai",
            "--memory",
            "none",
        ],
        None,
    );
    assert_success(&result, "onboard quick setup");
}

fn workspace_path(home: &Path) -> PathBuf {
    home.join(".asterel").join("workspace")
}

fn test_postgres_url() -> Option<String> {
    std::env::var("TEST_DATABASE_URL")
        .ok()
        .or_else(|| std::env::var("ASTEREL_POSTGRES_URL").ok())
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
}

fn find_uuid(text: &str) -> Option<String> {
    for token in text.split_whitespace() {
        let trimmed = token.trim_matches(|c: char| !c.is_ascii_hexdigit() && c != '-');
        if trimmed.len() == 36
            && trimmed.chars().enumerate().all(|(i, ch)| {
                if [8, 13, 18, 23].contains(&i) {
                    ch == '-'
                } else {
                    ch.is_ascii_hexdigit()
                }
            })
        {
            return Some(trimmed.to_string());
        }
    }
    None
}

fn is_missing_local_postgres_runtime(result: &CmdResult) -> bool {
    let combined = result.combined().to_ascii_lowercase();
    combined.contains("native setup failed")
        && combined.contains("initdb")
        && combined.contains("docker")
        && combined.contains("docker.sock")
}

#[test]
fn cli_onboard_normal_abnormal_boundary() {
    let temp = TempDir::new().expect("temp dir");
    let home = temp.path();

    let normal = run_cli(
        home,
        &[
            "onboard",
            "--api-key",
            "sk-test",
            "--provider",
            "openai",
            "--memory",
            "none",
        ],
        None,
    );
    assert_success(&normal, "onboard normal");
    assert!(
        normal.combined().contains("Quick Setup"),
        "onboard normal output should contain quick setup marker"
    );

    let abnormal = run_cli(home, &["onboard", "--interactive", "--channels-only"], None);
    assert_failure(&abnormal, "onboard abnormal");

    let boundary = run_cli(home, &["onboard"], None);
    if boundary.code == Some(0) {
        return;
    }
    assert!(
        is_missing_local_postgres_runtime(&boundary),
        "onboard boundary should succeed or fail only when local postgres runtime is unavailable.\ncode={:?}\nstdout:\n{}\nstderr:\n{}",
        boundary.code,
        boundary.stdout,
        boundary.stderr
    );
}

#[test]
fn cli_top_level_smoke_matrix() {
    let temp = TempDir::new().expect("temp dir");
    let home = temp.path();
    bootstrap_quick_setup(home);

    // agent (normal/help, abnormal: invalid value, boundary: temperature lower bound parsing)
    let agent_normal = run_cli(home, &["agent", "--help"], None);
    assert_success(&agent_normal, "agent normal");
    let agent_abnormal = run_cli(home, &["agent", "--temperature", "abc"], None);
    assert_failure(&agent_abnormal, "agent abnormal");
    let agent_boundary = run_cli(home, &["agent", "--temperature", "0", "--help"], None);
    assert_success(&agent_boundary, "agent boundary");

    // gateway (normal/help, abnormal: invalid port, boundary: public bind denied)
    let gateway_normal = run_cli(home, &["gateway", "--help"], None);
    assert_success(&gateway_normal, "gateway normal");
    let gateway_abnormal = run_cli(home, &["gateway", "--port", "70000"], None);
    assert_failure(&gateway_abnormal, "gateway abnormal");
    let gateway_boundary = run_cli(home, &["gateway", "--port", "65535", "--help"], None);
    assert_success(&gateway_boundary, "gateway boundary");

    // daemon (normal/help, abnormal: invalid port, boundary: public bind denied)
    let daemon_normal = run_cli(home, &["daemon", "--help"], None);
    assert_success(&daemon_normal, "daemon normal");
    let daemon_abnormal = run_cli(home, &["daemon", "--port", "70000"], None);
    assert_failure(&daemon_abnormal, "daemon abnormal");
    let daemon_boundary = run_cli(home, &["daemon", "--port", "65535", "--help"], None);
    assert_success(&daemon_boundary, "daemon boundary");

    // doctor
    let doctor_normal = run_cli(home, &["doctor"], None);
    assert_success(&doctor_normal, "doctor normal");
    let doctor_abnormal = run_cli(home, &["doctor", "--unknown"], None);
    assert_failure(&doctor_abnormal, "doctor abnormal");
    let doctor_boundary = run_cli(home, &["doctor", "--repair"], None);
    assert_success(&doctor_boundary, "doctor boundary");

    // config validate
    let config_normal = run_cli(home, &["config", "validate"], None);
    assert_success(&config_normal, "config normal");
    let config_abnormal = run_cli(home, &["config", "unknown"], None);
    assert_failure(&config_abnormal, "config abnormal");
    let config_boundary = run_cli(home, &["config", "validate", "--help"], None);
    assert_success(&config_boundary, "config boundary");

    // status
    let status_normal = run_cli(home, &["status"], None);
    assert_success(&status_normal, "status normal");
    let status_abnormal = run_cli(home, &["status", "--unknown"], None);
    assert_failure(&status_abnormal, "status abnormal");
    let status_boundary = run_cli(home, &["status", "--help"], None);
    assert_success(&status_boundary, "status boundary");

    // eval baseline
    let eval_normal = run_cli(
        home,
        &[
            "eval",
            "baseline",
            "--seed",
            "1",
            "--evidence-slug",
            "smoke",
        ],
        None,
    );
    assert_success(&eval_normal, "eval normal");
    let eval_abnormal = run_cli(home, &["eval", "baseline", "--seed", "NaN"], None);
    assert_failure(&eval_abnormal, "eval abnormal");
    let eval_boundary = run_cli(
        home,
        &[
            "eval",
            "baseline",
            "--seed",
            "0",
            "--evidence-slug",
            "../A/B C?*",
        ],
        None,
    );
    assert_success(&eval_boundary, "eval boundary");

    // model
    let model_normal = run_cli(
        home,
        &["model", "--set", "gpt-4o-mini", "--provider", "openai"],
        None,
    );
    assert_success(&model_normal, "model normal");
    let model_abnormal = run_cli(
        home,
        &["model", "--set", "gpt-4o-mini", "--provider", ""],
        None,
    );
    assert_failure(&model_abnormal, "model abnormal");
    let model_boundary = run_cli(home, &["model", "--set", "m"], None);
    assert_success(&model_boundary, "model boundary");
}

#[test]
fn cli_subcommand_smoke_matrix() {
    let temp = TempDir::new().expect("temp dir");
    let home = temp.path();
    bootstrap_quick_setup(home);
    let postgres_url = test_postgres_url();
    let _postgres_guard = postgres_url
        .as_deref()
        .map(|url| test_env::ScopedEnvVar::set("ASTEREL_POSTGRES_URL", url));

    // service subcommands
    let service_status = run_cli(home, &["service", "status"], None);
    assert_not_security_policy_block(&service_status, "service status");
    if service_status.code == Some(0) {
        assert_success(&service_status, "service status");
    } else {
        let combined = service_status.combined().to_ascii_lowercase();
        assert!(
            combined.contains("service manager unavailable") || combined.contains("systemctl"),
            "service status should only fail with explicit manager diagnostics, got code={:?}\nstdout:\n{}\nstderr:\n{}",
            service_status.code,
            service_status.stdout,
            service_status.stderr
        );
    }
    // service install — may fail on non-systemd (WSL2, containers)
    let service_install = run_cli(home, &["service", "install"], None);
    assert_not_security_policy_block(&service_install, "service install");
    if service_install.code != Some(0) {
        let combined = service_install.combined().to_ascii_lowercase();
        assert!(
            combined.contains("systemctl")
                || combined.contains("failed to enable")
                || combined.contains("does not exist"),
            "service install should succeed or fail only due to missing systemd, \
             got code={:?}\nstdout:\n{}\nstderr:\n{}",
            service_install.code,
            service_install.stdout,
            service_install.stderr
        );
    }
    let service_start = run_cli(home, &["service", "start"], None);
    assert_not_security_policy_block(&service_start, "service start");
    if service_start.code == Some(0) {
        assert_success(&service_start, "service start");
    } else {
        assert!(
            !service_start.combined().trim().is_empty(),
            "service start failure should include diagnostics"
        );
    }
    let service_stop = run_cli(home, &["service", "stop"], None);
    assert_not_security_policy_block(&service_stop, "service stop");
    if service_stop.code == Some(0) {
        assert_success(&service_stop, "service stop");
    } else {
        assert!(
            !service_stop.combined().trim().is_empty(),
            "service stop failure should include diagnostics"
        );
    }
    // service uninstall — may fail on non-systemd (WSL2, containers)
    let service_uninstall = run_cli(home, &["service", "uninstall"], None);
    assert_not_security_policy_block(&service_uninstall, "service uninstall");
    if service_uninstall.code != Some(0) {
        let combined = service_uninstall.combined().to_ascii_lowercase();
        assert!(
            combined.contains("systemctl")
                || combined.contains("failed to disable")
                || combined.contains("does not exist")
                || combined.contains("not loaded"),
            "service uninstall should succeed or fail only due to missing systemd, \
             got code={:?}\nstdout:\n{}\nstderr:\n{}",
            service_uninstall.code,
            service_uninstall.stdout,
            service_uninstall.stderr
        );
    }

    if postgres_url.is_some() {
        // cron subcommands: list/add/remove + error path and boundary expression
        assert_success(&run_cli(home, &["cron", "list"], None), "cron list");
        let cron_add = run_cli(home, &["cron", "add", "*/5 * * * *", "echo hi"], None);
        assert_success(&cron_add, "cron add");
        let job_id = find_uuid(&cron_add.combined()).expect("extract cron job id");
        assert_success(
            &run_cli(home, &["cron", "remove", &job_id], None),
            "cron remove",
        );
        assert_failure(
            &run_cli(home, &["cron", "add", "bad-expression", "echo hi"], None),
            "cron add invalid",
        );
        let legacy_cron_add = run_cli(
            home,
            &["cron", "add", "*/5 * * * *", "plan -m legacy"],
            None,
        );
        assert_failure(&legacy_cron_add, "cron add legacy planner executable");
        assert!(
            legacy_cron_add
                .combined()
                .contains("legacy planner cron commands are no longer accepted")
        );
        assert_success(
            &run_cli(home, &["cron", "add", "0 0 1 1 *", "echo boundary"], None),
            "cron boundary schedule",
        );
    }

    // channel subcommands
    assert_success(&run_cli(home, &["channel", "list"], None), "channel list");
    assert_success(&run_cli(home, &["channel", "start"], None), "channel start");
    assert_success(
        &run_cli(home, &["channel", "doctor"], None),
        "channel doctor",
    );
    assert_failure(
        &run_cli(home, &["channel", "add", "telegram", "{}"], None),
        "channel add",
    );
    assert_failure(
        &run_cli(home, &["channel", "remove", "telegram"], None),
        "channel remove",
    );

    // integrations subcommands
    assert_success(
        &run_cli(home, &["integrations", "info", "cron"], None),
        "integrations info normal",
    );
    assert_failure(
        &run_cli(home, &["integrations", "info", "not-existing"], None),
        "integrations info abnormal",
    );
    assert_success(
        &run_cli(home, &["integrations", "info", "CrOn"], None),
        "integrations info boundary",
    );

    // auth subcommands
    assert_success(&run_cli(home, &["auth", "list"], None), "auth list");
    assert_success(
        &run_cli(
            home,
            &[
                "auth",
                "login",
                "--provider",
                "openai",
                "--api-key",
                "sk-auth-test",
            ],
            None,
        ),
        "auth login",
    );
    assert_failure(
        &run_cli(
            home,
            &[
                "auth",
                "login",
                "--provider",
                "",
                "--api-key",
                "sk-auth-test",
            ],
            None,
        ),
        "auth login invalid provider",
    );
    let auth_status = run_cli(home, &["auth", "status", "--provider", "openai"], None);
    assert_success(&auth_status, "auth status");
    let auth_status_output = auth_status.combined();
    assert!(auth_status_output.contains("Default mapping"));
    assert!(auth_status_output.contains("openai-default"));
    assert!(!auth_status_output.contains("Default mapping        (none)"));

    let oauth_status = run_cli(
        home,
        &["auth", "oauth-status", "--provider", "openai"],
        None,
    );
    assert_success(&oauth_status, "auth oauth-status");
    assert!(
        !oauth_status
            .combined()
            .to_ascii_lowercase()
            .contains("not allowlisted")
    );
    let setup_token = format!("sk-ant-oat01-{}", "a".repeat(80));
    assert_success(
        &run_cli(
            home,
            &[
                "auth",
                "oauth-login",
                "--provider",
                "claude",
                "--setup-token",
                &setup_token,
                "--no-default",
            ],
            None,
        ),
        "auth oauth-login",
    );

    // skills subcommands
    assert_success(&run_cli(home, &["skills", "list"], None), "skills list");
    let skill_src = workspace_path(home).join("skilldemo");
    fs::create_dir_all(&skill_src).expect("create skill src");
    fs::write(skill_src.join("SKILL.md"), "# Skill demo").expect("write skill md");
    assert_success(
        &run_cli(
            home,
            &["skills", "install", skill_src.to_string_lossy().as_ref()],
            None,
        ),
        "skills install",
    );
    assert_success(
        &run_cli(home, &["skills", "remove", "skilldemo"], None),
        "skills remove",
    );
    assert_failure(
        &run_cli(home, &["skills", "remove", "../escape"], None),
        "skills remove invalid name",
    );
}
