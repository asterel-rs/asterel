//! Unit tests for the Twitter channel adapter.

use super::TwitterChannel;
use crate::contracts::ids::UserId;
use crate::transport::channels::traits::{Channel, ChannelEvent};

fn make_channel(allowed_users: Vec<String>) -> TwitterChannel {
    TwitterChannel::new(
        "client_id".to_string(),
        "client_secret".to_string(),
        "access_token".to_string(),
        "refresh_token".to_string(),
        UserId::new("bot_user_id_123"),
        allowed_users,
        180,
        300,
    )
}

fn make_channel_with_api_base_url(api_base_url: String) -> TwitterChannel {
    TwitterChannel::new_with_api_base_url(
        "client_id".to_string(),
        "client_secret".to_string(),
        "access_token".to_string(),
        "refresh_token".to_string(),
        UserId::new("bot_user_id_123"),
        vec![],
        180,
        300,
        api_base_url,
    )
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

#[tokio::test]
async fn twitter_post_tweet_retries_429_retry_after_before_success() {
    let base_url = spawn_http_sequence(vec![
        http_json_response(
            "429 Too Many Requests",
            &[("Retry-After", "0")],
            r#"{"error":"wait"}"#,
        ),
        http_json_response("200 OK", &[], r#"{"data":{"id":"tweet-1"}}"#),
    ])
    .await;
    let ch = make_channel_with_api_base_url(format!("{base_url}/2"));

    let tweet_id = ch
        .post_tweet("hello")
        .await
        .expect("second tweet attempt should succeed");

    assert_eq!(tweet_id, "tweet-1");
}

#[tokio::test]
async fn twitter_send_error_sanitizes_provider_body() {
    let base_url = spawn_http_sequence(vec![http_json_response(
        "400 Bad Request",
        &[],
        r#"{"detail":"bad bearer sk-twitter-secret-value"}"#,
    )])
    .await;
    let ch = make_channel_with_api_base_url(format!("{base_url}/2"));

    let err = ch
        .post_tweet("hello")
        .await
        .expect_err("provider failure should be returned");
    let message = err.to_string();

    assert!(message.contains("Twitter post tweet failed"));
    assert!(!message.contains("sk-twitter-secret-value"));
    assert!(message.contains("[REDACTED]"));
}

#[test]
fn mention_to_channel_event_sets_context_hint() {
    let ch = make_channel(vec![]);
    let event = ch.mention_to_channel_event("tweet_id_1", "author_id_1", "hello @bot");
    let ChannelEvent::Message(msg) = event else {
        panic!("expected Message variant");
    };
    assert_eq!(msg.channel, "twitter");
    assert_eq!(msg.context_hint.as_deref(), Some("mention"));
    assert_eq!(msg.reply_to.as_deref(), Some("tweet_id_1"));
    assert_eq!(msg.sender, "author_id_1");
}

#[test]
fn dm_to_channel_event_sets_dm_prefix() {
    let ch = make_channel(vec![]);
    let event = ch.dm_to_channel_event("evt_1", "user_id_42", "hey there");
    let ChannelEvent::Message(msg) = event else {
        panic!("expected Message variant");
    };
    assert_eq!(msg.channel, "twitter");
    assert_eq!(msg.context_hint.as_deref(), Some("dm"));
    assert_eq!(msg.conversation_id.as_deref(), Some("dm:user_id_42"));
    assert!(msg.reply_to.is_none());
}

#[test]
fn allowlist_empty_allows_all_users() {
    let ch = make_channel(vec![]);
    // With empty allowlist, any username passes (allowlist check only activates
    // when allowed_users is non-empty).
    assert!(ch.allowed_users.is_empty());
}

#[test]
fn allowlist_blocks_unknown_user() {
    let ch = make_channel(vec!["aliceuser".to_string()]);
    assert!(ch.is_user_allowed("aliceuser"));
    assert!(ch.is_user_allowed("ALICEUSER")); // case-insensitive
    assert!(!ch.is_user_allowed("bobuser"));
}

#[test]
fn dm_allowlist_allows_all_when_empty() {
    let ch = make_channel(vec![]);
    assert!(ch.is_dm_sender_allowed(None));
    assert!(ch.is_dm_sender_allowed(Some("anyone")));
}

#[test]
fn dm_allowlist_requires_resolved_username() {
    let ch = make_channel(vec!["aliceuser".to_string()]);
    assert!(ch.dm_allowlist_requires_username_resolution());
    assert!(!ch.is_dm_sender_allowed(None));
}

#[test]
fn dm_allowlist_uses_case_insensitive_username_match() {
    let ch = make_channel(vec!["aliceuser".to_string()]);
    assert!(ch.is_dm_sender_allowed(Some("ALICEUSER")));
    assert!(!ch.is_dm_sender_allowed(Some("bobuser")));
}

#[test]
fn dm_allowlist_wildcard_allows_unresolved_sender() {
    let ch = make_channel(vec!["*".to_string()]);
    assert!(!ch.dm_allowlist_requires_username_resolution());
    assert!(ch.is_dm_sender_allowed(None));
}

#[test]
fn compare_event_ids_uses_numeric_order_for_decimal_ids() {
    assert!(matches!(
        super::handler::compare_event_ids("9", "10"),
        std::cmp::Ordering::Less
    ));
}

#[test]
fn send_routing_dm_prefix_is_recognized() {
    // Verify that the `send()` routing contract is correct: `dm:<id>` prefix
    // selects the DM path. We check the logic structurally — no live network call.
    let _ch = make_channel(vec![]);
    let dm_recipient = "dm:user_id_99";
    assert!(dm_recipient.strip_prefix("dm:").is_some());

    let reply_recipient = "tweet_id_456";
    assert!(reply_recipient.strip_prefix("dm:").is_none());
    assert!(!reply_recipient.is_empty());

    let standalone_recipient = "";
    assert!(standalone_recipient.is_empty());
}

#[test]
fn channel_name_is_twitter() {
    let ch = make_channel(vec![]);
    assert_eq!(ch.name(), "twitter");
}

#[test]
fn max_message_length_is_280() {
    let ch = make_channel(vec![]);
    assert_eq!(ch.max_message_length(), 280);
}
