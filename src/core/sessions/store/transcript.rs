use std::collections::HashMap;

use anyhow::Result;

use crate::contracts::ids::MessageId;
use crate::core::sessions::types::{
    ChatMessage, ChatMessagePart, ChatMessagePartInput, MessagePartKind, TranscriptMessage,
};

pub(super) fn assemble_transcript(
    messages: Vec<ChatMessage>,
    mut parts_by_message: HashMap<MessageId, Vec<ChatMessagePart>>,
) -> Vec<TranscriptMessage> {
    messages
        .into_iter()
        .map(|message| TranscriptMessage {
            parts: parts_by_message.remove(&message.id).unwrap_or_default(),
            message,
        })
        .collect()
}

pub(super) fn flatten_message_parts(parts: &[ChatMessagePartInput]) -> String {
    let mut result = String::new();
    for part in parts {
        if part.kind == MessagePartKind::Reasoning {
            continue;
        }
        let content = part.content.trim();
        if content.is_empty() {
            continue;
        }
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(content);
    }
    result
}

pub(super) fn tail_messages_within_token_limit(
    messages: &[ChatMessage],
    max_tokens: usize,
) -> Vec<ChatMessage> {
    if max_tokens == 0 || messages.is_empty() {
        return Vec::new();
    }

    let mut total = 0usize;
    let mut selected = Vec::new();
    for message in messages.iter().rev() {
        let tokens = message.estimated_tokens();
        if !selected.is_empty() && total.saturating_add(tokens) > max_tokens {
            break;
        }
        total = total.saturating_add(tokens);
        selected.push(message.clone());
    }
    selected.reverse();
    selected
}

pub(super) fn i64_to_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|error| anyhow::anyhow!("token conversion failed: {error}"))
}

pub(super) fn u64_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|error| anyhow::anyhow!("token conversion failed: {error}"))
}
