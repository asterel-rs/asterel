//! `Channel` trait implementation for iMessage: `AppleScript` send dispatch,
//! inbound polling via `osascript`, and contact allowlist checks.
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::pin::Pin;

use anyhow::Context;
use tokio::sync::mpsc;

use super::IMessageChannel;
use super::auth::{escape_applescript, is_valid_imessage_target};
use crate::contracts::ids::MessageId;
use crate::security::{ProcessSpawnClass, enforce_spawn_policy};
use crate::transport::channels::traits::{Channel, ChannelEvent, ChannelMessage};

const APPLESCRIPT_RECORD_SEPARATOR: char = '\u{001e}';
const APPLESCRIPT_FIELD_SEPARATOR: char = '\u{001f}';
const POLL_MESSAGES_PER_CHAT: usize = 20;
const SEEN_MESSAGE_CACHE_LIMIT: usize = 2_048;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PolledMessage {
    cursor: i64,
    message_id: MessageId,
    sender: String,
    text: String,
}

impl Channel for IMessageChannel {
    fn name(&self) -> &'static str {
        "imessage"
    }

    fn max_message_length(&self) -> usize {
        20_000
    }

    fn send<'a>(
        &'a self,
        message: &'a str,
        target: &'a str,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            // Defense-in-depth: validate target format before any interpolation
            if !is_valid_imessage_target(target) {
                anyhow::bail!(
                    "Invalid iMessage target: must be a phone number (+1234567890) or email (user@example.com)"
                );
            }

            // SECURITY: Escape both message AND target to prevent AppleScript injection
            // See: CWE-78 (OS Command Injection)
            let escaped_msg = escape_applescript(message);
            let escaped_target = escape_applescript(target);

            let script = format!(
                r#"tell application "Messages"
    set targetService to 1st account whose service type = iMessage
    set targetBuddy to participant "{escaped_target}" of targetService
    send "{escaped_msg}" to targetBuddy
end tell"#
            );

            enforce_spawn_policy(
                self.security.as_ref(),
                "osascript",
                "channels_imessage_send",
                ProcessSpawnClass::OperatorPlane,
            )?;

            let output = tokio::process::Command::new("osascript")
                .arg("-e")
                .arg(&script)
                .output()
                .await
                .context("run iMessage AppleScript command")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                anyhow::bail!("iMessage send failed: {stderr}");
            }

            Ok(())
        })
    }

    fn listen<'a>(
        &'a self,
        tx: mpsc::Sender<ChannelEvent>,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            tracing::info!("iMessage channel listening (AppleScript bridge)...");

            enforce_spawn_policy(
                self.security.as_ref(),
                "osascript",
                "channels_imessage_listen",
                ProcessSpawnClass::OperatorPlane,
            )?;

            let mut seen_ids = HashSet::with_capacity(SEEN_MESSAGE_CACHE_LIMIT);
            let mut seen_order = VecDeque::with_capacity(SEEN_MESSAGE_CACHE_LIMIT);

            // Seed cache with currently visible messages so startup does not replay historical traffic.
            let seeded_messages = fetch_new_messages(i64::MIN).await.unwrap_or_default();
            let mut last_rowid = seeded_messages.last().map_or(0, |msg| msg.cursor);
            for message in seeded_messages
                .into_iter()
                .rev()
                .take(SEEN_MESSAGE_CACHE_LIMIT)
                .rev()
            {
                register_seen_message(&mut seen_ids, &mut seen_order, message.message_id);
            }

            // Fallback path for empty bootstrap payloads.
            if last_rowid == 0 {
                last_rowid = get_max_rowid().await.unwrap_or(0);
            }

            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(self.poll_interval_secs)).await;

                // Include one-second overlap to avoid missing messages that share the same timestamp.
                let cursor_floor = last_rowid.saturating_sub(1);
                match fetch_new_messages(cursor_floor).await {
                    Ok(messages) => {
                        for message in messages {
                            if message.cursor > last_rowid {
                                last_rowid = message.cursor;
                            }

                            if !register_seen_message(
                                &mut seen_ids,
                                &mut seen_order,
                                message.message_id.clone(),
                            ) {
                                continue;
                            }

                            if !self.is_contact_allowed(&message.sender) {
                                continue;
                            }

                            if message.text.trim().is_empty() {
                                continue;
                            }

                            let msg = channel_message_from_polled_message(message);

                            if tx.send(ChannelEvent::Message(msg)).await.is_err() {
                                return Ok(());
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "iMessage poll error");
                    }
                }
            }
        })
    }

    fn health_check<'a>(&'a self) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>> {
        Box::pin(async move {
            if !cfg!(target_os = "macos") {
                return false;
            }

            Path::new("/usr/bin/osascript").exists()
        })
    }
}

