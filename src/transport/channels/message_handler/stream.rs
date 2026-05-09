//! Streaming tool-loop runner for channel messages: sets up stream sinks,
//! manages Discord thinking embeds, and handles cancellation tokens.
#[cfg(feature = "discord")]
use std::fmt::Write as FmtWrite;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "discord")]
use std::time::Duration;
#[cfg(feature = "discord")]
use std::time::Instant;

use tokio::task::JoinHandle;

use anyhow::Result;

use super::super::attachments::prepare_channel_input_and_images;
#[cfg(feature = "discord")]
use super::super::discord::http_client::DiscordHttpClient;
use super::super::ingress_policy::ExternalIngressPolicyOutcome;
use super::super::startup::{ChannelRuntime, ChannelThinkingState};
use super::super::traits::ChannelMessage;
#[cfg(feature = "discord")]
use super::DiscordThinkingEmbedSink;
use super::media::media_processor_for_runtime;
use super::prompt::load_channel_thinking_state;
use super::reply::{handle_tool_loop_success, reply_to_origin, send_tool_loop_error_reply};
use super::{ChannelToolLoopInput, ToolLoopExecutionArtifacts, ToolLoopStreamState};
use crate::contracts::ids::PersonId;
use crate::core::agent::LoopStopReason;
use crate::core::persona::person_identity::{channel_entity_id, sanitize_person_id};
use crate::core::providers::InferenceOpts;
#[cfg(feature = "discord")]
use crate::core::providers::StreamEvent;
use crate::core::providers::streaming::{ChannelStreamSink, FanoutStreamSink, StreamSink};
use crate::core::tools::middleware::ExecutionContext;
use crate::runtime::services::{
    CompanionTransportTurnRequest, CompanionTurnRuntimeDeps, run_transport_companion_turn,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChannelTurnSessionBinding {
    canonical_session_id: Option<String>,
    owner_scope: String,
    tenant_id: Option<String>,
}

impl ChannelTurnSessionBinding {
    fn context_session_id<'a>(&'a self, owner_key: &'a str) -> &'a str {
        self.canonical_session_id.as_deref().unwrap_or(owner_key)
    }

    fn history_session_key<'a>(&'a self, owner_key: &'a str) -> &'a str {
        self.context_session_id(owner_key)
    }

    fn working_memory_session_id<'a>(&'a self, owner_key: &'a str) -> &'a str {
        self.context_session_id(owner_key)
    }

    fn history_tenant_id(&self) -> Option<&str> {
        self.tenant_id.as_deref()
    }
}

pub(super) fn tenant_scoped_owner_scope(
    owner_key: &str,
    policy_context: &crate::security::policy::TenantPolicyContext,
) -> String {
    match policy_context
        .tenant_id
        .as_deref()
        .map(str::trim)
        .filter(|tenant| !tenant.is_empty())
    {
        Some(tenant) => crate::core::sessions::render_tenant_owner_scope(tenant, owner_key),
        None => owner_key.to_string(),
    }
}

async fn resolve_channel_turn_session_binding(
    session_manager: Option<&crate::core::sessions::SessionOrchestrator>,
    surface: &str,
    owner_key: &str,
    policy_context: &crate::security::policy::TenantPolicyContext,
) -> Result<ChannelTurnSessionBinding> {
    let owner_scope = tenant_scoped_owner_scope(owner_key, policy_context);
    let canonical_session_id = match session_manager {
        Some(session_manager) => Some(
            session_manager
                .resolve_session(surface, &owner_scope)
                .await?
                .id
                .to_string(),
        ),
        None => None,
    };

    Ok(ChannelTurnSessionBinding {
        canonical_session_id,
        owner_scope,
        tenant_id: policy_context.tenant_id.clone(),
    })
}

fn build_channel_turn_base_prompt(
    workspace_dir: &std::path::Path,
    model: &str,
    tools: &[(&str, &str)],
    skill_entries: &[crate::plugins::skills::PromptSkillIndexEntry],
    channel_capabilities_section: Option<&str>,
    config: &crate::config::Config,
) -> String {
    let options = crate::transport::channels::SystemPromptOptions {
        companion_behavior: Some(config.persona.companion.clone()),
    };
    crate::transport::channels::build_system_prompt_from_index_opts(
        workspace_dir,
        model,
        tools,
        skill_entries,
        channel_capabilities_section,
        &options,
    )
}

/// Appends a text delta to a preview buffer, trimming from the front
/// when it exceeds `max_chars`.
#[cfg(feature = "discord")]
pub(super) fn append_preview(preview: &mut String, delta: &str, max_chars: usize) {
    preview.push_str(delta);
    // Byte count ≥ char count; skip O(n) scan when buffer is clearly short.
    if preview.len() <= max_chars {
        return;
    }
    let count = preview.chars().count();
    if count <= max_chars {
        return;
    }

    *preview = preview
        .chars()
        .skip(count.saturating_sub(max_chars))
        .collect();
}

/// Renders the description body for a Discord thinking embed message.
#[cfg(feature = "discord")]
pub(super) fn render_discord_thinking_description(
    status: &str,
    active_tool: Option<&str>,
    preview: &str,
    usage: Option<(u64, u64)>,
) -> String {
    let mut out = format!("Status: {status}");
    if let Some(tool_name) = active_tool {
        let _ = write!(out, "\nTool: `{tool_name}`");
    }
    if let Some((input_tokens, output_tokens)) = usage {
        let _ = write!(out, "\nUsage: in={input_tokens} out={output_tokens}");
    }
    if !preview.is_empty() {
        let escaped_preview = preview.replace("```", "'''");
        let _ = write!(out, "\n\nPreview:\n```text\n{escaped_preview}\n```");
    }
    out
}

/// Creates or edits a Discord embed message used as a thinking indicator.
#[cfg(feature = "discord")]
pub(super) async fn upsert_discord_thinking_embed(
    client: &DiscordHttpClient,
    channel_id: &str,
    message_id: &mut Option<String>,
    description: &str,
    color: u32,
) {
    if let Some(existing_message_id) = message_id.as_deref() {
        if let Err(error) = client
            .edit_embed(
                channel_id,
                existing_message_id,
                Some("Thinking"),
                description,
                Some(color),
            )
            .await
        {
            tracing::warn!(error = %error, "failed to edit discord thinking embed");
        }
        return;
    }

    match client
        .send_embed_message(channel_id, Some("Thinking"), description, Some(color))
        .await
    {
        Ok(created_message_id) => {
            *message_id = Some(created_message_id);
        }
        Err(error) => {
            tracing::warn!(error = %error, "failed to send discord thinking embed");
        }
    }
}

/// Consumes a stream-event channel and updates a Discord thinking embed
/// in real time as the tool loop progresses.
#[cfg(feature = "discord")]
pub(super) async fn run_discord_thinking_embed_forwarder(
    bot_token: String,
    channel_id: String,
    mut rx: tokio::sync::mpsc::Receiver<StreamEvent>,
    include_preview: bool,
) {
    use super::{DISCORD_THINKING_COLOR_ACTIVE, DISCORD_THINKING_COLOR_COMPLETED};

    let client = DiscordHttpClient::new(bot_token);
    let mut embed_message_id: Option<String> = None;
    let mut status = String::from("Preparing response...");
    let mut active_tool: Option<String> = None;
    let mut preview = String::new();
    let mut usage: Option<(u64, u64)> = None;
    let mut last_flush = Instant::now()
        .checked_sub(Duration::from_millis(850))
        .unwrap_or_else(Instant::now);
    let mut dirty = true;

    while let Some(event) = rx.recv().await {
        let mut force_flush = false;
        let mut completed = false;
        match event {
            StreamEvent::ResponseStart { .. } => {
                status = "Thinking...".to_string();
                active_tool = None;
                dirty = true;
                force_flush = true;
            }
            StreamEvent::TextDelta { text } => {
                status = "Generating response...".to_string();
                if include_preview {
                    append_preview(&mut preview, &text, 450);
                }
                dirty = true;
            }
            StreamEvent::ToolCallDelta { name, .. } => {
                if let Some(tool_name) = name {
                    active_tool = Some(tool_name.clone());
                    status = format!("Running tool `{tool_name}`...");
                    dirty = true;
                    force_flush = true;
                }
            }
            StreamEvent::ToolCallComplete { name, .. } => {
                active_tool = Some(name.clone());
                status = format!("Tool `{name}` completed.");
                dirty = true;
                force_flush = true;
            }
            StreamEvent::Done {
                input_tokens,
                output_tokens,
                ..
            } => {
                status = "Completed".to_string();
                usage = match (input_tokens, output_tokens) {
                    (Some(input), Some(output)) => Some((input, output)),
                    _ => None,
                };
                dirty = true;
                force_flush = true;
                completed = true;
            }
        }

        if dirty && (force_flush || last_flush.elapsed() >= Duration::from_millis(850)) {
            let description = render_discord_thinking_description(
                &status,
                active_tool.as_deref(),
                &preview,
                usage,
            );
            let color = if completed {
                DISCORD_THINKING_COLOR_COMPLETED
            } else {
                DISCORD_THINKING_COLOR_ACTIVE
            };
            upsert_discord_thinking_embed(
                &client,
                &channel_id,
                &mut embed_message_id,
                &description,
                color,
            )
            .await;
            last_flush = Instant::now();
            dirty = false;
        }

        if completed {
            break;
        }
    }
}

/// Creates a channel-side stream sink that forwards text chunks to the
/// originating channel in real time.
pub(super) fn setup_channel_stream_sink(
    rt: &ChannelRuntime,
    channel_name: &str,
    reply_target: &str,
    streamed_output: &Arc<AtomicBool>,
    show_reasoning: bool,
) -> (Option<JoinHandle<()>>, Option<Arc<dyn StreamSink>>) {
    let Some(channel) = rt
        .channels
        .iter()
        .find(|channel| channel.name() == channel_name)
    else {
        return (None, None);
    };

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(64);
    let channel = Arc::clone(channel);
    let recipient = reply_target.to_string();
    let channel_name = channel_name.to_string();
    let streamed_flag = Arc::clone(streamed_output);
    // Concurrency is bounded by the routing queue's global_concurrency
    // semaphore. If this function is called outside the routing queue,
    // add an explicit semaphore here.
    let handle = tokio::spawn(async move {
        while let Some(chunk) = rx.recv().await {
            if chunk.is_empty() {
                continue;
            }
            if let Err(error) = channel.send(&chunk, &recipient).await {
                tracing::warn!(
                    channel = %channel_name,
                    recipient = %recipient,
                    error = %error,
                    "failed to stream channel chunk"
                );
                break;
            }
            streamed_flag.store(true, Ordering::SeqCst);
        }
    });

    let sink: Arc<dyn StreamSink> = Arc::new(ChannelStreamSink::new_with_reasoning(
        tx,
        80,
        show_reasoning,
    ));
    (Some(handle), Some(sink))
}

#[cfg(feature = "discord")]
pub(super) fn setup_discord_thinking_sink(
    rt: &ChannelRuntime,
    channel_name: &str,
    conversation_id: Option<&str>,
    reply_target: &str,
    thinking_state: ChannelThinkingState,
) -> Option<(JoinHandle<()>, Arc<dyn StreamSink>)> {
    if channel_name != "discord" {
        return None;
    }

    let discord = rt.config.channels_config.discord.as_ref()?;
    if !discord.thinking_embed {
        return None;
    }

    let (event_tx, event_rx) = tokio::sync::mpsc::channel::<StreamEvent>(32);
    let raw_id = conversation_id.map_or_else(|| reply_target.to_string(), ToString::to_string);
    // Extract the real Discord channel ID if this is an interaction-routed conversation_id.
    // Format: "discord_interaction|{channel_id}|{route_id}"
    let channel_id = if let Some(rest) = raw_id.strip_prefix("discord_interaction|") {
        rest.split('|').next().unwrap_or(&raw_id).to_string()
    } else {
        raw_id
    };
    let handle = tokio::spawn(run_discord_thinking_embed_forwarder(
        discord.bot_token.clone(),
        channel_id,
        event_rx,
        discord.thinking_embed_include_preview && thinking_state.show_reasoning,
    ));
    let sink: Arc<dyn StreamSink> = Arc::new(DiscordThinkingEmbedSink::new(event_tx));
    Some((handle, sink))
}

#[cfg(not(feature = "discord"))]
pub(super) fn setup_discord_thinking_sink(
    _rt: &ChannelRuntime,
    _channel_name: &str,
    _conversation_id: Option<&str>,
    _reply_target: &str,
    _thinking_state: ChannelThinkingState,
) -> Option<(JoinHandle<()>, Arc<dyn StreamSink>)> {
    None
}

/// Assembles the combined stream sink (channel + Discord thinking embed)
/// and tracking state for a tool loop execution.
pub(super) fn build_stream_sink_and_state(
    rt: &ChannelRuntime,
    channel_name: &str,
    conversation_id: Option<&str>,
    reply_target: &str,
    thinking_state: ChannelThinkingState,
    enable_streaming: bool,
) -> (Option<Arc<dyn StreamSink>>, ToolLoopStreamState) {
    if !enable_streaming {
        return (
            None,
            ToolLoopStreamState {
                stream_forward_handle: None,
                discord_thinking_embed_handle: None,
                streamed_output: Arc::new(AtomicBool::new(false)),
            },
        );
    }

    let streamed_output = Arc::new(AtomicBool::new(false));
    let (stream_forward_handle, channel_sink) = setup_channel_stream_sink(
        rt,
        channel_name,
        reply_target,
        &streamed_output,
        thinking_state.show_reasoning,
    );
    let mut stream_sinks: Vec<Arc<dyn StreamSink>> = Vec::new();
    if let Some(sink) = channel_sink {
        stream_sinks.push(sink);
    }

    let mut discord_thinking_embed_handle = None;
    if let Some((handle, sink)) = setup_discord_thinking_sink(
        rt,
        channel_name,
        conversation_id,
        reply_target,
        thinking_state,
    ) {
        discord_thinking_embed_handle = Some(handle);
        stream_sinks.push(sink);
    }

    let stream_sink = match stream_sinks.len() {
        0 => None,
        1 => stream_sinks.pop(),
        _ => Some(Arc::new(FanoutStreamSink::new(stream_sinks)) as Arc<dyn StreamSink>),
    };

    (
        stream_sink,
        ToolLoopStreamState {
            stream_forward_handle,
            discord_thinking_embed_handle,
            streamed_output,
        },
    )
}

fn session_history_token_limit(config: &crate::config::Config) -> usize {
    usize::try_from(config.session.parent_fork_max_tokens).unwrap_or(usize::MAX)
}

fn channel_turn_person_id(channel_name: &str, sender: &str) -> PersonId {
    PersonId::new(sanitize_person_id(&format!("{channel_name}.{sender}")))
}

fn resolve_channel_inference_target<'a>(
    rt: &'a ChannelRuntime,
    channel_name: &str,
) -> (&'a dyn crate::core::providers::Provider, &'a str) {
    if let Some(target) = rt.channel_inference.get(channel_name) {
        (target.provider.as_ref(), target.model.as_str())
    } else {
        (rt.provider.as_ref(), rt.model.as_str())
    }
}

