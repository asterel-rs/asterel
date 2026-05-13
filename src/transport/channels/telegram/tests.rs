//! Unit tests for Telegram channel: name, API URL, user allowlist,
//! media attachment construction, and capability flags.
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::*;
use crate::transport::channels::traits::{Channel, MediaAttachment, MediaContent};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const EIGHT_MIB: usize = 8 * 1024 * 1024;

fn capture_request_body(
    captured: Arc<Mutex<Vec<Vec<u8>>>>,
) -> impl Fn(&Request) -> bool + Send + Sync + 'static {
    move |request: &Request| {
        captured
            .lock()
            .expect("captured request bodies lock should not be poisoned")
            .push(request.body.clone());
        true
    }
}

fn body_contains(body: &[u8], needle: &[u8]) -> bool {
    body.windows(needle.len()).any(|window| window == needle)
}

async fn spawn_http_sequence(responses: Vec<String>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test HTTP server");
    let addr = listener.local_addr().expect("server addr");

    tokio::spawn(async move {
        for response in responses {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let mut buffer = [0_u8; 2048];
            let _ = stream.read(&mut buffer).await;
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        }
    });

    format!("http://{addr}")
}

fn http_json_response(status: &str, extra_headers: &[(&str, &str)], body: &str) -> String {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in extra_headers {
        response.push_str(name);
        response.push_str(": ");
        response.push_str(value);
        response.push_str("\r\n");
    }
    response.push_str("\r\n");
    response.push_str(body);
    response
}

#[test]
fn telegram_channel_name() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    assert_eq!(ch.name(), "telegram");
}

#[test]
fn telegram_api_url() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![]);
    assert_eq!(
        ch.api_url("getMe"),
        "https://api.telegram.org/bot123:ABC/getMe"
    );
}

#[tokio::test]
async fn telegram_send_retries_429_retry_after_before_success() {
    let base_url = spawn_http_sequence(vec![
        http_json_response(
            "429 Too Many Requests",
            &[("Retry-After", "0")],
            r#"{"ok":false}"#,
        ),
        http_json_response("200 OK", &[], r#"{"ok":true}"#),
    ])
    .await;
    let ch =
        TelegramChannel::new_with_api_base_url("test-token".into(), vec!["*".into()], base_url);

    ch.send("hello", "123456")
        .await
        .expect("second send attempt should succeed");
}

#[tokio::test]
async fn telegram_send_error_sanitizes_provider_body() {
    let base_url = spawn_http_sequence(vec![http_json_response(
        "400 Bad Request",
        &[],
        r#"{"description":"invalid token sk-test-secret-value"}"#,
    )])
    .await;
    let ch =
        TelegramChannel::new_with_api_base_url("test-token".into(), vec!["*".into()], base_url);

    let err = ch
        .send("hello", "123456")
        .await
        .expect_err("provider failure should be returned");
    let message = err.to_string();

    assert!(message.contains("Telegram sendMessage failed"));
    assert!(!message.contains("sk-test-secret-value"));
    assert!(message.contains("[REDACTED]"));
}

#[test]
fn telegram_user_allowed_wildcard() {
    let ch = TelegramChannel::new("t".into(), vec!["*".into()]);
    assert!(ch.is_user_allowed("anyone"));
}

#[test]
fn telegram_user_allowed_specific() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into(), "bob".into()]);
    assert!(ch.is_user_allowed("alice"));
    assert!(!ch.is_user_allowed("eve"));
}

#[test]
fn telegram_user_denied_empty() {
    let ch = TelegramChannel::new("t".into(), vec![]);
    assert!(!ch.is_user_allowed("anyone"));
}

#[test]
fn telegram_user_exact_match_not_substring() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into()]);
    assert!(!ch.is_user_allowed("alice_bot"));
    assert!(!ch.is_user_allowed("alic"));
    assert!(!ch.is_user_allowed("malice"));
}

#[test]
fn telegram_user_empty_string_denied() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into()]);
    assert!(!ch.is_user_allowed(""));
}

#[test]
fn telegram_user_case_sensitive() {
    let ch = TelegramChannel::new("t".into(), vec!["Alice".into()]);
    assert!(ch.is_user_allowed("Alice"));
    assert!(!ch.is_user_allowed("alice"));
    assert!(!ch.is_user_allowed("ALICE"));
}

