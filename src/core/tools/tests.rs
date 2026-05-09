//! Unit tests for tool factory and registry integration.

#[cfg(not(feature = "mcp"))]
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tempfile::TempDir;

use super::*;
use crate::config::schema::{McpConfig, ToolEntry, ToolsConfig};
#[cfg(not(feature = "mcp"))]
use crate::config::schema::{McpServerConfig, McpTransport};
use crate::config::{BrowserConfig, MemoryConfig};
use crate::contracts::channels::ChannelCapabilities;
use crate::core::memory::Memory;
use crate::core::tools::{DelegateTool, SubagentSpawnTool};
use crate::security::SecurityPolicy;

async fn markdown_memory(tmp: &TempDir) -> Arc<dyn Memory> {
    let mem_cfg = MemoryConfig {
        backend: crate::config::MemoryBackend::Markdown,
        ..MemoryConfig::default()
    };
    Arc::from(
        crate::core::memory::create_memory(&mem_cfg, tmp.path(), None)
            .await
            .unwrap(),
    )
}

fn noop_mcp_tool_provider() -> Arc<dyn McpToolProvider> {
    Arc::new(NoopMcpToolProvider::new())
}

fn enabled_mcp_config_without_servers() -> McpConfig {
    McpConfig {
        enabled: true,
        import_json: None,
        servers: Vec::new(),
    }
}

#[cfg(not(feature = "mcp"))]
fn enabled_mcp_config_with_empty_server() -> McpConfig {
    McpConfig {
        enabled: true,
        import_json: None,
        servers: vec![McpServerConfig {
            name: "empty".to_string(),
            transport: McpTransport::Stdio {
                command: String::new(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            enabled: true,
            max_call_seconds: 30,
        }],
    }
}

struct MockTool {
    name: String,
    description: String,
}

impl Tool for MockTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    fn execute<'a>(
        &'a self,
        _args: serde_json::Value,
        _ctx: &'a ExecutionContext,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move {
            Ok(ToolResult {
                success: true,
                output: String::new(),
                error: None,

                attachments: Vec::new(),
                taint_labels: Vec::new(),
                semantic: crate::core::tools::traits::ToolResultSemanticMetadata::default(),
            })
        })
    }
}

#[test]
fn default_tools_has_expected_count() {
    let security = Arc::new(SecurityPolicy::default());
    let tools = default_tools(&security);
    // shell + file_read + file_write + file_delete
    assert_eq!(tools.len(), 4);
}

#[tokio::test]
async fn all_tools_excludes_browser_when_disabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: crate::config::MemoryBackend::Markdown,
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::core::memory::create_memory(&mem_cfg, tmp.path(), None)
            .await
            .unwrap(),
    );

    let browser = BrowserConfig {
        enabled: false,
        allowed_domains: vec!["example.com".into()],
        session_name: None,
    };

    let tools_cfg = ToolsConfig::default();
    let tools = all_tools(ToolRegistryConfig {
        security: &security,
        memory: mem,
        composio_key: None,
        browser: &browser,
        tools: &tools_cfg,
        mcp: None,
        mcp_tool_provider: noop_mcp_tool_provider(),
        taste: &crate::config::TasteConfig::default(),
        taste_provider: None,
        taste_model: "test-model",
        channel_capabilities: None,
        codespace: &crate::config::CodespaceConfig::default(),
    });
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(!names.contains(&"browser_open"));
}

#[tokio::test]
async fn all_tools_includes_browser_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: crate::config::MemoryBackend::Markdown,
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::core::memory::create_memory(&mem_cfg, tmp.path(), None)
            .await
            .unwrap(),
    );

    let browser = BrowserConfig {
        enabled: true,
        allowed_domains: vec!["example.com".into()],
        session_name: None,
    };

    let tools_cfg = ToolsConfig::default();
    let tools = all_tools(ToolRegistryConfig {
        security: &security,
        memory: mem,
        composio_key: None,
        browser: &browser,
        tools: &tools_cfg,
        mcp: None,
        mcp_tool_provider: noop_mcp_tool_provider(),
        taste: &crate::config::TasteConfig::default(),
        taste_provider: None,
        taste_model: "test-model",
        channel_capabilities: None,
        codespace: &crate::config::CodespaceConfig::default(),
    });
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"browser_open"));
}