/// Runs the full tool loop for a channel message, including media
/// processing, streaming setup, and inference.
#[allow(clippy::too_many_lines)]
pub(super) async fn execute_channel_tool_loop(
    rt: &ChannelRuntime,
    input: ChannelToolLoopInput<'_>,
    ctx: &ExecutionContext,
) -> ToolLoopExecutionArtifacts {
    let (provider, model) = resolve_channel_inference_target(rt, input.channel_name);
    let policy_context = rt.tenant_policy_context.clone();
    let session_binding = resolve_channel_turn_session_binding(
        rt.session_manager.as_deref(),
        input.channel_name,
        input.thinking_key,
        &policy_context,
    )
    .await;
    let media_processor = media_processor_for_runtime(rt);
    let (message_input, image_blocks) = prepare_channel_input_and_images(
        input.user_message,
        input.attachments,
        rt.media_store.as_ref(),
        &media_processor,
    )
    .await;
    let thinking_state = if input.enable_streaming {
        load_channel_thinking_state(rt, input.channel_name, input.thinking_key).await
    } else {
        ChannelThinkingState::from_config(&rt.config)
    };
    let (stream_sink, stream_state) = build_stream_sink_and_state(
        rt,
        input.channel_name,
        input.conversation_id,
        input.reply_target,
        thinking_state,
        input.enable_streaming,
    );
    let mut turn_ctx = ctx.clone();
    let session_binding = match session_binding {
        Ok(binding) => binding,
        Err(error) => {
            return ToolLoopExecutionArtifacts {
                result: Err(error),
                stream_state,
                show_reasoning: thinking_state.show_reasoning,
                media_processor,
            };
        }
    };
    turn_ctx.session_id = Some(
        session_binding
            .context_session_id(input.thinking_key)
            .to_string(),
    );
    let tool_specs = rt.registry.specs_for_context(&turn_ctx);
    let tool_descs = tool_specs
        .iter()
        .map(|spec| (spec.name.clone(), spec.description.clone()))
        .collect::<Vec<_>>();
    let prompt_tool_descs = tool_descs
        .iter()
        .map(|(name, description)| (name.as_str(), description.as_str()))
        .collect::<Vec<_>>();
    let prompt_skill_snapshot =
        crate::plugins::skills::load_skill_metadata_snapshot_with_policy_and_config(
            turn_ctx.workspace_dir.as_path(),
            rt.security.as_ref(),
            &rt.config.skills,
        );
    let prompt_skill_entries = prompt_skill_snapshot.search_index().prompt_index_entries();
    let effective_prompt = build_channel_turn_base_prompt(
        turn_ctx.workspace_dir.as_path(),
        model,
        &prompt_tool_descs,
        &prompt_skill_entries,
        rt.channel_capabilities_section.as_deref(),
        &rt.config,
    );
    turn_ctx.delegation_system_prompt = Some(effective_prompt.clone());
    let entity_id =
        policy_context.scope_entity_id(&channel_entity_id(input.channel_name, input.thinking_key));
    let person_id = channel_turn_person_id(input.channel_name, input.sender);
    let result = run_transport_companion_turn(CompanionTransportTurnRequest {
        runtime: CompanionTurnRuntimeDeps {
            mem: Arc::clone(&rt.mem),
            persona_config: &rt.config.persona,
            session_manager: rt.session_manager.as_deref(),
            working_memory_capacity: rt.config.memory.working_memory_capacity,
            registry: Arc::clone(&rt.registry),
            max_tool_iterations: rt.config.autonomy.max_tool_loop_iterations,
            loop_detection: rt.config.tools.loop_detection.clone(),
            response_finalization_enabled: rt.config.persona.enable_response_finalization,
            naturalness_gate_enabled: rt.config.persona.enable_naturalness_gate,
            self_amendment_candidate_sink: Some(Arc::new(
                rt.self_amendment_candidate_review.clone(),
            )),
        },
        workspace_dir: turn_ctx.workspace_dir.as_path(),
        base_prompt: &effective_prompt,
        user_message: &message_input,
        entity_id: &entity_id,
        person_id: person_id.as_str(),
        base_temperature: rt.temperature,
        policy_context: &policy_context,
        session_surface: Some(input.channel_name),
        channel_context_hint: input.channel_context_hint,
        surface_realization_policy: rt.channel_surface_policies_by_name.get(input.channel_name),
        session_owner_scope: Some(session_binding.owner_scope.as_str()),
        working_memory_session_id: session_binding.working_memory_session_id(input.thinking_key),
        history_channel_name: input.channel_name,
        history_session_key: Some(session_binding.history_session_key(input.thinking_key)),
        history_tenant_id: session_binding.history_tenant_id(),
        history_max_tokens: session_history_token_limit(&rt.config),
        fallback_history: &[],
        provider,
        image_content: &image_blocks,
        model,
        inference_options: Some(InferenceOpts::from_thinking_level(
            thinking_state.thinking_level,
        )),
        ctx: &turn_ctx,
        stream_sink,
        state_notifier: None,
        transcript_log_target: "transport::channels::message_handler",
    })
    .await;

    ToolLoopExecutionArtifacts {
        result,
        stream_state,
        show_reasoning: thinking_state.show_reasoning,
        media_processor,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_channel_turn_base_prompt, channel_turn_person_id, tenant_scoped_owner_scope,
    };
    use crate::config::Config;
    use crate::plugins::skills::PromptSkillIndexEntry;
    use tempfile::TempDir;

    #[test]
    fn channel_turn_person_id_is_sender_scoped() {
        assert_eq!(
            channel_turn_person_id("discord", "u/123").as_str(),
            "discord.u_123__h9f5e7e9a62f3"
        );
    }

    #[test]
    fn tenant_scoped_owner_scope_prefixes_active_tenant() {
        let context = crate::security::policy::TenantPolicyContext::enabled("tenant-alpha");
        assert_eq!(
            tenant_scoped_owner_scope("conversation::discord::room-1", &context),
            "tenant::tenant-alpha::conversation::discord::room-1"
        );
    }

    #[test]
    fn build_channel_turn_base_prompt_uses_current_workspace_and_tools() {
        let root = TempDir::new().expect("temp dir");
        let turn_workspace = root.path().join("tenant-workspace");
        std::fs::create_dir_all(&turn_workspace).expect("create workspace");
        std::fs::write(turn_workspace.join("SOUL.md"), "# Soul\nTenant persona")
            .expect("write SOUL");
        std::fs::write(turn_workspace.join("CHARACTER.md"), "# Character\nWarm")
            .expect("write CHARACTER");
        std::fs::write(turn_workspace.join("USER.md"), "# User\nScoped").expect("write USER");
        std::fs::write(
            turn_workspace.join("AGENTS.md"),
            "# Agents\nScoped instructions",
        )
        .expect("write AGENTS");
        std::fs::write(turn_workspace.join("TOOLS.md"), "# Tools\nScoped tools")
            .expect("write TOOLS");
        std::fs::write(
            turn_workspace.join("HEARTBEAT.md"),
            "# Heartbeat\nScoped status",
        )
        .expect("write HEARTBEAT");
        std::fs::write(turn_workspace.join("MEMORY.md"), "# Memory\nScoped memory")
            .expect("write MEMORY");

        let prompt = build_channel_turn_base_prompt(
            &turn_workspace,
            "model",
            &[("memory_recall", "Search memory")],
            &[PromptSkillIndexEntry {
                name: "code-review".to_string(),
                location: "skills/code-review/SKILL.md".to_string(),
            }],
            Some("## Channels\nDiscord"),
            &Config::default(),
        );

        assert!(prompt.contains(&turn_workspace.display().to_string()));
        assert!(prompt.contains("**memory_recall**"));
        assert!(prompt.contains("Tenant persona"));
        assert!(!prompt.contains("[Channel Context: Thread continuation]"));
    }
}

