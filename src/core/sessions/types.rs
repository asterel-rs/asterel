//! Session data types: `Session`, `ChatMessage`, `MessageRole`,
//! `SessionState`, compaction configuration, and context budget types.
use serde::{Deserialize, Serialize};

use crate::contracts::ids::{MessageId, SessionId, UserId};
use crate::contracts::session_control::SessionControlState;
use crate::core::providers::ThinkingLevel;
use crate::core::providers::response::{
    ContentBlock, MessageRole as ProviderMessageRole, ProviderMessage,
};

/// Role of a chat message participant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// Message from the user.
    User,
    /// Message from the assistant.
    Assistant,
    /// System-level message (e.g. compaction summary).
    System,
}

/// Fine-grained transcript part kind stored alongside a chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessagePartKind {
    /// Raw user-authored text.
    UserText,
    /// Raw assistant-authored text.
    AssistantText,
    /// System-generated text.
    SystemText,
    /// Hidden or explainability-only reasoning trace.
    Reasoning,
    /// Tool invocation record.
    ToolCall,
    /// Tool output or result payload.
    ToolResult,
    /// Patch or diff content.
    Patch,
    /// Compaction summary or rehydration message.
    Compaction,
    /// Event emitted by a sub-agent.
    SubagentEvent,
    /// Runtime metadata recorded for the turn.
    RuntimeMetadata,
    /// Structured tool loop detection telemetry.
    LoopDetection,
}

/// Input for creating a transcript part before ids/timestamps exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessagePartInput {
    /// Semantic kind of transcript part.
    pub kind: MessagePartKind,
    /// Optional mime type when the content has a richer representation.
    pub mime_type: Option<String>,
    /// Payload content for the part.
    pub content: String,
    /// Optional structured metadata for the part.
    pub metadata: Option<serde_json::Value>,
}

impl ChatMessagePartInput {
    /// Create a new transcript part input with plain text content.
    #[must_use]
    pub fn new(kind: MessagePartKind, content: impl Into<String>) -> Self {
        Self {
            kind,
            mime_type: None,
            content: content.into(),
            metadata: None,
        }
    }

    /// Attach JSON metadata to the part.
    #[must_use]
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Attach a mime type to the part.
    #[must_use]
    pub fn with_mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }
}

/// Persisted transcript part belonging to a chat message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessagePart {
    /// Unique transcript part identifier.
    pub id: String,
    /// Parent message id.
    pub message_id: MessageId,
    /// Parent session id.
    pub session_id: SessionId,
    /// Stable ordering within the message.
    pub ordinal: usize,
    /// Semantic part kind.
    pub kind: MessagePartKind,
    /// Optional mime type when the payload is structured.
    pub mime_type: Option<String>,
    /// Payload content for the transcript part.
    pub content: String,
    /// Optional JSON metadata associated with the part.
    pub metadata: Option<serde_json::Value>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
}

/// A single chat message within a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Unique message identifier.
    pub id: MessageId,
    /// Session this message belongs to.
    pub session_id: SessionId,
    /// Role of the message author.
    pub role: MessageRole,
    /// Message text content.
    pub content: String,
    /// Input token count for billing, if tracked.
    pub input_tokens: Option<u64>,
    /// Output token count for billing, if tracked.
    pub output_tokens: Option<u64>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
}

impl ChatMessage {
    /// Return the token count for this message, preferring tracked values
    /// and falling back to a character-based estimate.
    #[must_use]
    pub fn estimated_tokens(&self) -> usize {
        if let Some(input) = self.input_tokens {
            // Cast safety: token counts are small values that fit in usize
            #[allow(clippy::cast_possible_truncation)]
            return input as usize;
        }
        if let Some(output) = self.output_tokens {
            // Cast safety: token counts are small values that fit in usize
            #[allow(clippy::cast_possible_truncation)]
            return output as usize;
        }
        estimate_tokens(&self.content)
    }

    /// Default transcript part kind for this message's flat content.
    #[must_use]
    pub const fn default_part_kind_for_role(role: MessageRole) -> MessagePartKind {
        match role {
            MessageRole::User => MessagePartKind::UserText,
            MessageRole::Assistant => MessagePartKind::AssistantText,
            MessageRole::System => MessagePartKind::SystemText,
        }
    }
}