#[test]
fn default_tools_names() {
    let security = Arc::new(SecurityPolicy::default());
    let tools = default_tools(&security);
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"shell"));
    assert!(names.contains(&"file_read"));
    assert!(names.contains(&"file_write"));
}

#[test]
fn default_tools_all_have_descriptions() {
    let security = Arc::new(SecurityPolicy::default());
    let tools = default_tools(&security);
    for tool in &tools {
        assert!(
            !tool.description().is_empty(),
            "Tool {} has empty description",
            tool.name()
        );
    }
}

#[test]
fn default_tools_all_have_schemas() {
    let security = Arc::new(SecurityPolicy::default());
    let tools = default_tools(&security);
    for tool in &tools {
        let schema = tool.parameters_schema();
        assert!(
            schema.is_object(),
            "Tool {} schema is not an object",
            tool.name()
        );
        assert!(
            schema["properties"].is_object(),
            "Tool {} schema has no properties",
            tool.name()
        );
    }
}

#[test]
fn tool_spec_generation() {
    let security = Arc::new(SecurityPolicy::default());
    let tools = default_tools(&security);
    for tool in &tools {
        let spec = tool.spec();
        assert_eq!(spec.name, tool.name());
        assert_eq!(spec.description, tool.description());
        assert!(spec.parameters.is_object());
    }
}

#[test]
fn tool_result_serde() {
    let result = ToolResult::success("hello");
    let json = serde_json::to_string(&result).unwrap();
    let parsed: ToolResult = serde_json::from_str(&json).unwrap();
    assert!(parsed.success);
    assert_eq!(parsed.output, "hello");
    assert!(parsed.error.is_none());
    assert!(parsed.attachments.is_empty());
}

#[test]
fn tool_result_with_error_serde() {
    let result = ToolResult::failure("boom");
    let json = serde_json::to_string(&result).unwrap();
    let parsed: ToolResult = serde_json::from_str(&json).unwrap();
    assert!(!parsed.success);
    assert_eq!(parsed.error.as_deref(), Some("boom"));
    assert!(parsed.attachments.is_empty());
}

#[test]
fn tool_result_deserialize_without_attachments_still_works() {
    let json = r#"{"success":true,"output":"ok","error":null}"#;
    let parsed: ToolResult = serde_json::from_str(json).unwrap();
    assert!(parsed.success);
    assert!(parsed.attachments.is_empty());
}

#[test]
fn tool_result_with_attachments_serde() {
    let result = ToolResult {
        attachments: vec![OutputAttachment::from_path(
            "image/png",
            "/tmp/generated.png",
            Some("generated.png".to_string()),
        )],
        ..ToolResult::success("image generated")
    };
    let json = serde_json::to_string(&result).unwrap();
    let parsed: ToolResult = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.attachments.len(), 1);
    assert_eq!(
        &parsed.attachments[0].source,
        &crate::core::tools::AttachmentSource::File {
            path: "/tmp/generated.png".to_string()
        }
    );
}

#[test]
fn tool_result_semantic_metadata_stays_internal() {
    let result = ToolResult::failure("boom")
        .with_output_kind("shell_command")
        .with_compaction_target(ToolResultCompactionTarget::OutputAndError);

    let json = serde_json::to_value(&result).unwrap();
    assert!(json.get("semantic").is_none());

    let parsed: ToolResult = serde_json::from_value(json).unwrap();
    assert!(parsed.semantic.output_kind.is_none());
    assert_eq!(
        parsed.semantic.compaction_target,
        ToolResultCompactionTarget::Output
    );
    assert!(parsed.semantic.stats.is_none());
}

#[test]
fn tool_spec_serde() {
    let spec = ToolSpec {
        name: "test".into(),
        description: "A test tool".into(),
        parameters: serde_json::json!({"type": "object"}),
        required_capabilities: Vec::new(),
        effect: crate::contracts::tools::ToolEffect::LocalMutation,
    };
    let json = serde_json::to_string(&spec).unwrap();
    let parsed: ToolSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.name, "test");
    assert_eq!(parsed.description, "A test tool");
}

#[tokio::test]
async fn all_tools_respects_disabled_shell() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: crate::config::MemoryBackend::Markdown,
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::core::memory::create_memory(&mem_cfg, tmp.path(), None)
            .await
            .unwrap(),
    );

    let browser = BrowserConfig::default();
    let mut tools_cfg = ToolsConfig::default();
    tools_cfg.shell.enabled = false;

    let tools = all_tools(ToolRegistryConfig {
        security: &security,
        memory: mem,
        composio_key: None,
        browser: &browser,
        tools: &tools_cfg,
        mcp: None,
        mcp_tool_provider: noop_mcp_tool_provider(),
        taste: &crate::config::TasteConfig::default(),
        taste_provider: None,
        taste_model: "test-model",
        channel_capabilities: None,
        codespace: &crate::config::CodespaceConfig::default(),
    });
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(!names.contains(&"shell"));
}

