//! Re-exports for the IRC-over-TLS channel adapter.
mod auth;
pub mod channel;
mod message;
mod parse;
mod tls;

pub use channel::{IrcChannel, IrcChannelConfig};

#[cfg(test)]
mod tests;