fn channel_message_from_polled_message(message: PolledMessage) -> ChannelMessage {
    ChannelMessage {
        id: message.message_id.to_string(),
        sender: message.sender.clone(),
        content: message.text,
        channel: "imessage".to_string(),
        context_hint: None,
        conversation_id: Some(message.sender),
        thread_id: None,
        reply_to: None,
        message_id: Some(message.message_id.to_string()),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        attachments: Vec::new(),
    }
}

/// Query the most recent inbound message cursor from the `AppleScript` poll payload.
pub(super) async fn get_max_rowid() -> anyhow::Result<i64> {
    let messages = poll_recent_messages().await?;
    Ok(messages
        .into_iter()
        .map(|msg| msg.cursor)
        .max()
        .unwrap_or(0))
}

/// Fetch inbound messages newer than `since_rowid` using `AppleScript` polling output.
pub(super) async fn fetch_new_messages(since_rowid: i64) -> anyhow::Result<Vec<PolledMessage>> {
    let messages = poll_recent_messages().await?;
    Ok(messages
        .into_iter()
        .filter(|msg| msg.cursor > since_rowid)
        .collect())
}

async fn poll_recent_messages() -> anyhow::Result<Vec<PolledMessage>> {
    let script = build_poll_script(POLL_MESSAGES_PER_CHAT);
    let output = tokio::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .await
        .context("run iMessage AppleScript polling command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("iMessage polling failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_poll_payload(&stdout))
}

fn build_poll_script(limit_per_chat: usize) -> String {
    format!(
        r#"on sanitize_field(rawText, fieldDelimiter, recordDelimiter)
    set sanitized to rawText as text
    set AppleScript's text item delimiters to fieldDelimiter
    set sanitized to text items of sanitized
    set AppleScript's text item delimiters to " "
    set sanitized to sanitized as text
    set AppleScript's text item delimiters to recordDelimiter
    set sanitized to text items of sanitized
    set AppleScript's text item delimiters to " "
    set sanitized to sanitized as text
    set AppleScript's text item delimiters to ""
    return sanitized
end sanitize_field

tell application "Messages"
    set fieldDelimiter to ASCII character 31
    set recordDelimiter to ASCII character 30
    set outputRecords to {{}}
    set maxPerChat to {limit}
    repeat with c in chats
        try
            set chatMessages to messages of c
            set messageCount to count of chatMessages
            if messageCount > 0 then
                set startIndex to messageCount - maxPerChat + 1
                if startIndex < 1 then set startIndex to 1
                repeat with i from startIndex to messageCount
                    try
                        set m to item i of chatMessages
                        set msgText to text of m as text
                        if msgText is not "" then
                            set senderId to ""
                            try
                                set senderId to sender of m as text
                            end try
                            set fromMe to false
                            try
                                set fromMe to (is from me of m as boolean)
                            on error
                                try
                                    set fromMe to (from me of m as boolean)
                                end try
                            end try

                            if senderId is not "" and fromMe is false then
                                set cursor to 0
                                try
                                    set cursor to (time sent of m) as integer
                                end try
                                set msgId to ""
                                try
                                    set msgId to id of m as text
                                end try
                                set safeId to sanitize_field(msgId, fieldDelimiter, recordDelimiter)
                                set safeSender to sanitize_field(senderId, fieldDelimiter, recordDelimiter)
                                set safeText to sanitize_field(msgText, fieldDelimiter, recordDelimiter)
                                set end of outputRecords to (cursor as text) & fieldDelimiter & safeId & fieldDelimiter & safeSender & fieldDelimiter & safeText
                            end if
                        end if
                    end try
                end repeat
            end if
        end try
    end repeat

    if (count of outputRecords) is 0 then
        return ""
    end if

    set AppleScript's text item delimiters to recordDelimiter
    set payload to outputRecords as text
    set AppleScript's text item delimiters to ""
    return payload
end tell"#,
        limit = limit_per_chat.max(1),
    )
}

fn parse_poll_payload(raw_payload: &str) -> Vec<PolledMessage> {
    let trimmed = raw_payload.trim_matches(|ch| ch == '\n' || ch == '\r');
    if trimmed.is_empty() {
        return Vec::new();
    }

    let mut messages: Vec<PolledMessage> = trimmed
        .split(APPLESCRIPT_RECORD_SEPARATOR)
        .filter_map(parse_poll_record)
        .collect();

    messages.sort_by(|left, right| {
        left.cursor
            .cmp(&right.cursor)
            .then_with(|| left.message_id.cmp(&right.message_id))
    });
    messages
}

