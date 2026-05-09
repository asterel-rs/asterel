//! Gateway turn-bridge helpers: request rate limiting, workspace scoping,
//! and lower-level turn execution.
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Json;
use uuid::Uuid;

use super::super::AppState;
use super::super::autosave::{tenant_scoped_entity_id, tenant_workspace_dir};
use super::super::events::GatewayAgentStateNotifier;
use super::super::problem_details::problem_response;
use super::auth_context::verified_source_identifier_from_headers;
use crate::contracts::channels::SurfaceRealizationPolicy;
use crate::contracts::ids::{EntityId, PersonId, SessionId};
use crate::core::agent::LoopStopReason;
use crate::core::persona::person_identity::channel_entity_id;
use crate::core::sessions::{
    SessionOrchestrator, SessionOwnerScope, render_principal_owner_scope,
    render_tenant_owner_scope, render_tenant_principal_owner_scope,
};
use crate::core::tools::middleware::ExecutionContext;
use crate::runtime::services::{
    CompanionTransportTurnRequest, CompanionTurnRuntimeDeps, run_transport_companion_turn,
};
use crate::security::policy::TenantPolicyContext;
use crate::utils::text::{strip_inference_markers, strip_internal_prompt_blocks, strip_reasoning};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in super::super) struct GatewayTurnSessionBinding {
    canonical_session_id: Option<SessionId>,
    owner_scope: Option<String>,
    fallback_session_key: Option<String>,
    source_identifier: String,
}

impl GatewayTurnSessionBinding {
    fn from_resolved_session(session: crate::core::sessions::types::Session) -> Self {
        let source_identifier = session.id.to_string();
        Self {
            canonical_session_id: Some(session.id),
            owner_scope: Some(session.owner_scope.to_string()),
            fallback_session_key: None,
            source_identifier,
        }
    }

    fn fallback(
        requested_session_key: Option<&str>,
        owner_scope: Option<String>,
        fallback_source_identifier: &str,
    ) -> Self {
        let fallback_session_key = requested_session_key
            .map(std::borrow::ToOwned::to_owned)
            .or_else(|| Some(fallback_source_identifier.to_string()));
        let source_identifier = fallback_session_key
            .clone()
            .unwrap_or_else(|| fallback_source_identifier.to_string());
        Self {
            canonical_session_id: None,
            owner_scope,
            fallback_session_key,
            source_identifier,
        }
    }

    pub(in super::super) fn source_identifier(&self) -> &str {
        &self.source_identifier
    }

    pub(in super::super) fn session_owner_scope(&self) -> Option<&str> {
        self.owner_scope.as_deref()
    }

    pub(in super::super) fn history_session_key(&self) -> Option<&str> {
        self.canonical_session_id
            .as_ref()
            .map(SessionId::as_str)
            .or(self.fallback_session_key.as_deref())
    }

    pub(in super::super) fn working_memory_session_id(&self) -> &str {
        self.history_session_key().unwrap_or("anonymous")
    }

    pub(in super::super) fn context_session_id(&self) -> Option<&str> {
        self.history_session_key()
    }
}

fn session_matches_gateway_scope(
    session: &crate::core::sessions::types::Session,
    surface: &str,
    policy_context: &TenantPolicyContext,
    principal: Option<&str>,
) -> bool {
    if session.surface != surface {
        return false;
    }

    let tenant = policy_context
        .tenant_id
        .as_deref()
        .map(str::trim)
        .filter(|tenant| !tenant.is_empty());
    let principal = principal.map(str::trim).filter(|value| !value.is_empty());

    match (
        tenant,
        principal,
        SessionOwnerScope::parse(session.owner_scope.as_str()),
    ) {
        (
            Some(expected_tenant),
            Some(expected_principal),
            SessionOwnerScope::TenantPrincipal {
                tenant_id,
                principal,
                ..
            },
        ) => tenant_id == expected_tenant && principal == expected_principal,
        (
            Some(expected_tenant),
            None,
            SessionOwnerScope::Tenant { tenant_id, .. }
            | SessionOwnerScope::TenantPrincipal { tenant_id, .. },
        ) => tenant_id == expected_tenant,
        (None, Some(expected_principal), SessionOwnerScope::Principal { principal, .. }) => {
            principal == expected_principal
        }
        (None, None, SessionOwnerScope::Unscoped { .. }) => true,
        _ => false,
    }
}

