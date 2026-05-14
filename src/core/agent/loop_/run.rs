//! Top-level agent run entry point.
//!
//! Assembles session-level dependencies and delegates to one of three
//! execution modes:
//!
//! - **Single-message** (`-m`): runs exactly one turn, prints the
//!   response, and exits.  Memory recall and auto-save are skipped
//!   (ephemeral mode).
//! - **Broadcast-interactive**: driven by a `tokio::sync::broadcast`
//!   channel supplied by a supervising surface (e.g. the desktop UI).
//!   Each received string is treated as one user turn.
//! - **CLI-interactive**: reads lines from an mpsc receiver connected
//!   to stdin.  Displays a prompt banner and supports `/quit` to exit.
//!
//! Slash commands (e.g. `/think`, `/new`) are handled inline by
//! `handle_interactive_command_input` before any turn is executed,
//! and do not trigger a provider call.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};

use super::session::{TurnExecutionSettings, execute_main_session_turn_with_metrics};
use super::types::MainSessionTurnParams;
use crate::config::Config;
use crate::contracts::observability::{Observer, ObserverEvent};
use crate::core::conversation_commands::{Command, handle_command, parse_command};
use crate::core::memory::Memory;
use crate::core::persona::person_identity::resolve_person_id;
use crate::core::providers::response::ProviderMessage;
use crate::core::providers::{Provider, ThinkingLevel};
use crate::core::subagents::{SkillMetadataProvider, SubagentOrchestrator};
use crate::core::tools::{ToolExecutionAuditSink, ToolRegistry};
use crate::security::policy::EntityRateLimiter;
use crate::security::{ApprovalBroker, PermissionStore, SecurityPolicy};

/// Parameters for launching an agent run session.
pub struct RunRequest {
    /// Single-shot message; `None` enters interactive mode.
    pub message: Option<String>,
    /// Override the default LLM provider name.
    pub provider_override: Option<String>,
    /// Override the default model name.
    pub model_override: Option<String>,
    /// Sampling temperature for inference.
    pub temperature: f64,
    /// System prompt injected at the start of every turn.
    pub system_prompt: String,
    /// Optional sink for streaming token events.
    pub stream_sink: Option<Arc<dyn crate::core::providers::StreamSink>>,
    /// Broadcast sender for interactive input from a supervised surface.
    pub interactive_input_tx: Option<tokio::sync::broadcast::Sender<String>>,
    /// Broker for human-in-the-loop approval of tool calls.
    pub approval_broker: Option<Arc<dyn ApprovalBroker>>,
    /// Audit sink receiving tool execution records.
    pub execution_audit_sink: Option<Arc<dyn ToolExecutionAuditSink>>,
    /// CLI input receiver for interactive terminal sessions.
    pub cli_input_rx: Option<tokio::sync::mpsc::Receiver<String>>,
}

/// Assembled agent-session dependencies supplied by the runtime surface.
pub struct RunContext {
    /// Runtime observer for start/end event emission.
    pub observer: Arc<dyn Observer>,
    /// Surface-resolved provider label for observability.
    pub provider_name: String,
    /// Surface-resolved model label for observability and inference.
    pub model_name: String,
    /// Active security policy for the session.
    pub security: Arc<SecurityPolicy>,
    /// Shared memory backend for recall and persistence.
    pub memory: Arc<dyn Memory>,
    /// Primary provider used for the answer phase.
    pub answer_provider: Box<dyn Provider>,
    /// Secondary provider used for the reflect phase.
    pub reflect_provider: Box<dyn Provider>,
    /// Optional provider for augmentor-side LLM helpers.
    pub augmentor_provider: Option<Arc<dyn Provider>>,
    /// Tool registry for the main session.
    pub registry: Arc<ToolRegistry>,
    /// Shared rate limiter for tool execution.
    pub rate_limiter: Arc<EntityRateLimiter>,
    /// Shared permission store for approval and grant reuse.
    pub permission_store: Arc<PermissionStore>,
    /// Shared subagent runtime for delegation tools.
    pub subagent_manager: Arc<SubagentOrchestrator>,
    /// Shared skill metadata provider for prompt/index lookup.
    pub skill_metadata_provider: Arc<dyn SkillMetadataProvider>,
}