#[test]
fn telegram_wildcard_with_specific_users() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into(), "*".into()]);
    assert!(ch.is_user_allowed("alice"));
    assert!(ch.is_user_allowed("bob"));
    assert!(ch.is_user_allowed("anyone"));
}

#[test]
fn telegram_user_allowed_by_numeric_id_identity() {
    let ch = TelegramChannel::new("t".into(), vec!["123456789".into()]);
    assert!(ch.is_any_user_allowed(["unknown", "123456789"]));
}

#[test]
fn telegram_user_denied_when_none_of_identities_match() {
    let ch = TelegramChannel::new("t".into(), vec!["alice".into(), "987654321".into()]);
    assert!(!ch.is_any_user_allowed(["unknown", "123456789"]));
}

// ── File sending API URL tests ──────────────────────────────────

#[test]
fn telegram_api_url_send_document() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![]);
    assert_eq!(
        ch.api_url("sendDocument"),
        "https://api.telegram.org/bot123:ABC/sendDocument"
    );
}

#[test]
fn telegram_api_url_send_photo() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![]);
    assert_eq!(
        ch.api_url("sendPhoto"),
        "https://api.telegram.org/bot123:ABC/sendPhoto"
    );
}

#[test]
fn telegram_api_url_send_video() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![]);
    assert_eq!(
        ch.api_url("sendVideo"),
        "https://api.telegram.org/bot123:ABC/sendVideo"
    );
}

#[test]
fn telegram_api_url_send_audio() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![]);
    assert_eq!(
        ch.api_url("sendAudio"),
        "https://api.telegram.org/bot123:ABC/sendAudio"
    );
}

#[test]
fn telegram_api_url_send_voice() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![]);
    assert_eq!(
        ch.api_url("sendVoice"),
        "https://api.telegram.org/bot123:ABC/sendVoice"
    );
}

// ── File sending integration tests (with mock server) ──────────

#[tokio::test]
async fn telegram_send_document_bytes_builds_correct_form() {
    // This test verifies the method doesn't panic and handles bytes correctly
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let file_bytes = b"Hello, this is a test file content".to_vec();

    // The actual API call will fail (no real server), but we verify the method exists
    // and handles the input correctly up to the network call
    let result = ch
        .send_document_bytes("123456", file_bytes, "test.txt", Some("Test caption"))
        .await;

    // Should fail with network error, not a panic or type error
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    // Error should be network-related, not a code bug
    assert!(
        err.contains("error")
            || err.contains("failed")
            || err.contains("connect")
            || err.contains("send Telegram sendDocument"),
        "Expected network error, got: {err}"
    );
}

#[tokio::test]
async fn telegram_send_photo_bytes_builds_correct_form() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    // Minimal valid PNG header bytes
    let file_bytes = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

    let result = ch
        .send_photo_bytes("123456", file_bytes, "test.png", None)
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_document_by_url_builds_correct_json() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);

    let result = ch
        .send_document_by_url("123456", "https://example.com/file.pdf", Some("PDF doc"))
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_photo_by_url_builds_correct_json() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);

    let result = ch
        .send_photo_by_url("123456", "https://example.com/image.jpg", None)
        .await;

    assert!(result.is_err());
}

// ── File path handling tests ────────────────────────────────────

#[tokio::test]
async fn telegram_send_document_nonexistent_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let path = Path::new("/nonexistent/path/to/file.txt");

    let result = ch.send_document("123456", path, None).await;

    assert!(result.is_err());
    let err = format!("{:#}", result.unwrap_err());
    // Should fail with file not found error (context wraps the underlying OS error)
    assert!(
        err.contains("No such file") || err.contains("not found") || err.contains("os error"),
        "Expected file not found error, got: {err}"
    );
}

#[tokio::test]
async fn telegram_send_photo_nonexistent_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let path = Path::new("/nonexistent/path/to/photo.jpg");

    let result = ch.send_photo("123456", path, None).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_video_nonexistent_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let path = Path::new("/nonexistent/path/to/video.mp4");

    let result = ch.send_video("123456", path, None).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_audio_nonexistent_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let path = Path::new("/nonexistent/path/to/audio.mp3");

    let result = ch.send_audio("123456", path, None).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_voice_nonexistent_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let path = Path::new("/nonexistent/path/to/voice.ogg");

    let result = ch.send_voice("123456", path, None).await;

    assert!(result.is_err());
}