pub(in super::super) fn tenant_scoped_owner_scope(
    session_key: &str,
    policy_context: &TenantPolicyContext,
    principal: Option<&str>,
) -> String {
    match (
        policy_context
            .tenant_id
            .as_deref()
            .map(str::trim)
            .filter(|tenant| !tenant.is_empty()),
        principal.map(str::trim).filter(|value| !value.is_empty()),
    ) {
        (Some(tenant), Some(principal)) => {
            render_tenant_principal_owner_scope(tenant, principal, session_key)
        }
        (Some(tenant), None) => render_tenant_owner_scope(tenant, session_key),
        (None, Some(principal)) => render_principal_owner_scope(principal, session_key),
        (None, None) => session_key.to_string(),
    }
}

pub(in super::super) async fn resolve_gateway_turn_session(
    session_manager: Option<&SessionOrchestrator>,
    surface: &str,
    requested_session_key: Option<&str>,
    policy_context: &TenantPolicyContext,
    principal: Option<&str>,
    create_if_missing: bool,
    fallback_source_identifier: &str,
) -> Result<GatewayTurnSessionBinding> {
    let requested_session_key = requested_session_key
        .map(str::trim)
        .filter(|key| !key.is_empty());

    if let Some(session_manager) = session_manager {
        if let Some(session_key) = requested_session_key {
            if let Some(session) = session_manager
                .get_session_by_id(&SessionId::new(session_key))
                .await?
            {
                if session_matches_gateway_scope(&session, surface, policy_context, principal) {
                    return Ok(GatewayTurnSessionBinding::from_resolved_session(session));
                }
                anyhow::bail!("requested session is not accessible for this caller scope");
            }

            anyhow::bail!("requested session was not found for this caller scope");
        }

        if create_if_missing {
            let generated_key = format!("{surface}-{}", Uuid::new_v4());
            let owner_scope = tenant_scoped_owner_scope(&generated_key, policy_context, principal);
            let session = session_manager
                .resolve_session(surface, &owner_scope)
                .await?;
            return Ok(GatewayTurnSessionBinding::from_resolved_session(session));
        }
    }

    let owner_scope = requested_session_key
        .map(|session_key| tenant_scoped_owner_scope(session_key, policy_context, principal));
    Ok(GatewayTurnSessionBinding::fallback(
        requested_session_key,
        owner_scope,
        fallback_source_identifier,
    ))
}

pub(in super::super) fn gateway_entity_id(
    source_identifier: &str,
    policy_context: &TenantPolicyContext,
) -> EntityId {
    let base_entity_id = EntityId::new(channel_entity_id("gateway", source_identifier));
    tenant_scoped_entity_id(base_entity_id, policy_context)
}

pub(in super::super) async fn gateway_workspace_dir(
    state: &AppState,
    policy_context: &TenantPolicyContext,
) -> Result<PathBuf> {
    tenant_workspace_dir(
        &state.runtime.security.workspace_dir,
        policy_context,
        "gateway",
    )
    .await
}

pub(in super::super) fn enforce_entity_rate_limit(
    state: &AppState,
    headers: &HeaderMap,
    policy_context: &TenantPolicyContext,
) -> Option<(StatusCode, Json<serde_json::Value>)> {
    let source_identifier = verified_source_identifier_from_headers(state, headers, "anonymous");
    enforce_entity_rate_limit_for_source(state, &source_identifier, policy_context)
}

pub(in super::super) fn enforce_entity_rate_limit_for_source(
    state: &AppState,
    source_identifier: &str,
    policy_context: &TenantPolicyContext,
) -> Option<(StatusCode, Json<serde_json::Value>)> {
    let entity_id = gateway_entity_id(source_identifier, policy_context);
    if let Err(rate_error) = state
        .runtime
        .rate_limiter
        .check_and_record(entity_id.as_str())
    {
        tracing::warn!(
            %entity_id,
            error = ?rate_error,
            "gateway rate limit exceeded"
        );
        let detail = match &rate_error {
            crate::security::policy::RateLimitError::GlobalExhausted => {
                "Global rate limit exceeded. Try again later.".to_string()
            }
            crate::security::policy::RateLimitError::BurstExhausted { .. } => {
                format!("Burst rate limit exceeded for {entity_id}. Slow down.")
            }
            _ => {
                format!("Rate limit exceeded for {entity_id}. Try again later.")
            }
        };
        return Some(problem_response(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limit_exceeded",
            "Too Many Requests",
            detail,
        ));
    }
    None
}

