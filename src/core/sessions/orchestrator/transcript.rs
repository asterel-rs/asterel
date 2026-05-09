use crate::contracts::ids::SessionId;
use crate::core::sessions::types::{
    ChatMessage, ChatMessagePart, ChatMessagePartInput, MessageRole, SessionTranscriptReadModel,
    TranscriptMessage, estimate_tokens,
};

pub(super) fn part_to_input(part: &ChatMessagePart) -> ChatMessagePartInput {
    let mut input = ChatMessagePartInput::new(part.kind, part.content.clone());
    if let Some(mime_type) = part.mime_type.as_deref() {
        input = input.with_mime_type(mime_type);
    }
    if let Some(metadata) = part.metadata.clone() {
        input = input.with_metadata(metadata);
    }
    input
}

pub(super) fn single_part(role: MessageRole, content: &str) -> [ChatMessagePartInput; 1] {
    [ChatMessagePartInput::new(
        ChatMessage::default_part_kind_for_role(role),
        content,
    )]
}

pub(super) fn resolved_token_count(content: &str, explicit: Option<u64>) -> Option<u64> {
    explicit.or_else(|| {
        #[allow(clippy::cast_possible_truncation)]
        {
            u64::try_from(estimate_tokens(content)).ok()
        }
    })
}

pub(super) fn build_transcript_read_model(
    session_id: &SessionId,
    transcript: Vec<TranscriptMessage>,
    max_tokens: Option<usize>,
) -> SessionTranscriptReadModel {
    let read_model = SessionTranscriptReadModel::new(session_id.clone(), transcript);
    match max_tokens {
        Some(limit) => read_model.tail_within_token_limit(limit),
        None => read_model,
    }
}