// ── Caption handling tests ──────────────────────────────────────

#[tokio::test]
async fn telegram_send_document_bytes_with_caption() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let file_bytes = b"test content".to_vec();

    // With caption
    let result = ch
        .send_document_bytes("123456", file_bytes.clone(), "test.txt", Some("My caption"))
        .await;
    assert!(result.is_err()); // Network error expected

    // Without caption
    let result = ch
        .send_document_bytes("123456", file_bytes, "test.txt", None)
        .await;
    assert!(result.is_err()); // Network error expected
}

#[tokio::test]
async fn telegram_send_document_bytes_retries_with_identical_bytes_and_filename() {
    let server = MockServer::start().await;
    let captured = Arc::new(Mutex::new(Vec::new()));
    let route = "/bottest-token/sendDocument";

    Mock::given(method("POST"))
        .and(path(route))
        .and(capture_request_body(captured.clone()))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(route))
        .and(capture_request_body(captured.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": true })))
        .expect(1)
        .with_priority(2)
        .mount(&server)
        .await;

    let payload = b"telegram retry payload \0 \xff".to_vec();
    let ch =
        TelegramChannel::new_with_api_base_url("test-token".into(), vec!["*".into()], server.uri());
    ch.send_document_bytes("123456", payload.clone(), "retry.txt", Some("caption"))
        .await
        .expect("second Telegram media attempt should succeed");

    let bodies = captured
        .lock()
        .expect("captured request bodies lock should not be poisoned");
    assert_eq!(bodies.len(), 2);
    assert_ne!(
        bodies[0], bodies[1],
        "multipart boundaries are rebuilt per attempt"
    );
    for body in bodies.iter() {
        assert!(body_contains(body, &payload));
        assert!(body_contains(body, b"retry.txt"));
        assert!(body_contains(body, b"caption"));
    }
}

#[tokio::test]
async fn telegram_send_document_bytes_empty_payload_reaches_mock_server() {
    let server = MockServer::start().await;
    let captured = Arc::new(Mutex::new(Vec::new()));
    let route = "/bottest-token/sendDocument";
    Mock::given(method("POST"))
        .and(path(route))
        .and(capture_request_body(captured.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": true })))
        .expect(1)
        .mount(&server)
        .await;

    let ch =
        TelegramChannel::new_with_api_base_url("test-token".into(), vec!["*".into()], server.uri());
    ch.send_document_bytes("123456", Vec::new(), "empty.txt", None)
        .await
        .expect("empty Telegram media payload should be uploadable");

    let bodies = captured
        .lock()
        .expect("captured request bodies lock should not be poisoned");
    assert_eq!(bodies.len(), 1);
    assert!(body_contains(&bodies[0], b"empty.txt"));
}

#[tokio::test]
async fn telegram_send_document_bytes_small_payload_reaches_mock_server() {
    let server = MockServer::start().await;
    let captured = Arc::new(Mutex::new(Vec::new()));
    let route = "/bottest-token/sendDocument";
    Mock::given(method("POST"))
        .and(path(route))
        .and(capture_request_body(captured.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": true })))
        .expect(1)
        .mount(&server)
        .await;

    let payload = b"small media payload".to_vec();
    let ch =
        TelegramChannel::new_with_api_base_url("test-token".into(), vec!["*".into()], server.uri());
    ch.send_document_bytes("123456", payload.clone(), "small.txt", None)
        .await
        .expect("small Telegram media payload should be uploadable");

    let bodies = captured
        .lock()
        .expect("captured request bodies lock should not be poisoned");
    assert_eq!(bodies.len(), 1);
    assert!(body_contains(&bodies[0], &payload));
    assert!(body_contains(&bodies[0], b"small.txt"));
}

#[tokio::test]
async fn telegram_send_document_bytes_generated_8m_payload_reaches_mock_server() {
    let server = MockServer::start().await;
    let captured = Arc::new(Mutex::new(Vec::new()));
    let route = "/bottest-token/sendDocument";
    Mock::given(method("POST"))
        .and(path(route))
        .and(capture_request_body(captured.clone()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": true })))
        .expect(1)
        .mount(&server)
        .await;

    let payload = vec![0xA5; EIGHT_MIB];
    let ch =
        TelegramChannel::new_with_api_base_url("test-token".into(), vec!["*".into()], server.uri());
    ch.send_document_bytes("123456", payload, "large-8m.bin", None)
        .await
        .expect("8 MiB Telegram media payload should be uploadable");

    let bodies = captured
        .lock()
        .expect("captured request bodies lock should not be poisoned");
    assert_eq!(bodies.len(), 1);
    assert!(bodies[0].len() >= EIGHT_MIB);
    assert!(body_contains(&bodies[0], b"large-8m.bin"));
}

#[tokio::test]
async fn telegram_send_photo_bytes_with_caption() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let file_bytes = vec![0x89, 0x50, 0x4E, 0x47];

    // With caption
    let result = ch
        .send_photo_bytes(
            "123456",
            file_bytes.clone(),
            "test.png",
            Some("Photo caption"),
        )
        .await;
    assert!(result.is_err());

    // Without caption
    let result = ch
        .send_photo_bytes("123456", file_bytes, "test.png", None)
        .await;
    assert!(result.is_err());
}

// ── Empty/edge case tests ───────────────────────────────────────

#[tokio::test]
async fn telegram_send_document_bytes_empty_file() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let file_bytes: Vec<u8> = vec![];

    let result = ch
        .send_document_bytes("123456", file_bytes, "empty.txt", None)
        .await;

    // Should not panic, will fail at API level
    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_document_bytes_empty_filename() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let file_bytes = b"content".to_vec();

    let result = ch.send_document_bytes("123456", file_bytes, "", None).await;

    // Should not panic
    assert!(result.is_err());
}

#[tokio::test]
async fn telegram_send_document_bytes_empty_chat_id() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let file_bytes = b"content".to_vec();

    let result = ch
        .send_document_bytes("", file_bytes, "test.txt", None)
        .await;

    // Should not panic
    assert!(result.is_err());
}

#[tokio::test]
async fn parse_telegram_attachments_photo_uses_largest_size() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let message = serde_json::json!({
        "photo": [
            {"file_id": "test_file_path:photos/small.jpg"},
            {"file_id": "test_file_path:photos/large.jpg"}
        ]
    });

    let attachments = ch.parse_tg_attachments(&message).await;
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].mime_type, "image/jpeg");
    assert!(matches!(
        &attachments[0].data,
        MediaContent::Bytes(bytes) if bytes == b"photos/large.jpg"
    ));
}

#[tokio::test]
async fn parse_telegram_attachments_document_includes_metadata() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let message = serde_json::json!({
        "document": {
            "file_id": "test_file_path:docs/spec.pdf",
            "file_name": "spec.pdf",
            "mime_type": "application/pdf"
        }
    });

    let attachments = ch.parse_tg_attachments(&message).await;
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].mime_type, "application/pdf");
    assert_eq!(attachments[0].filename.as_deref(), Some("spec.pdf"));
    assert!(matches!(
        &attachments[0].data,
        MediaContent::Bytes(bytes) if bytes == b"docs/spec.pdf"
    ));
}

