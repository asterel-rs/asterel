//! Unit tests for the runtime environment factory and adapters.

use super::*;
use crate::config::{RuntimeConfig, RuntimeKind};

#[test]
fn factory_native() {
    let cfg = RuntimeConfig {
        kind: RuntimeKind::Native,
        enable_docker_runtime: false,
        ..RuntimeConfig::default()
    };
    let rt = create_runtime(&cfg).unwrap();
    assert_eq!(rt.name(), "native");
    assert!(rt.has_shell_access());
}

#[test]
fn factory_docker_disabled_without_gate() {
    let cfg = RuntimeConfig {
        kind: RuntimeKind::Docker,
        enable_docker_runtime: false,
        ..RuntimeConfig::default()
    };

    match create_runtime(&cfg) {
        Err(err) => {
            let message = err.to_string();
            assert!(message.contains("disabled by rollout gate"));
            assert!(message.contains("runtime.enable_docker_runtime=true"));
        }
        Ok(_) => panic!("docker runtime should be gated by default"),
    }
}

#[test]
fn factory_docker_enabled_with_gate() {
    let cfg = RuntimeConfig {
        kind: RuntimeKind::Docker,
        enable_docker_runtime: true,
        ..RuntimeConfig::default()
    };

    let rt = create_runtime(&cfg).unwrap();
    assert_eq!(rt.name(), "docker");
    assert!(rt.has_shell_access());
}

#[test]
fn factory_auto_prefers_native_when_docker_gate_disabled() {
    let cfg = RuntimeConfig {
        kind: RuntimeKind::Auto,
        enable_docker_runtime: false,
        ..RuntimeConfig::default()
    };

    let rt =
        create_runtime(&cfg).expect("auto runtime should resolve to native without docker gate");
    assert_eq!(rt.name(), "native");
}

#[test]
fn factory_auto_prefers_docker_when_docker_gate_enabled() {
    let cfg = RuntimeConfig {
        kind: RuntimeKind::Auto,
        enable_docker_runtime: true,
        ..RuntimeConfig::default()
    };

    let rt = create_runtime(&cfg)
        .expect("auto runtime should resolve to docker when docker gate is enabled");
    assert_eq!(rt.name(), "docker");
}

#[test]
fn factory_wasm_runtime_is_available() {
    let cfg = RuntimeConfig {
        kind: RuntimeKind::Wasm,
        ..RuntimeConfig::default()
    };

    let rt = create_runtime(&cfg).expect("wasm runtime should be creatable");
    assert_eq!(rt.name(), "wasm");
    assert!(!rt.has_shell_access());
}

#[test]
fn factory_auto_without_docker_gate_uses_workspace_sandbox() {
    let cfg = RuntimeConfig {
        kind: RuntimeKind::Auto,
        enable_docker_runtime: false,
        ..RuntimeConfig::default()
    };

    let rt = create_runtime(&cfg).expect("auto runtime should resolve to native");
    assert_eq!(rt.name(), "native");
    assert_eq!(rt.sandbox_class(), RuntimeSandboxClass::Workspace);
}

#[test]
fn factory_auto_with_docker_gate_uses_container_sandbox() {
    let cfg = RuntimeConfig {
        kind: RuntimeKind::Auto,
        enable_docker_runtime: true,
        ..RuntimeConfig::default()
    };

    let rt = create_runtime(&cfg).expect("auto runtime should resolve to docker");
    assert_eq!(rt.name(), "docker");
    assert_eq!(rt.sandbox_class(), RuntimeSandboxClass::Container);
}

#[test]
fn factory_wasm_runtime_has_restricted_capabilities() {
    let cfg = RuntimeConfig {
        kind: RuntimeKind::Wasm,
        ..RuntimeConfig::default()
    };

    let rt = create_runtime(&cfg).expect("wasm runtime should be creatable");
    assert_eq!(rt.sandbox_class(), RuntimeSandboxClass::Restricted);
    assert!(!rt.has_fs_access());
    assert!(!rt.supports_long_runs());
}

#[test]
fn factory_docker_gate_error_matches_constant_message() {
    let cfg = RuntimeConfig {
        kind: RuntimeKind::Docker,
        enable_docker_runtime: false,
        ..RuntimeConfig::default()
    };

    match create_runtime(&cfg) {
        Err(error) => assert_eq!(error.to_string(), DOCKER_ROLLOUT_GATE_MESSAGE),
        Ok(_) => panic!("docker runtime should be gated by default"),
    }
}
