//! Session manager: resolves or creates sessions, records turns,
//! triggers compaction, and persists thinking-level state.

mod session_control_state;
mod thinking_state;
mod transcript;

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use super::compaction;
use super::presenter::render_history_fragment as render_transcript_history_fragment;
use super::store::PostgresSessionStore;
use super::types::{
    ChatMessage, ChatMessagePartInput, MessageRole, Session, SessionCompanionAffectState,
    SessionConfig, SessionMetadata, SessionState, SessionTranscriptReadModel, TranscriptMessage,
    estimate_tokens,
};
use crate::contracts::ids::SessionId;
use crate::core::providers::ThinkingLevel;

/// High-level session lifecycle manager with compaction support.
#[derive(Clone)]
pub struct SessionOrchestrator {
    store: Arc<PostgresSessionStore>,
    config: SessionConfig,
}

impl SessionOrchestrator {
    /// # Errors
    /// Returns an error if the session store cannot be initialized.
    pub fn new(db_path: &Path, config: SessionConfig) -> Result<Self> {
        let runtime = tokio::runtime::Runtime::new().map_err(anyhow::Error::from)?;
        crate::utils::postgres::block_on_sync(&runtime, Self::connect(db_path, config))
    }

    /// # Errors
    /// Returns an error if the session store cannot be initialized.
    pub async fn connect(db_path: &Path, config: SessionConfig) -> Result<Self> {
        let store = Arc::new(PostgresSessionStore::connect(db_path).await?);
        Ok(Self { store, config })
    }

    /// # Errors
    /// Returns an error if session lookup or creation fails.
    pub async fn resolve_session(&self, surface: &str, owner_scope: &str) -> Result<Session> {
        self.store
            .resolve_or_create_bound_session(surface, owner_scope, owner_scope)
            .await
    }

    /// # Errors
    /// Returns an error if the session cannot be found.
    pub async fn get_session_by_id(&self, id: &SessionId) -> Result<Option<Session>> {
        self.store.get_session(id).await
    }

    /// # Errors
    /// Returns an error if session lookup fails.
    pub async fn get_active_session_for_scope(
        &self,
        surface: &str,
        owner_scope: &str,
    ) -> Result<Option<Session>> {
        self.find_active_session(surface, owner_scope).await
    }