/// Message together with its structured transcript parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptMessage {
    /// Flat chat message row: carries id, role, content, and token counts.
    /// The `parts` field holds structured content blocks; `message` is the
    /// authoritative source for metadata (role, tokens) and the fallback
    /// content when parts are absent.
    pub message: ChatMessage,
    /// Ordered transcript parts belonging to the message.
    pub parts: Vec<ChatMessagePart>,
}

impl TranscriptMessage {
    /// Estimated token count for this transcript message.
    #[must_use]
    pub fn estimated_tokens(&self) -> usize {
        self.message.estimated_tokens()
    }

    /// Convert to provider-side messages for prompt/history reconstruction.
    #[must_use]
    pub fn to_provider_messages(&self) -> Vec<ProviderMessage> {
        if self.parts.is_empty() {
            return vec![provider_message_from_role(
                self.message.role,
                self.message.content.clone(),
            )];
        }

        let mut messages = Vec::new();
        let mut buffered_blocks = Vec::new();
        let base_role = provider_role_from_session_role(self.message.role);

        for part in &self.parts {
            match part.to_provider_block() {
                TranscriptProviderBlock::SameRole(block) => buffered_blocks.push(block),
                TranscriptProviderBlock::Standalone(message) => {
                    if !buffered_blocks.is_empty() {
                        messages.push(ProviderMessage {
                            role: base_role,
                            content: std::mem::take(&mut buffered_blocks),
                        });
                    }
                    messages.push(message);
                }
                TranscriptProviderBlock::Skip => {}
            }
        }

        if !buffered_blocks.is_empty() {
            messages.push(ProviderMessage {
                role: base_role,
                content: buffered_blocks,
            });
        }

        messages
    }
}

/// Read model for a transcript tail that can be rendered or converted into
/// provider history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTranscriptReadModel {
    /// Session identifier the transcript was loaded from.
    pub session_id: SessionId,
    /// Ordered transcript messages.
    pub messages: Vec<TranscriptMessage>,
    /// Best-effort token estimate for the selected transcript.
    pub estimated_tokens: usize,
}

impl SessionTranscriptReadModel {
    /// Create a read model from an ordered transcript.
    #[must_use]
    pub fn new(session_id: SessionId, messages: Vec<TranscriptMessage>) -> Self {
        let estimated_tokens = messages
            .iter()
            .map(TranscriptMessage::estimated_tokens)
            .sum();
        Self {
            session_id,
            messages,
            estimated_tokens,
        }
    }

    /// Keep only the newest messages that fit within the requested token budget.
    #[must_use]
    pub fn tail_within_token_limit(&self, max_tokens: usize) -> Self {
        if max_tokens == 0 || self.messages.is_empty() {
            return Self::new(self.session_id.clone(), Vec::new());
        }

        let mut total = 0usize;
        let mut selected = Vec::new();
        for message in self.messages.iter().rev() {
            let tokens = message.estimated_tokens();
            if !selected.is_empty() && total.saturating_add(tokens) > max_tokens {
                break;
            }
            total = total.saturating_add(tokens);
            selected.push(message.clone());
        }
        selected.reverse();
        Self::new(self.session_id.clone(), selected)
    }

