//! Core channel trait and associated types: `Channel`, `ChannelEvent`,
//! `ChannelMessage`, `ChannelCapabilities`, and `MediaAttachment`.
use std::future::Future;
use std::pin::Pin;

pub use crate::contracts::channels::ChannelCapabilities;
pub use crate::contracts::channels::SurfaceRealizationPolicy;
use crate::contracts::ids::{ChannelId, MessageId, UserId};

#[derive(Debug, Clone)]
pub struct MediaAttachment {
    pub mime_type: String,
    pub data: MediaContent,
    pub filename: Option<String>,
}

#[derive(Debug, Clone)]
pub enum MediaContent {
    Url(String),
    Bytes(Vec<u8>),
}

/// A message received from or sent to a channel.
///
/// `sender` identifies the user (e.g. Discord user ID, Telegram user ID).
/// `conversation_id` identifies the conversation context (e.g. Discord channel/thread ID).
#[derive(Debug, Clone)]
pub struct ChannelMessage {
    pub id: String,
    pub sender: String,
    pub content: String,
    pub channel: String,
    pub context_hint: Option<String>,
    pub conversation_id: Option<String>,
    pub thread_id: Option<String>,
    pub reply_to: Option<String>,
    pub message_id: Option<String>,
    pub timestamp: u64,
    pub attachments: Vec<MediaAttachment>,
}

/// An event received from a channel.
///
/// `Message` wraps a regular text message. Other variants represent
/// platform-specific signals (reactions, edits, deletes, typing) that
/// may not be supported by every channel implementation.
#[derive(Debug, Clone)]
pub enum ChannelEvent {
    /// A regular text message (the primary event type).
    Message(ChannelMessage),
    /// A reaction was added to a message.
    ReactionAdd {
        channel_name: String,
        channel_id: ChannelId,
        message_id: MessageId,
        user_id: UserId,
        emoji: String,
    },
    /// A reaction was removed from a message.
    ReactionRemove {
        channel_name: String,
        channel_id: ChannelId,
        message_id: MessageId,
        user_id: UserId,
        emoji: String,
    },
    /// A message was edited.
    MessageEdit {
        channel_name: String,
        channel_id: ChannelId,
        message_id: MessageId,
        new_content: String,
        user_id: UserId,
    },
    /// A message was deleted.
    MessageDelete {
        channel_name: String,
        channel_id: ChannelId,
        message_id: MessageId,
    },
    /// A user started typing.
    TypingStart {
        channel_name: String,
        channel_id: ChannelId,
        user_id: UserId,
    },
}

impl ChannelEvent {
    /// Returns the channel name (e.g. "discord", "telegram") for any event variant.
    #[must_use]
    pub fn channel_name(&self) -> &str {
        match self {
            Self::Message(msg) => &msg.channel,
            Self::ReactionAdd { channel_name, .. }
            | Self::ReactionRemove { channel_name, .. }
            | Self::MessageEdit { channel_name, .. }
            | Self::MessageDelete { channel_name, .. }
            | Self::TypingStart { channel_name, .. } => channel_name,
        }
    }

    /// Returns the sender/user ID for routing purposes, if available.
    #[must_use]
    pub fn sender(&self) -> Option<&str> {
        match self {
            Self::Message(msg) => Some(&msg.sender),
            Self::ReactionAdd { user_id, .. }
            | Self::ReactionRemove { user_id, .. }
            | Self::MessageEdit { user_id, .. }
            | Self::TypingStart { user_id, .. } => Some(user_id.as_str()),
            Self::MessageDelete { .. } => None,
        }
    }

    /// Returns the conversation/channel ID for routing purposes, if available.
    #[must_use]
    pub fn conversation_id(&self) -> Option<&str> {
        match self {
            Self::Message(msg) => msg.conversation_id.as_deref(),
            Self::ReactionAdd { channel_id, .. }
            | Self::ReactionRemove { channel_id, .. }
            | Self::MessageEdit { channel_id, .. }
            | Self::MessageDelete { channel_id, .. }
            | Self::TypingStart { channel_id, .. } => Some(channel_id.as_str()),
        }
    }
}

/// Core channel trait — implement for any messaging platform
pub trait Channel: Send + Sync {
    /// Human-readable channel name
    fn name(&self) -> &str;