#[tokio::test]
async fn all_tools_respects_disabled_file_read() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: crate::config::MemoryBackend::Markdown,
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::core::memory::create_memory(&mem_cfg, tmp.path(), None)
            .await
            .unwrap(),
    );

    let browser = BrowserConfig::default();
    let mut tools_cfg = ToolsConfig::default();
    tools_cfg.file_read.enabled = false;

    let tools = all_tools(ToolRegistryConfig {
        security: &security,
        memory: mem,
        composio_key: None,
        browser: &browser,
        tools: &tools_cfg,
        mcp: None,
        mcp_tool_provider: noop_mcp_tool_provider(),
        taste: &crate::config::TasteConfig::default(),
        taste_provider: None,
        taste_model: "test-model",
        channel_capabilities: None,
        codespace: &crate::config::CodespaceConfig::default(),
    });
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(!names.contains(&"file_read"));
}

#[tokio::test]
async fn all_tools_respects_disabled_memory_forget() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: crate::config::MemoryBackend::Markdown,
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::core::memory::create_memory(&mem_cfg, tmp.path(), None)
            .await
            .unwrap(),
    );

    let browser = BrowserConfig::default();
    let tools_cfg = ToolsConfig::default();

    let tools = all_tools(ToolRegistryConfig {
        security: &security,
        memory: mem,
        composio_key: None,
        browser: &browser,
        tools: &tools_cfg,
        mcp: None,
        mcp_tool_provider: noop_mcp_tool_provider(),
        taste: &crate::config::TasteConfig::default(),
        taste_provider: None,
        taste_model: "test-model",
        channel_capabilities: None,
        codespace: &crate::config::CodespaceConfig::default(),
    });
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(!names.contains(&"memory_forget"));
}

#[tokio::test]
async fn all_tools_includes_memory_forget_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: crate::config::MemoryBackend::Markdown,
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::core::memory::create_memory(&mem_cfg, tmp.path(), None)
            .await
            .unwrap(),
    );

    let browser = BrowserConfig::default();
    let mut tools_cfg = ToolsConfig::default();
    tools_cfg.memory_forget.enabled = true;

    let tools = all_tools(ToolRegistryConfig {
        security: &security,
        memory: mem,
        composio_key: None,
        browser: &browser,
        tools: &tools_cfg,
        mcp: None,
        mcp_tool_provider: noop_mcp_tool_provider(),
        taste: &crate::config::TasteConfig::default(),
        taste_provider: None,
        taste_model: "test-model",
        channel_capabilities: None,
        codespace: &crate::config::CodespaceConfig::default(),
    });
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"memory_forget"));
}

#[tokio::test]
async fn all_tools_with_all_disabled_yields_only_always_on_tools() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: crate::config::MemoryBackend::Markdown,
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::core::memory::create_memory(&mem_cfg, tmp.path(), None)
            .await
            .unwrap(),
    );

    let browser = BrowserConfig::default();
    let tools_cfg = ToolsConfig {
        shell: ToolEntry { enabled: false },
        file_read: ToolEntry { enabled: false },
        file_write: ToolEntry { enabled: false },
        file_delete: ToolEntry { enabled: false },
        memory_store: ToolEntry { enabled: false },
        memory_recall: ToolEntry { enabled: false },
        memory_lookup: ToolEntry { enabled: false },
        memory_correct: ToolEntry { enabled: false },
        memory_forget: ToolEntry { enabled: false },
        memory_governance: ToolEntry { enabled: false },
        loop_detection: crate::config::LoopDetectionConfig::default(),
    };
    let taste_config = crate::config::TasteConfig {
        enabled: false,
        ..crate::config::TasteConfig::default()
    };
    let tools = all_tools(ToolRegistryConfig {
        security: &security,
        memory: mem,
        composio_key: None,
        browser: &browser,
        tools: &tools_cfg,
        mcp: None,
        mcp_tool_provider: noop_mcp_tool_provider(),
        taste: &taste_config,
        taste_provider: None,
        taste_model: "test-model",
        channel_capabilities: None,
        codespace: &crate::config::CodespaceConfig::default(),
    });
    // Always-on tools: delegate, subagent_spawn, subagent_output, subagent_cancel
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(!names.contains(&"shell"));
    assert!(!names.contains(&"file_read"));
    assert!(!names.contains(&"memory_store"));
    assert!(!names.contains(&"taste_evaluate"));
    assert!(!names.contains(&"taste_compare"));
    assert!(names.contains(&"delegate"));
    let expected_always_on = 4;
    assert_eq!(
        tools.len(),
        expected_always_on,
        "only always-on tools should remain: {names:?}"
    );
}

