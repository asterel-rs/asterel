//! `ChannelActionBroker` — runtime adapter for platform-specific channel APIs.
//!
//! # Purpose
//!
//! The five channel tools (`create_thread`, `add_reaction`, `send_with_components`,
//! `get_messages`, `send_embed`) are platform-agnostic. Each tool holds a
//! reference to a `ChannelActionBroker` injected via the `ExecutionContext`
//! at construction time. The broker translates generic tool calls into
//! platform-specific API calls (e.g., `Discord`'s REST API).
//!
//! # Implementing a broker
//!
//! Implementations must be `Send + Sync` because the `ExecutionContext` is
//! shared across async tasks. All methods return `Pin<Box<dyn Future<…>>>` to
//! allow `async` impls without boxing overhead at the call site.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;
use serde_json::Value;

/// Trait that adapts generic channel tool calls to platform-specific API requests.
///
/// Implementations are injected into the `ExecutionContext` and used by all
/// five channel tools. A missing broker causes tools to return a non-success
/// result with message `"channel action broker is not available"`.
pub trait ChannelActionBroker: Send + Sync {
    /// Name of the platform this broker serves (e.g. `"discord"`).
    fn channel_name(&self) -> &str;

    /// Create a new thread in the given channel.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel API rejects the request.
    fn create_thread<'a>(
        &'a self,
        channel_id: &'a str,
        name: &'a str,
        message_id: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;

    /// Add a reaction emoji to a message.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel API rejects the request.
    fn add_reaction<'a>(
        &'a self,
        channel_id: &'a str,
        message_id: &'a str,
        emoji: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

    /// Send a message with interactive components (buttons, etc.).
    ///
    /// # Errors
    ///
    /// Returns an error if the channel API rejects the request.
    fn send_with_components<'a>(
        &'a self,
        channel_id: &'a str,
        content: &'a str,
        components: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;

    /// Retrieve recent messages from a channel.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel API rejects the request.
    fn get_messages<'a>(
        &'a self,
        channel_id: &'a str,
        limit: Option<u32>,
        before: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>;

    /// Send a rich embed message to a channel.
    ///
    /// # Errors
    ///
    /// Returns an error if the channel API rejects the request.
    fn send_embed<'a>(
        &'a self,
        channel_id: &'a str,
        title: &'a str,
        description: &'a str,
        color: Option<u32>,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>>;
}