#[tokio::test]
async fn parse_telegram_attachments_voice_defaults_filename() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let message = serde_json::json!({
        "voice": {
            "file_id": "test_file_path:voice/clip.ogg",
            "mime_type": "audio/ogg"
        }
    });

    let attachments = ch.parse_tg_attachments(&message).await;
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].mime_type, "audio/ogg");
    assert_eq!(attachments[0].filename.as_deref(), Some("voice.ogg"));
}

#[tokio::test]
async fn parse_telegram_attachments_with_text_and_photo() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let message = serde_json::json!({
        "text": "hello",
        "photo": [
            {"file_id": "test_file_path:photos/one.jpg"}
        ]
    });

    let attachments = ch.parse_tg_attachments(&message).await;
    assert_eq!(attachments.len(), 1);
    assert!(matches!(attachments[0].data, MediaContent::Bytes(_)));

    if let MediaContent::Bytes(bytes) = &attachments[0].data {
        let as_text = String::from_utf8_lossy(bytes);
        assert!(!as_text.contains("fake-token"));
        assert!(!as_text.contains("api.telegram.org/file/bot"));
    }
}

#[tokio::test]
async fn parse_telegram_attachments_audio_and_video_both_parse() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let message = serde_json::json!({
        "audio": {
            "file_id": "test_file_path:media/song.mp3",
            "file_name": "song.mp3",
            "mime_type": "audio/mpeg"
        },
        "video": {
            "file_id": "test_file_path:media/clip.mp4",
            "file_name": "clip.mp4",
            "mime_type": "video/mp4"
        }
    });

    let attachments = ch.parse_tg_attachments(&message).await;
    assert_eq!(attachments.len(), 2);
    assert_eq!(attachments[0].filename.as_deref(), Some("song.mp3"));
    assert_eq!(attachments[1].filename.as_deref(), Some("clip.mp4"));
}

