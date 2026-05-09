//! Tests for OS service management, policy enforcement,
//! and XML escaping utilities.

use std::process::Command;

use super::platform::linux_service_file;
use super::spawn::{
    run_capture, run_checked, run_program_capture, run_program_checked, xml_escape,
};
use crate::config::Config;
use crate::security::{ProcessSpawnClass, SecurityPolicy};

fn service_test_policy() -> SecurityPolicy {
    SecurityPolicy {
        allowed_commands: vec!["echo".to_string(), "ls".to_string(), "false".to_string()],
        ..SecurityPolicy::default()
    }
}

#[test]
fn xml_escape_escapes_reserved_chars() {
    let escaped = xml_escape("<&>\"' and text");
    assert_eq!(escaped, "&lt;&amp;&gt;&quot;&apos; and text");
}

#[test]
fn run_capture_reads_stdout() {
    let out = run_capture(
        &service_test_policy(),
        "service_test_capture_stdout",
        ProcessSpawnClass::OperatorPlane,
        Command::new("echo").arg("hello"),
    )
    .expect("stdout capture should succeed");
    assert_eq!(out.trim(), "hello");
}

#[test]
fn run_capture_falls_back_to_stderr() {
    let out = run_capture(
        &service_test_policy(),
        "service_test_capture_stderr",
        ProcessSpawnClass::OperatorPlane,
        Command::new("ls").arg("definitely_missing_file_for_stderr_test"),
    )
    .expect("stderr capture should succeed");
    assert!(out.contains("definitely_missing_file_for_stderr_test"));
}

#[test]
fn run_program_capture_builds_command_before_execution() {
    let out = run_program_capture(
        &service_test_policy(),
        "service_test_program_capture",
        ProcessSpawnClass::OperatorPlane,
        "echo",
        |command| {
            command.arg("from-builder");
        },
    )
    .expect("builder capture should succeed");
    assert_eq!(out.trim(), "from-builder");
}

#[test]
fn run_checked_errors_on_non_zero_status() {
    let mut command = Command::new("false");
    let err = run_checked(
        &service_test_policy(),
        "service_test_checked_nonzero",
        ProcessSpawnClass::OperatorPlane,
        &mut command,
    )
    .expect_err("non-zero exit should error");
    assert!(err.to_string().contains("Command failed"));
}

#[test]
fn run_program_checked_blocks_non_allowlisted_program() {
    let policy = SecurityPolicy {
        allowed_commands: vec!["git".to_string()],
        ..SecurityPolicy::default()
    };
    let err = run_program_checked(
        &policy,
        "service_test_program_checked_blocked",
        ProcessSpawnClass::OperatorPlane,
        "sh",
        |command| {
            command.args(["-lc", "echo hi"]);
        },
    )
    .expect_err("non-allowlisted program should fail");
    assert!(err.to_string().contains("not allowlisted"));
}

#[test]
fn run_checked_blocks_non_allowlisted_program() {
    let policy = SecurityPolicy {
        allowed_commands: vec!["git".to_string()],
        ..SecurityPolicy::default()
    };
    let err = run_checked(
        &policy,
        "service_test_checked_blocked",
        ProcessSpawnClass::OperatorPlane,
        Command::new("sh").args(["-lc", "echo hi"]),
    )
    .expect_err("non-allowlisted program should fail");
    assert!(err.to_string().contains("not allowlisted"));
}

#[test]
fn linux_service_file_has_expected_suffix() {
    let file = linux_service_file(&Config::default()).unwrap();
    let path = file.to_string_lossy();
    assert!(path.ends_with(".config/systemd/user/asterel.service"));
}
