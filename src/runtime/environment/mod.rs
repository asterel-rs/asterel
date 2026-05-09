//! Re-exports for runtime environment adapters (native, Docker, WASM).

pub mod docker;
mod factory;
pub mod native;
pub mod traits;
pub mod wasm;

pub use docker::DockerRuntime;
pub use factory::{DOCKER_ROLLOUT_GATE_MESSAGE, create_runtime};
pub use native::NativeRuntime;
pub use traits::{RuntimeAdapter, RuntimeSandboxClass};
pub use wasm::WasmRuntime;

#[cfg(test)]
mod tests;
