use std::path::Path;

use asterel::config::{AutonomyConfig, RuntimeConfig, RuntimeKind, SandboxSelectorMode};
use asterel::runtime::create_runtime;
use asterel::security::SecurityPolicy;

#[test]
fn auto_runtime_resolves_to_native_without_docker_gate() {
    let config = RuntimeConfig {
        kind: RuntimeKind::Auto,
        enable_docker_runtime: false,
        ..RuntimeConfig::default()
    };

    let runtime = create_runtime(&config).expect("auto runtime should resolve to native");
    assert_eq!(runtime.name(), "native");
}

#[test]
fn auto_runtime_resolves_to_docker_with_docker_gate() {
    let config = RuntimeConfig {
        kind: RuntimeKind::Auto,
        enable_docker_runtime: true,
        ..RuntimeConfig::default()
    };

    let runtime = create_runtime(&config).expect("auto runtime should resolve to docker");
    assert_eq!(runtime.name(), "docker");
}

#[test]
fn wasm_runtime_contract_is_restricted() {
    let config = RuntimeConfig {
        kind: RuntimeKind::Wasm,
        ..RuntimeConfig::default()
    };

    let runtime = create_runtime(&config).expect("wasm runtime should be creatable");
    assert_eq!(runtime.name(), "wasm");
    assert!(!runtime.has_shell_access());
    assert!(!runtime.has_fs_access());
    assert!(!runtime.supports_long_runs());
}

#[test]
fn sandbox_selector_auto_relaxes_workspace_only_for_docker() {
    let autonomy = AutonomyConfig::default();
    let runtime = RuntimeConfig {
        kind: RuntimeKind::Docker,
        enable_docker_runtime: true,
        sandbox_selector: SandboxSelectorMode::Auto,
        ..RuntimeConfig::default()
    };

    let policy =
        SecurityPolicy::from_config_runtime(&autonomy, &runtime, Path::new("/tmp/runtime"));
    assert!(!policy.workspace_only);
}

#[test]
fn sandbox_selector_fixed_keeps_configured_workspace_only() {
    let autonomy = AutonomyConfig {
        workspace_only: false,
        ..AutonomyConfig::default()
    };
    let runtime = RuntimeConfig {
        kind: RuntimeKind::Docker,
        enable_docker_runtime: true,
        sandbox_selector: SandboxSelectorMode::Fixed,
        ..RuntimeConfig::default()
    };

    let policy =
        SecurityPolicy::from_config_runtime(&autonomy, &runtime, Path::new("/tmp/runtime"));
    assert!(!policy.workspace_only);
}
