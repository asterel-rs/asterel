//! Unit tests for Matrix channel: homeserver normalization, user allowlist,
//! sync response parsing, media URL conversion, and capability flags.
use super::MatrixChannel;
use super::models::{EventContent, EventContentInfo, SyncResponse, TimelineEvent, WhoAmIResponse};
use crate::transport::channels::traits::{Channel, MediaContent};

fn make_channel() -> MatrixChannel {
    MatrixChannel::new(
        "https://matrix.org".to_string(),
        "syt_test_token".to_string(),
        "!room:matrix.org".to_string(),
        vec!["@user:matrix.org".to_string()],
    )
}

#[test]
fn creates_with_correct_fields() {
    let ch = make_channel();
    assert_eq!(ch.homeserver, "https://matrix.org");
    assert_eq!(ch.access_token, "syt_test_token");
    assert_eq!(ch.room_id, "!room:matrix.org");
    assert_eq!(ch.allowed_users.len(), 1);
}

#[test]
fn strips_trailing_slash() {
    let ch = MatrixChannel::new(
        "https://matrix.org/".to_string(),
        "tok".to_string(),
        "!r:m".to_string(),
        vec![],
    );
    assert_eq!(ch.homeserver, "https://matrix.org");
}

#[test]
fn no_trailing_slash_unchanged() {
    let ch = MatrixChannel::new(
        "https://matrix.org".to_string(),
        "tok".to_string(),
        "!r:m".to_string(),
        vec![],
    );
    assert_eq!(ch.homeserver, "https://matrix.org");
}

#[test]
fn multiple_trailing_slashes_strips_one() {
    let ch = MatrixChannel::new(
        "https://matrix.org//".to_string(),
        "tok".to_string(),
        "!r:m".to_string(),
        vec![],
    );
    assert_eq!(ch.homeserver, "https://matrix.org/");
}

#[test]
fn wildcard_allows_anyone() {
    let ch = MatrixChannel::new(
        "https://m.org".to_string(),
        "tok".to_string(),
        "!r:m".to_string(),
        vec!["*".to_string()],
    );
    assert!(ch.is_user_allowed("@anyone:matrix.org"));
    assert!(ch.is_user_allowed("@hacker:evil.org"));
}

#[test]
fn specific_user_allowed() {
    let ch = make_channel();
    assert!(ch.is_user_allowed("@user:matrix.org"));
}

#[test]
fn unknown_user_denied() {
    let ch = make_channel();
    assert!(!ch.is_user_allowed("@stranger:matrix.org"));
    assert!(!ch.is_user_allowed("@evil:hacker.org"));
}

#[test]
fn user_case_insensitive() {
    let ch = MatrixChannel::new(
        "https://m.org".to_string(),
        "tok".to_string(),
        "!r:m".to_string(),
        vec!["@User:Matrix.org".to_string()],
    );
    assert!(ch.is_user_allowed("@user:matrix.org"));
    assert!(ch.is_user_allowed("@USER:MATRIX.ORG"));
}

#[test]
fn empty_allowlist_denies_all() {
    let ch = MatrixChannel::new(
        "https://m.org".to_string(),
        "tok".to_string(),
        "!r:m".to_string(),
        vec![],
    );
    assert!(!ch.is_user_allowed("@anyone:matrix.org"));
}

#[test]
fn name_returns_matrix() {
    let ch = make_channel();
    assert_eq!(ch.name(), "matrix");
}

#[test]
fn sync_response_deserializes_empty() {
    let json = r#"{"next_batch":"s123","rooms":{"join":{}}}"#;
    let resp: SyncResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.next_batch, "s123");
    assert!(resp.rooms.join.is_empty());
}

