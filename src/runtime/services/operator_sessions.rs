use anyhow::Result;

use super::operator_scope::OperatorScope;
use crate::contracts::ids::SessionId;
use crate::core::sessions::SessionOrchestrator;
use crate::core::sessions::types::{
    ChatMessage, MessageRole, Session, SessionMetadata, SessionOwnerScope,
};
use crate::runtime::diagnostics::control_plane_read_models::{
    SessionMessageReadModel, SessionSummaryReadModel, build_session_list_read_model,
    build_session_message_list_read_model,
};

const DEFAULT_PAGE_LIMIT: usize = 50;
const MAX_PAGE_LIMIT: usize = 200;
const OPERATOR_SCOPE_FETCH_MULTIPLIER: usize = 4;
const OPERATOR_SCOPE_MAX_SCANS: usize = 8;

#[derive(Debug, Clone, serde::Serialize)]
pub struct PagedOperatorSessionListReadModel {
    pub items: Vec<SessionSummaryReadModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PagedOperatorSessionMessageListReadModel {
    pub items: Vec<SessionMessageReadModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatorSessionAccess<T> {
    Visible(T),
    NotFound,
    ScopeDenied,
}

/// # Errors
/// Returns an error if the session store query fails.
pub async fn list_operator_sessions(
    session_manager: &SessionOrchestrator,
    scope: &OperatorScope,
    cursor: Option<&str>,
    limit: Option<usize>,
) -> Result<PagedOperatorSessionListReadModel> {
    let tenant_id = scope.require_tenant_id().unwrap_or_default();
    let limit = page_limit(limit);
    let mut scan_cursor = cursor.map(ToString::to_string);
    let mut visible = Vec::new();
    let mut exhausted = false;
    let fetch_limit = limit
        .saturating_mul(OPERATOR_SCOPE_FETCH_MULTIPLIER)
        .max(limit)
        .max(1);

    for _ in 0..OPERATOR_SCOPE_MAX_SCANS {
        let batch = session_manager
            .store()
            .list_tenant_scoped_sessions_page(tenant_id, scan_cursor.as_deref(), fetch_limit)
            .await?;
        if batch.is_empty() {
            exhausted = true;
            break;
        }

        let reached_batch_end = batch.len() < fetch_limit;
        scan_cursor = batch.last().map(|session| session.id.to_string());
        visible.extend(
            batch
                .into_iter()
                .filter(|session| session_matches_operator_scope(session, scope)),
        );

        if visible.len() > limit {
            break;
        }
        if reached_batch_end {
            exhausted = true;
            break;
        }
    }

    let has_more = visible.len() > limit || !exhausted;
    let next_cursor = if has_more {
        if visible.len() > limit {
            visible
                .get(limit.saturating_sub(1))
                .map(|session| session.id.to_string())
        } else {
            scan_cursor.clone()
        }
    } else {
        None
    };
    visible.truncate(limit);
    let list = build_session_list_read_model(&visible);
    Ok(PagedOperatorSessionListReadModel {
        items: list.items,
        next_cursor,
    })
}

/// # Errors
/// Returns an error if the session lookup fails.
pub async fn load_operator_session(
    session_manager: &SessionOrchestrator,
    scope: &OperatorScope,
    session_id: &SessionId,
) -> Result<OperatorSessionAccess<SessionSummaryReadModel>> {
    Ok(
        match load_operator_session_entity(session_manager, scope, session_id).await? {
            OperatorSessionAccess::Visible(session) => {
                OperatorSessionAccess::Visible(build_operator_session_summary_read_model(&session))
            }
            OperatorSessionAccess::NotFound => OperatorSessionAccess::NotFound,
            OperatorSessionAccess::ScopeDenied => OperatorSessionAccess::ScopeDenied,
        },
    )
}

/// # Errors
/// Returns an error if the session history query fails.
pub async fn list_operator_session_messages(
    session_manager: &SessionOrchestrator,
    scope: &OperatorScope,
    session_id: &SessionId,
    cursor: Option<&str>,
    limit: Option<usize>,
) -> Result<OperatorSessionAccess<PagedOperatorSessionMessageListReadModel>> {
    match load_operator_session_entity(session_manager, scope, session_id).await? {
        OperatorSessionAccess::Visible(_) => {}
        OperatorSessionAccess::NotFound => return Ok(OperatorSessionAccess::NotFound),
        OperatorSessionAccess::ScopeDenied => return Ok(OperatorSessionAccess::ScopeDenied),
    }

    let limit = page_limit(limit);
    let fetch_limit = limit.saturating_add(1);
    let messages = session_manager
        .store()
        .get_messages_page(session_id, cursor, fetch_limit)
        .await?;
    let has_more = messages.len() > limit;
    let page = if has_more {
        &messages[..limit]
    } else {
        messages.as_slice()
    };
    let list = build_session_message_list_read_model(page);
    Ok(OperatorSessionAccess::Visible(
        PagedOperatorSessionMessageListReadModel {
            items: list.items,
            next_cursor: if has_more {
                page.last().map(|message| message.id.to_string())
            } else {
                None
            },
        },
    ))
}

/// # Errors
/// Returns an error if the underlying store fails to persist the session.
pub async fn create_operator_session(
    session_manager: &SessionOrchestrator,
    scope: &OperatorScope,
    title: Option<&str>,
) -> Result<SessionSummaryReadModel> {
    let tenant_id = scope.require_tenant_id().unwrap_or_default();
    let owner_scope = crate::core::sessions::render_tenant_principal_owner_scope(
        tenant_id,
        &scope.principal,
        "admin",
    );
    let session = session_manager
        .store()
        .create_session("gateway_ws", &owner_scope)
        .await?;

    if let Some(title) = title.filter(|title| !title.is_empty()) {
        let metadata = SessionMetadata {
            title: Some(title.to_string()),
            ..SessionMetadata::default()
        };
        session_manager
            .store()
            .update_session_metadata(&session.id, Some(metadata))
            .await?;
    }

    let session = session_manager
        .store()
        .get_session(&session.id)
        .await?
        .unwrap_or(session);
    Ok(build_operator_session_summary_read_model(&session))
}

/// # Errors
/// Returns an error if session lookup or deletion fails.
pub async fn delete_operator_session(
    session_manager: &SessionOrchestrator,
    scope: &OperatorScope,
    session_id: &str,
) -> Result<OperatorSessionAccess<()>> {
    let session_id = SessionId::new(session_id);
    let Some(session) = session_manager.store().get_session(&session_id).await? else {
        return Ok(OperatorSessionAccess::NotFound);
    };
    if !session_matches_operator_scope(&session, scope) {
        return Ok(OperatorSessionAccess::ScopeDenied);
    }

    if session_manager.delete_session(&session_id).await? {
        Ok(OperatorSessionAccess::Visible(()))
    } else {
        Ok(OperatorSessionAccess::NotFound)
    }
}

/// # Errors
/// Returns an error if the session lookup or message append fails.
pub async fn append_operator_session_message(
    session_manager: &SessionOrchestrator,
    scope: &OperatorScope,
    session_id: &str,
    role: MessageRole,
    content: &str,
) -> Result<OperatorSessionAccess<ChatMessage>> {
    let session_id = SessionId::new(session_id);
    let Some(session) = session_manager.store().get_session(&session_id).await? else {
        return Ok(OperatorSessionAccess::NotFound);
    };
    if !session_matches_operator_scope(&session, scope) {
        return Ok(OperatorSessionAccess::ScopeDenied);
    }

    let message = session_manager
        .store()
        .append_message(&session_id, role, content, None, None)
        .await?;
    Ok(OperatorSessionAccess::Visible(message))
}

#[must_use]
pub fn build_operator_session_summary_read_model(session: &Session) -> SessionSummaryReadModel {
    SessionSummaryReadModel {
        id: session.id.to_string(),
        surface: session.surface.clone(),
        owner_scope: session.owner_scope.clone(),
        state: format!("{:?}", session.state).to_lowercase(),
        created_at: session.created_at.clone(),
        updated_at: session.updated_at.clone(),
    }
}

#[must_use]
pub fn build_operator_session_message_read_model(message: &ChatMessage) -> SessionMessageReadModel {
    SessionMessageReadModel {
        id: message.id.to_string(),
        role: format!("{:?}", message.role).to_lowercase(),
        content: message.content.clone(),
        input_tokens: message.input_tokens,
        output_tokens: message.output_tokens,
        created_at: message.created_at.clone(),
    }
}

#[must_use]
pub fn session_matches_operator_scope(session: &Session, scope: &OperatorScope) -> bool {
    let Some(tenant_id) = scope.require_tenant_id() else {
        return false;
    };
    let principal = scope.principal.trim();
    if tenant_id.trim().is_empty() || principal.is_empty() {
        return false;
    }

    match SessionOwnerScope::parse(session.owner_scope.as_str()) {
        SessionOwnerScope::TenantPrincipal {
            tenant_id: scope_tenant,
            principal: scope_principal,
            ..
        } => scope_tenant == tenant_id && scope_principal == principal,
        SessionOwnerScope::Tenant { .. }
        | SessionOwnerScope::Unscoped { .. }
        | SessionOwnerScope::Principal { .. }
        | SessionOwnerScope::Opaque { .. } => false,
    }
}

async fn load_operator_session_entity(
    session_manager: &SessionOrchestrator,
    scope: &OperatorScope,
    session_id: &SessionId,
) -> Result<OperatorSessionAccess<Session>> {
    let Some(session) = session_manager.store().get_session(session_id).await? else {
        return Ok(OperatorSessionAccess::NotFound);
    };
    if session_matches_operator_scope(&session, scope) {
        Ok(OperatorSessionAccess::Visible(session))
    } else {
        Ok(OperatorSessionAccess::ScopeDenied)
    }
}

fn page_limit(requested: Option<usize>) -> usize {
    requested.unwrap_or(DEFAULT_PAGE_LIMIT).min(MAX_PAGE_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::{
        build_operator_session_message_read_model, build_operator_session_summary_read_model,
        page_limit, session_matches_operator_scope,
    };
    use crate::contracts::ids::{MessageId, SessionId, UserId};
    use crate::core::sessions::types::{ChatMessage, MessageRole, Session, SessionState};
    use crate::runtime::services::operator_scope::OperatorScope;

    fn make_scope(tenant_id: &str, principal: &str) -> OperatorScope {
        OperatorScope {
            principal: principal.to_string(),
            tenant_id: Some(tenant_id.to_string()),
            tenant_mode_available: true,
        }
    }

    #[test]
    fn operator_scope_matches_tenant_principal_session() {
        let session = Session {
            id: SessionId::new("session-1"),
            surface: "gateway_ws".to_string(),
            owner_scope: UserId::new("tenant::alpha::principal::auth-123::admin"),
            state: SessionState::Active,
            model: None,
            metadata: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            archived_at: None,
        };

        assert!(session_matches_operator_scope(
            &session,
            &make_scope("alpha", "auth-123")
        ));
        assert!(!session_matches_operator_scope(
            &session,
            &make_scope("alpha", "auth-other")
        ));
    }

    #[test]
    fn operator_scope_rejects_tenant_shared_session_even_in_same_tenant() {
        let session = Session {
            id: SessionId::new("session-1"),
            surface: "gateway_ws".to_string(),
            owner_scope: UserId::new("tenant::alpha::shared-session"),
            state: SessionState::Active,
            model: None,
            metadata: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            archived_at: None,
        };

        assert!(!session_matches_operator_scope(
            &session,
            &make_scope("alpha", "auth-123")
        ));
        assert!(!session_matches_operator_scope(
            &session,
            &make_scope("beta", "auth-123")
        ));
    }

    #[test]
    fn operator_session_summary_read_model_maps_core_fields() {
        let session = Session {
            id: SessionId::new("session-99"),
            surface: "gateway_ws".to_string(),
            owner_scope: UserId::new("tenant::alpha::principal::auth-123::admin"),
            state: SessionState::Archived,
            model: None,
            metadata: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-02T00:00:00Z".to_string(),
            archived_at: Some("2026-01-03T00:00:00Z".to_string()),
        };

        let model = build_operator_session_summary_read_model(&session);

        assert_eq!(model.id, "session-99");
        assert_eq!(model.surface, "gateway_ws");
        assert_eq!(model.state, "archived");
        assert_eq!(model.created_at, "2026-01-01T00:00:00Z");
        assert_eq!(model.updated_at, "2026-01-02T00:00:00Z");
    }

    #[test]
    fn operator_session_message_read_model_maps_message_fields() {
        let message = ChatMessage {
            id: MessageId::new("msg-1"),
            session_id: SessionId::new("session-1"),
            role: MessageRole::Assistant,
            content: "hello".to_string(),
            input_tokens: Some(3),
            output_tokens: Some(5),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let model = build_operator_session_message_read_model(&message);

        assert_eq!(model.id, "msg-1");
        assert_eq!(model.role, "assistant");
        assert_eq!(model.content, "hello");
        assert_eq!(model.input_tokens, Some(3));
        assert_eq!(model.output_tokens, Some(5));
    }

    #[test]
    fn page_limit_defaults_and_caps() {
        assert_eq!(page_limit(None), 50);
        assert_eq!(page_limit(Some(10)), 10);
        assert_eq!(page_limit(Some(500)), 200);
    }

    #[test]
    fn operator_scope_rejects_unscoped_and_principal_only_sessions() {
        let unscoped = Session {
            id: SessionId::new("session-1"),
            surface: "gateway_ws".to_string(),
            owner_scope: UserId::new("shared-session"),
            state: SessionState::Active,
            model: None,
            metadata: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            archived_at: None,
        };
        let principal_only = Session {
            id: SessionId::new("session-2"),
            surface: "gateway_ws".to_string(),
            owner_scope: UserId::new("principal::auth-123::admin"),
            state: SessionState::Active,
            model: None,
            metadata: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            archived_at: None,
        };

        assert!(!session_matches_operator_scope(
            &unscoped,
            &make_scope("alpha", "auth-123")
        ));
        assert!(!session_matches_operator_scope(
            &principal_only,
            &make_scope("alpha", "auth-123")
        ));
    }

    #[test]
    fn operator_scope_rejects_empty_scope_values() {
        let session = Session {
            id: SessionId::new("session-1"),
            surface: "gateway_ws".to_string(),
            owner_scope: UserId::new("tenant::alpha::principal::auth-123::admin"),
            state: SessionState::Active,
            model: None,
            metadata: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            archived_at: None,
        };
        let empty_tenant = OperatorScope {
            principal: "auth-123".to_string(),
            tenant_id: Some("   ".to_string()),
            tenant_mode_available: true,
        };
        let empty_principal = OperatorScope {
            principal: "   ".to_string(),
            tenant_id: Some("alpha".to_string()),
            tenant_mode_available: true,
        };

        assert!(!session_matches_operator_scope(&session, &empty_tenant));
        assert!(!session_matches_operator_scope(&session, &empty_principal));
    }
}