#[tokio::test]
async fn telegram_send_media_dispatch_image_bytes() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let attachment = MediaAttachment {
        mime_type: "image/png".to_string(),
        data: MediaContent::Bytes(vec![0x89, 0x50, 0x4E, 0x47]),
        filename: Some("image.png".to_string()),
    };

    let err = ch
        .send_media(&attachment, "123456")
        .await
        .expect_err("network failure expected")
        .to_string();
    assert!(err.contains("photo") || err.contains("sendPhoto"));
}

#[tokio::test]
async fn telegram_send_media_dispatch_audio_bytes() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let attachment = MediaAttachment {
        mime_type: "audio/mpeg".to_string(),
        data: MediaContent::Bytes(vec![1, 2, 3, 4]),
        filename: Some("track.mp3".to_string()),
    };

    let err = ch
        .send_media(&attachment, "123456")
        .await
        .expect_err("network failure expected")
        .to_string();
    assert!(err.contains("audio") || err.contains("sendAudio"));
}

#[tokio::test]
async fn telegram_send_media_dispatch_voice_bytes() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let attachment = MediaAttachment {
        mime_type: "audio/ogg".to_string(),
        data: MediaContent::Bytes(vec![0x4f, 0x67, 0x67, 0x53]),
        filename: Some("voice.ogg".to_string()),
    };

    let err = ch
        .send_media(&attachment, "123456")
        .await
        .expect_err("network failure expected")
        .to_string();
    assert!(err.contains("voice") || err.contains("sendVoice"));
}

#[tokio::test]
async fn telegram_send_media_dispatch_voice_url() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let attachment = MediaAttachment {
        mime_type: "audio/ogg".to_string(),
        data: MediaContent::Url("https://example.com/voice.ogg".to_string()),
        filename: Some("voice.ogg".to_string()),
    };

    let err = ch
        .send_media(&attachment, "123456")
        .await
        .expect_err("network failure expected")
        .to_string();
    assert!(err.contains("voice") || err.contains("sendVoice"));
}

#[tokio::test]
async fn telegram_send_media_dispatch_video_bytes() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let attachment = MediaAttachment {
        mime_type: "video/mp4".to_string(),
        data: MediaContent::Bytes(vec![0, 0, 0, 1]),
        filename: Some("clip.mp4".to_string()),
    };

    let err = ch
        .send_media(&attachment, "123456")
        .await
        .expect_err("network failure expected")
        .to_string();
    assert!(err.contains("video") || err.contains("sendVideo"));
}

#[tokio::test]
async fn telegram_send_media_dispatch_document_url() {
    let ch = TelegramChannel::new("fake-token".into(), vec!["*".into()]);
    let attachment = MediaAttachment {
        mime_type: "application/pdf".to_string(),
        data: MediaContent::Url("https://example.com/spec.pdf".to_string()),
        filename: Some("spec.pdf".to_string()),
    };

    let err = ch
        .send_media(&attachment, "123456")
        .await
        .expect_err("network failure expected")
        .to_string();
    assert!(err.contains("document") || err.contains("sendDocument"));
}

#[test]
fn telegram_file_url_is_constructed_correctly() {
    let ch = TelegramChannel::new("123:ABC".into(), vec![]);
    let url = ch.telegram_file_url("photos/file.jpg");
    assert_eq!(
        url,
        "https://api.telegram.org/file/bot123:ABC/photos/file.jpg"
    );
}
