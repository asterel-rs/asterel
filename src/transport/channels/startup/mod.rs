//! Re-exports for channel startup: runtime initialization, listener
//! spawning, routing queues, and the doctor health-check command.
mod doctor;
mod listener;
mod prompt;
mod routing_queue;
mod runtime;

pub use doctor::doctor_channels;
pub(crate) use listener::request_channel_surface_reload;
pub(crate) use listener::run_channels_surface;
pub use listener::start_channels;
#[cfg(test)]
pub(crate) use listener::subscribe_channel_surface_reload_for_tests;
pub(super) use runtime::{ChannelRuntime, ChannelThinkingState};
