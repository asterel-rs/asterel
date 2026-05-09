//! Discord API constants and type definitions.

/// Discord API base URL (v10).
pub const API_BASE: &str = "https://discord.com/api/v10";

/// Default Gateway intents bitmask.
///
/// GUILDS (1) | `GUILD_MESSAGES` (512) | `GUILD_MESSAGE_REACTIONS` (1024)
/// | `GUILD_MESSAGE_TYPING` (2048) | `DIRECT_MESSAGES` (4096)
/// | `DIRECT_MESSAGE_REACTIONS` (8192) | `DIRECT_MESSAGE_TYPING` (16384)
/// | `MESSAGE_CONTENT` (32768) = 65025
pub const DEFAULT_INTENTS: u64 = 65025;

/// Default heartbeat interval when server does not provide one (ms).
pub const DEFAULT_HEARTBEAT_INTERVAL_MS: u64 = 41250;

/// Discord maximum message length (characters).
pub const MAX_MESSAGE_LENGTH: usize = 2000;

/// Gateway opcodes used in the Discord WebSocket protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GatewayOpcode {
    /// An event was dispatched (server → client).
    Dispatch = 0,
    /// Fired periodically to keep the connection alive.
    Heartbeat = 1,
    /// Starts a new session during the initial handshake.
    Identify = 2,
    /// Update the client's presence.
    PresenceUpdate = 3,
    /// Join/leave or move between voice channels.
    VoiceStateUpdate = 4,
    /// Resume a previous session that was disconnected.
    Resume = 6,
    /// Server is telling the client to reconnect.
    Reconnect = 7,
    /// Request information about offline guild members.
    RequestGuildMembers = 8,
    /// The session has been invalidated.
    InvalidSession = 9,
    /// Sent immediately after connecting; contains heartbeat interval.
    Hello = 10,
    /// Acknowledges a received heartbeat.
    HeartbeatAck = 11,
}

impl GatewayOpcode {
    /// Convert a raw u64 value to an opcode, if valid.
    #[must_use]
    pub fn from_u64(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::Dispatch),
            1 => Some(Self::Heartbeat),
            2 => Some(Self::Identify),
            3 => Some(Self::PresenceUpdate),
            4 => Some(Self::VoiceStateUpdate),
            6 => Some(Self::Resume),
            7 => Some(Self::Reconnect),
            8 => Some(Self::RequestGuildMembers),
            9 => Some(Self::InvalidSession),
            10 => Some(Self::Hello),
            11 => Some(Self::HeartbeatAck),
            _ => None,
        }
    }
}

/// Discord interaction types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InteractionType {
    /// A ping interaction (used for endpoint verification).
    Ping = 1,
    /// A slash command or context menu command invocation.
    ApplicationCommand = 2,
    /// A button click or select menu selection on a message.
    MessageComponent = 3,
    /// An autocomplete request for a command option.
    ApplicationCommandAutocomplete = 4,
    /// A modal dialog submission.
    ModalSubmit = 5,
}

impl InteractionType {
    /// Convert a raw `u64` value to an interaction type, if valid.
    #[must_use]
    pub fn from_u64(value: u64) -> Option<Self> {
        match value {
            1 => Some(Self::Ping),
            2 => Some(Self::ApplicationCommand),
            3 => Some(Self::MessageComponent),
            4 => Some(Self::ApplicationCommandAutocomplete),
            5 => Some(Self::ModalSubmit),
            _ => None,
        }
    }
}

/// Interaction callback types for responding to interactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum InteractionCallbackType {
    /// ACK a Ping.
    Pong = 1,
    /// Respond to an interaction with a message.
    ChannelMessageWithSource = 4,
    /// ACK an interaction and edit a response later (shows "thinking...").
    DeferredChannelMessageWithSource = 5,
    /// For components: ACK an interaction and edit the original message later.
    DeferredUpdateMessage = 6,
    /// For components: edit the message the component was attached to.
    UpdateMessage = 7,
    /// Respond to an autocomplete interaction with choices.
    ApplicationCommandAutocompleteResult = 8,
    /// Respond to an interaction by opening a modal.
    Modal = 9,
}

/// Application command types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ApplicationCommandType {
    /// Slash command (text input).
    ChatInput = 1,
    /// Right-click user context menu.
    User = 2,
    /// Right-click message context menu.
    Message = 3,
}

impl ApplicationCommandType {
    /// Convert a raw `u8` value to a command type, if valid.
    #[must_use]
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::ChatInput),
            2 => Some(Self::User),
            3 => Some(Self::Message),
            _ => None,
        }
    }
}