#[tokio::test]
async fn all_tools_default_config_has_expected_tools() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: crate::config::MemoryBackend::Markdown,
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> = Arc::from(
        crate::core::memory::create_memory(&mem_cfg, tmp.path(), None)
            .await
            .unwrap(),
    );

    let browser = BrowserConfig::default();
    let tools_cfg = ToolsConfig::default();

    let tools = all_tools(ToolRegistryConfig {
        security: &security,
        memory: mem,
        composio_key: None,
        browser: &browser,
        tools: &tools_cfg,
        mcp: None,
        mcp_tool_provider: noop_mcp_tool_provider(),
        taste: &crate::config::TasteConfig::default(),
        taste_provider: None,
        taste_model: "test-model",
        channel_capabilities: None,
        codespace: &crate::config::CodespaceConfig::default(),
    });
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();

    assert!(names.contains(&"shell"));
    assert!(names.contains(&"file_read"));
    assert!(names.contains(&"file_write"));
    assert!(names.contains(&"memory_store"));
    assert!(names.contains(&"memory_recall"));
    assert!(!names.contains(&"memory_forget"));
    assert!(!names.contains(&"memory_governance"));
}

#[cfg(feature = "taste")]
#[tokio::test]
async fn all_tools_keeps_taste_tools_visible_when_backend_unavailable() {
    let tmp = TempDir::new().unwrap();
    let mut security_policy = SecurityPolicy::default();
    security_policy.workspace_dir = tmp.path().to_path_buf();
    let security = Arc::new(security_policy);
    let mem = markdown_memory(&tmp).await;

    let tools = all_tools(ToolRegistryConfig {
        security: &security,
        memory: mem,
        composio_key: None,
        browser: &BrowserConfig::default(),
        tools: &ToolsConfig::default(),
        mcp: None,
        mcp_tool_provider: noop_mcp_tool_provider(),
        taste: &crate::config::TasteConfig::default(),
        taste_provider: None,
        taste_model: "test-model",
        channel_capabilities: None,
        codespace: &crate::config::CodespaceConfig::default(),
    });

    let taste_tool = tools
        .iter()
        .find(|tool| tool.name() == "taste_evaluate")
        .expect("taste tool should be present as visible failure");
    let ctx = crate::core::tools::middleware::ExecutionContext::test_default(security);
    let result = taste_tool
        .execute(serde_json::json!({}), &ctx)
        .await
        .expect("unavailable tool should return a tool failure");

    assert!(!result.success);
    assert!(
        result
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("Taste tool unavailable")
    );
}

#[test]
fn tool_descriptions_contains_core_tools() {
    let descriptions = tool_descriptions(false, false, None);
    let names: Vec<&str> = descriptions
        .iter()
        .map(|(name, _description)| name.as_str())
        .collect();
    assert!(names.contains(&"shell"));
    assert!(names.contains(&"file_read"));
    assert!(names.contains(&"file_write"));
    assert!(names.contains(&"memory_store"));
    assert!(names.contains(&"memory_recall"));
    assert!(names.contains(&"memory_forget"));
}

#[test]
fn tool_descriptions_respects_browser_flag() {
    let disabled = tool_descriptions(false, false, None);
    let enabled = tool_descriptions(true, false, None);
    let disabled_names: Vec<&str> = disabled
        .iter()
        .map(|(name, _description)| name.as_str())
        .collect();
    let enabled_names: Vec<&str> = enabled
        .iter()
        .map(|(name, _description)| name.as_str())
        .collect();
    assert!(!disabled_names.contains(&"browser_open"));
    assert!(enabled_names.contains(&"browser_open"));
}