    /// Convert the transcript into provider-side history messages.
    #[must_use]
    pub fn to_provider_messages(&self) -> Vec<ProviderMessage> {
        self.messages
            .iter()
            .flat_map(TranscriptMessage::to_provider_messages)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOwnerScope<'a> {
    Unscoped {
        key: &'a str,
    },
    Principal {
        principal: &'a str,
        key: &'a str,
    },
    Tenant {
        tenant_id: &'a str,
        key: &'a str,
    },
    TenantPrincipal {
        tenant_id: &'a str,
        principal: &'a str,
        key: &'a str,
    },
    Opaque {
        raw: &'a str,
    },
}

impl<'a> SessionOwnerScope<'a> {
    #[must_use]
    pub fn parse(raw: &'a str) -> Self {
        if let Some(rest) = raw.strip_prefix("tenant::") {
            let Some((tenant_id, remainder)) = rest.split_once("::") else {
                return Self::Opaque { raw };
            };
            if tenant_id.is_empty() || remainder.is_empty() {
                return Self::Opaque { raw };
            }
            if let Some(principal_rest) = remainder.strip_prefix("principal::") {
                let Some((principal, key)) = principal_rest.split_once("::") else {
                    return Self::Opaque { raw };
                };
                if principal.is_empty() || key.is_empty() {
                    return Self::Opaque { raw };
                }
                return Self::TenantPrincipal {
                    tenant_id,
                    principal,
                    key,
                };
            }
            return Self::Tenant {
                tenant_id,
                key: remainder,
            };
        }

        if let Some(rest) = raw.strip_prefix("principal::") {
            let Some((principal, key)) = rest.split_once("::") else {
                return Self::Opaque { raw };
            };
            if principal.is_empty() || key.is_empty() {
                return Self::Opaque { raw };
            }
            return Self::Principal { principal, key };
        }

        if raw.trim().is_empty() {
            return Self::Opaque { raw };
        }

        Self::Unscoped { key: raw }
    }

    #[must_use]
    pub const fn tenant_id(self) -> Option<&'a str> {
        match self {
            Self::Tenant { tenant_id, .. } | Self::TenantPrincipal { tenant_id, .. } => {
                Some(tenant_id)
            }
            Self::Unscoped { .. } | Self::Principal { .. } | Self::Opaque { .. } => None,
        }
    }

    #[must_use]
    pub const fn principal(self) -> Option<&'a str> {
        match self {
            Self::Principal { principal, .. } | Self::TenantPrincipal { principal, .. } => {
                Some(principal)
            }
            Self::Unscoped { .. } | Self::Tenant { .. } | Self::Opaque { .. } => None,
        }
    }
}

#[must_use]
pub fn render_tenant_owner_scope(tenant_id: &str, key: &str) -> String {
    format!("tenant::{tenant_id}::{key}")
}

#[must_use]
pub fn render_principal_owner_scope(principal: &str, key: &str) -> String {
    format!("principal::{principal}::{key}")
}

#[must_use]
pub fn render_tenant_principal_owner_scope(tenant_id: &str, principal: &str, key: &str) -> String {
    format!("tenant::{tenant_id}::principal::{principal}::{key}")
}

pub(crate) const TOOL_CALL_METADATA_ID: &str = "id";
pub(crate) const TOOL_CALL_METADATA_NAME: &str = "name";
pub(crate) const TOOL_CALL_METADATA_INPUT: &str = "input";
pub(crate) const TOOL_RESULT_METADATA_TOOL_USE_ID: &str = "tool_use_id";
pub(crate) const TOOL_RESULT_METADATA_IS_ERROR: &str = "is_error";

enum TranscriptProviderBlock {
    SameRole(ContentBlock),
    Standalone(ProviderMessage),
    Skip,
}

impl ChatMessagePart {
    pub(crate) fn render_for_history(&self) -> String {
        if self.kind == MessagePartKind::Reasoning {
            return String::new();
        }
        let label = match self.kind {
            MessagePartKind::UserText
            | MessagePartKind::AssistantText
            | MessagePartKind::SystemText
            | MessagePartKind::Reasoning => None,
            MessagePartKind::ToolCall => Some("tool_call"),
            MessagePartKind::ToolResult => Some("tool_result"),
            MessagePartKind::Patch => Some("patch"),
            MessagePartKind::Compaction => Some("compaction"),
            MessagePartKind::SubagentEvent => Some("subagent"),
            MessagePartKind::RuntimeMetadata => Some("runtime"),
            MessagePartKind::LoopDetection => Some("loop_detection"),
        };
        match label {
            Some(label) => format!("[{label}] {}", self.content.trim()),
            None => self.content.trim().to_string(),
        }
    }

