//! Shared helpers for mapping tool-loop runs to structured transcript parts
//! and rehydrating transcript history into provider messages.

use anyhow::Result;

use crate::contracts::ids::SessionId;
use crate::core::agent::tool_loop::{
    LoopDetectionEvent, LoopStopReason, ToolCallRecord, ToolLoopResult,
};
use crate::core::providers::response::ProviderMessage;
use crate::core::sessions::orchestrator::SessionOrchestrator;
use crate::core::sessions::types::{
    ChatMessagePartInput, MessagePartKind, TOOL_CALL_METADATA_ID, TOOL_CALL_METADATA_INPUT,
    TOOL_CALL_METADATA_NAME, TOOL_RESULT_METADATA_IS_ERROR, TOOL_RESULT_METADATA_TOOL_USE_ID,
};
use crate::utils::text::strip_reasoning;

pub async fn load_provider_history_async(
    session_manager: Option<&SessionOrchestrator>,
    channel_name: &str,
    session_key: Option<&str>,
    tenant_id: Option<&str>,
    max_tokens: usize,
) -> (Option<SessionId>, Vec<ProviderMessage>) {
    let Some(session_manager) = session_manager else {
        return (None, Vec::new());
    };
    let Some(session_key) = normalized_session_key(session_key).map(str::to_owned) else {
        return (None, Vec::new());
    };
    let scoped_session_key = tenant_scoped_session_key(&session_key, tenant_id);

    let session = match try_load_session_by_id(session_manager, &session_key).await {
        Some(session) => session,
        None => {
            match session_manager
                .resolve_session(channel_name, &scoped_session_key)
                .await
            {
                Ok(session) => session,
                Err(error) => {
                    tracing::warn!(
                        channel = channel_name,
                        session_key = scoped_session_key,
                        error = %error,
                        "failed to resolve transcript session"
                    );
                    return (None, Vec::new());
                }
            }
        }
    };

    let read_model = match session_manager
        .load_transcript_read_model(&session.id, Some(max_tokens))
        .await
    {
        Ok(read_model) => read_model,
        Err(error) => {
            tracing::warn!(
                channel = channel_name,
                session_key = scoped_session_key,
                session_id = %session.id,
                error = %error,
                "failed to load transcript history"
            );
            return (Some(session.id), Vec::new());
        }
    };
    provider_history_response(session.id, read_model.to_provider_messages())
}

/// Persist a completed tool-loop turn without blocking the async runtime.
///
/// # Errors
///
/// Returns an error if transcript persistence fails.
pub async fn persist_tool_loop_turn_async(
    session_manager: Option<&SessionOrchestrator>,
    session_id: Option<&SessionId>,
    user_message: &str,
    result: &ToolLoopResult,
) -> Result<()> {
    let Some(session_manager) = session_manager else {
        return Ok(());
    };
    let Some(session_id) = session_id else {
        return Ok(());
    };

    let (user_parts, assistant_parts) = build_turn_transcript_parts(user_message, result);
    session_manager
        .record_turn_with_parts(
            session_id,
            &user_parts,
            &assistant_parts,
            None,
            result.tokens_used,
        )
        .await
}

async fn try_load_session_by_id(
    manager: &SessionOrchestrator,
    key: &str,
) -> Option<crate::core::sessions::types::Session> {
    let id = SessionId::new(key);
    match manager.get_session_by_id(&id).await {
        Ok(Some(session)) => Some(session),
        _ => None,
    }
}

fn normalized_session_key(session_key: Option<&str>) -> Option<&str> {
    session_key.map(str::trim).filter(|key| !key.is_empty())
}

fn tenant_scoped_session_key(session_key: &str, tenant_id: Option<&str>) -> String {
    match tenant_id.map(str::trim).filter(|tenant| !tenant.is_empty()) {
        Some(tenant) => format!("tenant::{tenant}::{session_key}"),
        None => session_key.to_string(),
    }
}

fn provider_history_response(
    session_id: SessionId,
    history: Vec<ProviderMessage>,
) -> (Option<SessionId>, Vec<ProviderMessage>) {
    (Some(session_id), history)
}

fn build_turn_transcript_parts(
    user_message: &str,
    result: &ToolLoopResult,
) -> (Vec<ChatMessagePartInput>, Vec<ChatMessagePartInput>) {
    (
        build_user_transcript_parts(user_message),
        build_assistant_transcript_parts(result),
    )
}

fn build_user_transcript_parts(user_message: &str) -> Vec<ChatMessagePartInput> {
    vec![ChatMessagePartInput::new(
        MessagePartKind::UserText,
        user_message.trim(),
    )]
}