#[test]
fn tool_descriptions_respects_composio_flag() {
    let disabled = tool_descriptions(false, false, None);
    let enabled = tool_descriptions(false, true, None);
    let disabled_names: Vec<&str> = disabled
        .iter()
        .map(|(name, _description)| name.as_str())
        .collect();
    let enabled_names: Vec<&str> = enabled
        .iter()
        .map(|(name, _description)| name.as_str())
        .collect();
    assert!(!disabled_names.contains(&"composio"));
    assert!(enabled_names.contains(&"composio"));
}

#[test]
fn delegate_and_subagent_spawn_expose_handoff_envelope_fields() {
    let delegate_schema = DelegateTool::new().parameters_schema();
    let spawn_schema = SubagentSpawnTool::new().parameters_schema();

    for schema in [&delegate_schema, &spawn_schema] {
        assert!(schema["properties"]["objective"].is_object());
        assert!(schema["properties"]["done_when"].is_object());
        assert!(schema["properties"]["context"].is_object());
        assert_eq!(schema["properties"]["constraints"]["type"], "array");
    }
}

#[tokio::test]
async fn delegate_run_rejects_when_delegation_depth_limit_is_reached() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy {
        workspace_dir: tmp.path().to_path_buf(),
        ..SecurityPolicy::default()
    });
    let mut ctx = ExecutionContext::test_default(security).with_delegation_limits(2, 2, 2, 2);
    ctx.subagent_manager = Some(Arc::new(crate::core::subagents::SubagentOrchestrator::new()));

    let error = DelegateTool::new()
        .execute(
            serde_json::json!({
                "action": "run",
                "task": "inspect the repository",
            }),
            &ctx,
        )
        .await
        .unwrap_err();

    assert!(error.to_string().contains("delegation depth limit reached"));
    assert_eq!(ctx.remaining_child_delegations(), 2);
}

#[tokio::test]
async fn subagent_spawn_rejects_when_child_quota_is_exhausted() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy {
        workspace_dir: tmp.path().to_path_buf(),
        ..SecurityPolicy::default()
    });
    let mut ctx = ExecutionContext::test_default(security).with_delegation_limits(1, 2, 1, 0);
    ctx.subagent_manager = Some(Arc::new(crate::core::subagents::SubagentOrchestrator::new()));

    let error = SubagentSpawnTool::new()
        .execute(
            serde_json::json!({
                "task": "inspect the repository",
                "run_in_background": true,
            }),
            &ctx,
        )
        .await
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("child delegation quota exhausted")
    );
    assert_eq!(ctx.remaining_child_delegations(), 0);
}

#[test]
fn tool_descriptions_include_channel_tools_for_capabilities() {
    let caps = ChannelCapabilities {
        can_create_thread: true,
        can_add_reaction: true,
        can_send_buttons: true,
        can_fetch_history: true,
        can_send_embed: true,
        ..ChannelCapabilities::default()
    };
    let descriptions =
        tool_desc_with_security(false, false, None, &SecurityPolicy::default(), Some(&caps));
    let names: Vec<&str> = descriptions.iter().map(|(name, _)| name.as_str()).collect();
    assert!(names.contains(&"channel_create_thread"));
    assert!(names.contains(&"channel_add_reaction"));
    assert!(names.contains(&"channel_send_rich"));
    assert!(names.contains(&"channel_get_history"));
    assert!(names.contains(&"channel_send_embed"));
}

#[tokio::test]
async fn all_tools_registers_channel_tools_based_on_capabilities() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig::default();
    let tools_cfg = ToolsConfig::default();
    let caps = ChannelCapabilities {
        can_create_thread: true,
        can_add_reaction: true,
        can_send_buttons: false,
        can_fetch_history: true,
        can_send_embed: true,
        ..ChannelCapabilities::default()
    };
    let mem = markdown_memory(&tmp).await;

    let tools = all_tools(ToolRegistryConfig {
        security: &security,
        memory: mem,
        composio_key: None,
        browser: &browser,
        tools: &tools_cfg,
        mcp: None,
        mcp_tool_provider: noop_mcp_tool_provider(),
        taste: &crate::config::TasteConfig::default(),
        taste_provider: None,
        taste_model: "test-model",
        channel_capabilities: Some(&caps),
        codespace: &crate::config::CodespaceConfig::default(),
    });
    let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
    assert!(names.contains(&"channel_create_thread"));
    assert!(names.contains(&"channel_add_reaction"));
    assert!(names.contains(&"channel_send_rich"));
    assert!(names.contains(&"channel_get_history"));
    assert!(names.contains(&"channel_send_embed"));
}

