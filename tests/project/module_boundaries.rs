use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_repo_file(relative: &str) -> String {
    std::fs::read_to_string(repo_root().join(relative)).unwrap_or_else(|error| {
        panic!("failed to read {relative}: {error}");
    })
}

fn collect_rs_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_rs_files(&path));
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    files
}

fn scan_struct_fields(dir: &str, field_pattern: &str) -> Vec<(String, usize, String)> {
    let base = repo_root().join(dir);
    let mut hits = Vec::new();
    for path in collect_rs_files(&base) {
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let lines: Vec<&str> = content.lines().collect();
        for (idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed.contains(field_pattern)
                && !trimmed.starts_with("//")
                && !trimmed.starts_with("*")
                && !trimmed.starts_with("let ")
                && !trimmed.contains("fn ")
                && !is_inside_fn_signature(&lines, idx)
            {
                let relative = path
                    .strip_prefix(repo_root())
                    .unwrap_or(&path)
                    .display()
                    .to_string();
                hits.push((relative, idx + 1, trimmed.to_string()));
            }
        }
    }
    hits
}

fn is_inside_fn_signature(lines: &[&str], idx: usize) -> bool {
    let lookback = 5.min(idx);
    for line in lines.iter().take(idx).skip(idx.saturating_sub(lookback)) {
        if line.contains("fn ") {
            return true;
        }
    }
    false
}