/// Joins stream tasks and dispatches the tool loop result as a reply
/// or error message back to the originating channel.
pub(super) async fn process_tool_loop_result(
    rt: &ChannelRuntime,
    msg: &ChannelMessage,
    reply_target: &str,
    execution: ToolLoopExecutionArtifacts,
) {
    let ToolLoopExecutionArtifacts {
        result,
        stream_state,
        show_reasoning,
        media_processor,
    } = execution;
    join_stream_task(stream_state.stream_forward_handle, "stream forward").await;
    join_stream_task(
        stream_state.discord_thinking_embed_handle,
        "discord thinking embed",
    )
    .await;
    let streamed_any_output = stream_state.streamed_output.load(Ordering::SeqCst);
    match result {
        Ok(outcome) => {
            let result = outcome.result;
            handle_tool_loop_success(
                rt,
                msg,
                reply_target,
                result,
                streamed_any_output,
                show_reasoning,
                &media_processor,
            )
            .await;
        }
        Err(error) => send_tool_loop_error_reply(rt, msg, reply_target, &error.to_string()).await,
    }
}

/// Awaits a spawned stream task, logging if it panicked.
pub(super) async fn join_stream_task(handle: Option<JoinHandle<()>>, label: &str) {
    if let Some(handle) = handle
        && let Err(error) = handle.await
    {
        tracing::warn!(%error, label, "channel stream task panicked");
    }
}