pub(in super::super) fn log_tool_loop_stop(
    source: &str,
    stop_reason: &LoopStopReason,
    iterations: u32,
) {
    match stop_reason {
        LoopStopReason::Completed => {}
        LoopStopReason::MaxIterations => {
            tracing::warn!(source, iterations, "tool loop hit max iterations");
        }
        LoopStopReason::RateLimited => {
            tracing::warn!(source, "tool loop halted by rate limiter");
        }
        LoopStopReason::ApprovalDenied => {
            tracing::warn!(source, "tool loop halted pending approval");
        }
        LoopStopReason::Error(error) => {
            tracing::warn!(source, error = %error, "tool loop ended with provider error");
        }
    }
}

pub(in super::super) fn gateway_delivery_text(final_text: &str) -> String {
    let without_internal_blocks = strip_internal_prompt_blocks(final_text);
    let without_reasoning = strip_reasoning(&without_internal_blocks);
    strip_inference_markers(&without_reasoning)
}

#[allow(clippy::too_many_lines)]
pub(in super::super) async fn run_tool_loop(
    state: &AppState,
    system_prompt: Option<&str>,
    user_message: &str,
    model: &str,
    temperature: f64,
    source_identifier: &str,
    session_principal: Option<&str>,
    policy_context: TenantPolicyContext,
) -> anyhow::Result<crate::core::agent::tool_loop::ToolLoopResult> {
    let base_prompt = system_prompt
        .filter(|system_prompt| !system_prompt.is_empty())
        .unwrap_or(
            "You are Asterel, a local autonomous agent with persistent identity, memory, and tool capabilities.",
        );
    let entity_id = gateway_entity_id(source_identifier, &policy_context);
    let person_id = PersonId::new(format!("gateway.{source_identifier}"));
    let workspace_dir = gateway_workspace_dir(state, &policy_context).await?;
    let session_key = gateway_session_key(source_identifier);
    let session_binding = resolve_gateway_turn_session(
        state.runtime.session_manager.as_deref(),
        "gateway_http",
        session_key,
        &policy_context,
        session_principal,
        false,
        source_identifier,
    )
    .await?;
    let tenant_id = policy_context.tenant_id.clone();
    let mut ctx = ExecutionContext::runtime_root(
        Arc::clone(&state.runtime.security),
        workspace_dir.clone(),
        Arc::clone(&state.runtime.rate_limiter),
        Some(Arc::clone(&state.runtime.permission_store)),
        policy_context.clone(),
    );
    ctx.entity_id = entity_id.clone();
    ctx.memory = Some(Arc::clone(&state.runtime.mem));
    ctx.observer = Arc::clone(&state.runtime.observer);
    ctx.session_id = session_binding
        .context_session_id()
        .map(std::string::ToString::to_string);
    ctx.subagent_manager = Some(Arc::clone(&state.runtime.subagent_manager));
    let surface_realization_policy = SurfaceRealizationPolicy::gateway_http();
    let outcome = run_transport_companion_turn(CompanionTransportTurnRequest {
        runtime: CompanionTurnRuntimeDeps {
            mem: Arc::clone(&state.runtime.mem),
            persona_config: &state.runtime.config.persona,
            session_manager: state.runtime.session_manager.as_deref(),
            working_memory_capacity: state.runtime.config.memory.working_memory_capacity,
            registry: Arc::clone(&state.runtime.registry),
            max_tool_iterations: state.runtime.max_tool_loop_iterations,
            loop_detection: state.runtime.loop_detection.clone(),
            response_finalization_enabled: state
                .runtime
                .config
                .persona
                .enable_response_finalization,
            naturalness_gate_enabled: state.runtime.config.persona.enable_naturalness_gate,
            self_amendment_candidate_sink: Some(Arc::new(
                state.runtime.self_amendment_candidate_review.clone(),
            )),
        },
        workspace_dir: workspace_dir.as_path(),
        base_prompt,
        user_message,
        entity_id: entity_id.as_str(),
        person_id: person_id.as_str(),
        base_temperature: temperature,
        policy_context: &policy_context,
        session_surface: Some("gateway_http"),
        channel_context_hint: None,
        surface_realization_policy: Some(&surface_realization_policy),
        session_owner_scope: session_binding.session_owner_scope(),
        working_memory_session_id: session_binding.working_memory_session_id(),
        history_channel_name: "gateway_http",
        history_session_key: session_binding.history_session_key(),
        history_tenant_id: tenant_id.as_deref(),
        history_max_tokens: state.runtime.session_history_max_tokens,
        fallback_history: &[],
        provider: state.runtime.provider.as_ref(),
        image_content: &[],
        model,
        inference_options: None,
        ctx: &ctx,
        stream_sink: None,
        state_notifier: Some(Arc::new(GatewayAgentStateNotifier::new(
            source_identifier,
            state.companion.gateway_events.clone(),
        ))),
        transcript_log_target: "transport::gateway::handlers",
    })
    .await?;

    if let LoopStopReason::Error(error) = &outcome.result.stop_reason {
        anyhow::bail!("tool loop failed: {error}");
    }
    Ok(outcome.result)
}