    /// Declares what this channel supports beyond basic text messaging.
    ///
    /// The default implementation returns [`ChannelCapabilities::default`] (text-only).
    /// Override in channel implementations that support richer interactions.
    fn capabilities(&self) -> ChannelCapabilities {
        ChannelCapabilities {
            max_message_length: self.max_message_length(),
            ..ChannelCapabilities::default()
        }
    }

    /// Send a message through this channel
    fn send<'a>(
        &'a self,
        message: &'a str,
        recipient: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

    /// Start listening for incoming events (long-running)
    fn listen<'a>(
        &'a self,
        tx: tokio::sync::mpsc::Sender<ChannelEvent>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

    /// Check if channel is healthy
    fn health_check<'a>(&'a self) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
        Box::pin(async move { true })
    }

    fn max_message_length(&self) -> usize {
        usize::MAX
    }

    /// Companion behavior policy for this surface (§6.4.F).
    ///
    /// Default returns a conservative public-channel policy. Override in
    /// implementations that can prove a narrower private/local surface.
    fn surface_realization_policy(&self) -> SurfaceRealizationPolicy {
        SurfaceRealizationPolicy::public_channel_default()
    }

    fn send_typing<'a>(
        &'a self,
        _recipient: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move { Ok(()) })
    }

    fn send_media<'a>(
        &'a self,
        _attachment: &'a MediaAttachment,
        _recipient: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move { anyhow::bail!("media sending not supported by this channel") })
    }

    fn edit_message<'a>(
        &'a self,
        _channel_id: &'a str,
        _message_id: &'a str,
        _content: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move { anyhow::bail!("message editing not supported by this channel") })
    }

    fn delete_message<'a>(
        &'a self,
        _channel_id: &'a str,
        _message_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move { anyhow::bail!("message deletion not supported by this channel") })
    }

    fn send_chunked<'a>(
        &'a self,
        message: &'a str,
        recipient: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let chunks = super::chunker::chunk_message(message, self.max_message_length());
            for chunk in chunks {
                self.send(&chunk, recipient).await?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;

    struct MinimalChannel;

    impl Channel for MinimalChannel {
        fn name(&self) -> &str {
            "minimal"
        }

        fn send<'a>(
            &'a self,
            _message: &'a str,
            _recipient: &'a str,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }

        fn listen<'a>(
            &'a self,
            _tx: tokio::sync::mpsc::Sender<ChannelEvent>,
        ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[test]
    fn channel_capabilities_default_is_text_only() {
        let caps = ChannelCapabilities::default();
        assert!(!caps.can_edit_message);
        assert!(!caps.can_delete_message);
        assert!(!caps.can_send_media);
        assert!(!caps.can_send_embed);
        assert!(!caps.can_send_typing);
        assert_eq!(caps.max_message_length, usize::MAX);
        assert!(!caps.can_create_thread);
        assert!(!caps.can_manage_thread_members);
        assert!(!caps.can_add_reaction);
        assert!(!caps.can_read_reactions);
        assert!(!caps.can_send_buttons);
        assert!(!caps.can_send_select_menu);
        assert!(!caps.can_send_modal);
        assert!(!caps.can_fetch_history);
        assert!(!caps.can_receive_reactions);
        assert!(!caps.can_receive_edits);
        assert!(!caps.can_receive_deletes);
        assert!(!caps.can_receive_typing);
    }

    #[test]
    fn channel_capabilities_partial_override() {
        let caps = ChannelCapabilities {
            can_send_media: true,
            can_send_typing: true,
            max_message_length: 4096,
            ..ChannelCapabilities::default()
        };
        assert!(caps.can_send_media);
        assert!(caps.can_send_typing);
        assert_eq!(caps.max_message_length, 4096);
        assert!(!caps.can_create_thread);
        assert!(!caps.can_send_buttons);
    }

    #[test]
    fn default_surface_realization_policy_is_public_safe() {
        let policy = MinimalChannel.surface_realization_policy();
        assert!(policy.is_public);
        assert!(policy.intimacy_cap < 0.5);
        assert!(policy.memory_exposure_cap < 0.3);
        assert_eq!(policy.default_density, "brief");
    }
}