/// Discord channel types relevant for message routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DiscordChannelType {
    /// A text channel within a server.
    GuildText = 0,
    /// A direct message between users.
    Dm = 1,
    /// A voice channel within a server.
    GuildVoice = 2,
    /// A DM between multiple users.
    GroupDm = 3,
    /// An organizational category containing other channels.
    GuildCategory = 4,
    /// A channel that broadcasts messages to subscribed channels.
    GuildAnnouncement = 5,
    /// A thread in an announcement channel.
    AnnouncementThread = 10,
    /// A public thread visible to all members.
    PublicThread = 11,
    /// A private thread visible only to invited members.
    PrivateThread = 12,
    /// A stage channel for audio events.
    GuildStageVoice = 13,
    /// A channel with threaded posts only.
    GuildForum = 15,
    /// A channel for media-rich threaded posts.
    GuildMedia = 16,
}

impl DiscordChannelType {
    /// Convert a raw `u64` value to a channel type, if valid.
    #[must_use]
    pub fn from_u64(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::GuildText),
            1 => Some(Self::Dm),
            2 => Some(Self::GuildVoice),
            3 => Some(Self::GroupDm),
            4 => Some(Self::GuildCategory),
            5 => Some(Self::GuildAnnouncement),
            10 => Some(Self::AnnouncementThread),
            11 => Some(Self::PublicThread),
            12 => Some(Self::PrivateThread),
            13 => Some(Self::GuildStageVoice),
            15 => Some(Self::GuildForum),
            16 => Some(Self::GuildMedia),
            _ => None,
        }
    }

    /// Whether this channel type represents a thread.
    #[must_use]
    pub fn is_thread(self) -> bool {
        matches!(
            self,
            Self::AnnouncementThread | Self::PublicThread | Self::PrivateThread
        )
    }

    /// Whether this channel type is a voice channel (including stage).
    #[must_use]
    pub fn is_voice(self) -> bool {
        matches!(self, Self::GuildVoice | Self::GuildStageVoice)
    }
}

/// Message flag bitfield constants.
pub mod message_flags {
    /// Suppresses all embed rendering for this message.
    pub const SUPPRESS_EMBEDS: u64 = 1 << 2;
    /// Only visible to the invoking user (interaction responses only).
    pub const EPHEMERAL: u64 = 1 << 6;
    /// Does not trigger push/desktop notifications.
    pub const SUPPRESS_NOTIFICATIONS: u64 = 1 << 12;
}

/// Individual intent bit flags.
pub mod intents {
    /// Receive guild create/update/delete events.
    pub const GUILDS: u64 = 1 << 0;
    /// Receive guild member add/update/remove events (privileged).
    pub const GUILD_MEMBERS: u64 = 1 << 1;
    /// Receive ban add/remove events.
    pub const GUILD_MODERATION: u64 = 1 << 2;
    /// Receive emoji and sticker update events.
    pub const GUILD_EXPRESSIONS: u64 = 1 << 3;
    /// Receive integration update events.
    pub const GUILD_INTEGRATIONS: u64 = 1 << 4;
    /// Receive webhook update events.
    pub const GUILD_WEBHOOKS: u64 = 1 << 5;
    /// Receive invite create/delete events.
    pub const GUILD_INVITES: u64 = 1 << 6;
    /// Receive voice state update events.
    pub const GUILD_VOICE_STATES: u64 = 1 << 7;
    /// Receive presence update events (privileged).
    pub const GUILD_PRESENCES: u64 = 1 << 8;
    /// Receive message create/update/delete in guilds.
    pub const GUILD_MESSAGES: u64 = 1 << 9;
    /// Receive reaction add/remove in guilds.
    pub const GUILD_MESSAGE_REACTIONS: u64 = 1 << 10;
    /// Receive typing start in guilds.
    pub const GUILD_MESSAGE_TYPING: u64 = 1 << 11;
    /// Receive message create/update/delete in DMs.
    pub const DIRECT_MESSAGES: u64 = 1 << 12;
    /// Receive reaction add/remove in DMs.
    pub const DIRECT_MESSAGE_REACTIONS: u64 = 1 << 13;
    /// Receive typing start in DMs.
    pub const DIRECT_MESSAGE_TYPING: u64 = 1 << 14;
    /// Receive message content (privileged).
    pub const MESSAGE_CONTENT: u64 = 1 << 15;
    /// Receive scheduled event create/update/delete.
    pub const GUILD_SCHEDULED_EVENTS: u64 = 1 << 16;
    /// Receive auto-moderation rule create/update/delete.
    pub const AUTO_MODERATION_CONFIGURATION: u64 = 1 << 20;
    /// Receive auto-moderation action execution events.
    pub const AUTO_MODERATION_EXECUTION: u64 = 1 << 21;
}