#[test]
fn session_id_uses_newtype_not_raw_string() {
    let hits = scan_struct_fields("src", "session_id: String");
    assert!(
        hits.is_empty(),
        "session_id must use SessionId newtype, not String. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn run_id_uses_newtype_not_raw_string() {
    let hits = scan_struct_fields("src", "run_id: String");
    assert!(
        hits.is_empty(),
        "run_id must use RunId newtype, not String. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn entity_id_uses_newtype_not_raw_string() {
    let hits = scan_struct_fields("src", "entity_id: String");
    assert!(
        hits.is_empty(),
        "entity_id must use EntityId newtype, not String. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn slot_key_uses_newtype_not_raw_string() {
    let hits = scan_struct_fields("src", "slot_key: String");
    assert!(
        hits.is_empty(),
        "slot_key must use SlotKey newtype, not String. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn channel_id_uses_newtype_not_raw_string() {
    let hits = scan_struct_fields("src", "channel_id: String");
    assert!(
        hits.is_empty(),
        "channel_id must use ChannelId newtype, not String. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn message_id_uses_newtype_not_raw_string() {
    let hits = scan_struct_fields("src", "message_id: String");
    assert!(
        hits.is_empty(),
        "message_id must use MessageId newtype, not String. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn user_id_uses_newtype_not_raw_string() {
    let hits = scan_struct_fields("src", "user_id: String");
    assert!(
        hits.is_empty(),
        "user_id must use UserId newtype, not String. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn planner_goal_id_fields_stay_out_of_active_src() {
    let hits = scan_struct_fields("src", "goal_id:");
    assert!(
        hits.is_empty(),
        "goal_id is planner-era runtime vocabulary and must stay out of active src. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn event_id_uses_newtype_not_raw_string() {
    let hits = scan_struct_fields("src", "event_id: String");
    assert!(
        hits.is_empty(),
        "event_id must use EventId newtype, not String. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn action_id_uses_newtype_not_raw_string() {
    let hits = scan_struct_fields("src", "action_id: String");
    assert!(
        hits.is_empty(),
        "action_id must use ActionId newtype, not String. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn planner_step_id_fields_stay_out_of_active_src() {
    let hits = scan_struct_fields("src", "step_id:");
    assert!(
        hits.is_empty(),
        "step_id is planner-era runtime vocabulary and must stay out of active src. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn observation_id_uses_newtype_not_raw_string() {
    let hits = scan_struct_fields("src", "observation_id: String");
    assert!(
        hits.is_empty(),
        "observation_id must use ObservationId newtype, not String. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn scenario_id_uses_newtype_not_raw_string() {
    let hits = scan_struct_fields("src", "scenario_id: String");
    assert!(
        hits.is_empty(),
        "scenario_id must use ScenarioId newtype, not String. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn request_id_uses_newtype_not_raw_string() {
    let hits = scan_struct_fields("src", "request_id: String");
    assert!(
        hits.is_empty(),
        "request_id must use RequestId newtype, not String. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn planner_plan_id_fields_stay_out_of_active_src() {
    let hits = scan_struct_fields("src", "plan_id:");
    assert!(
        hits.is_empty(),
        "plan_id is planner-era runtime vocabulary and must stay out of active src. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn core_agent_run_entrypoint_does_not_bootstrap_runtime_services() {
    let content = read_repo_file("src/core/agent/loop_/run.rs");
    assert!(
        !content.contains("bootstrap_runtime_services("),
        "core agent run entrypoint must not bootstrap runtime services directly"
    );
    assert!(
        !content.contains(".assemble_surface("),
        "core agent run entrypoint must not assemble runtime surfaces directly"
    );
    assert!(
        !content.contains("create_runtime(&config.runtime)"),
        "core agent run entrypoint must not initialize the runtime adapter directly"
    );
    assert!(
        !content.contains("create_observer(&config.observability)"),
        "core agent run entrypoint must not create its own observer directly"
    );
    assert!(
        !content.contains("runtime::services::build_tool_registry"),
        "core agent run entrypoint must not assemble tool registries through runtime services"
    );
    assert!(
        !content.contains("RuntimeModelSelection"),
        "core agent run entrypoint must not depend on runtime service model-selection structs"
    );
}

#[test]
fn gateway_server_does_not_bootstrap_runtime_services_directly() {
    let content = read_repo_file("src/transport/gateway/server.rs");
    assert!(
        !content.contains("bootstrap_runtime_services("),
        "gateway server must not bootstrap runtime services directly"
    );
    assert!(
        !content.contains(".assemble_surface("),
        "gateway server must not assemble runtime surfaces directly"
    );
    assert!(
        !content.contains("RuntimeSurfaceAssembly"),
        "gateway server must not own runtime surface assembly policy"
    );
}

#[test]
fn channel_startup_runtime_does_not_bootstrap_runtime_services_directly() {
    let content = read_repo_file("src/transport/channels/startup/runtime.rs");
    assert!(
        !content.contains("bootstrap_runtime_services("),
        "channel startup runtime must not bootstrap runtime services directly"
    );
    assert!(
        !content.contains(".assemble_surface("),
        "channel startup runtime must not assemble runtime surfaces directly"
    );
    assert!(
        !content.contains("crate::runtime::create_runtime(&config.runtime)"),
        "channel startup runtime must not create runtime adapters directly"
    );
}

#[test]
fn admin_channel_handlers_do_not_call_low_level_runtime_management_helpers() {
    let content = read_repo_file("src/transport/gateway/handlers/admin_channels.rs");
    for forbidden in [
        "load_managed_channel_inventory(",
        "create_managed_channel(",
        "update_managed_channel(",
        "run_managed_channel_action(",
    ] {
        assert!(
            !content.contains(forbidden),
            "admin channel handlers must route through runtime-owned facades, not {forbidden}"
        );
    }
}

#[test]
fn admin_skill_handlers_do_not_call_low_level_runtime_management_helpers() {
    let content = read_repo_file("src/transport/gateway/handlers/admin_skills.rs");
    for forbidden in [
        "load_managed_skills(",
        "install_managed_skill(",
        "remove_managed_skill(",
        "update_managed_skill(",
    ] {
        assert!(
            !content.contains(forbidden),
            "admin skill handlers must route through runtime-owned facades, not {forbidden}"
        );
    }
}

#[test]
fn admin_tenant_handlers_do_not_call_low_level_binding_helpers() {
    let content = read_repo_file("src/transport/gateway/handlers/admin_tenants.rs");
    for forbidden in [
        "load_persisted_bindings(",
        "save_persisted_bindings(",
        "resolve_selected_tenant_for_principal(",
    ] {
        assert!(
            !content.contains(forbidden),
            "admin tenant handlers must route through runtime-owned facades, not {forbidden}"
        );
    }
}

#[test]
fn admin_settings_handlers_do_not_call_transport_local_persistence_helpers() {
    let content = read_repo_file("src/transport/gateway/handlers/admin_settings.rs");
    for forbidden in [
        "load_persisted_config_snapshot(",
        "save_persisted_config(",
        "load_auth_profile_store(",
        "save_auth_profile_store(",
    ] {
        assert!(
            !content.contains(forbidden),
            "admin settings handlers must route persisted config/auth state through runtime-owned facades, not {forbidden}"
        );
    }
}

#[test]
fn admin_auth_handlers_do_not_call_transport_local_persistence_helpers() {
    let content = read_repo_file("src/transport/gateway/handlers/admin_auth.rs");
    for forbidden in ["load_auth_profile_store(", "save_auth_profile_store("] {
        assert!(
            !content.contains(forbidden),
            "admin auth handlers must route persisted auth state through runtime-owned facades, not {forbidden}"
        );
    }
}

#[test]
fn admin_runtime_handlers_do_not_build_runtime_read_models_directly() {
    let content = read_repo_file("src/transport/gateway/handlers/admin_runtime.rs");
    for forbidden in [
        "build_runtime_status_read_model(",
        "impl SessionSummarySource for Session",
        "impl SessionMessageSource for ChatMessage",
    ] {
        assert!(
            !content.contains(forbidden),
            "admin runtime handlers must route runtime/session read-model assembly through runtime-owned facades, not {forbidden}"
        );
    }
}

#[test]
fn admin_session_handlers_do_not_stitch_session_queries_or_read_models_directly() {
    let content = read_repo_file("src/transport/gateway/handlers/admin_sessions.rs");
    for forbidden in [
        "build_session_list_read_model(",
        "build_session_message_list_read_model(",
        ".list_sessions(",
        ".get_history(",
        ".store()",
        "SessionSummaryReadModel {",
        "SessionMessageReadModel {",
    ] {
        assert!(
            !content.contains(forbidden),
            "admin session handlers must route session queries and read-model assembly through runtime-owned facades, not {forbidden}"
        );
    }
}

#[test]
fn gateway_handler_mod_does_not_define_auth_tenant_or_turn_bridge_helpers() {
    let content = read_repo_file("src/transport/gateway/handlers/mod.rs");
    for forbidden in [
        "fn bearer_token(",
        "fn hashed_auth_principal(",
        "fn source_identifier_from_headers(",
        "pub(super) fn paired_bearer_principal(",
        "pub(super) fn bind_principal_to_tenant(",
        "pub(super) fn request_policy_context(",
        "pub(super) fn require_management_principal(",
        "fn gateway_entity_id(",
        "async fn gateway_workspace_dir(",
        "pub(super) fn enforce_entity_rate_limit(",
        "fn log_tool_loop_stop(",
        "async fn run_tool_loop(",
    ] {
        assert!(
            !content.contains(forbidden),
            "gateway handlers/mod.rs must not remain the mixed auth/tenant/turn-bridge helper owner: {forbidden}"
        );
    }
}

#[test]
fn gateway_server_does_not_import_handler_inventory_directly() {
    let content = read_repo_file("src/transport/gateway/server.rs");
    for forbidden in ["use super::handlers::admin_", "use super::handlers::{"] {
        assert!(
            !content.contains(forbidden),
            "gateway server must route through grouped transport route builders, not direct handler inventory imports: {forbidden}"
        );
    }
}

#[test]
fn channel_dispatch_does_not_define_policy_routing_or_execution_context_helpers() {
    let content = read_repo_file("src/transport/channels/message_handler/dispatch.rs");
    for forbidden in [
        "pub(super) fn approval_context_for_message(",
        "pub(super) fn approval_context_for_event(",
        "pub(super) fn resolve_channel_policy(",
        "pub(super) fn resolve_channel_policy_for_name(",
        "pub(super) fn resolve_routing_group(",
        "pub(super) fn normalize_group_component(",
        "pub(super) fn resolve_group_isolation(",
        "pub(super) async fn build_execution_context(",
        "pub(super) async fn build_event_execution_context(",
    ] {
        assert!(
            !content.contains(forbidden),
            "channel dispatch.rs must not remain the mixed policy/routing/context helper owner: {forbidden}"
        );
    }
}

#[test]
fn gateway_companion_surfaces_do_not_import_plugin_companion_types_directly() {
    for relative in [
        "src/transport/gateway/types.rs",
        "src/transport/gateway/events.rs",
        "src/transport/gateway/handlers/mod.rs",
        "src/transport/gateway/handlers/companion.rs",
        "src/transport/gateway/handlers/companion_helpers.rs",
        "src/transport/gateway/handlers/companion_surface.rs",
    ] {
        let content = read_repo_file(relative);
        assert!(
            !content.contains("crate::plugins::companion::"),
            "gateway companion surfaces must route plugin companion types through an explicit transport bridge: {relative}"
        );
    }
}

#[test]
fn prompt_builder_root_does_not_define_mixed_capability_posture_or_bootstrap_sections() {
    let content = read_repo_file("src/transport/channels/prompt_builder.rs");
    for forbidden in [
        "const GATEWAY_CAPABILITIES_GUIDANCE",
        "const GATEWAY_MEMORY_GUIDANCE",
        "const INTROSPECTION_GUIDANCE",
        "const SAFETY_TEXT",
        "const PROMPT_CONFIDENTIALITY_TEXT",
        "policy_blocks",
        "PolicyBlock",
        "render_policy_section",
        "fn render_companion_posture_section(",
        "fn render_response_texture_section(",
        "fn inject_workspace_file(",
    ] {
        assert!(
            !content.contains(forbidden),
            "prompt builder root must delegate mixed prompt sections to focused modules, not {forbidden}"
        );
    }
    for required in [
        "crate::runtime::services::render_prompt_confidentiality_section()",
        "crate::runtime::services::render_baseline_safety_section()",
    ] {
        assert!(
            content.contains(required),
            "prompt builder root must consume runtime-owned baseline guardrail sections: {required}"
        );
    }
}

#[test]
fn channel_attachments_do_not_own_media_description_dispatch() {
    let content = read_repo_file("src/transport/channels/attachments.rs");
    assert!(
        !content.contains("describe_stored_attachment("),
        "channel attachments should not own stored-media description dispatch; keep media intelligence in src/media/*"
    );
}

#[test]
fn runtime_evolution_surface_is_fully_removed() {
    for removed in [
        "src/runtime/evolution/mod.rs",
        "src/runtime/evolution/cycle.rs",
        "src/runtime/evolution/proposals.rs",
        "src/runtime/evolution/outcome_store.rs",
    ] {
        assert!(
            !repo_root().join(removed).exists(),
            "runtime evolution surface must be deleted: {removed}"
        );
    }
}

#[test]
fn runtime_services_mod_does_not_define_bootstrap_provider_or_surface_plan_internals() {
    let content = read_repo_file("src/runtime/services/mod.rs");
    for forbidden in [
        "struct RuntimeExtensionLoader;",
        "struct RuntimeSkillMetadataProvider;",
        "struct RuntimeSkillMetadataSnapshot {",
        "struct RuntimeMcpToolProvider;",
        "pub struct RuntimeSurfaceResources {",
        "pub struct SharedRuntimeServices {",
        "pub struct GatewaySurfacePlan {",
        "pub struct ChannelsSurfacePlan {",
        "pub async fn bootstrap_runtime_services(",
        "async fn init_memory(",
        "pub fn create_resilient_provider(",
        "pub fn create_resilient_provider_box(",
        "pub fn create_provider_box(",
        "fn create_taste_provider(",
    ] {
        assert!(
            !content.contains(forbidden),
            "runtime services root should be a thin facade over focused modules, not {forbidden}"
        );
    }
}

#[test]
fn runtime_services_management_does_not_import_transport_channel_internals_directly() {
    let content = read_repo_file("src/runtime/services/management.rs");
    for forbidden in [
        "use crate::transport::channels::factory;",
        "use crate::transport::channels::{ChannelHealthState, classify_health_result};",
    ] {
        assert!(
            !content.contains(forbidden),
            "runtime management root should not import transport channel internals directly, not {forbidden}"
        );
    }
}

#[test]
fn runtime_services_management_root_does_not_define_mixed_channel_skill_or_config_helpers() {
    let content = read_repo_file("src/runtime/services/management.rs");
    for forbidden in [
        "enum ManagedChannelKind {",
        "fn load_persisted_runtime_config(",
        "fn save_persisted_runtime_config(",
        "fn load_channel_inventory(",
        "async fn run_channel_health_check(",
        "fn load_skill_inventory(",
        "fn install_skill(",
        "fn remove_skill(",
        "fn update_skill(",
    ] {
        assert!(
            !content.contains(forbidden),
            "runtime management root should delegate mixed admin helper clusters to focused modules, not {forbidden}"
        );
    }
}

#[test]
fn session_store_root_does_not_define_schema_mapping_or_transcript_helper_clusters() {
    let content = read_repo_file("src/core/sessions/store.rs");
    for forbidden in [
        "async fn ensure_sessions_schema(",
        "async fn ensure_binding_schema(",
        "fn map_session_row(",
        "fn map_chat_message_row(",
        "fn map_chat_message_part_row(",
        "fn assemble_transcript(",
        "fn flatten_message_parts(",
        "fn tail_messages_within_token_limit(",
    ] {
        assert!(
            !content.contains(forbidden),
            "session store root should delegate schema/mapping/transcript helpers to focused modules, not {forbidden}"
        );
    }
}

#[test]
fn session_orchestrator_root_does_not_define_transcript_or_thinking_state_helpers() {
    let content = read_repo_file("src/core/sessions/orchestrator.rs");
    for forbidden in [
        "const THINKING_STATE_USER_ID: &str =",
        "fn part_to_input(",
        "fn single_part(",
        "fn resolved_token_count(",
        "fn build_transcript_read_model(",
        "fn with_thinking_level_metadata(",
        "fn extract_thinking_level(",
        "fn cleared_thinking_level_metadata(",
    ] {
        assert!(
            !content.contains(forbidden),
            "session orchestrator root should delegate transcript/thinking-state helpers to focused modules, not {forbidden}"
        );
    }
}

#[test]
fn persona_state_persistence_root_does_not_define_transition_or_mirror_helper_clusters() {
    let content = read_repo_file("src/core/persona/state_persistence.rs");
    for forbidden in [
        "async fn persist_transition_record(",
        "async fn persist_transition_records(",
        "async fn emit_identity_transition_events(",
        "fn parse_state_header_mirror_markdown(",
        "fn write_atomic(",
    ] {
        assert!(
            !content.contains(forbidden),
            "persona state persistence root should delegate transition/mirror helpers to focused modules, not {forbidden}"
        );
    }
}

#[test]
fn memory_sleep_root_does_not_define_postgres_or_grouping_helper_clusters() {
    let content = read_repo_file("src/core/memory/hygiene/sleep.rs");
    for forbidden in [
        "async fn refresh_graph_entity_decay_scores(",
        "async fn load_sleep_consolidation_groups(",
        "async fn promote_sleep_episode_groups(",
        "async fn load_episode_promotion_candidates(",
        "fn aggregate_group(",
        "fn visibility_rank(",
        "fn visibility_from_rank(",
        "fn signal_tier_rank(",
        "fn open_pool(",
        "fn block_on_pg_result(",
    ] {
        assert!(
            !content.contains(forbidden),
            "memory sleep root should delegate postgres/grouping helpers to focused modules, not {forbidden}"
        );
    }
}

#[test]
fn evidence_id_uses_newtype_not_raw_string() {
    let hits = scan_struct_fields("src", "evidence_id: String");
    assert!(
        hits.is_empty(),
        "evidence_id must use EvidenceId newtype, not String. Found {} violation(s):\n{}",
        hits.len(),
        hits.iter()
            .map(|(f, l, s)| format!("  {f}:{l}: {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn contracts_ids_module_exists_and_exports_core_types() {
    let ids = std::fs::read_to_string(repo_root().join("src/contracts/ids.rs"))
        .expect("src/contracts/ids.rs must exist");
    for type_name in [
        "SessionId",
        "EntityId",
        "ChannelId",
        "UserId",
        "MessageId",
        "EventId",
        "SlotKey",
        "RunId",
        "ActionId",
        "ObservationId",
        "ScenarioId",
        "RequestId",
        "EvidenceId",
    ] {
        assert!(
            ids.contains(type_name),
            "contracts/ids.rs must define {type_name}"
        );
    }
    for removed_type in ["GoalId", "StepId", "PlanId", "BranchId"] {
        assert!(
            !ids.contains(removed_type),
            "contracts/ids.rs must not retain planner-era {removed_type}"
        );
    }
}

// ── Layer violation guards (Phase A) ─────────────────────────────────────────

#[test]
fn core_memory_ingestion_pipeline_does_not_import_runtime_observability() {
    let content = read_repo_file("src/core/memory/ingestion/pipeline.rs");
    assert!(
        !content.contains("runtime::observability"),
        "core/memory/ingestion/pipeline.rs (L1) must not import from runtime::observability (L4)"
    );
}

#[test]
fn core_agent_loop_does_not_import_runtime_observability() {
    for relative in [
        "src/core/agent/loop_/mod.rs",
        "src/core/agent/loop_/session.rs",
    ] {
        let content = read_repo_file(relative);
        assert!(
            !content.contains("runtime::observability"),
            "{relative} (L3) must not import from runtime::observability (L4)"
        );
    }
}

#[test]
fn core_agent_loop_does_not_own_simulation_presentation() {
    let loop_dir = repo_root().join("src/core/agent/loop_");
    assert!(
        !loop_dir.join("simulation_presenter.rs").exists(),
        "core/agent/loop_ must not retain simulation_presenter.rs after Packet A"
    );
    assert!(
        !loop_dir.join("session_planning.rs").exists(),
        "core/agent/loop_ must not retain session_planning.rs after Packet A"
    );

    let content = read_repo_file("src/core/agent/loop_/mod.rs");
    let forbidden = "simulation_presenter";
    assert!(
        !content.contains(forbidden),
        "core/agent/loop_ must not own simulation presentation after Packet A: {forbidden}"
    );

    for relative in ["src/core/agent/loop_/session_posturn.rs"] {
        let content = read_repo_file(relative);
        for forbidden in [
            "simulation_presenter",
            "fn render_simulation_for_planner(",
            "fn render_simulation_for_operator(",
            "pub fn render_simulation_for_planner(",
            "pub fn render_simulation_for_operator(",
        ] {
            assert!(
                !content.contains(forbidden),
                "{relative} must not own simulation presentation after Packet A: {forbidden}"
            );
        }
    }
}

#[test]
fn companion_turn_loop_keeps_character_runtime_modules_wired() {
    let pre_answer = read_repo_file("src/core/agent/loop_/augment/pipeline.rs");
    for required in [
        "build_taste_block(&self.workspace_dir)",
        "crate::core::affect::cause::attribute_cause_vad",
        "build_topology_block(",
        "snapshot.diffusion_diagnostics()",
        "crate::core::agent::session_control::update_control_state",
    ] {
        assert!(
            pre_answer.contains(required),
            "pre-answer augmentor must keep character runtime wiring: {required}"
        );
    }

    let persona_updates = read_repo_file("src/core/agent/loop_/augment/persona_updates.rs");
    for required in [
        "crate::core::persona::proactive::evaluate_proactive_triggers",
        "crate::core::persona::drift_detector::",
    ] {
        assert!(
            persona_updates.contains(required),
            "post-answer persona updates must keep character runtime wiring: {required}"
        );
    }

    let post_turn = read_repo_file("src/core/agent/loop_/session_posturn.rs");
    for required in [
        "crate::core::persona::user_facts::extract_user_facts",
        "crate::core::persona::relationship::update_relationship_after_turn",
    ] {
        assert!(
            post_turn.contains(required),
            "session post-turn pipeline must keep continuity wiring: {required}"
        );
    }

    let relationship = read_repo_file("src/core/persona/relationship.rs");
    assert!(
        relationship.contains("RelationshipEventInput"),
        "relationship post-turn updates must pass before/after state into event detection"
    );
}

#[test]
fn transport_turn_enrichment_keeps_matching_character_context_wiring() {
    let transport_enrichment = [
        "src/core/agent/turn_enrichment/mod.rs",
        "src/core/agent/turn_enrichment/turn_enrichment_io.rs",
        "src/core/agent/turn_enrichment/turn_enrichment_pipeline.rs",
    ]
    .into_iter()
    .map(read_repo_file)
    .collect::<Vec<_>>()
    .join("\n");
    for required in [
        "load_user_profile_for_entity",
        "load_and_update_session_control_block(",
        "build_transport_topology_snapshot(",
        "render_topology_block",
    ] {
        assert!(
            transport_enrichment.contains(required),
            "transport-facing turn enrichment must keep matching character context wiring: {required}"
        );
    }
}

#[test]
fn mcp_stdio_connection_spawn_stays_manager_owned() {
    let mcp_mod = read_repo_file("src/plugins/mcp/mod.rs");
    assert!(
        !mcp_mod.contains("pub use client_connection::McpConnection"),
        "McpConnection must not be publicly re-exported; stdio spawn policy is manager-owned"
    );
    assert!(
        !mcp_mod.contains("pub use client_proxy_tool::McpToolProxy"),
        "McpToolProxy must not be publicly re-exported with a direct connection constructor"
    );

    let connection = read_repo_file("src/plugins/mcp/client_connection.rs");
    assert!(
        connection.contains("pub(super) async fn connect_stdio("),
        "McpConnection::connect_stdio must stay visible only to the MCP manager boundary"
    );
    assert!(
        !connection.contains("pub async fn connect_stdio("),
        "McpConnection::connect_stdio must not be crate/public callable without process policy"
    );
}

#[test]
fn tunnel_concrete_connectors_stay_factory_owned() {
    let tunnel_mod = read_repo_file("src/runtime/tunnel/mod.rs");
    for forbidden in [
        "pub use cloudflare::CloudflareTunnel",
        "pub use custom::CustomTunnel",
        "pub use ngrok::NgrokTunnel",
        "pub use none::NoneTunnel",
        "pub use tailscale::TailscaleTunnel",
    ] {
        assert!(
            !tunnel_mod.contains(forbidden),
            "concrete tunnel adapters must not be publicly re-exported; use create_tunnel so process-spawn policy runs first: {forbidden}"
        );
    }

    let factory = read_repo_file("src/runtime/tunnel/factory.rs");
    assert!(
        factory.contains("enforce_spawn_policy(")
            && factory.contains("enforce_process_spawn_policy_with_args("),
        "tunnel factory must remain the process-spawn policy boundary"
    );
}

#[test]
fn packet_d_removes_primary_plan_surface_files() {
    for relative in [
        "src/transport/gateway/handlers/admin_plans.rs",
        "src/transport/gateway/handlers/admin_plan_trace.rs",
        "src/transport/gateway/handlers/plan_lifecycle_response.rs",
        "src/core/agent/loop_/plan_lifecycle_extension.rs",
        "src/core/agent/loop_/plan_lifecycle_handlers.rs",
        "src/core/agent/loop_/self_task_planner.rs",
        "src/runtime/services/plan_queries.rs",
        "src/runtime/diagnostics/control_plane_read_models/plan_trace.rs",
        "desktop/src/routes/plans.tsx",
        "src/cli/app/dispatch/plan_renderer.rs",
    ] {
        assert!(
            !repo_root().join(relative).exists(),
            "Packet D must remove primary plan surface file: {relative}"
        );
    }
}

#[test]
fn core_sessions_does_not_import_core_agent() {
    // tool_loop_transcript.rs was moved to core/agent/transcript.rs (A-3)
    let content = read_repo_file("src/core/sessions/compaction_context.rs");
    assert!(
        !content.contains("crate::core::agent"),
        "core/sessions/compaction_context.rs (L1) must not import from core::agent (L3)"
    );
}

#[test]
fn core_persona_scaffolding_does_not_import_core_agent() {
    let content = read_repo_file("src/core/persona/scaffolding.rs");
    assert!(
        !content.contains("crate::core::agent"),
        "core/persona/scaffolding.rs (L1) must not import from core::agent (L3)"
    );
}