fn build_assistant_transcript_parts(result: &ToolLoopResult) -> Vec<ChatMessagePartInput> {
    let mut parts = result
        .tool_calls
        .iter()
        .enumerate()
        .flat_map(|(ordinal, tool_call)| transcript_parts_for_tool_call(tool_call, ordinal))
        .collect::<Vec<_>>();
    parts.extend(
        result
            .loop_detection_events
            .iter()
            .map(transcript_part_for_loop_detection),
    );

    let visible_final_text = strip_reasoning(&result.final_text).trim().to_string();
    if !visible_final_text.is_empty() {
        parts.push(ChatMessagePartInput::new(
            MessagePartKind::AssistantText,
            visible_final_text,
        ));
    }

    if parts.is_empty() {
        parts.push(ChatMessagePartInput::new(
            MessagePartKind::AssistantText,
            stop_reason_fallback_text(&result.stop_reason),
        ));
    }

    parts
}

fn transcript_parts_for_tool_call(
    tool_call: &ToolCallRecord,
    ordinal: usize,
) -> [ChatMessagePartInput; 2] {
    let synthetic_tool_use_id = format!("tool-call-{}-{ordinal}", tool_call.iteration);
    let tool_call_part = ChatMessagePartInput::new(
        MessagePartKind::ToolCall,
        format_tool_call_content(tool_call),
    )
    .with_metadata(serde_json::json!({
        TOOL_CALL_METADATA_ID: synthetic_tool_use_id,
        TOOL_CALL_METADATA_NAME: tool_call.tool_name,
        TOOL_CALL_METADATA_INPUT: tool_call.args,
        "iteration": tool_call.iteration,
    }));
    let tool_result_part = ChatMessagePartInput::new(
        MessagePartKind::ToolResult,
        format_tool_result_content(tool_call),
    )
    .with_metadata(serde_json::json!({
        TOOL_RESULT_METADATA_TOOL_USE_ID: synthetic_tool_use_id,
        "tool_name": tool_call.tool_name,
        "success": tool_call.result.success,
        TOOL_RESULT_METADATA_IS_ERROR: !tool_call.result.success,
        "iteration": tool_call.iteration,
        "taint_labels": tool_call.result.taint_labels,
        "attachments": tool_call.result.attachments,
        "error": tool_call.result.error,
    }));

    [tool_call_part, tool_result_part]
}

fn format_tool_call_content(tool_call: &ToolCallRecord) -> String {
    format!(
        "{tool_name} {args}",
        tool_name = tool_call.tool_name,
        args = tool_call.args
    )
}

fn format_tool_result_content(tool_call: &ToolCallRecord) -> String {
    let output = tool_call.result.output.trim();
    let error = tool_call.result.error.as_deref().map(str::trim);

    match (output.is_empty(), error) {
        (false, Some(error)) => format!("{output}\n[error] {error}"),
        (false, None) => output.to_string(),
        (true, Some(error)) => error.to_string(),
        (true, None) if tool_call.result.success => {
            "tool completed with no textual output".to_string()
        }
        (true, None) => "tool failed without error details".to_string(),
    }
}

fn transcript_part_for_loop_detection(event: &LoopDetectionEvent) -> ChatMessagePartInput {
    ChatMessagePartInput::new(
        MessagePartKind::LoopDetection,
        format!(
            "{} {} count={} threshold={} iteration={}",
            event.severity.as_str(),
            event.kind.as_str(),
            event.count,
            event.threshold,
            event.iteration
        ),
    )
    .with_metadata(serde_json::to_value(event).unwrap_or_else(|_| serde_json::json!({})))
}