/// Activity type for bot presence display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ActivityType {
    /// "Playing {name}".
    Playing = 0,
    /// "Streaming {name}".
    Streaming = 1,
    /// "Listening to {name}".
    Listening = 2,
    /// "Watching {name}".
    Watching = 3,
    /// Custom status text.
    Custom = 4,
    /// "Competing in {name}".
    Competing = 5,
}

impl ActivityType {
    /// Convert a raw `u8` value to an activity type, if valid.
    #[must_use]
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Playing),
            1 => Some(Self::Streaming),
            2 => Some(Self::Listening),
            3 => Some(Self::Watching),
            4 => Some(Self::Custom),
            5 => Some(Self::Competing),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_intents_match_expected_flags() {
        assert_ne!(DEFAULT_INTENTS & intents::GUILDS, 0, "GUILDS");
        assert_ne!(
            DEFAULT_INTENTS & intents::GUILD_MESSAGES,
            0,
            "GUILD_MESSAGES"
        );
        assert_ne!(
            DEFAULT_INTENTS & intents::GUILD_MESSAGE_REACTIONS,
            0,
            "GUILD_MESSAGE_REACTIONS"
        );
        assert_ne!(
            DEFAULT_INTENTS & intents::DIRECT_MESSAGES,
            0,
            "DIRECT_MESSAGES"
        );
        assert_ne!(
            DEFAULT_INTENTS & intents::MESSAGE_CONTENT,
            0,
            "MESSAGE_CONTENT"
        );
        assert_ne!(
            DEFAULT_INTENTS & intents::GUILD_MESSAGE_TYPING,
            0,
            "GUILD_MESSAGE_TYPING"
        );
        assert_ne!(
            DEFAULT_INTENTS & intents::DIRECT_MESSAGE_REACTIONS,
            0,
            "DIRECT_MESSAGE_REACTIONS"
        );
        assert_ne!(
            DEFAULT_INTENTS & intents::DIRECT_MESSAGE_TYPING,
            0,
            "DIRECT_MESSAGE_TYPING"
        );
    }

    #[test]
    fn opcode_roundtrip() {
        for v in [0, 1, 2, 3, 4, 6, 7, 8, 9, 10, 11] {
            assert!(GatewayOpcode::from_u64(v).is_some(), "opcode {v}");
        }
        assert!(GatewayOpcode::from_u64(5).is_none());
        assert!(GatewayOpcode::from_u64(99).is_none());
    }

    #[test]
    fn channel_type_thread_detection() {
        assert!(DiscordChannelType::PublicThread.is_thread());
        assert!(DiscordChannelType::PrivateThread.is_thread());
        assert!(DiscordChannelType::AnnouncementThread.is_thread());
        assert!(!DiscordChannelType::GuildText.is_thread());
        assert!(!DiscordChannelType::Dm.is_thread());
    }

    #[test]
    fn channel_type_voice_detection() {
        assert!(DiscordChannelType::GuildVoice.is_voice());
        assert!(DiscordChannelType::GuildStageVoice.is_voice());
        assert!(!DiscordChannelType::GuildText.is_voice());
        assert!(!DiscordChannelType::PublicThread.is_voice());
    }

    #[test]
    fn message_flags_values() {
        assert_eq!(message_flags::SUPPRESS_EMBEDS, 4);
        assert_eq!(message_flags::EPHEMERAL, 64);
        assert_eq!(message_flags::SUPPRESS_NOTIFICATIONS, 4096);
    }

    #[test]
    fn interaction_type_roundtrip() {
        assert_eq!(
            InteractionType::from_u64(2),
            Some(InteractionType::ApplicationCommand)
        );
        assert!(InteractionType::from_u64(0).is_none());
        assert!(InteractionType::from_u64(99).is_none());
    }

    #[test]
    fn activity_type_roundtrip() {
        assert_eq!(ActivityType::from_u8(0), Some(ActivityType::Playing));
        assert_eq!(ActivityType::from_u8(3), Some(ActivityType::Watching));
        assert!(ActivityType::from_u8(99).is_none());
    }

    #[test]
    fn application_command_type_roundtrip() {
        assert_eq!(
            ApplicationCommandType::from_u8(1),
            Some(ApplicationCommandType::ChatInput)
        );
        assert_eq!(
            ApplicationCommandType::from_u8(2),
            Some(ApplicationCommandType::User)
        );
        assert_eq!(
            ApplicationCommandType::from_u8(3),
            Some(ApplicationCommandType::Message)
        );
        assert_eq!(ApplicationCommandType::ChatInput as u8, 1);
        assert_eq!(ApplicationCommandType::User as u8, 2);
        assert_eq!(ApplicationCommandType::Message as u8, 3);
        assert!(ApplicationCommandType::from_u8(0).is_none());
        assert!(ApplicationCommandType::from_u8(99).is_none());
    }
}