#[tokio::test]
async fn all_tools_channel_capability_combination_is_respected() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig::default();
    let tools_cfg = ToolsConfig::default();
    let caps = ChannelCapabilities {
        can_create_thread: false,
        can_add_reaction: false,
        can_send_buttons: false,
        can_fetch_history: false,
        can_send_embed: true,
        ..ChannelCapabilities::default()
    };
    let mem = markdown_memory(&tmp).await;

    let tools = all_tools(ToolRegistryConfig {
        security: &security,
        memory: mem,
        composio_key: None,
        browser: &browser,
        tools: &tools_cfg,
        mcp: None,
        mcp_tool_provider: noop_mcp_tool_provider(),
        taste: &crate::config::TasteConfig::default(),
        taste_provider: None,
        taste_model: "test-model",
        channel_capabilities: Some(&caps),
        codespace: &crate::config::CodespaceConfig::default(),
    });
    let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
    assert!(!names.contains(&"channel_create_thread"));
    assert!(!names.contains(&"channel_add_reaction"));
    assert!(!names.contains(&"channel_get_history"));
    assert!(names.contains(&"channel_send_rich"));
    assert!(names.contains(&"channel_send_embed"));
}

#[tokio::test]
async fn all_tools_none_mcp_matches_empty_mcp_config() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig::default();
    let tools_cfg = ToolsConfig::default();
    let mem = markdown_memory(&tmp).await;
    let baseline = all_tools(ToolRegistryConfig {
        security: &security,
        memory: Arc::clone(&mem),
        composio_key: None,
        browser: &browser,
        tools: &tools_cfg,
        mcp: None,
        mcp_tool_provider: noop_mcp_tool_provider(),
        taste: &crate::config::TasteConfig::default(),
        taste_provider: None,
        taste_model: "test-model",
        channel_capabilities: None,
        codespace: &crate::config::CodespaceConfig::default(),
    });
    let with_empty_config = all_tools(ToolRegistryConfig {
        security: &security,
        memory: mem,
        composio_key: None,
        browser: &browser,
        tools: &tools_cfg,
        mcp: Some(&enabled_mcp_config_without_servers()),
        mcp_tool_provider: noop_mcp_tool_provider(),
        taste: &crate::config::TasteConfig::default(),
        taste_provider: None,
        taste_model: "test-model",
        channel_capabilities: None,
        codespace: &crate::config::CodespaceConfig::default(),
    });

    let baseline_names: Vec<&str> = baseline.iter().map(|tool| tool.name()).collect();
    let empty_names: Vec<&str> = with_empty_config.iter().map(|tool| tool.name()).collect();
    assert_eq!(baseline_names, empty_names);
}

#[test]
fn tool_descriptions_none_mcp_matches_empty_mcp_config() {
    let baseline = tool_descriptions(false, false, None);
    let with_empty_config =
        tool_descriptions(false, false, Some(&enabled_mcp_config_without_servers()));
    assert_eq!(baseline, with_empty_config);
}

#[test]
fn append_dynamic_tool_descriptions_keeps_namespaced_mcp_names() {
    let mut descriptions = vec![("shell".to_string(), "run commands".to_string())];
    let dynamic_tools: Vec<Box<dyn Tool>> = vec![
        Box::new(MockTool {
            name: "mcp_filesystem_search".to_string(),
            description: "Search files".to_string(),
        }),
        Box::new(MockTool {
            name: "mcp_github_get_issue".to_string(),
            description: "Fetch issue".to_string(),
        }),
    ];

    append_dynamic_tool_descriptions(&mut descriptions, &dynamic_tools);
    let dynamic_names: Vec<&str> = descriptions
        .iter()
        .skip(1)
        .map(|(name, _description)| name.as_str())
        .collect();
    assert!(dynamic_names.iter().all(|name| name.starts_with("mcp_")));
}

#[test]
fn append_dynamic_tool_descriptions_appends_tool_descriptions() {
    let mut descriptions = vec![("shell".to_string(), "run commands".to_string())];
    let dynamic_tools: Vec<Box<dyn Tool>> = vec![Box::new(MockTool {
        name: "mcp_docs_lookup".to_string(),
        description: "Lookup docs".to_string(),
    })];

    append_dynamic_tool_descriptions(&mut descriptions, &dynamic_tools);

    assert_eq!(descriptions.len(), 2);
    assert_eq!(descriptions[1].0, "mcp_docs_lookup");
    assert_eq!(descriptions[1].1, "Lookup docs");
}

