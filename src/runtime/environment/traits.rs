//! `RuntimeAdapter` trait and `RuntimeSandboxClass` enum defining the
//! platform abstraction layer for execution environments.

use std::path::PathBuf;

/// Classification of the runtime's sandbox isolation level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSandboxClass {
    /// Full workspace access (native host).
    Workspace,
    /// Container-isolated (Docker, Podman).
    Container,
    /// Highly restricted (WASM, embedded).
    Restricted,
}

/// Runtime adapter — abstracts platform differences so the same agent
/// code runs on native, Docker, Cloudflare Workers, Raspberry Pi, etc.
pub trait RuntimeAdapter: Send + Sync {
    /// Human-readable runtime name
    fn name(&self) -> &str;

    /// Whether this runtime supports shell access
    fn has_shell_access(&self) -> bool;

    /// Whether this runtime supports filesystem access
    fn has_fs_access(&self) -> bool;

    /// Base storage path for this runtime
    fn storage_path(&self) -> PathBuf;

    /// Whether long-running processes (gateway, heartbeat) are supported
    fn supports_long_runs(&self) -> bool;

    /// Sandbox isolation class for this runtime environment.
    fn sandbox_class(&self) -> RuntimeSandboxClass;

    /// Maximum memory budget in bytes (0 = unlimited)
    fn memory_budget(&self) -> u64 {
        0
    }
}