/// # Errors
///
/// Returns an error when agent turn execution fails over the supplied runtime
/// context.
pub async fn run(config: Arc<Config>, request: RunRequest, context: RunContext) -> Result<()> {
    let RunRequest {
        message,
        provider_override: _,
        model_override: _,
        temperature,
        system_prompt,
        stream_sink,
        interactive_input_tx,
        approval_broker,
        execution_audit_sink,
        cli_input_rx,
    } = request;
    let RunContext {
        observer,
        provider_name,
        model_name,
        security,
        memory,
        answer_provider,
        reflect_provider,
        augmentor_provider,
        registry,
        rate_limiter,
        permission_store,
        subagent_manager,
        skill_metadata_provider,
    } = context;

    let start = Instant::now();
    observer.record_event(&ObserverEvent::AgentStart {
        provider: provider_name,
        model: model_name.clone(),
    });

    let person_id = resolve_person_id(&config);
    let turn_params = MainSessionTurnParams {
        answer_provider: answer_provider.as_ref(),
        reflect_provider: reflect_provider.as_ref(),
        augmentor_provider,
        stream_sink,
        interactive_input_tx,
        approval_broker,
        execution_audit_sink,
        person_id: &person_id,
        system_prompt: &system_prompt,
        model_name: model_name.as_str(),
        temperature,
        registry,
        max_tool_iterations: config.autonomy.max_tool_loop_iterations,
        loop_detection: config.tools.loop_detection.clone(),
        rate_limiter,
        permission_store,
        subagent_manager,
        skill_metadata_provider,
    };

    let (token_sum, saw_token_usage) = run_session(
        &config,
        security.as_ref(),
        &memory,
        &turn_params,
        message,
        cli_input_rx,
        &observer,
    )
    .await
    .context("run agent session")?;

    let duration = start.elapsed();
    observer.record_event(&ObserverEvent::AgentEnd {
        duration,
        tokens_used: saw_token_usage.then_some(token_sum),
    });

    Ok(())
}

/// Build and populate the tool registry from explicit runtime parts.
#[must_use]
pub(super) fn init_tools(
    config: &Config,
    security: &Arc<SecurityPolicy>,
    mem: &Arc<dyn Memory>,
    _auth_broker: Option<&crate::security::auth::AuthBroker>,
) -> Arc<ToolRegistry> {
    let model_selection = config.resolve_model(None, None);
    crate::core::tools::build_tool_registry_from_parts(crate::core::tools::ToolRegistryConfig {
        security,
        memory: Arc::clone(mem),
        composio_key: if config.composio.enabled {
            config.composio.api_key.as_deref()
        } else {
            None
        },
        browser: &config.browser,
        tools: &config.tools,
        mcp: Some(&config.mcp),
        mcp_tool_provider: Arc::new(crate::core::tools::NoopMcpToolProvider::new()),
        taste: &config.taste,
        taste_provider: None,
        taste_model: &model_selection.model,
        channel_capabilities: None,
        codespace: &config.codespace,
    })
}

/// Append a completed user+assistant exchange to the rolling
/// conversation history, evicting the oldest messages once the
/// `max_history_messages` cap is reached.
///
/// The cap prevents unbounded growth during long interactive sessions
/// where the provider context window would otherwise overflow.
fn append_conversation_turn(
    conversation_history: &mut Vec<ProviderMessage>,
    user_message: &str,
    assistant_response: &str,
    max_history_messages: usize,
) {
    conversation_history.push(ProviderMessage::user(user_message));
    conversation_history.push(ProviderMessage {
        role: crate::core::providers::response::MessageRole::Assistant,
        content: vec![crate::core::providers::response::ContentBlock::Text {
            text: assistant_response.to_string(),
        }],
    });

    if conversation_history.len() > max_history_messages {
        let excess = conversation_history.len() - max_history_messages;
        conversation_history.drain(..excess);
    }
}

