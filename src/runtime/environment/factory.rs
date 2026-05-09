//! Runtime adapter factory: selects native, Docker, or WASM runtime
//! based on configuration, with rollout gating for experimental backends.

use super::{DockerRuntime, NativeRuntime, WasmRuntime};
use crate::config::{RuntimeConfig, RuntimeKind};

/// Error message shown when Docker runtime is disabled by rollout gate.
pub const DOCKER_ROLLOUT_GATE_MESSAGE: &str = "runtime.kind='docker' is disabled by rollout gate. Set runtime.enable_docker_runtime=true to enable experimental docker runtime.";

/// Factory: create the right runtime from config
///
/// # Errors
///
/// Returns an error when runtime selection is invalid or blocked by rollout
/// gates.
pub fn create_runtime(config: &RuntimeConfig) -> anyhow::Result<Box<dyn super::RuntimeAdapter>> {
    match config.kind {
        RuntimeKind::Auto => create_runtime_for_kind(config.resolved_runtime_kind(), config),
        _ => create_runtime_for_kind(config.kind, config),
    }
}

fn create_runtime_for_kind(
    kind: RuntimeKind,
    config: &RuntimeConfig,
) -> anyhow::Result<Box<dyn super::RuntimeAdapter>> {
    match kind {
        RuntimeKind::Native => Ok(Box::new(NativeRuntime::new())),
        RuntimeKind::Docker => {
            if config.enable_docker_runtime {
                Ok(Box::new(DockerRuntime::new()))
            } else {
                anyhow::bail!(DOCKER_ROLLOUT_GATE_MESSAGE)
            }
        }
        RuntimeKind::Wasm => Ok(Box::new(WasmRuntime::new())),
        RuntimeKind::Auto => anyhow::bail!("auto kind must be resolved before runtime creation"),
    }
}
