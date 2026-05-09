use crate::core::providers::ThinkingLevel;
use crate::core::sessions::types::{Session, SessionMetadata};

pub(super) fn with_thinking_level_metadata(
    session: Session,
    level: ThinkingLevel,
) -> SessionMetadata {
    let mut metadata = session.metadata.unwrap_or_default();
    metadata.thinking_level = Some(level);
    metadata
}

pub(super) fn extract_thinking_level(session: &Session) -> Option<ThinkingLevel> {
    session
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.thinking_level)
}

pub(super) fn cleared_thinking_level_metadata(session: Session) -> Option<SessionMetadata> {
    let mut metadata = session.metadata?;
    metadata.thinking_level = None;
    Some(metadata)
}