/// Try to handle `content` as a slash command.
///
/// Returns `true` if the input was recognized and consumed as a
/// command (no provider call should follow).  Returns `false` if the
/// input should be forwarded to the turn pipeline as a normal message.
///
/// Handles `/think [level|show|hide|status]` (extended thinking
/// control), `/new` (reset conversation history), and any other
/// registered commands via `handle_command`.
fn handle_interactive_command_input(
    content: &str,
    conversation_history: &mut Vec<ProviderMessage>,
    thinking_level: &mut ThinkingLevel,
    show_reasoning: &mut bool,
    default_thinking_level: ThinkingLevel,
) -> bool {
    let Some(command) = parse_command(content) else {
        return false;
    };

    match command {
        Command::Think { level } => {
            if let Some(raw_level) = level.as_deref() {
                if let Some(parsed) = ThinkingLevel::parse(raw_level) {
                    *thinking_level = parsed;
                    println!("Thinking level set to: {}", thinking_level.as_str());
                } else if raw_level.eq_ignore_ascii_case("show") {
                    *show_reasoning = true;
                    println!("Thinking visibility set to: show");
                } else if raw_level.eq_ignore_ascii_case("hide") {
                    *show_reasoning = false;
                    println!("Thinking visibility set to: hide");
                } else if raw_level.eq_ignore_ascii_case("status") {
                    println!(
                        "Thinking level: {}, visibility: {}",
                        thinking_level.as_str(),
                        if *show_reasoning { "show" } else { "hide" }
                    );
                } else {
                    println!(
                        "Unsupported /think argument: {raw_level} (use off|low|medium|high|show|hide|status)"
                    );
                }
            } else {
                *thinking_level = thinking_level.toggled();
                println!("Thinking level set to: {}", thinking_level.as_str());
            }
        }
        Command::New => {
            conversation_history.clear();
            *thinking_level = default_thinking_level;
            *show_reasoning = false;
            let result = handle_command(&Command::New);
            println!("{}", result.text);
        }
        other => {
            let result = handle_command(&other);
            println!("{}", result.text);
        }
    }

    true
}

/// Accumulates token usage across multiple turns for end-of-session
/// reporting.  Providers may not always report usage (e.g. during
/// streaming), so `saw_usage` tracks whether at least one turn
/// provided a non-`None` count before the total is surfaced to the
/// observer.
struct TokenAccumulator {
    sum: u64,
    saw_usage: bool,
}

impl TokenAccumulator {
    fn new() -> Self {
        Self {
            sum: 0,
            saw_usage: false,
        }
    }

    fn record(&mut self, tokens: Option<u64>) {
        if let Some(t) = tokens {
            self.sum = self.sum.saturating_add(t);
            self.saw_usage = true;
        }
    }

    fn into_pair(self) -> (u64, bool) {
        (self.sum, self.saw_usage)
    }
}

/// Mutable state shared across all turns of a single interactive
/// session.  Holds the rolling conversation history, the current
/// extended-thinking level (adjustable via `/think`), whether the
/// reasoning trace is visible, and cumulative token accounting.
struct InteractiveState {
    conversation_history: Vec<ProviderMessage>,
    thinking_level: ThinkingLevel,
    show_reasoning: bool,
    tokens: TokenAccumulator,
}

impl InteractiveState {
    fn new(default_thinking_level: ThinkingLevel) -> Self {
        Self {
            conversation_history: Vec::new(),
            thinking_level: default_thinking_level,
            show_reasoning: false,
            tokens: TokenAccumulator::new(),
        }
    }
}

fn is_quit_command(content: &str) -> bool {
    content == "/quit" || content == "/exit"
}

/// Processes one interactive input line and returns the assistant response,
/// or `None` if the input was a slash command handled inline.
async fn process_interactive_input(
    content: &str,
    config: &Config,
    security: &SecurityPolicy,
    mem: &Arc<dyn Memory>,
    turn_params: &MainSessionTurnParams<'_>,
    observer: &Arc<dyn Observer>,
    state: &mut InteractiveState,
) -> Result<Option<String>> {
    if handle_interactive_command_input(
        content,
        &mut state.conversation_history,
        &mut state.thinking_level,
        &mut state.show_reasoning,
        config.inference.default_thinking_level,
    ) {
        return Ok(None);
    }

    let outcome = execute_main_session_turn_with_metrics(
        config,
        security,
        Arc::clone(mem),
        turn_params,
        content,
        observer,
        TurnExecutionSettings {
            conversation_history: &state.conversation_history,
            thinking_level: state.thinking_level,
            show_reasoning: state.show_reasoning,
            ephemeral: false,
        },
    )
    .await
    .context("execute agent session turn")?;

    append_conversation_turn(
        &mut state.conversation_history,
        content,
        &outcome.response,
        20,
    );
    state.tokens.record(outcome.tokens_used);

    Ok(Some(outcome.response))
}