/// Logs notable tool loop stop reasons (max iterations, rate limited).
pub(super) fn log_stop_reason(msg: &ChannelMessage, stop_reason: &LoopStopReason) {
    match stop_reason {
        LoopStopReason::MaxIterations => {
            tracing::warn!(channel = %msg.channel, sender = %msg.sender, "tool loop hit max iterations");
        }
        LoopStopReason::RateLimited => {
            tracing::warn!(channel = %msg.channel, sender = %msg.sender, "tool loop halted by rate limiting");
        }
        LoopStopReason::Completed | LoopStopReason::ApprovalDenied | LoopStopReason::Error(_) => {}
    }
}

/// Sends a safety-block reply if the ingress policy blocked the message.
/// Returns `true` when the message was blocked.
pub(super) async fn handle_blocked_ingress_reply(
    rt: &ChannelRuntime,
    msg: &ChannelMessage,
    reply_target: &str,
    source: &str,
    ingress: &ExternalIngressPolicyOutcome,
) -> bool {
    if !ingress.blocked {
        return false;
    }

    tracing::warn!(
        source,
        "blocked high-risk external content at channel ingress"
    );
    if let Err(error) = reply_to_origin(
        &rt.channels,
        &msg.channel,
        "⚠️ External content was blocked by safety policy.",
        reply_target,
    )
    .await
    {
        tracing::warn!(%error, "failed to send channel safety block reply");
    }
    true
}