fn gateway_session_key(source_identifier: &str) -> Option<&str> {
    match source_identifier.trim() {
        "" | "anonymous" => None,
        other => Some(other),
    }
}

pub(in super::super) fn webhook_replay_ack_response() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({"status": "duplicate_ignored"})),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::ids::UserId;
    use crate::core::sessions::types::{Session, SessionState};
    use crate::core::sessions::{SessionOrchestrator, types::SessionConfig};
    use tempfile::{NamedTempFile, TempDir};

    async fn session_manager() -> (
        TempDir,
        NamedTempFile,
        SessionOrchestrator,
        crate::utils::test_env::TestDbGuard,
    ) {
        let db_guard = crate::utils::test_env::acquire_test_db().await;
        let database_url = crate::utils::test_env::postgres_url()
            .expect("test requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL");
        let temp_dir = TempDir::new().expect("tempdir should be created");
        let workspace_dir = temp_dir.path().join("workspace");
        crate::utils::test_env::write_workspace_postgres_config(&workspace_dir, &database_url)
            .expect("test config should be written");
        let db_file = NamedTempFile::new_in(&workspace_dir).expect("session db file should exist");
        let manager = SessionOrchestrator::connect(db_file.path(), SessionConfig::default())
            .await
            .expect("session manager should be created");
        (temp_dir, db_file, manager, db_guard)
    }

    #[test]
    fn tenant_scoped_owner_scope_adds_tenant_prefix_when_enabled() {
        let tenant = TenantPolicyContext::enabled("alpha");
        assert_eq!(
            tenant_scoped_owner_scope("session-1", &tenant, None),
            "tenant::alpha::session-1"
        );
    }

    #[test]
    fn tenant_scoped_owner_scope_adds_principal_prefix_when_present() {
        let tenant = TenantPolicyContext::enabled("alpha");
        assert_eq!(
            tenant_scoped_owner_scope("session-1", &tenant, Some("auth-123")),
            "tenant::alpha::principal::auth-123::session-1"
        );
    }

    #[test]
    fn gateway_turn_session_binding_uses_canonical_session_id_when_resolved() {
        let session = Session {
            id: SessionId::new("session-42"),
            surface: "gateway_ws".to_string(),
            owner_scope: UserId::new("tenant::alpha::owner-key"),
            state: SessionState::Active,
            model: None,
            metadata: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            archived_at: None,
        };

        let binding = GatewayTurnSessionBinding::from_resolved_session(session);

        assert_eq!(binding.context_session_id(), Some("session-42"));
        assert_eq!(binding.history_session_key(), Some("session-42"));
        assert_eq!(binding.working_memory_session_id(), "session-42");
        assert_eq!(
            binding.session_owner_scope(),
            Some("tenant::alpha::owner-key")
        );
        assert_eq!(binding.source_identifier(), "session-42");
    }

    #[test]
    fn session_matches_gateway_scope_rejects_foreign_tenant_session() {
        let session = Session {
            id: SessionId::new("session-42"),
            surface: "gateway_ws".to_string(),
            owner_scope: UserId::new("tenant::beta::owner-key"),
            state: SessionState::Active,
            model: None,
            metadata: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            archived_at: None,
        };

        assert!(!session_matches_gateway_scope(
            &session,
            "gateway_ws",
            &TenantPolicyContext::enabled("alpha"),
            None,
        ));
    }

    #[test]
    fn session_matches_gateway_scope_rejects_foreign_principal_session() {
        let session = Session {
            id: SessionId::new("session-42"),
            surface: "gateway_ws".to_string(),
            owner_scope: UserId::new("tenant::alpha::principal::auth-other::owner-key"),
            state: SessionState::Active,
            model: None,
            metadata: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            archived_at: None,
        };

        assert!(!session_matches_gateway_scope(
            &session,
            "gateway_ws",
            &TenantPolicyContext::enabled("alpha"),
            Some("auth-123"),
        ));
    }

    #[test]
    fn session_matches_gateway_scope_rejects_tenant_only_session_when_principal_is_required() {
        let session = Session {
            id: SessionId::new("session-42"),
            surface: "gateway_ws".to_string(),
            owner_scope: UserId::new("tenant::alpha::owner-key"),
            state: SessionState::Active,
            model: None,
            metadata: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            archived_at: None,
        };

        assert!(!session_matches_gateway_scope(
            &session,
            "gateway_ws",
            &TenantPolicyContext::enabled("alpha"),
            Some("auth-123"),
        ));
    }

    #[test]
    fn gateway_turn_session_binding_falls_back_to_requested_key_without_manager() {
        let binding = GatewayTurnSessionBinding::fallback(
            Some("client-session"),
            Some("tenant::alpha::client-session".to_string()),
            "websocket",
        );

        assert_eq!(binding.context_session_id(), Some("client-session"));
        assert_eq!(binding.history_session_key(), Some("client-session"));
        assert_eq!(binding.working_memory_session_id(), "client-session");
        assert_eq!(
            binding.session_owner_scope(),
            Some("tenant::alpha::client-session")
        );
        assert_eq!(binding.source_identifier(), "client-session");
    }

    #[test]
    fn gateway_turn_session_binding_uses_fallback_source_for_history_without_manager() {
        let binding = GatewayTurnSessionBinding::fallback(None, None, "ws-conn-123");

        assert_eq!(binding.context_session_id(), Some("ws-conn-123"));
        assert_eq!(binding.history_session_key(), Some("ws-conn-123"));
        assert_eq!(binding.working_memory_session_id(), "ws-conn-123");
        assert_eq!(binding.source_identifier(), "ws-conn-123");
    }

    #[test]
    fn gateway_delivery_text_strips_internal_reasoning_and_inference_markers() {
        let text =
            "[Behavior Selection]\ninternal\n\nvisible<think>hidden</think>\nINFERRED_CLAIM secret";

        let visible = gateway_delivery_text(text);

        assert_eq!(visible.trim(), "visible");
        assert!(!visible.contains("Behavior Selection"));
        assert!(!visible.contains("hidden"));
        assert!(!visible.contains("INFERRED_CLAIM"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn resolve_gateway_turn_session_rejects_foreign_principal_canonical_session() {
        let (_temp_dir, _db_file, manager, _db_guard) = session_manager().await;
        let foreign = manager
            .store()
            .create_session(
                "gateway_ws",
                "tenant::alpha::principal::auth-other::session-foreign",
            )
            .await
            .expect("foreign session should be created");

        let error = resolve_gateway_turn_session(
            Some(&manager),
            "gateway_ws",
            Some(foreign.id.as_str()),
            &TenantPolicyContext::enabled("alpha"),
            Some("auth-123"),
            true,
            "ws-conn-123",
        )
        .await
        .expect_err("foreign principal canonical session must be rejected");

        assert!(error.to_string().contains("not accessible"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn resolve_gateway_turn_session_rejects_missing_caller_supplied_session_key() {
        let (_temp_dir, _db_file, manager, _db_guard) = session_manager().await;

        let error = resolve_gateway_turn_session(
            Some(&manager),
            "gateway_ws",
            Some("caller-chosen-missing-session"),
            &TenantPolicyContext::enabled("alpha"),
            Some("auth-123"),
            true,
            "ws-conn-123",
        )
        .await
        .expect_err("caller-supplied missing session key must not create a session");

        assert!(error.to_string().contains("not found"));
    }
}
