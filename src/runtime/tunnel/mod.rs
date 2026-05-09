//! Tunnel subsystem entry point.
//!
//! Concrete tunnel adapters remain module-private so production callers go
//! through `create_tunnel`, where process-spawn policy is enforced before any
//! external connector is constructed.

mod cloudflare;
mod custom;
mod factory;
mod ngrok;
#[cfg(test)]
mod none;
mod process;
mod tailscale;
mod traits;

#[cfg(test)]
mod tests;

pub use factory::create_tunnel;
pub(crate) use process::{SharedProcess, TunnelProcess, kill_shared, new_shared_process};
pub use traits::Tunnel;
