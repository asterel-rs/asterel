use std::time::Instant;

use super::traits::ChannelEvent;
use crate::contracts::ids::EntityId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalEventKind {
    Message,
    ReactionAdd,
    ReactionRemove,
    MessageEdit,
    MessageDelete,
    TypingStart,
}

#[derive(Debug, Clone)]
pub struct CanonicalEvent {
    pub kind: CanonicalEventKind,
    pub channel_name: String,
    pub entity_id: Option<EntityId>,
    pub conversation_id: Option<String>,
    pub workspace_id: Option<String>,
    pub raw: ChannelEvent,
    pub received_at: Instant,
}

impl CanonicalEvent {
    #[must_use]
    pub fn from_channel_event(event: ChannelEvent, workspace_id: Option<String>) -> Self {
        let kind = match &event {
            ChannelEvent::Message(_) => CanonicalEventKind::Message,
            ChannelEvent::ReactionAdd { .. } => CanonicalEventKind::ReactionAdd,
            ChannelEvent::ReactionRemove { .. } => CanonicalEventKind::ReactionRemove,
            ChannelEvent::MessageEdit { .. } => CanonicalEventKind::MessageEdit,
            ChannelEvent::MessageDelete { .. } => CanonicalEventKind::MessageDelete,
            ChannelEvent::TypingStart { .. } => CanonicalEventKind::TypingStart,
        };
        let channel_name = event.channel_name().to_string();
        let entity_id = event.sender().map(EntityId::new);
        let conversation_id = event.conversation_id().map(ToString::to_string);

        Self {
            kind,
            channel_name,
            entity_id,
            conversation_id,
            workspace_id,
            raw: event,
            received_at: Instant::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::ids::{ChannelId, MessageId, UserId};
    use crate::transport::channels::traits::{ChannelEvent, ChannelMessage};

    fn test_message() -> ChannelEvent {
        ChannelEvent::Message(ChannelMessage {
            id: "m1".into(),
            sender: "user-42".into(),
            content: "hello".into(),
            channel: "discord".into(),
            context_hint: None,
            conversation_id: Some("chan-99".into()),
            thread_id: None,
            reply_to: None,
            message_id: None,
            timestamp: 0,
            attachments: vec![],
        })
    }

    #[test]
    fn from_channel_event_extracts_message_fields() {
        let event = CanonicalEvent::from_channel_event(test_message(), Some("ws-1".into()));

        assert_eq!(event.kind, CanonicalEventKind::Message);
        assert_eq!(event.channel_name, "discord");
        assert_eq!(
            event.entity_id.as_ref().map(EntityId::as_str),
            Some("user-42")
        );
        assert_eq!(event.conversation_id.as_deref(), Some("chan-99"));
        assert_eq!(event.workspace_id.as_deref(), Some("ws-1"));
    }

    #[test]
    fn from_channel_event_maps_reaction_add() {
        let raw = ChannelEvent::ReactionAdd {
            channel_name: "slack".into(),
            channel_id: ChannelId::new("c1"),
            message_id: MessageId::new("m1"),
            user_id: UserId::new("u1"),
            emoji: "+1".into(),
        };
        let event = CanonicalEvent::from_channel_event(raw, None);

        assert_eq!(event.kind, CanonicalEventKind::ReactionAdd);
        assert_eq!(event.entity_id.as_ref().map(EntityId::as_str), Some("u1"));
        assert!(event.workspace_id.is_none());
    }

    #[test]
    fn from_channel_event_maps_typing_start() {
        let raw = ChannelEvent::TypingStart {
            channel_name: "telegram".into(),
            channel_id: ChannelId::new("ch"),
            user_id: UserId::new("u2"),
        };
        let event = CanonicalEvent::from_channel_event(raw, None);

        assert_eq!(event.kind, CanonicalEventKind::TypingStart);
        assert_eq!(event.channel_name, "telegram");
    }
}
