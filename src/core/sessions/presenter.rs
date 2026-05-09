use std::fmt::Write as _;

use super::types::{MessageRole, SessionTranscriptReadModel, TranscriptMessage};
use crate::utils::text::truncate_ellipsis;

#[must_use]
pub(crate) fn render_message_for_history(message: &TranscriptMessage, max_chars: usize) -> String {
    let mut rendered = String::new();
    let role = match message.message.role {
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant",
        MessageRole::System => "system",
    };
    let _ = write!(rendered, "{role}: ");

    if message.parts.is_empty() {
        rendered.push_str(&truncate_ellipsis(&message.message.content, max_chars));
        return rendered;
    }

    let mut joined = String::new();
    for part in &message.parts {
        let rendered_part = super::types::ChatMessagePart::render_for_history(part);
        if rendered_part.is_empty() {
            continue;
        }
        if !joined.is_empty() {
            joined.push_str(" | ");
        }
        joined.push_str(&rendered_part);
    }
    rendered.push_str(&truncate_ellipsis(&joined, max_chars));
    rendered
}

#[must_use]
pub(crate) fn render_history_fragment(
    read_model: &SessionTranscriptReadModel,
    max_chars: usize,
) -> String {
    if read_model.messages.is_empty() || max_chars == 0 {
        return String::new();
    }

    let mut rendered = String::from("[History]\n");
    for message in &read_model.messages {
        let _ = writeln!(rendered, "- {}", render_message_for_history(message, 240));
    }
    truncate_ellipsis(&rendered, max_chars)
}