#[cfg(not(feature = "mcp"))]
#[test]
fn all_tools_accepts_mcp_config_but_ignores_it_without_feature() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let tmp = TempDir::new().unwrap();
        let security = Arc::new(SecurityPolicy::default());
        let browser = BrowserConfig::default();
        let tools_cfg = ToolsConfig::default();
        let mem = markdown_memory(&tmp).await;
        let with_none = all_tools(ToolRegistryConfig {
            security: &security,
            memory: Arc::clone(&mem),
            composio_key: None,
            browser: &browser,
            tools: &tools_cfg,
            mcp: None,
            mcp_tool_provider: noop_mcp_tool_provider(),
            taste: &crate::config::TasteConfig::default(),
            taste_provider: None,
            taste_model: "test-model",
            channel_capabilities: None,
            codespace: &crate::config::CodespaceConfig::default(),
        });
        let with_enabled_mcp = all_tools(ToolRegistryConfig {
            security: &security,
            memory: mem,
            composio_key: None,
            browser: &browser,
            tools: &tools_cfg,
            mcp: Some(&enabled_mcp_config_with_empty_server()),
            mcp_tool_provider: noop_mcp_tool_provider(),
            taste: &crate::config::TasteConfig::default(),
            taste_provider: None,
            taste_model: "test-model",
            channel_capabilities: None,
            codespace: &crate::config::CodespaceConfig::default(),
        });

        let none_names: Vec<&str> = with_none.iter().map(|tool| tool.name()).collect();
        let enabled_names: Vec<&str> = with_enabled_mcp.iter().map(|tool| tool.name()).collect();
        assert_eq!(none_names, enabled_names);
    });
}

#[cfg(not(feature = "mcp"))]
#[test]
fn tool_descriptions_accepts_mcp_config_but_ignores_it_without_feature() {
    let with_none = tool_descriptions(false, false, None);
    let with_enabled_mcp =
        tool_descriptions(false, false, Some(&enabled_mcp_config_with_empty_server()));
    assert_eq!(with_none, with_enabled_mcp);
}

#[cfg(feature = "mcp")]
#[tokio::test]
async fn all_tools_with_empty_mcp_servers_matches_none() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig::default();
    let tools_cfg = ToolsConfig::default();
    let mem = markdown_memory(&tmp).await;
    let with_none = all_tools(ToolRegistryConfig {
        security: &security,
        memory: Arc::clone(&mem),
        composio_key: None,
        browser: &browser,
        tools: &tools_cfg,
        mcp: None,
        mcp_tool_provider: noop_mcp_tool_provider(),
        taste: &crate::config::TasteConfig::default(),
        taste_provider: None,
        taste_model: "test-model",
        channel_capabilities: None,
        codespace: &crate::config::CodespaceConfig::default(),
    });
    let with_empty_servers = all_tools(ToolRegistryConfig {
        security: &security,
        memory: mem,
        composio_key: None,
        browser: &browser,
        tools: &tools_cfg,
        mcp: Some(&enabled_mcp_config_without_servers()),
        mcp_tool_provider: noop_mcp_tool_provider(),
        taste: &crate::config::TasteConfig::default(),
        taste_provider: None,
        taste_model: "test-model",
        channel_capabilities: None,
        codespace: &crate::config::CodespaceConfig::default(),
    });

    let none_names: Vec<&str> = with_none.iter().map(|tool| tool.name()).collect();
    let empty_names: Vec<&str> = with_empty_servers.iter().map(|tool| tool.name()).collect();
    assert_eq!(none_names, empty_names);
}

#[cfg(feature = "mcp")]
#[test]
fn tool_descriptions_with_empty_mcp_servers_matches_none() {
    let with_none = tool_descriptions(false, false, None);
    let with_empty_servers =
        tool_descriptions(false, false, Some(&enabled_mcp_config_without_servers()));
    assert_eq!(with_none, with_empty_servers);
}

#[cfg(feature = "mcp")]
#[test]
fn tool_descriptions_with_disabled_mcp_config_matches_none() {
    let with_none = tool_descriptions(false, false, None);
    let disabled_mcp = McpConfig::default();
    let with_disabled = tool_descriptions(false, false, Some(&disabled_mcp));
    assert_eq!(with_none, with_disabled);
}