    fn to_provider_block(&self) -> TranscriptProviderBlock {
        match self.kind {
            MessagePartKind::UserText
            | MessagePartKind::AssistantText
            | MessagePartKind::SystemText => {
                TranscriptProviderBlock::SameRole(ContentBlock::Text {
                    text: self.content.clone(),
                })
            }
            MessagePartKind::ToolCall => {
                if let Some(metadata) = self.metadata.as_ref()
                    && let (Some(id), Some(name), Some(input)) = (
                        metadata
                            .get(TOOL_CALL_METADATA_ID)
                            .and_then(serde_json::Value::as_str),
                        metadata
                            .get(TOOL_CALL_METADATA_NAME)
                            .and_then(serde_json::Value::as_str),
                        metadata.get(TOOL_CALL_METADATA_INPUT),
                    )
                {
                    return TranscriptProviderBlock::SameRole(ContentBlock::ToolUse {
                        id: id.to_string(),
                        name: name.to_string(),
                        input: input.clone(),
                    });
                }
                TranscriptProviderBlock::SameRole(ContentBlock::Text {
                    text: format!("[tool_call] {}", self.content),
                })
            }
            MessagePartKind::ToolResult => {
                if let Some(metadata) = self.metadata.as_ref()
                    && let Some(tool_use_id) = metadata
                        .get(TOOL_RESULT_METADATA_TOOL_USE_ID)
                        .and_then(serde_json::Value::as_str)
                {
                    let is_error = metadata
                        .get(TOOL_RESULT_METADATA_IS_ERROR)
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false);
                    return TranscriptProviderBlock::Standalone(ProviderMessage::tool_result(
                        tool_use_id,
                        self.content.clone(),
                        is_error,
                    ));
                }
                TranscriptProviderBlock::SameRole(ContentBlock::Text {
                    text: format!("[tool_result] {}", self.content),
                })
            }
            MessagePartKind::Reasoning => TranscriptProviderBlock::Skip,
            MessagePartKind::Patch => TranscriptProviderBlock::SameRole(ContentBlock::Text {
                text: format!("[patch] {}", self.content),
            }),
            MessagePartKind::Compaction => TranscriptProviderBlock::SameRole(ContentBlock::Text {
                text: format!("[compaction] {}", self.content),
            }),
            MessagePartKind::SubagentEvent => {
                TranscriptProviderBlock::SameRole(ContentBlock::Text {
                    text: format!("[subagent] {}", self.content),
                })
            }
            MessagePartKind::RuntimeMetadata => {
                TranscriptProviderBlock::SameRole(ContentBlock::Text {
                    text: format!("[runtime] {}", self.content),
                })
            }
            MessagePartKind::LoopDetection => {
                TranscriptProviderBlock::SameRole(ContentBlock::Text {
                    text: format!("[loop_detection] {}", self.content),
                })
            }
        }
    }
}

const fn provider_role_from_session_role(role: MessageRole) -> ProviderMessageRole {
    match role {
        MessageRole::User => ProviderMessageRole::User,
        MessageRole::Assistant => ProviderMessageRole::Assistant,
        MessageRole::System => ProviderMessageRole::System,
    }
}

fn provider_message_from_role(role: MessageRole, text: String) -> ProviderMessage {
    ProviderMessage {
        role: provider_role_from_session_role(role),
        content: vec![ContentBlock::Text { text }],
    }
}

/// Lifecycle state of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Currently in use.
    Active,
    /// Manually archived by the user.
    Archived,
    /// Older messages have been compacted into a summary.
    Compacted,
}

pub const SESSION_COMPANION_AFFECT_SOURCE_TOPOLOGY: &str = "affect_topology_snapshot";
pub const SESSION_COMPANION_AFFECT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SessionCompanionAffectState {
    /// Schema version for persisted companion affect metadata.
    #[serde(default)]
    pub schema_version: u16,
    /// Source pipeline that produced these cues. Current valid source is topology projection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// RFC 3339 timestamp for freshness checks before compaction rehydration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<String>,
    /// RFC 3339 expiry derived from the persona affect inactivity boundary at capture time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Top surfaced affect nodes captured from topology projection as per-mille intensity.
    #[serde(default)]
    pub affect_surface: Vec<(String, u16)>,
    /// Suppressed affect nodes captured from topology projection as per-mille intensity.
    #[serde(default)]
    pub affect_suppressed: Vec<(String, u16)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SessionMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_control: Option<SessionControlState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub companion_affect: Option<SessionCompanionAffectState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forked_from_session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_message_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_estimated_tokens: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session identifier.
    pub id: SessionId,
    pub surface: String,
    pub owner_scope: UserId,
    /// Current lifecycle state.
    pub state: SessionState,
    /// Model override for this session, if any.
    pub model: Option<String>,
    pub metadata: Option<SessionMetadata>,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
    /// RFC 3339 timestamp of the most recent update.
    pub updated_at: String,
    pub archived_at: Option<String>,
}