fn stop_reason_fallback_text(stop_reason: &LoopStopReason) -> String {
    match stop_reason {
        LoopStopReason::Completed => "assistant completed without a textual reply".to_string(),
        LoopStopReason::MaxIterations => {
            "tool loop stopped after reaching the maximum iteration count".to_string()
        }
        LoopStopReason::Error(error) => error.clone(),
        LoopStopReason::ApprovalDenied => {
            "tool execution was denied by approval policy".to_string()
        }
        LoopStopReason::RateLimited => "tool loop stopped because it was rate limited".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::{NamedTempFile, TempDir};

    use super::{
        build_assistant_transcript_parts, load_provider_history_async, persist_tool_loop_turn_async,
    };
    use crate::contracts::ids::SessionId;
    use crate::core::agent::tool_loop::{
        LoopDetectionEvent, LoopDetectionKind, LoopDetectionSeverity, LoopStopReason,
        ToolCallRecord, ToolLoopResult,
    };
    use crate::core::sessions::SessionOrchestrator;
    use crate::core::sessions::types::{MessagePartKind, SessionConfig};
    use crate::core::tools::traits::ToolResult;
    async fn session_manager() -> (
        TempDir,
        NamedTempFile,
        SessionOrchestrator,
        crate::utils::test_env::TestDbGuard,
    ) {
        // Acquire exclusive access to the shared test Postgres DB and
        // TRUNCATE every table so we start from a clean slate. The guard
        // must be held for the entire test body to keep other tests blocked.
        let db_guard = crate::utils::test_env::acquire_test_db().await;
        let database_url = crate::utils::test_env::postgres_url()
            .expect("test requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL");
        let temp_dir = TempDir::new().expect("tempdir should be created");
        let workspace_dir = temp_dir.path().join("workspace");
        crate::utils::test_env::write_workspace_postgres_config(&workspace_dir, &database_url)
            .expect("test config should be written");
        let db_file = NamedTempFile::new_in(&workspace_dir).expect("session db file should exist");
        // Use the async `connect` path rather than the sync `new` path.
        // `new` spins up its own runtime and drops it at the end of the
        // function, which panics when called from inside an async test.
        let manager = SessionOrchestrator::connect(db_file.path(), SessionConfig::default())
            .await
            .expect("session manager should be created");
        (temp_dir, db_file, manager, db_guard)
    }

    fn sample_tool_loop_result() -> ToolLoopResult {
        ToolLoopResult {
            final_text: "<think>inspect repo shape</think>answer ready".to_string(),
            tool_calls: vec![ToolCallRecord {
                tool_name: "shell".to_string(),
                args: serde_json::json!({ "cmd": "pwd" }),
                result: ToolResult {
                    success: true,
                    output: "/tmp/workspace".to_string(),
                    error: None,
                    attachments: Vec::new(),
                    taint_labels: vec!["fs:read".to_string()],
                    semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
                },
                iteration: 1,
            }],
            attachments: Vec::new(),
            loop_detection_events: vec![LoopDetectionEvent {
                kind: LoopDetectionKind::Repeat,
                severity: LoopDetectionSeverity::Warning,
                count: 1,
                threshold: 1,
                iteration: 2,
            }],
            iterations: 1,
            tokens_used: None,
            stop_reason: LoopStopReason::Completed,
            logprobs: None,
            streaming_delivered: false,
        }
    }

    #[test]
    fn build_assistant_transcript_parts_capture_tools_and_visible_text_only() {
        let parts = build_assistant_transcript_parts(&sample_tool_loop_result());
        let part_kinds = parts.iter().map(|part| part.kind).collect::<Vec<_>>();

        assert_eq!(
            part_kinds,
            vec![
                MessagePartKind::ToolCall,
                MessagePartKind::ToolResult,
                MessagePartKind::LoopDetection,
                MessagePartKind::AssistantText,
            ]
        );
        assert_eq!(parts[2].metadata.as_ref().unwrap()["kind"], "repeat");
        assert_eq!(parts[3].content, "answer ready");
        assert_eq!(
            parts[0].metadata.as_ref().unwrap()["id"],
            serde_json::Value::String("tool-call-1-0".to_string())
        );
        assert_eq!(
            parts[1].metadata.as_ref().unwrap()["tool_use_id"],
            serde_json::Value::String("tool-call-1-0".to_string())
        );
    }

    #[test]
    fn build_assistant_transcript_parts_uses_finalized_visible_text() {
        let mut result = sample_tool_loop_result();
        result.final_text = "原因は接続順です。".to_string();

        let parts = build_assistant_transcript_parts(&result);
        assert_eq!(parts[3].content, "原因は接続順です。");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn load_provider_history_rehydrates_provider_messages() {
        let (_temp_dir, _db_file, manager, _db_guard) = session_manager().await;
        let session = manager
            .resolve_session("discord", "conversation::discord::channel-1")
            .await
            .unwrap();
        persist_tool_loop_turn_async(
            Some(&manager),
            Some(&session.id),
            "hello",
            &sample_tool_loop_result(),
        )
        .await
        .unwrap();

        let (session_id, history) = load_provider_history_async(
            Some(&manager),
            "discord",
            Some("conversation::discord::channel-1"),
            None,
            8_192,
        )
        .await;

        assert_eq!(
            session_id.as_ref().map(SessionId::as_str),
            Some(session.id.as_str())
        );
        assert_eq!(history.len(), 4);
        assert!(matches!(
            history[0].role,
            crate::core::providers::response::MessageRole::User
        ));
        assert!(matches!(
            history[1].role,
            crate::core::providers::response::MessageRole::Assistant
        ));
        assert!(matches!(
            history[2].role,
            crate::core::providers::response::MessageRole::User
        ));
        assert!(matches!(
            history[3].role,
            crate::core::providers::response::MessageRole::Assistant
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn load_provider_history_async_rehydrates_provider_messages() {
        let (_temp_dir, _db_file, manager, _db_guard) = session_manager().await;
        let session = manager
            .resolve_session("discord", "conversation::discord::channel-async")
            .await
            .unwrap();
        persist_tool_loop_turn_async(
            Some(&manager),
            Some(&session.id),
            "hello",
            &sample_tool_loop_result(),
        )
        .await
        .unwrap();

        let (session_id, history) = load_provider_history_async(
            Some(&manager),
            "discord",
            Some("conversation::discord::channel-async"),
            None,
            8_192,
        )
        .await;

        assert_eq!(
            session_id.as_ref().map(SessionId::as_str),
            Some(session.id.as_str())
        );
        assert_eq!(history.len(), 4);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn load_provider_history_prefers_direct_session_id_lookup() {
        let (_temp_dir, _db_file, manager, _db_guard) = session_manager().await;
        let session = manager
            .resolve_session("gateway_ws", "tenant::tenant-a::desktop")
            .await
            .unwrap();
        persist_tool_loop_turn_async(
            Some(&manager),
            Some(&session.id),
            "hello",
            &sample_tool_loop_result(),
        )
        .await
        .unwrap();

        let (session_id, history) = load_provider_history_async(
            Some(&manager),
            "gateway_ws",
            Some(session.id.as_str()),
            Some("tenant-a"),
            8_192,
        )
        .await;

        assert_eq!(
            session_id.as_ref().map(SessionId::as_str),
            Some(session.id.as_str())
        );
        assert_eq!(history.len(), 4);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn load_provider_history_scopes_session_key_by_tenant() {
        let (_temp_dir, _db_file, manager, _db_guard) = session_manager().await;

        let (first_tenant_session_id, _first_tenant_history) = load_provider_history_async(
            Some(&manager),
            "gateway_http",
            Some("shared-source"),
            Some("tenant-a"),
            8_192,
        )
        .await;
        let (second_tenant_session_id, _second_tenant_history) = load_provider_history_async(
            Some(&manager),
            "gateway_http",
            Some("shared-source"),
            Some("tenant-b"),
            8_192,
        )
        .await;

        assert_ne!(first_tenant_session_id, second_tenant_session_id);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn persist_tool_loop_turn_records_structured_transcript_parts() {
        let (_temp_dir, _db_file, manager, _db_guard) = session_manager().await;
        let session = manager
            .resolve_session("discord", "conversation::discord::channel-2")
            .await
            .unwrap();
        persist_tool_loop_turn_async(
            Some(&manager),
            Some(&session.id),
            "summarize this",
            &sample_tool_loop_result(),
        )
        .await
        .unwrap();

        let transcript = manager.get_transcript(&session.id).await.unwrap();
        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript[0].parts.len(), 1);
        assert_eq!(transcript[0].parts[0].kind, MessagePartKind::UserText);
        assert_eq!(transcript[1].parts[0].kind, MessagePartKind::ToolCall);
        assert_eq!(transcript[1].parts[1].kind, MessagePartKind::ToolResult);
        assert_eq!(transcript[1].parts[2].kind, MessagePartKind::LoopDetection);
        assert_eq!(transcript[1].parts[3].kind, MessagePartKind::AssistantText);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "requires postgres via TEST_DATABASE_URL or ASTEREL_POSTGRES_URL"]
    async fn persist_tool_loop_turn_async_records_structured_transcript_parts() {
        let (_temp_dir, _db_file, manager, _db_guard) = session_manager().await;
        let session = manager
            .resolve_session("discord", "conversation::discord::channel-async-2")
            .await
            .unwrap();
        persist_tool_loop_turn_async(
            Some(&manager),
            Some(&session.id),
            "summarize this",
            &sample_tool_loop_result(),
        )
        .await
        .unwrap();

        let transcript = manager.get_transcript(&session.id).await.unwrap();
        assert_eq!(transcript.len(), 2);
        assert_eq!(transcript[1].parts[0].kind, MessagePartKind::ToolCall);
        assert_eq!(transcript[1].parts[3].kind, MessagePartKind::AssistantText);
    }
}