    /// # Errors
    /// Returns an error if either message cannot be appended to the session.
    pub async fn record_turn(
        &self,
        session_id: &SessionId,
        user_message: &str,
        assistant_response: &str,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> Result<()> {
        let resolved_input_tokens = transcript::resolved_token_count(user_message, input_tokens);
        let resolved_output_tokens =
            transcript::resolved_token_count(assistant_response, output_tokens);

        self.store
            .append_message_with_parts(
                session_id,
                MessageRole::User,
                &transcript::single_part(MessageRole::User, user_message),
                resolved_input_tokens,
                None,
            )
            .await?;
        self.store
            .append_message_with_parts(
                session_id,
                MessageRole::Assistant,
                &transcript::single_part(MessageRole::Assistant, assistant_response),
                None,
                resolved_output_tokens,
            )
            .await?;

        self.maybe_compact_session(session_id).await;
        Ok(())
    }

    /// # Errors
    /// Returns an error if transcript persistence fails.
    pub async fn record_turn_with_parts(
        &self,
        session_id: &SessionId,
        user_parts: &[ChatMessagePartInput],
        assistant_parts: &[ChatMessagePartInput],
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> Result<()> {
        let resolved_input = input_tokens.or_else(|| {
            let text: String = user_parts.iter().map(|p| p.content.as_str()).collect();
            u64::try_from(estimate_tokens(&text)).ok()
        });
        let resolved_output = output_tokens.or_else(|| {
            let text: String = assistant_parts.iter().map(|p| p.content.as_str()).collect();
            u64::try_from(estimate_tokens(&text)).ok()
        });

        self.store
            .append_message_with_parts(
                session_id,
                MessageRole::User,
                user_parts,
                resolved_input,
                None,
            )
            .await?;
        self.store
            .append_message_with_parts(
                session_id,
                MessageRole::Assistant,
                assistant_parts,
                None,
                resolved_output,
            )
            .await?;

        self.maybe_compact_session(session_id).await;
        Ok(())
    }

    /// # Errors
    /// Returns an error if reading session history fails.
    pub async fn get_history(&self, session_id: &SessionId) -> Result<Vec<ChatMessage>> {
        self.store
            .get_messages(session_id, self.history_limit())
            .await
    }

    /// # Errors
    /// Returns an error if reading transcript history fails.
    pub async fn get_transcript(&self, session_id: &SessionId) -> Result<Vec<TranscriptMessage>> {
        self.store
            .get_transcript(session_id, self.history_limit())
            .await
    }

    /// # Errors
    /// Returns an error if transcript retrieval fails.
    pub async fn load_transcript_read_model(
        &self,
        session_id: &SessionId,
        max_tokens: Option<usize>,
    ) -> Result<SessionTranscriptReadModel> {
        let transcript = match max_tokens {
            Some(limit) => {
                self.store
                    .get_transcript_tail_by_tokens(session_id, limit)
                    .await?
            }
            None => self.store.get_transcript(session_id, None).await?,
        };
        Ok(transcript::build_transcript_read_model(
            session_id, transcript, max_tokens,
        ))
    }

    /// # Errors
    /// Returns an error if the session cannot be loaded.
    pub async fn resume_session(&self, session_id: &SessionId) -> Result<Option<Session>> {
        self.store.get_session(session_id).await
    }

    /// # Errors
    /// Returns an error if transcript retrieval fails.
    pub async fn render_history_fragment(
        &self,
        session_id: &SessionId,
        max_tokens: usize,
        max_chars: usize,
    ) -> Result<String> {
        let read_model = self
            .load_transcript_read_model(session_id, Some(max_tokens))
            .await?;
        Ok(render_transcript_history_fragment(&read_model, max_chars))
    }

    /// # Errors
    /// Returns an error if the parent transcript cannot be loaded or the fork
    /// cannot be created.
    pub async fn fork_session(
        &self,
        parent_session_id: &SessionId,
        surface: &str,
        owner_scope: &str,
        max_tokens: usize,
    ) -> Result<Session> {
        let Some(_parent) = self.store.get_session(parent_session_id).await? else {
            anyhow::bail!("parent session not found: {parent_session_id}");
        };
        let read_model = self
            .load_transcript_read_model(parent_session_id, Some(max_tokens))
            .await?;
        let forked = self.store.create_session(surface, owner_scope).await?;
        let metadata = SessionMetadata {
            forked_from_session_id: Some(parent_session_id.clone()),
            fork_message_count: Some(read_model.messages.len()),
            fork_estimated_tokens: Some(read_model.estimated_tokens),
            ..SessionMetadata::default()
        };
        self.store
            .update_session_metadata(&forked.id, Some(metadata))
            .await?;

        for transcript_message in &read_model.messages {
            let parts = transcript_message
                .parts
                .iter()
                .map(transcript::part_to_input)
                .collect::<Vec<_>>();
            let part_inputs = if parts.is_empty() {
                vec![ChatMessagePartInput::new(
                    ChatMessage::default_part_kind_for_role(transcript_message.message.role),
                    transcript_message.message.content.clone(),
                )]
            } else {
                parts
            };
            self.store
                .append_message_with_parts(
                    &forked.id,
                    transcript_message.message.role,
                    &part_inputs,
                    transcript_message.message.input_tokens,
                    transcript_message.message.output_tokens,
                )
                .await?;
        }

        self.store
            .get_session(&forked.id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("forked session disappeared: {}", forked.id))
    }

    /// # Errors
    /// Returns an error if archiving an existing session or creating a new one fails.
    pub async fn reset_session(&self, surface: &str, owner_scope: &str) -> Result<Session> {
        if let Some(existing) = self.find_active_session(surface, owner_scope).await? {
            self.store
                .update_session_state(&existing.id, SessionState::Archived)
                .await?;
        }

        self.store.release_binding(surface, owner_scope).await?;
        let session = self.store.create_session(surface, owner_scope).await?;
        self.store
            .create_binding(surface, owner_scope, &session.id)
            .await?;

        Ok(session)
    }

    /// # Errors
    /// Returns an error if listing sessions fails.
    pub async fn list_sessions(&self, surface: Option<&str>) -> Result<Vec<Session>> {
        self.store.list_sessions(surface).await
    }

    /// # Errors
    /// Returns an error if deleting the session fails.
    pub async fn delete_session(&self, id: &SessionId) -> Result<bool> {
        self.store.delete_session(id).await
    }

    /// # Errors
    /// Returns an error if the session cannot be resolved or metadata update fails.
    pub async fn save_thinking_level(
        &self,
        session_id: &SessionId,
        level: ThinkingLevel,
    ) -> Result<()> {
        let Some(session) = self.store.get_session(session_id).await? else {
            anyhow::bail!(
                "cannot persist thinking level for unknown canonical session '{session_id}'"
            );
        };
        let session_record_id = session.id.clone();

        self.store
            .update_session_metadata(
                &session_record_id,
                Some(thinking_state::with_thinking_level_metadata(session, level)),
            )
            .await
    }

    /// # Errors
    /// Returns an error if querying session state fails.
    pub async fn load_thinking_level(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<ThinkingLevel>> {
        let Some(session) = self.store.get_session(session_id).await? else {
            return Ok(None);
        };

        Ok(thinking_state::extract_thinking_level(&session))
    }

    /// # Errors
    /// Returns an error if querying session state or metadata update fails.
    pub async fn clear_session_thinking_level(&self, session_id: &SessionId) -> Result<()> {
        let Some(session) = self.store.get_session(session_id).await? else {
            return Ok(());
        };
        let session_record_id = session.id.clone();
        let Some(metadata) = thinking_state::cleared_thinking_level_metadata(session) else {
            return Ok(());
        };

        self.store
            .update_session_metadata(&session_record_id, Some(metadata))
            .await
    }

    /// # Errors
    /// Returns an error if the session cannot be loaded or metadata update fails.
    pub async fn save_session_control(
        &self,
        session_id: &SessionId,
        state: crate::contracts::session_control::SessionControlState,
    ) -> Result<()> {
        let Some(session) = self.store.get_session(session_id).await? else {
            anyhow::bail!("session not found: {session_id}");
        };
        let session_record_id = session.id.clone();

        self.store
            .update_session_metadata(
                &session_record_id,
                Some(session_control_state::with_session_control_metadata(
                    session, state,
                )),
            )
            .await
    }

    /// # Errors
    /// Returns an error if querying session state fails.
    pub async fn load_session_control(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<crate::contracts::session_control::SessionControlState>> {
        let Some(session) = self.store.get_session(session_id).await? else {
            return Ok(None);
        };

        Ok(session_control_state::extract_session_control(&session))
    }

    /// # Errors
    /// Returns an error if the session cannot be loaded or metadata update fails.
    pub async fn save_companion_affect_state(
        &self,
        session_id: &SessionId,
        state: SessionCompanionAffectState,
    ) -> Result<()> {
        let Some(session) = self.store.get_session(session_id).await? else {
            anyhow::bail!("session not found: {session_id}");
        };
        let session_record_id = session.id.clone();
        let metadata = with_companion_affect_metadata(session, state);

        self.store
            .update_session_metadata(&session_record_id, Some(metadata))
            .await
    }

    /// # Errors
    /// Returns an error if the session cannot be loaded or metadata update fails.
    pub async fn clear_companion_affect_state(&self, session_id: &SessionId) -> Result<()> {
        let Some(session) = self.store.get_session(session_id).await? else {
            anyhow::bail!("session not found: {session_id}");
        };
        let session_record_id = session.id.clone();
        let Some(metadata) = without_companion_affect_metadata(session) else {
            return Ok(());
        };

        self.store
            .update_session_metadata(&session_record_id, Some(metadata))
            .await
    }

    async fn find_active_session(
        &self,
        surface: &str,
        owner_scope: &str,
    ) -> Result<Option<Session>> {
        if let Some(session) = self.store.resolve_binding(surface, owner_scope).await? {
            return Ok(Some(session));
        }

        let sessions = self.store.list_sessions(Some(surface)).await?;
        Ok(sessions.into_iter().find(|session| {
            session.owner_scope.as_str() == owner_scope && session.state == SessionState::Active
        }))
    }

    async fn maybe_compact_session(&self, session_id: &SessionId) {
        if self.config.compaction.token_threshold == 0 {
            return;
        }

        let companion_snapshot = match self.store.get_session(session_id).await {
            Ok(Some(session)) => {
                super::compaction_context::CompanionStateSnapshot::from_session_context(
                    session_control_state::extract_session_control(&session),
                    session
                        .metadata
                        .as_ref()
                        .and_then(|metadata| metadata.companion_affect.clone()),
                    0,
                )
            }
            Ok(None) => super::compaction_context::CompanionStateSnapshot::empty(0),
            Err(error) => {
                tracing::debug!(session_id = %session_id, %error, "failed to load companion snapshot for compaction");
                super::compaction_context::CompanionStateSnapshot::empty(0)
            }
        };

        if let Err(error) = compaction::compact_session_with_config(
            self.store.as_ref(),
            session_id,
            &self.config.compaction,
            Some(&companion_snapshot),
        )
        .await
        {
            tracing::warn!(session_id = %session_id, %error, "session compaction task failed");
        }
    }

    fn history_limit(&self) -> Option<usize> {
        (self.config.max_history > 0).then_some(self.config.max_history)
    }

    /// Return a reference to the underlying session store.
    #[must_use]
    pub fn store(&self) -> &PostgresSessionStore {
        self.store.as_ref()
    }
}

fn with_companion_affect_metadata(
    session: Session,
    state: SessionCompanionAffectState,
) -> SessionMetadata {
    let mut metadata = session.metadata.unwrap_or_default();
    metadata.companion_affect = Some(state);
    metadata
}

fn without_companion_affect_metadata(session: Session) -> Option<SessionMetadata> {
    let mut metadata = session.metadata.unwrap_or_default();
    metadata.companion_affect.as_ref()?;
    metadata.companion_affect = None;
    Some(metadata)
}

#[cfg(test)]
mod tests {
    use tempfile::{NamedTempFile, TempDir};

    use super::{
        SessionOrchestrator, with_companion_affect_metadata, without_companion_affect_metadata,
    };
    use crate::contracts::ids::{SessionId, UserId};
    use crate::core::providers::ThinkingLevel;
    use crate::core::sessions::types::{
        ChatMessagePartInput, MessagePartKind, MessageRole,
        SESSION_COMPANION_AFFECT_SCHEMA_VERSION, SESSION_COMPANION_AFFECT_SOURCE_TOPOLOGY, Session,
        SessionCompanionAffectState, SessionConfig, SessionMetadata, SessionState,
    };

    fn session_with_metadata(metadata: Option<SessionMetadata>) -> Session {
        Session {
            id: SessionId::new("session-test"),
            surface: "gateway_ws".to_string(),
            owner_scope: UserId::new("user-test"),
            state: SessionState::Active,
            model: None,
            metadata,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            archived_at: None,
        }
    }
    async fn manager() -> (
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
    fn companion_affect_metadata_helpers_save_and_clear_state() {
        let state = SessionCompanionAffectState {
            schema_version: SESSION_COMPANION_AFFECT_SCHEMA_VERSION,
            source: Some(SESSION_COMPANION_AFFECT_SOURCE_TOPOLOGY.to_string()),
            captured_at: Some("2026-01-01T00:00:00Z".to_string()),
            expires_at: Some("2026-01-01T02:00:00Z".to_string()),
            affect_surface: vec![("attachment".to_string(), 700)],
            affect_suppressed: vec![("guardedness".to_string(), 250)],
        };
        let metadata = with_companion_affect_metadata(session_with_metadata(None), state.clone());

        assert_eq!(metadata.companion_affect, Some(state));

        let cleared = without_companion_affect_metadata(session_with_metadata(Some(metadata)))
            .expect("existing companion affect should produce cleared metadata");

        assert!(cleared.companion_affect.is_none());
    }

    #[test]
    fn companion_affect_metadata_clear_is_noop_when_absent() {
        assert!(without_companion_affect_metadata(session_with_metadata(None)).is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn resolve_session_creates_then_reuses_session() {
        let (_temp_dir, _db_file, manager, _db_guard) = manager().await;

        let first = manager.resolve_session("cli", "user-1").await.unwrap();
        let second = manager.resolve_session("cli", "user-1").await.unwrap();

        assert_eq!(first.id, second.id);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn resolve_session_concurrent_calls_share_one_active_session() {
        let (_temp_dir, _db_file, manager, _db_guard) = manager().await;
        let manager = std::sync::Arc::new(manager);

        let mut tasks = Vec::new();
        for _ in 0..8 {
            let manager = std::sync::Arc::clone(&manager);
            tasks.push(tokio::spawn(async move {
                manager
                    .resolve_session("gateway_ws", "tenant::alpha::binding-1")
                    .await
                    .expect("session resolution should succeed")
                    .id
            }));
        }

        let mut ids = std::collections::BTreeSet::new();
        for task in tasks {
            ids.insert(task.await.expect("task should join"));
        }

        let sessions = manager
            .list_sessions(Some("gateway_ws"))
            .await
            .expect("session listing should succeed");
        let active_for_scope = sessions
            .into_iter()
            .filter(|session| {
                session.owner_scope.as_str() == "tenant::alpha::binding-1"
                    && session.state == SessionState::Active
            })
            .count();

        assert_eq!(ids.len(), 1);
        assert_eq!(active_for_scope, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn record_turn_appends_user_and_assistant_messages() {
        let (_temp_dir, _db_file, manager, _db_guard) = manager().await;
        let session = manager.resolve_session("cli", "user-1").await.unwrap();

        manager
            .record_turn(&session.id, "hello", "world", Some(10), Some(20))
            .await
            .unwrap();

        let messages = manager
            .store()
            .get_messages(&session.id, None)
            .await
            .unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, MessageRole::User);
        assert_eq!(messages[1].role, MessageRole::Assistant);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn get_history_returns_messages() {
        let (_temp_dir, _db_file, manager, _db_guard) = manager().await;
        let session = manager.resolve_session("cli", "user-1").await.unwrap();
        manager
            .record_turn(&session.id, "hello", "world", None, None)
            .await
            .unwrap();

        let history = manager.get_history(&session.id).await.unwrap();
        assert!(!history.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn reset_session_archives_existing_and_creates_new() {
        let (_temp_dir, _db_file, manager, _db_guard) = manager().await;
        let first = manager.resolve_session("cli", "user-1").await.unwrap();

        let second = manager.reset_session("cli", "user-1").await.unwrap();
        assert_ne!(first.id, second.id);

        let archived = manager
            .store()
            .get_session(&first.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(archived.state, SessionState::Archived);
        assert_eq!(second.state, SessionState::Active);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn save_and_load_thinking_level_round_trip() {
        let (_temp_dir, _db_file, manager, _db_guard) = manager().await;
        let session = manager
            .resolve_session("discord", "channel-77::user-42")
            .await
            .unwrap();

        manager
            .save_thinking_level(&session.id, ThinkingLevel::High)
            .await
            .unwrap();

        let loaded = manager.load_thinking_level(&session.id).await.unwrap();
        assert_eq!(loaded, Some(ThinkingLevel::High));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn thinking_level_round_trip() {
        let (_temp_dir, _db_file, manager, _db_guard) = manager().await;
        let session = manager
            .resolve_session("discord", "channel-88::user-async")
            .await
            .unwrap();

        manager
            .save_thinking_level(&session.id, ThinkingLevel::Low)
            .await
            .unwrap();

        let loaded = manager.load_thinking_level(&session.id).await.unwrap();
        assert_eq!(loaded, Some(ThinkingLevel::Low));

        manager
            .clear_session_thinking_level(&session.id)
            .await
            .unwrap();
        let cleared = manager.load_thinking_level(&session.id).await.unwrap();
        assert_eq!(cleared, None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn clear_session_thinking_level_removes_saved_value() {
        let (_temp_dir, _db_file, manager, _db_guard) = manager().await;
        let session = manager
            .resolve_session("discord", "channel-77::user-42")
            .await
            .unwrap();

        manager
            .save_thinking_level(&session.id, ThinkingLevel::Medium)
            .await
            .unwrap();
        manager
            .clear_session_thinking_level(&session.id)
            .await
            .unwrap();

        let loaded = manager.load_thinking_level(&session.id).await.unwrap();
        assert_eq!(loaded, None);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn load_thinking_level_returns_none_for_unknown_session() {
        let (_temp_dir, _db_file, manager, _db_guard) = manager().await;

        let loaded = manager
            .load_thinking_level(&SessionId::new("discord::channel-99::user-24"))
            .await
            .unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn save_and_load_session_control_round_trip() {
        let (_temp_dir, _db_file, manager, _db_guard) = manager().await;
        let session = manager
            .resolve_session("gateway_ws", "tenant::alpha::sess-1")
            .await
            .unwrap();
        let state = crate::contracts::session_control::SessionControlState {
            mode: crate::contracts::session_control::ConversationMode::Empathy,
            density: crate::contracts::session_control::ExpectedDensity::Brief,
            avoid: vec![crate::contracts::session_control::AvoidBehavior::AnalysisBeforeEmpathy],
            mode_turns: 3,
        };

        manager
            .save_session_control(&session.id, state.clone())
            .await
            .unwrap();

        let loaded = manager.load_session_control(&session.id).await.unwrap();
        assert_eq!(loaded.as_ref().map(|entry| entry.mode), Some(state.mode));
        assert_eq!(
            loaded.as_ref().map(|entry| entry.density),
            Some(state.density)
        );
        assert_eq!(loaded.as_ref().map(|entry| entry.mode_turns), Some(3));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn build_transcript_read_model_renders_history_fragment() {
        let (_temp_dir, _db_file, manager, _db_guard) = manager().await;
        let session = manager.resolve_session("cli", "user-1").await.unwrap();
        manager
            .record_turn_with_parts(
                &session.id,
                &[ChatMessagePartInput::new(
                    MessagePartKind::UserText,
                    "hello",
                )],
                &[
                    ChatMessagePartInput::new(MessagePartKind::AssistantText, "world"),
                    ChatMessagePartInput::new(MessagePartKind::Reasoning, "trace"),
                ],
                Some(5),
                Some(7),
            )
            .await
            .unwrap();

        let fragment = manager
            .render_history_fragment(&session.id, 100, 500)
            .await
            .unwrap();
        assert!(fragment.contains("[History]"));
        assert!(fragment.contains("assistant: world"));
        assert!(!fragment.contains("trace"));
        assert!(!fragment.contains("[reasoning]"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn fork_session_copies_recent_transcript_and_sets_parent_metadata() {
        let (_temp_dir, _db_file, manager, _db_guard) = manager().await;
        let session = manager.resolve_session("cli", "user-1").await.unwrap();
        manager
            .record_turn_with_parts(
                &session.id,
                &[ChatMessagePartInput::new(
                    MessagePartKind::UserText,
                    "hello",
                )],
                &[ChatMessagePartInput::new(
                    MessagePartKind::AssistantText,
                    "world",
                )],
                Some(5),
                Some(7),
            )
            .await
            .unwrap();

        let forked = manager
            .fork_session(&session.id, "cli-fork", "user-2", 100)
            .await
            .unwrap();
        let transcript = manager.get_transcript(&forked.id).await.unwrap();
        assert_eq!(transcript.len(), 2);
        assert_eq!(
            forked
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.forked_from_session_id.as_ref())
                .map(SessionId::as_str),
            Some(session.id.as_str())
        );
    }
}