/// Configuration for session persistence and compaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionConfig {
    /// Whether session management is enabled.
    pub enabled: bool,
    /// Maximum number of messages to retain in history.
    pub max_history: usize,
    /// Fine-grained compaction configuration.
    #[serde(default)]
    pub compaction: CompactionConfig,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_history: 100,
            compaction: CompactionConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::contracts::ids::{MessageId, SessionId};
    use crate::core::sessions::presenter::render_history_fragment;

    use super::{
        ChatMessage, ChatMessagePart, MessagePartKind, MessageRole, SessionConfig, SessionMetadata,
        SessionOwnerScope, SessionTranscriptReadModel, TranscriptMessage,
        render_principal_owner_scope, render_tenant_owner_scope,
        render_tenant_principal_owner_scope,
    };
    use crate::core::providers::ThinkingLevel;

    #[test]
    fn session_config_rejects_legacy_compaction_threshold_field() {
        let legacy_toml = r"
enabled = true
max_history = 100
compaction_threshold = 50
";

        let error = toml::from_str::<SessionConfig>(legacy_toml)
            .expect_err("legacy compaction_threshold must be rejected explicitly");
        assert!(error.to_string().contains("unknown field"));
        assert!(error.to_string().contains("compaction_threshold"));
    }

    #[test]
    fn transcript_read_model_renders_history_fragment() {
        let transcript = vec![
            TranscriptMessage {
                message: ChatMessage {
                    id: MessageId::new("m1"),
                    session_id: SessionId::new("s1"),
                    role: MessageRole::User,
                    content: "hello".to_string(),
                    input_tokens: Some(5),
                    output_tokens: None,
                    created_at: "now".to_string(),
                },
                parts: vec![ChatMessagePart {
                    id: "p1".to_string(),
                    message_id: MessageId::new("m1"),
                    session_id: SessionId::new("s1"),
                    ordinal: 0,
                    kind: MessagePartKind::UserText,
                    mime_type: None,
                    content: "hello".to_string(),
                    metadata: None,
                    created_at: "now".to_string(),
                }],
            },
            TranscriptMessage {
                message: ChatMessage {
                    id: MessageId::new("m2"),
                    session_id: SessionId::new("s1"),
                    role: MessageRole::Assistant,
                    content: "world".to_string(),
                    input_tokens: None,
                    output_tokens: Some(7),
                    created_at: "now".to_string(),
                },
                parts: vec![
                    ChatMessagePart {
                        id: "p2".to_string(),
                        message_id: MessageId::new("m2"),
                        session_id: SessionId::new("s1"),
                        ordinal: 0,
                        kind: MessagePartKind::AssistantText,
                        mime_type: None,
                        content: "world".to_string(),
                        metadata: None,
                        created_at: "now".to_string(),
                    },
                    ChatMessagePart {
                        id: "p3".to_string(),
                        message_id: MessageId::new("m2"),
                        session_id: SessionId::new("s1"),
                        ordinal: 1,
                        kind: MessagePartKind::Reasoning,
                        mime_type: None,
                        content: "trace".to_string(),
                        metadata: None,
                        created_at: "now".to_string(),
                    },
                ],
            },
        ];

        let read_model = SessionTranscriptReadModel::new(SessionId::new("s1"), transcript);
        let fragment = render_history_fragment(&read_model, 400);
        assert!(fragment.contains("[History]"));
        assert!(fragment.contains("assistant: world"));
        assert!(!fragment.contains("trace"));
        assert!(!fragment.contains("[reasoning]"));
    }

    #[test]
    fn transcript_read_model_tail_within_token_limit_keeps_recent_messages() {
        let messages = vec![
            TranscriptMessage {
                message: ChatMessage {
                    id: MessageId::new("m1"),
                    session_id: SessionId::new("s1"),
                    role: MessageRole::User,
                    content: "first".to_string(),
                    input_tokens: Some(10),
                    output_tokens: None,
                    created_at: "now".to_string(),
                },
                parts: vec![],
            },
            TranscriptMessage {
                message: ChatMessage {
                    id: MessageId::new("m2"),
                    session_id: SessionId::new("s1"),
                    role: MessageRole::Assistant,
                    content: "second".to_string(),
                    input_tokens: None,
                    output_tokens: Some(12),
                    created_at: "now".to_string(),
                },
                parts: vec![],
            },
        ];

        let read_model = SessionTranscriptReadModel::new(SessionId::new("s1"), messages);
        let tail = read_model.tail_within_token_limit(12);
        assert_eq!(tail.messages.len(), 1);
        assert_eq!(tail.messages[0].message.id, MessageId::new("m2"));
    }

    #[test]
    fn session_metadata_serde_round_trip_preserves_existing_shape() {
        let metadata = SessionMetadata {
            title: Some("My session".to_string()),
            thinking_level: Some(ThinkingLevel::High),
            session_control: None,
            companion_affect: None,
            forked_from_session_id: Some(SessionId::new("parent-session")),
            fork_message_count: Some(4),
            fork_estimated_tokens: Some(1200),
        };

        let json = serde_json::to_value(&metadata).unwrap();
        assert_eq!(json["title"], "My session");
        assert_eq!(json["thinking_level"], "high");
        assert_eq!(json["forked_from_session_id"], "parent-session");
        assert_eq!(json["fork_message_count"], 4);
        assert_eq!(json["fork_estimated_tokens"], 1200);

        let parsed: SessionMetadata = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, metadata);
    }

    #[test]
    fn session_metadata_defaults_missing_fields() {
        let parsed: SessionMetadata = serde_json::from_str(r#"{"title":"Inbox"}"#).unwrap();
        assert_eq!(parsed.title.as_deref(), Some("Inbox"));
        assert_eq!(parsed.thinking_level, None);
        assert_eq!(parsed.forked_from_session_id, None);
        assert_eq!(parsed.fork_message_count, None);
        assert_eq!(parsed.fork_estimated_tokens, None);
    }

    #[test]
    fn session_owner_scope_parse_covers_supported_variants() {
        assert_eq!(
            SessionOwnerScope::parse("conversation::discord::room-1"),
            SessionOwnerScope::Unscoped {
                key: "conversation::discord::room-1"
            }
        );
        assert_eq!(
            SessionOwnerScope::parse("principal::auth-123::gateway-1"),
            SessionOwnerScope::Principal {
                principal: "auth-123",
                key: "gateway-1"
            }
        );
        assert_eq!(
            SessionOwnerScope::parse("tenant::tenant-a::conversation::discord::room-1"),
            SessionOwnerScope::Tenant {
                tenant_id: "tenant-a",
                key: "conversation::discord::room-1"
            }
        );
        assert_eq!(
            SessionOwnerScope::parse("tenant::tenant-a::principal::auth-123::gateway-1"),
            SessionOwnerScope::TenantPrincipal {
                tenant_id: "tenant-a",
                principal: "auth-123",
                key: "gateway-1"
            }
        );
    }

    #[test]
    fn session_owner_scope_render_helpers_match_parser() {
        let tenant_scope = render_tenant_owner_scope("tenant-a", "conversation::discord::room-1");
        let principal_scope = render_principal_owner_scope("auth-123", "gateway-1");
        let tenant_principal_scope =
            render_tenant_principal_owner_scope("tenant-a", "auth-123", "gateway-1");

        assert_eq!(
            SessionOwnerScope::parse(&tenant_scope),
            SessionOwnerScope::Tenant {
                tenant_id: "tenant-a",
                key: "conversation::discord::room-1"
            }
        );
        assert_eq!(
            SessionOwnerScope::parse(&principal_scope),
            SessionOwnerScope::Principal {
                principal: "auth-123",
                key: "gateway-1"
            }
        );
        assert_eq!(
            SessionOwnerScope::parse(&tenant_principal_scope),
            SessionOwnerScope::TenantPrincipal {
                tenant_id: "tenant-a",
                principal: "auth-123",
                key: "gateway-1"
            }
        );
    }
}

