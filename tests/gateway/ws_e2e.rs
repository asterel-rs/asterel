use asterel::contracts::ids::SessionId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessageContract {
    ChatResponse {
        session_id: Option<SessionId>,
        content: String,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    Typing {
        agent: bool,
    },
    Error {
        message: String,
    },
    Pong,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessageContract {
    Chat {
        session_id: Option<SessionId>,
        message: String,
        #[serde(default)]
        attachments: Option<Vec<ClientAttachmentContract>>,
    },
    Typing {
        session_id: Option<SessionId>,
    },
    Ping,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct ClientAttachmentContract {
    upload_id: String,
    filename: String,
    content_type: String,
}

#[test]
fn chat_response_serialization() {
    let msg = ServerMessageContract::ChatResponse {
        session_id: Some(SessionId::new("s1")),
        content: "Hello!".to_string(),
        input_tokens: Some(10),
        output_tokens: Some(20),
    };

    let value = serde_json::to_value(&msg).expect("chat response should serialize");

    assert_eq!(value["type"], "chat_response");
    assert_eq!(value["content"], "Hello!");
    assert_eq!(value["session_id"], "s1");
    assert_eq!(value["input_tokens"], 10);
    assert_eq!(value["output_tokens"], 20);
}

#[test]
fn typing_indicator_serialization() {
    let value = serde_json::to_value(ServerMessageContract::Typing { agent: true })
        .expect("typing message should serialize");

    assert_eq!(value["type"], "typing");
    assert_eq!(value["agent"], true);
}

#[test]
fn error_message_serialization() {
    let value = serde_json::to_value(ServerMessageContract::Error {
        message: "boom".to_string(),
    })
    .expect("error message should serialize");

    assert_eq!(value["type"], "error");
    assert_eq!(value["message"], "boom");
}

#[test]
fn pong_serialization() {
    let value =
        serde_json::to_value(ServerMessageContract::Pong).expect("pong message should serialize");

    assert_eq!(value["type"], "pong");
}

#[test]
fn client_message_deserialization_chat() {
    let parsed: ClientMessageContract = serde_json::from_str(r#"{"type":"chat","message":"hi"}"#)
        .expect("chat message should deserialize");

    match parsed {
        ClientMessageContract::Chat {
            session_id,
            message,
            attachments,
        } => {
            assert_eq!(session_id, None);
            assert_eq!(message, "hi");
            assert_eq!(attachments, None);
        }
        ClientMessageContract::Typing { .. } | ClientMessageContract::Ping => {
            panic!("expected chat message")
        }
    }
}

#[test]
fn client_message_deserialization_ping() {
    let parsed: ClientMessageContract =
        serde_json::from_str(r#"{"type":"ping"}"#).expect("ping should deserialize");

    assert!(matches!(parsed, ClientMessageContract::Ping));
}

#[test]
fn client_message_deserialization_typing() {
    let parsed: ClientMessageContract =
        serde_json::from_str(r#"{"type":"typing"}"#).expect("typing should deserialize");

    match parsed {
        ClientMessageContract::Typing { session_id } => {
            assert_eq!(session_id, None);
        }
        ClientMessageContract::Chat { .. } | ClientMessageContract::Ping => {
            panic!("expected typing message")
        }
    }
}

#[test]
fn client_message_deserialization_chat_with_attachments() {
    let parsed: ClientMessageContract = serde_json::from_str(
        r#"{
            "type":"chat",
            "session_id":"s2",
            "message":"hi",
            "attachments":[{
                "upload_id":"u1",
                "filename":"note.txt",
                "content_type":"text/plain"
            }]
        }"#,
    )
    .expect("chat with attachment should deserialize");

    match parsed {
        ClientMessageContract::Chat {
            session_id,
            message,
            attachments,
        } => {
            assert_eq!(session_id, Some(SessionId::new("s2")));
            assert_eq!(message, "hi");
            let attachments = attachments.expect("attachments should be present");
            assert_eq!(attachments.len(), 1);
            assert_eq!(attachments[0].upload_id, "u1");
            assert_eq!(attachments[0].filename, "note.txt");
            assert_eq!(attachments[0].content_type, "text/plain");
        }
        ClientMessageContract::Typing { .. } | ClientMessageContract::Ping => {
            panic!("expected chat message")
        }
    }
}