#[test]
fn sync_response_deserializes_with_events() {
    let json = r#"{
            "next_batch": "s456",
            "rooms": {
                "join": {
                    "!room:matrix.org": {
                        "timeline": {
                            "events": [
                                {
                                    "event_id": "$event-1",
                                    "type": "m.room.message",
                                    "sender": "@user:matrix.org",
                                    "content": {
                                        "msgtype": "m.text",
                                        "body": "Hello!"
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        }"#;
    let resp: SyncResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.next_batch, "s456");
    let room = resp.rooms.join.get("!room:matrix.org").unwrap();
    assert_eq!(room.timeline.events.len(), 1);
    assert_eq!(room.timeline.events[0].sender, "@user:matrix.org");
    assert_eq!(
        room.timeline.events[0].event_id.as_deref(),
        Some("$event-1")
    );
    assert_eq!(
        room.timeline.events[0].content.body.as_deref(),
        Some("Hello!")
    );
    assert_eq!(
        room.timeline.events[0].content.msgtype.as_deref(),
        Some("m.text")
    );
}

#[test]
fn process_event_preserves_room_and_event_identity() {
    let ch = make_channel();
    let event = TimelineEvent {
        event_id: Some("$event-1".to_string()),
        event_type: "m.room.message".to_string(),
        sender: "@user:matrix.org".to_string(),
        content: EventContent {
            body: Some("Hello".to_string()),
            msgtype: Some("m.text".to_string()),
            url: None,
            info: None,
        },
    };

    let message = ch
        .process_event(
            &event,
            &crate::contracts::ids::UserId::new("@bot:matrix.org"),
        )
        .expect("event should become channel message");

    assert_eq!(message.id, "$event-1");
    assert_eq!(message.sender, "@user:matrix.org");
    assert_eq!(message.conversation_id.as_deref(), Some("!room:matrix.org"));
    assert_eq!(message.message_id.as_deref(), Some("$event-1"));
}

#[test]
fn sync_response_ignores_non_text_events() {
    let json = r#"{
            "next_batch": "s789",
            "rooms": {
                "join": {
                    "!room:m": {
                        "timeline": {
                            "events": [
                                {
                                    "type": "m.room.member",
                                    "sender": "@user:m",
                                    "content": {}
                                }
                            ]
                        }
                    }
                }
            }
        }"#;
    let resp: SyncResponse = serde_json::from_str(json).unwrap();
    let room = resp.rooms.join.get("!room:m").unwrap();
    assert_eq!(room.timeline.events[0].event_type, "m.room.member");
    assert!(room.timeline.events[0].content.body.is_none());
}

#[test]
fn whoami_response_deserializes() {
    let json = r#"{"user_id":"@bot:matrix.org"}"#;
    let resp: WhoAmIResponse = serde_json::from_str(json).unwrap();
    assert_eq!(
        resp.user_id,
        crate::contracts::ids::UserId::new("@bot:matrix.org")
    );
}

#[test]
fn event_content_defaults() {
    let json = r#"{"type":"m.room.message","sender":"@u:m","content":{}}"#;
    let event: TimelineEvent = serde_json::from_str(json).unwrap();
    assert!(event.content.body.is_none());
    assert!(event.content.msgtype.is_none());
}

#[test]
fn sync_response_missing_rooms_defaults() {
    let json = r#"{"next_batch":"s0"}"#;
    let resp: SyncResponse = serde_json::from_str(json).unwrap();
    assert!(resp.rooms.join.is_empty());
}

#[test]
fn mxc_to_http_converts_valid_mxc_url() {
    let ch = make_channel();
    let http = ch.mxc_to_http("mxc://matrix.org/abc123");
    assert_eq!(
        http.as_deref(),
        Some("https://matrix.org/_matrix/media/v3/download/matrix.org/abc123")
    );
}

#[test]
fn mxc_to_http_rejects_non_mxc_url() {
    let ch = make_channel();
    assert!(ch.mxc_to_http("https://matrix.org/media").is_none());
}

#[test]
fn parse_media_attachments_for_image_event() {
    let ch = make_channel();
    let content = EventContent {
        body: Some("photo.png".to_string()),
        msgtype: Some("m.image".to_string()),
        url: Some("mxc://matrix.org/image123".to_string()),
        info: Some(EventContentInfo {
            mimetype: Some("image/png".to_string()),
        }),
    };

    let attachments = ch.parse_media_attachments(&content);
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].mime_type, "image/png");
    assert_eq!(attachments[0].filename.as_deref(), Some("photo.png"));
    assert!(matches!(
        &attachments[0].data,
        MediaContent::Url(url) if url.contains("/download/matrix.org/image123")
    ));
}

#[test]
fn parse_media_attachments_for_file_event() {
    let ch = make_channel();
    let content = EventContent {
        body: Some("doc.pdf".to_string()),
        msgtype: Some("m.file".to_string()),
        url: Some("mxc://matrix.org/file123".to_string()),
        info: Some(EventContentInfo {
            mimetype: Some("application/pdf".to_string()),
        }),
    };

    let attachments = ch.parse_media_attachments(&content);
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].mime_type, "application/pdf");
    assert_eq!(attachments[0].filename.as_deref(), Some("doc.pdf"));
}

#[tokio::test]
async fn send_media_rejects_private_url_before_matrix_upload() {
    let ch = make_channel();
    let attachment = crate::transport::channels::traits::MediaAttachment {
        mime_type: "image/png".to_string(),
        data: MediaContent::Url("http://127.0.0.1/private.png".to_string()),
        filename: Some("private.png".to_string()),
    };

    let error = ch
        .send_media(&attachment, "ignored")
        .await
        .expect_err("private URL should be rejected before Matrix upload")
        .to_string();

    assert!(error.contains("load Matrix media before upload"));
}

#[test]
fn parse_media_attachments_text_event_has_no_attachments() {
    let ch = make_channel();
    let content = EventContent {
        body: Some("hello".to_string()),
        msgtype: Some("m.text".to_string()),
        url: None,
        info: None,
    };

    let attachments = ch.parse_media_attachments(&content);
    assert!(attachments.is_empty());
    assert_eq!(content.body.as_deref(), Some("hello"));
}

#[test]
fn parse_media_attachments_requires_url_for_media_msgtypes() {
    let ch = make_channel();
    let content = EventContent {
        body: Some("clip.mp4".to_string()),
        msgtype: Some("m.video".to_string()),
        url: None,
        info: Some(EventContentInfo {
            mimetype: Some("video/mp4".to_string()),
        }),
    };

    assert!(ch.parse_media_attachments(&content).is_empty());
}

#[test]
fn parse_media_attachments_defaults_mime_type() {
    let ch = make_channel();
    let content = EventContent {
        body: Some("audio.ogg".to_string()),
        msgtype: Some("m.audio".to_string()),
        url: Some("mxc://matrix.org/audio999".to_string()),
        info: None,
    };

    let attachments = ch.parse_media_attachments(&content);
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].mime_type, "application/octet-stream");
}

#[test]
fn event_content_deserializes_media_fields() {
    let json = r#"{
            "type":"m.room.message",
            "sender":"@u:m",
            "content":{
                "msgtype":"m.image",
                "body":"cat.png",
                "url":"mxc://matrix.org/cat123",
                "info":{"mimetype":"image/png"}
            }
        }"#;

    let event: TimelineEvent = serde_json::from_str(json).unwrap();
    assert_eq!(event.content.msgtype.as_deref(), Some("m.image"));
    assert_eq!(event.content.body.as_deref(), Some("cat.png"));
    assert_eq!(
        event.content.url.as_deref(),
        Some("mxc://matrix.org/cat123")
    );
    assert_eq!(
        event
            .content
            .info
            .as_ref()
            .and_then(|info| info.mimetype.as_deref()),
        Some("image/png")
    );
}
