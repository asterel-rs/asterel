//! Re-exports for the `WhatsApp` Business Cloud API channel adapter.
mod channel;

pub use channel::WhatsAppChannel;

#[cfg(test)]
use super::traits::Channel;

#[cfg(test)]
mod tests;