/// Fine-grained compaction configuration controlling token-aware triggers,
/// structured summarization, microcompaction, and post-compaction rehydration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Token count threshold that triggers compaction. When total session
    /// tokens exceed this value, compaction fires.
    /// Set to 0 to disable automatic compaction.
    /// Default: `100_000` (suitable for 128K context windows).
    pub token_threshold: usize,
    /// Fraction of recent messages to preserve after compaction (0.0–1.0).
    /// Default: 0.4 (keep newest 40%).
    pub keep_fraction: f64,
    /// Maximum character budget for the compaction summary.
    /// Default: `4_000`.
    pub summary_max_chars: usize,
    /// When true, use LLM-based structured summarization instead of
    /// rule-based truncation. Requires async context with a provider.
    /// Default: false.
    pub use_llm_summarization: bool,
    /// Whether to prune large tool outputs before full compaction triggers
    /// (microcompaction phase).
    /// Default: true.
    pub enable_microcompaction: bool,
    /// Tool output character threshold for microcompaction pruning.
    /// Outputs longer than this are truncated to `hot_tail_chars`.
    /// Default: `2_000`.
    pub tool_output_prune_threshold: usize,
    /// Characters to keep from the end of pruned tool outputs (hot tail).
    /// Default: 500.
    pub hot_tail_chars: usize,
    /// Whether to re-inject critical context after compaction
    /// (recent decisions, active goal, continuation state).
    /// Default: true.
    pub enable_rehydration: bool,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            token_threshold: 100_000,
            keep_fraction: 0.4,
            summary_max_chars: 4_000,
            use_llm_summarization: false,
            enable_microcompaction: true,
            tool_output_prune_threshold: 2_000,
            hot_tail_chars: 500,
            enable_rehydration: true,
        }
    }
}

