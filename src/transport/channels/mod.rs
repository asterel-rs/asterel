//! Channel transport facade.
//!
//! Contains inbound adapters (Discord, Slack, Telegram, etc.),
//! routing/ingress policy helpers, startup orchestration, and the
//! channel trait used by the runtime.

#[cfg(any(feature = "telegram", feature = "twitter", feature = "whatsapp"))]
mod api_request;
mod attachments;
pub mod canonical_event;
pub mod chunker;
pub mod cli;
mod coalescer;
mod command;
#[cfg(feature = "discord")]
pub mod discord;
#[cfg(feature = "email")]
pub mod email;
pub mod factory;
mod health;
#[cfg(feature = "imessage")]
pub mod imessage;
/// Ingress policy and guardrails.
pub mod ingress_policy;
#[cfg(feature = "irc")]
pub mod irc;
#[cfg(feature = "matrix")]
pub mod matrix;
mod message_handler;
pub mod policy;
pub mod prompt_builder;
pub mod runtime;
#[cfg(feature = "slack")]
pub mod slack;
mod startup;
pub mod style_profile;
#[cfg(feature = "telegram")]
pub mod telegram;
pub mod traits;
#[cfg(feature = "twitter")]
pub mod twitter;
#[cfg(feature = "whatsapp")]
pub mod whatsapp;

#[cfg(test)]
mod tests;

pub use command::handle_command;
#[cfg(feature = "discord")]
pub use discord::DiscordChannel;
#[cfg(feature = "email")]
pub use email::EmailChannel;
pub(crate) use health::{ChannelHealthState, classify_health_result};
#[cfg(feature = "imessage")]
pub use imessage::IMessageChannel;
#[cfg(feature = "irc")]
pub use irc::{IrcChannel, IrcChannelConfig};
#[cfg(feature = "matrix")]
pub use matrix::MatrixChannel;
pub use prompt_builder::build_system_prompt;
pub(crate) use prompt_builder::{
    SystemPromptOptions, build_system_prompt_from_index_opts, gateway_base_prompt,
};
#[cfg(feature = "slack")]
pub use slack::SlackChannel;
pub(crate) use startup::request_channel_surface_reload;
pub(crate) use startup::run_channels_surface;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use startup::subscribe_channel_surface_reload_for_tests;
pub use startup::{doctor_channels, start_channels};
#[cfg(feature = "telegram")]
pub use telegram::TelegramChannel;
pub use traits::{Channel, ChannelEvent};
#[cfg(feature = "twitter")]
pub use twitter::TwitterChannel;
#[cfg(feature = "whatsapp")]
pub use whatsapp::WhatsAppChannel;
