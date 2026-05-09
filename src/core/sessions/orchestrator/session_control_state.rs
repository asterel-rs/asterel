use crate::contracts::session_control::SessionControlState;
use crate::core::sessions::types::{Session, SessionMetadata};

pub(super) fn with_session_control_metadata(
    session: Session,
    state: SessionControlState,
) -> SessionMetadata {
    let mut metadata = session.metadata.unwrap_or_default();
    metadata.session_control = Some(state);
    metadata
}

pub(super) fn extract_session_control(session: &Session) -> Option<SessionControlState> {
    session
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.session_control.clone())
}
