//! Deserialization models for Matrix Client-Server API `/sync` responses,
//! room timelines, and event content types.
use serde::Deserialize;

use crate::contracts::ids::UserId;

/// Top-level response from the Matrix `/sync` endpoint.
#[derive(Debug, Deserialize)]
pub(super) struct SyncResponse {
    pub(super) next_batch: String,
    #[serde(default)]
    pub(super) rooms: Rooms,
}

/// Container for joined/invited/left rooms in a sync response.
#[derive(Debug, Deserialize, Default)]
pub(super) struct Rooms {
    #[serde(default)]
    pub(super) join: std::collections::HashMap<String, JoinedRoom>,
}

/// A room the user has joined, containing its event timeline.
#[derive(Debug, Deserialize)]
pub(super) struct JoinedRoom {
    #[serde(default)]
    pub(super) timeline: Timeline,
}

/// Ordered list of events in a room timeline.
#[derive(Debug, Deserialize, Default)]
pub(super) struct Timeline {
    #[serde(default)]
    pub(super) events: Vec<TimelineEvent>,
}

/// A single event in a room timeline (e.g. `m.room.message`).
#[derive(Debug, Deserialize)]
pub(super) struct TimelineEvent {
    #[serde(default)]
    pub(super) event_id: Option<String>,
    #[serde(rename = "type")]
    pub(super) event_type: String,
    pub(super) sender: String,
    #[serde(default)]
    pub(super) content: EventContent,
}

/// Content payload of a Matrix event (body, type, media URL, info).
#[derive(Debug, Deserialize, Default)]
pub(super) struct EventContent {
    #[serde(default)]
    pub(super) body: Option<String>,
    #[serde(default)]
    pub(super) msgtype: Option<String>,
    #[serde(default)]
    pub(super) url: Option<String>,
    #[serde(default)]
    pub(super) info: Option<EventContentInfo>,
}

/// Optional metadata about event content (e.g. MIME type for media).
#[derive(Debug, Deserialize, Default)]
pub(super) struct EventContentInfo {
    #[serde(default)]
    pub(super) mimetype: Option<String>,
}

/// Response from the Matrix `/_matrix/client/v3/account/whoami` endpoint.
#[derive(Debug, Deserialize)]
pub(super) struct WhoAmIResponse {
    pub(super) user_id: UserId,
}