fn parse_poll_record(record: &str) -> Option<PolledMessage> {
    let mut fields = record.splitn(4, APPLESCRIPT_FIELD_SEPARATOR);
    let cursor = fields.next()?.trim().parse::<i64>().ok()?;
    let raw_message_id = fields.next().unwrap_or_default().trim();
    let sender = fields.next()?.trim().to_string();
    let text = fields.next().unwrap_or_default().to_string();

    if sender.is_empty() {
        return None;
    }

    let message_id = if raw_message_id.is_empty() {
        fallback_message_id(cursor, &sender, &text)
    } else {
        MessageId::new(raw_message_id)
    };

    Some(PolledMessage {
        cursor,
        message_id,
        sender,
        text,
    })
}

fn fallback_message_id(cursor: i64, sender: &str, text: &str) -> MessageId {
    let mut hasher = DefaultHasher::new();
    sender.hash(&mut hasher);
    text.hash(&mut hasher);
    let digest = hasher.finish();
    MessageId::new(format!("imessage-{cursor}-{digest:016x}"))
}

fn register_seen_message(
    seen_ids: &mut HashSet<MessageId>,
    seen_order: &mut VecDeque<MessageId>,
    message_id: MessageId,
) -> bool {
    if !seen_ids.insert(message_id.clone()) {
        return false;
    }

    seen_order.push_back(message_id);
    while seen_order.len() > SEEN_MESSAGE_CACHE_LIMIT {
        if let Some(oldest) = seen_order.pop_front() {
            seen_ids.remove(&oldest);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_poll_payload_parses_and_sorts_records() {
        let payload = format!(
            "120{APPLESCRIPT_FIELD_SEPARATOR}msg-b{APPLESCRIPT_FIELD_SEPARATOR}+15550002\
             {APPLESCRIPT_FIELD_SEPARATOR}second{APPLESCRIPT_RECORD_SEPARATOR}110\
             {APPLESCRIPT_FIELD_SEPARATOR}msg-a{APPLESCRIPT_FIELD_SEPARATOR}+15550001\
             {APPLESCRIPT_FIELD_SEPARATOR}first"
        );

        let parsed = parse_poll_payload(&payload);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].cursor, 110);
        assert_eq!(parsed[0].message_id, MessageId::new("msg-a"));
        assert_eq!(parsed[1].cursor, 120);
        assert_eq!(parsed[1].message_id, MessageId::new("msg-b"));
    }

    #[test]
    fn parse_poll_payload_skips_invalid_rows() {
        let payload = format!(
            "bad{APPLESCRIPT_FIELD_SEPARATOR}x{APPLESCRIPT_FIELD_SEPARATOR}+15550001\
             {APPLESCRIPT_FIELD_SEPARATOR}text{APPLESCRIPT_RECORD_SEPARATOR}130\
             {APPLESCRIPT_FIELD_SEPARATOR}msg-x{APPLESCRIPT_FIELD_SEPARATOR}+15550001\
             {APPLESCRIPT_FIELD_SEPARATOR}ok"
        );

        let parsed = parse_poll_payload(&payload);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].cursor, 130);
    }

    #[test]
    fn parse_poll_record_generates_fallback_id_when_missing() {
        let record = format!(
            "140{APPLESCRIPT_FIELD_SEPARATOR}{APPLESCRIPT_FIELD_SEPARATOR}+15550001\
             {APPLESCRIPT_FIELD_SEPARATOR}hello"
        );

        let parsed = parse_poll_record(&record).expect("record should parse");
        assert!(parsed.message_id.as_str().starts_with("imessage-140-"));
    }

    #[test]
    fn channel_message_from_polled_message_preserves_identity() {
        let message = PolledMessage {
            cursor: 150,
            message_id: MessageId::new("msg-identity"),
            sender: "+15550001".to_string(),
            text: "hello".to_string(),
        };

        let channel_message = channel_message_from_polled_message(message);

        assert_eq!(channel_message.id, "msg-identity");
        assert_eq!(channel_message.sender, "+15550001");
        assert_eq!(
            channel_message.conversation_id.as_deref(),
            Some("+15550001")
        );
        assert_eq!(channel_message.message_id.as_deref(), Some("msg-identity"));
    }

    #[test]
    fn register_seen_message_deduplicates() {
        let mut seen_ids = HashSet::new();
        let mut seen_order = VecDeque::new();

        assert!(register_seen_message(
            &mut seen_ids,
            &mut seen_order,
            MessageId::new("msg-1")
        ));
        assert!(!register_seen_message(
            &mut seen_ids,
            &mut seen_order,
            MessageId::new("msg-1")
        ));
    }

    #[test]
    fn build_poll_script_embeds_limit() {
        let script = build_poll_script(7);
        assert!(script.contains("set maxPerChat to 7"));
    }
}