impl CompactionConfig {
    /// Apply environment variable overrides (WP-H3).
    ///
    /// Checks `ASTEREL_COMPACTION_TOKEN_THRESHOLD` for runtime-tunable
    /// compaction without recompiling or editing config files.
    #[must_use]
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(val) = std::env::var("ASTEREL_COMPACTION_TOKEN_THRESHOLD")
            && let Ok(threshold) = val.parse::<usize>()
        {
            self.token_threshold = threshold;
        }
        self
    }
}

/// Result of a compaction operation with detailed metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionResult {
    /// Whether compaction was actually performed.
    pub compacted: bool,
    /// Number of messages removed during compaction.
    pub messages_removed: usize,
    /// Estimated total tokens before compaction.
    pub tokens_before: usize,
    /// Estimated total tokens after compaction.
    pub tokens_after: usize,
    /// Number of tool outputs pruned during microcompaction.
    pub tool_outputs_pruned: usize,
}

impl CompactionResult {
    /// Create a result indicating no compaction was needed.
    #[must_use]
    pub const fn skipped() -> Self {
        Self {
            compacted: false,
            messages_removed: 0,
            tokens_before: 0,
            tokens_after: 0,
            tool_outputs_pruned: 0,
        }
    }
}

/// Estimate token count from text content.
///
/// Uses a simple heuristic: ~4 characters per token for ASCII/Latin text,
/// ~2 characters per token for CJK text. This matches production estimates
/// used by Claude Code and `OpenCode` within ±15%.
#[must_use]
pub fn estimate_tokens(content: &str) -> usize {
    if content.is_empty() {
        return 0;
    }
    let mut ascii_chars: usize = 0;
    let mut non_ascii_chars: usize = 0;
    for c in content.chars() {
        if c.is_ascii() {
            ascii_chars += 1;
        } else {
            non_ascii_chars += 1;
        }
    }
    // ASCII text: ~4 chars/token; CJK/non-ASCII: ~2 chars/token
    let ascii_tokens = ascii_chars.div_ceil(4);
    let non_ascii_tokens = non_ascii_chars.div_ceil(2);
    ascii_tokens + non_ascii_tokens
}