/// Run a single non-interactive turn, print the response to stdout,
/// and return token usage.  Used by the `-m` / `--message` CLI flag.
/// The turn is executed in ephemeral mode so no memory is recalled or
/// written, making it safe for scripted or one-shot invocations.
async fn run_session_single_message(
    config: &Config,
    security: &SecurityPolicy,
    mem: &Arc<dyn Memory>,
    turn_params: &MainSessionTurnParams<'_>,
    msg: &str,
    observer: &Arc<dyn Observer>,
) -> Result<(u64, bool)> {
    let mut tokens = TokenAccumulator::new();
    let outcome = execute_main_session_turn_with_metrics(
        config,
        security,
        Arc::clone(mem),
        turn_params,
        msg,
        observer,
        TurnExecutionSettings {
            conversation_history: &[],
            thinking_level: config.inference.default_thinking_level,
            show_reasoning: false,
            ephemeral: true,
        },
    )
    .await
    .context("execute agent session turn")?;
    tokens.record(outcome.tokens_used);
    println!("{}", outcome.response);
    Ok(tokens.into_pair())
}

/// Run an interactive session driven by a broadcast channel.
/// Used when a supervising surface (e.g. the desktop workbench) owns
/// the input stream and fans it out to multiple subscribers.
/// Lagged messages are warned about and skipped rather than causing
/// the session to abort.
async fn run_session_broadcast_interactive(
    config: &Config,
    security: &SecurityPolicy,
    mem: &Arc<dyn Memory>,
    turn_params: &MainSessionTurnParams<'_>,
    observer: &Arc<dyn Observer>,
    input_tx: &tokio::sync::broadcast::Sender<String>,
) -> Result<(u64, bool)> {
    let mut state = InteractiveState::new(config.inference.default_thinking_level);
    let mut rx = input_tx.subscribe();

    loop {
        match rx.recv().await {
            Ok(input) => {
                let content = input.trim().to_string();
                if content.is_empty() {
                    continue;
                }
                if is_quit_command(&content) {
                    break;
                }
                let _response = process_interactive_input(
                    &content,
                    config,
                    security,
                    mem,
                    turn_params,
                    observer,
                    &mut state,
                )
                .await?;
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                tracing::warn!(skipped, "broadcast receiver lagged, resuming");
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                break;
            }
        }
    }

    Ok(state.tokens.into_pair())
}

/// Run a terminal interactive session driven by an mpsc receiver
/// connected to stdin.  Displays the interactive-mode banner, then
/// processes each line as a user turn until `/quit` or channel close.
async fn run_session_cli_interactive(
    config: &Config,
    security: &SecurityPolicy,
    mem: &Arc<dyn Memory>,
    turn_params: &MainSessionTurnParams<'_>,
    observer: &Arc<dyn Observer>,
    cli_rx: &mut tokio::sync::mpsc::Receiver<String>,
) -> Result<(u64, bool)> {
    let mut state = InteractiveState::new(config.inference.default_thinking_level);

    println!("🐢 Asterel Interactive Mode");
    println!("Type /quit to exit.\n");

    while let Some(content) = cli_rx.recv().await {
        let content = content.trim().to_string();
        if content.is_empty() {
            continue;
        }
        if is_quit_command(&content) {
            break;
        }
        if let Some(response) = process_interactive_input(
            &content,
            config,
            security,
            mem,
            turn_params,
            observer,
            &mut state,
        )
        .await?
        {
            println!("\n{response}\n");
        }
    }

    Ok(state.tokens.into_pair())
}

/// Dispatch to the correct execution mode based on what was provided
/// in `RunRequest`.  Priority order:
/// 1. Single-message (if `message` is `Some`).
/// 2. Broadcast-interactive (if `interactive_input_tx` is present).
/// 3. CLI-interactive (if `cli_input_rx` is present).
///
/// Errors if none of the three inputs is present.
async fn run_session(
    config: &Config,
    security: &SecurityPolicy,
    mem: &Arc<dyn Memory>,
    turn_params: &MainSessionTurnParams<'_>,
    message: Option<String>,
    cli_input_rx: Option<tokio::sync::mpsc::Receiver<String>>,
    observer: &Arc<dyn Observer>,
) -> Result<(u64, bool)> {
    if let Some(msg) = message {
        return run_session_single_message(config, security, mem, turn_params, &msg, observer)
            .await;
    }

    if let Some(input_tx) = &turn_params.interactive_input_tx {
        return run_session_broadcast_interactive(
            config,
            security,
            mem,
            turn_params,
            observer,
            input_tx,
        )
        .await;
    }

    let Some(mut cli_rx) = cli_input_rx else {
        anyhow::bail!("interactive mode requires either interactive_input_tx or cli_input_rx");
    };

    run_session_cli_interactive(config, security, mem, turn_params, observer, &mut cli_rx).await
}
