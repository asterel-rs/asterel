//! Read models for the session-list and session-message control-plane endpoints.

use serde::{Deserialize, Serialize};

use crate::contracts::ids::UserId;
use crate::core::sessions::types::{ChatMessage, Session};

/// Paginated list of session summaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListReadModel {
    /// Session summaries in reverse-chronological order.
    pub items: Vec<SessionSummaryReadModel>,
}

/// One session entry in the session list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummaryReadModel {
    /// Session UUID.
    pub id: String,
    /// Surface / channel label (e.g. `"cli"`, `"slack"`, `"discord"`).
    pub surface: String,
    /// User ID that owns this session.
    pub owner_scope: UserId,
    /// Session state label (e.g. `"Active"`, `"Idle"`, `"Closed"`).
    pub state: String,
    /// RFC-3339 session creation timestamp.
    pub created_at: String,
    /// RFC-3339 last-activity timestamp.
    pub updated_at: String,
}

/// Paginated list of messages within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessageListReadModel {
    /// Messages in chronological order.
    pub items: Vec<SessionMessageReadModel>,
}

/// One message entry in the session message list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessageReadModel {
    /// Message UUID.
    pub id: String,
    /// Role label (`"User"`, `"Assistant"`, `"System"`).
    pub role: String,
    /// Full text content of the message.
    pub content: String,
    /// Number of input tokens charged for this message (assistant turns only).
    pub input_tokens: Option<u64>,
    /// Number of output tokens charged for this message (assistant turns only).
    pub output_tokens: Option<u64>,
    /// RFC-3339 timestamp when the message was stored.
    pub created_at: String,
}

/// Abstracts over concrete session types so the builder can project from
/// both in-memory (`Session`) and future storage-backed session representations.
pub trait SessionSummarySource {
    fn id(&self) -> &str;
    fn surface(&self) -> &str;
    fn owner_scope(&self) -> &str;
    fn state_label(&self) -> String;
    fn created_at(&self) -> &str;
    fn updated_at(&self) -> &str;
}

/// Abstracts over concrete chat-message types for the same reason as
/// `SessionSummarySource`: keeps the builder independent of storage details.
pub trait SessionMessageSource {
    fn id(&self) -> &str;
    fn role_label(&self) -> String;
    fn content(&self) -> &str;
    fn input_tokens(&self) -> Option<u64>;
    fn output_tokens(&self) -> Option<u64>;
    fn created_at(&self) -> &str;
}

impl SessionSummarySource for Session {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn surface(&self) -> &str {
        &self.surface
    }

    fn owner_scope(&self) -> &str {
        self.owner_scope.as_str()
    }

    fn state_label(&self) -> String {
        format!("{:?}", self.state)
    }

    fn created_at(&self) -> &str {
        &self.created_at
    }

    fn updated_at(&self) -> &str {
        &self.updated_at
    }
}

impl SessionMessageSource for ChatMessage {
    fn id(&self) -> &str {
        self.id.as_str()
    }

    fn role_label(&self) -> String {
        format!("{:?}", self.role)
    }

    fn content(&self) -> &str {
        &self.content
    }

    fn input_tokens(&self) -> Option<u64> {
        self.input_tokens
    }

    fn output_tokens(&self) -> Option<u64> {
        self.output_tokens
    }

    fn created_at(&self) -> &str {
        &self.created_at
    }
}

#[must_use]
pub fn build_session_list_read_model<T>(sessions: &[T]) -> SessionListReadModel
where
    T: SessionSummarySource,
{
    SessionListReadModel {
        items: sessions
            .iter()
            .map(|session| SessionSummaryReadModel {
                id: session.id().to_string(),
                surface: session.surface().to_string(),
                owner_scope: UserId::new(session.owner_scope()),
                state: session.state_label(),
                created_at: session.created_at().to_string(),
                updated_at: session.updated_at().to_string(),
            })
            .collect(),
    }
}

#[must_use]
pub fn build_session_message_list_read_model<T>(messages: &[T]) -> SessionMessageListReadModel
where
    T: SessionMessageSource,
{
    SessionMessageListReadModel {
        items: messages
            .iter()
            .map(|message| SessionMessageReadModel {
                id: message.id().to_string(),
                role: message.role_label(),
                content: message.content().to_string(),
                input_tokens: message.input_tokens(),
                output_tokens: message.output_tokens(),
                created_at: message.created_at().to_string(),
            })
            .collect(),
    }
}
