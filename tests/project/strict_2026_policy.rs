use std::collections::BTreeMap;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_repo_file(relative: &str) -> String {
    std::fs::read_to_string(repo_root().join(relative)).unwrap_or_else(|error| {
        panic!("failed to read {relative}: {error}");
    })
}

fn markdown_section<'a>(content: &'a str, heading: &str) -> &'a str {
    let start = content
        .find(heading)
        .unwrap_or_else(|| panic!("failed to find markdown heading: {heading}"));
    let after_heading = start + heading.len();
    let end = content[after_heading..]
        .find("\n## ")
        .map(|offset| after_heading + offset)
        .unwrap_or(content.len());

    &content[start..end]
}

#[test]
fn rust_toolchain_is_pinned_to_specific_version() {
    let toolchain =
        std::fs::read_to_string(repo_root().join("rust-toolchain.toml")).expect("read toolchain");

    assert!(
        toolchain.contains("channel = \"1.88.0\""),
        "toolchain channel must be fixed to 1.88.0: {toolchain}"
    );
    assert!(
        !toolchain.contains("channel = \"stable\""),
        "strict policy forbids floating stable channel"
    );
}

#[test]
fn rustfmt_style_edition_is_locked_to_2024() {
    let rustfmt = std::fs::read_to_string(repo_root().join("rustfmt.toml")).expect("read rustfmt");
    assert!(
        rustfmt.contains("style_edition = \"2024\""),
        "rustfmt.toml must pin style_edition to 2024"
    );
}

#[test]
fn crate_roots_deny_unsafe_code() {
    let lib_rs = std::fs::read_to_string(repo_root().join("src/lib.rs")).expect("read src/lib.rs");
    let main_rs =
        std::fs::read_to_string(repo_root().join("src/main.rs")).expect("read src/main.rs");

    assert!(
        lib_rs.contains("#![deny(unsafe_code)]"),
        "src/lib.rs must deny unsafe_code at crate root"
    );
    assert!(
        main_rs.contains("#![deny(unsafe_code)]"),
        "src/main.rs must deny unsafe_code at crate root"
    );
}

#[test]
fn intent_classifier_has_no_unsafe_send_sync_impls() {
    let classify =
        std::fs::read_to_string(repo_root().join("src/security/intent_classifier/classify.rs"))
            .expect("read intent classifier");

    assert!(
        !classify.contains("unsafe impl Send"),
        "intent classifier must not use unsafe impl Send"
    );
    assert!(
        !classify.contains("unsafe impl Sync"),
        "intent classifier must not use unsafe impl Sync"
    );
}

#[test]
fn release_gate_script_exists_and_contains_required_quality_gates() {
    let gate = read_repo_file("scripts/release/human_like_release_gate.sh");
    for command in [
        "cargo fmt -- --check",
        "cargo clippy -- -D warnings",
        "cargo check-all",
        "cargo test",
        "cargo fuzz-smoke",
        "cargo audit",
        "cargo run -- eval baseline --seed",
        "cargo run -- eval replay --input",
    ] {
        assert!(
            gate.contains(command),
            "release gate script must include: {command}"
        );
    }
    assert!(
        gate.contains("discord_companion_bad_turns.jsonl"),
        "release gate script must include the Discord companion bad-turn replay fixture"
    );
    assert!(
        gate.contains("verifier_event_ratio_bps"),
        "release gate script must assert replay verifier event metrics"
    );
    for reason in ["anti_template", "exposure_violation", "over_explain"] {
        assert!(
            gate.contains(reason),
            "release gate script must assert replay verifier reason: {reason}"
        );
    }
}

#[test]
fn public_docs_do_not_reference_removed_turn_enrichment_file() {
    for file in [
        "README.md",
        "docs/src/content/docs/architecture/turn-pipeline.mdx",
        "docs/src/content/docs/ja/architecture/turn-pipeline.mdx",
    ] {
        let content = read_repo_file(file);
        assert!(
            !content.contains("src/core/agent/turn_enrichment.rs"),
            "{file} must point to the turn_enrichment module directory, not the removed flat file"
        );
    }
}

#[test]
fn public_docs_track_current_release_gate_and_aliases() {
    let readme = read_repo_file("README.md");
    for alias in ["cargo fuzz-smoke", "cargo ntest-supported-features"] {
        assert!(
            readme.contains(alias),
            "README useful aliases should include gate-relevant alias: {alias}"
        );
    }

    let release = read_repo_file("docs/src/content/docs/guide/release.md");
    for required in [
        "scripts/release/human_like_release_gate.sh",
        "cargo check-all",
        "cargo fuzz-smoke",
        "cargo audit",
        "baseline",
        "replay",
    ] {
        assert!(
            release.contains(required),
            "release guide must document strict gate requirement: {required}"
        );
    }
}

#[test]
fn public_docs_do_not_advertise_removed_security_perimeter_config() {
    let readme = read_repo_file("README.md");
    for removed in [
        "[security.perimeter]",
        "enforce_uniform_inner_freedom",
        "supported_targets",
    ] {
        assert!(
            !readme.contains(removed),
            "README must not advertise unsupported config key: {removed}"
        );
    }
}

#[test]
fn env_example_covers_documented_local_runtime_overrides() {
    let env_example = read_repo_file(".env.example");
    for variable in [
        "ASTEREL_POSTGRES_URL",
        "ASTEREL_GATEWAY_ALLOW_PUBLIC_BIND",
        "ASTEREL_INTENT_CLASSIFIER_ENABLED",
    ] {
        assert!(
            env_example.contains(variable),
            ".env.example should include documented local runtime override: {variable}"
        );
    }
}

#[test]
fn postgres_schema_runner_includes_v3_guardrail_migration() {
    let schema_runner = read_repo_file("src/core/memory/postgres/schema.rs");
    assert!(
        schema_runner.contains("MIGRATION_V3_SQL"),
        "schema runner must include v3 migration payload"
    );
    assert!(
        schema_runner.contains("if current_version < 3"),
        "schema runner must execute v3 migration when needed"
    );

    let v3 = read_repo_file("migrations/003_retrieval_units_guardrails.sql");
    assert!(
        v3.contains("INSERT INTO schema_version (version) VALUES (3)"),
        "v3 migration must record schema version 3"
    );
    assert!(
        v3.contains("chk_retrieval_units_recency_score_range"),
        "v3 migration must add recency score guardrail constraint"
    );
}

#[test]
fn graphrag_migrations_do_not_create_dimensionless_vector_hnsw_index() {
    for file in [
        "migrations/007_graphrag_extensions.sql",
        "migrations/008_graphrag_ontology.sql",
    ] {
        let content = read_repo_file(file);
        assert!(
            !content.contains("idx_graph_entities_embedding"),
            "{file} must not create an HNSW index on graph_entities.embedding without fixed dimensions"
        );
        assert!(
            content.contains("embedding vector"),
            "{file} should still keep the graph_entities.embedding column available"
        );
    }
}

#[test]
fn graphrag_ontology_migration_no_longer_creates_legacy_planner_simulation_tables() {
    let content = read_repo_file("migrations/008_graphrag_ontology.sql");
    for forbidden in [
        "CREATE TABLE IF NOT EXISTS scenarios",
        "CREATE TABLE IF NOT EXISTS scenario_actors",
        "CREATE TABLE IF NOT EXISTS simulation_runs",
        "CREATE TABLE IF NOT EXISTS simulation_events",
        "CREATE TABLE IF NOT EXISTS action_candidates",
        "CREATE TABLE IF NOT EXISTS outcome_observations",
    ] {
        assert!(
            !content.contains(forbidden),
            "graph ontology migration must not recreate legacy planner/simulation table: {forbidden}"
        );
    }
}

#[test]
fn schema_runner_uses_terminal_legacy_cleanup_migration() {
    let schema_runner = read_repo_file("src/core/memory/postgres/schema.rs");
    for forbidden in [
        "MIGRATION_V10_SQL",
        "MIGRATION_V11_SQL",
        "MIGRATION_V16_SQL",
        "MIGRATION_V17_SQL",
        "MIGRATION_V18_SQL",
        "MIGRATION_V19_SQL",
    ] {
        assert!(
            !schema_runner.contains(forbidden),
            "schema runner must not keep deleted legacy migration hook: {forbidden}"
        );
    }
    assert!(
        schema_runner.contains("MIGRATION_V20_SQL"),
        "schema runner must include the terminal legacy cleanup migration"
    );
    assert!(
        schema_runner.contains("if current_version < 20"),
        "schema runner must run the terminal legacy cleanup migration when needed"
    );

    let v15 = read_repo_file("migrations/015_operator_trust_state.sql");
    assert!(
        !v15.contains("plan_trace_observations"),
        "operator trust migration must no longer create planner-era trace tables"
    );

    let cleanup = read_repo_file("migrations/020_drop_legacy_planner_simulation_tables.sql");
    for required in [
        "DROP TABLE IF EXISTS outcome_observations",
        "DROP TABLE IF EXISTS action_candidates",
        "DROP TABLE IF EXISTS simulation_events",
        "DROP TABLE IF EXISTS simulation_runs",
        "DROP TABLE IF EXISTS scenario_actors",
        "DROP TABLE IF EXISTS scenarios",
        "DROP TABLE IF EXISTS plan_trace_observations",
        "INSERT INTO schema_version (version) VALUES (20)",
    ] {
        assert!(
            cleanup.contains(required),
            "terminal legacy cleanup migration must include: {required}"
        );
    }
}

#[test]
fn deleted_legacy_planner_simulation_migration_files_are_absent() {
    for file in [
        "migrations/010_simulation_scenarios.sql",
        "migrations/011_simulation_runs.sql",
        "migrations/016_scenario_actor_fields.sql",
        "migrations/017_plan_trace_observations_realign.sql",
        "migrations/018_scenarios_drop_legacy_columns.sql",
        "migrations/019_simulation_events_schema_realign.sql",
    ] {
        assert!(
            !repo_root().join(file).exists(),
            "legacy migration file must be deleted: {file}"
        );
    }
}

#[test]
fn active_desktop_and_runtime_copy_do_not_reintroduce_planner_wording() {
    for file in ["desktop/src/locales/en.json", "desktop/src/locales/ja.json"] {
        let content = read_repo_file(file);
        for forbidden in [
            "Plan inbox",
            "Pending plans",
            "Autonomy plans",
            "selected plan",
            "plan approvals",
            "high plan failure rate",
        ] {
            assert!(
                !content.contains(forbidden),
                "active surface copy must not reintroduce deleted planner wording ({forbidden}) in {file}"
            );
        }
    }
}

#[test]
fn generated_admin_contract_client_is_checked_in() {
    let content = read_repo_file("desktop/src/lib/admin-contract.generated.ts");
    assert!(
        content.contains("export const ADMIN_PATHS"),
        "generated admin contract client must export ADMIN_PATHS"
    );
    assert!(
        content.contains("export function adminPath"),
        "generated admin contract client must export adminPath formatter"
    );
}

#[test]
fn desktop_route_tree_no_longer_contains_lab_routes() {
    let route_tree = read_repo_file("desktop/src/routeTree.gen.ts");
    for forbidden in ["'/lab", "\"/lab", "./routes/lab/"] {
        assert!(
            !route_tree.contains(forbidden),
            "desktop route tree must not include removed lab routes: {forbidden}"
        );
    }

    let sidebar = read_repo_file("desktop/src/components/sidebar.tsx");
    assert!(
        !sidebar.contains("\"/lab"),
        "desktop sidebar must not include removed lab navigation"
    );
    assert!(
        sidebar.contains("\"/companion\""),
        "desktop sidebar must expose the stable companion route"
    );
}

#[test]
fn admin_contract_excludes_removed_lab_only_admin_routes() {
    let contract = read_repo_file("src/transport/gateway/admin_contract.json");
    for forbidden in [
        "/admin/v1/a2a/tasks",
        "/admin/v1/a2a/stats",
        "/admin/v1/a2a/agent-card",
        "A2aTaskListResponse",
        "A2aTaskDetail",
        "A2aStats",
        "A2aAgentCard",
        "/admin/v1/character-runtime",
        "/admin/v1/auth-profiles",
        "/admin/v1/auth-profiles/{id}",
        "/admin/v1/auth-profiles/{id}/test",
        "/admin/v1/auth-profiles/{id}/oauth/start",
        "adminCharacterRuntime",
        "adminAuthProfileCreate",
        "adminAuthProfileDelete",
        "adminAuthProfileTest",
        "adminAuthProfileOauthStart",
    ] {
        assert!(
            !contract.contains(forbidden),
            "admin contract must not retain removed lab-only admin surface: {forbidden}"
        );
    }
}

#[test]
fn admin_contract_cron_kind_matches_runtime_enum() {
    let contract = read_repo_file("src/transport/gateway/admin_contract.json");
    assert!(
        contract
            .contains("\"job_kind\": { \"type\": \"string\", \"enum\": [\"user\", \"agent\"] }"),
        "admin contract cron enum must match runtime-supported job kinds"
    );
    assert!(
        !contract.contains("\"evolution\""),
        "admin contract must not advertise removed cron kind 'evolution'"
    );
}

#[test]
fn fake_memory_pin_and_unwired_context_event_surfaces_are_removed() {
    for removed in [
        "src/core/tools/memory/pin.rs",
        "src/core/agent/context_events.rs",
        "src/config/layered.rs",
    ] {
        assert!(
            !repo_root().join(removed).exists(),
            "removed placeholder surface must stay deleted: {removed}"
        );
    }

    let tools_cfg = read_repo_file("src/config/schema/tools.rs");
    assert!(
        !tools_cfg.contains("memory_pin"),
        "tools config must not keep the removed memory_pin surface"
    );

    let agent_mod = read_repo_file("src/core/agent/mod.rs");
    assert!(
        !agent_mod.contains("context_events"),
        "agent module must not export the removed context-events subsystem"
    );

    let config_mod = read_repo_file("src/config/mod.rs");
    assert!(
        !config_mod.contains("mod layered"),
        "config module must not retain the removed layered-config abstraction"
    );
}

#[test]
fn desktop_locales_do_not_keep_removed_lab_or_evolution_copy() {
    for file in ["desktop/src/locales/en.json", "desktop/src/locales/ja.json"] {
        let content = read_repo_file(file);
        for forbidden in [
            "\"Evolution\"",
            "No candidates to compare. Run an evolution cycle first.",
            "No plans found",
            "No run selected",
            "No task selected",
        ] {
            assert!(
                !content.contains(forbidden),
                "desktop locale bundle must not retain removed surface copy ({forbidden}) in {file}"
            );
        }
    }
}

#[test]
fn runtime_evolution_and_config_surface_are_removed() {
    for removed in [
        "src/runtime/evolution/mod.rs",
        "src/runtime/evolution/cycle.rs",
        "src/config/schema/core/evolution.rs",
        "src/contracts/evolution.rs",
    ] {
        assert!(
            !repo_root().join(removed).exists(),
            "evolution surface file must be deleted: {removed}"
        );
    }

    let runtime_mod = read_repo_file("src/runtime/mod.rs");
    assert!(
        !runtime_mod.contains("pub mod evolution;"),
        "runtime root must not export an evolution module"
    );

    let commands = read_repo_file("src/cli/commands/mod.rs");
    assert!(
        !commands.contains("Evolve {"),
        "CLI command model must not expose evolve"
    );

    let config_mod = read_repo_file("src/config/mod.rs");
    assert!(
        !config_mod.contains("EvolutionConfig"),
        "config facade must not re-export EvolutionConfig"
    );

    let config_root = read_repo_file("src/config/schema/core/types.rs");
    assert!(
        !config_root.contains("pub evolution:"),
        "root Config must not contain an evolution section"
    );
}

#[test]
fn extensions_route_no_longer_duplicates_primary_channels_surface() {
    let route = read_repo_file("desktop/src/routes/extensions/index.tsx");
    assert!(
        !route.contains("ChannelsTab"),
        "/extensions must not duplicate the primary channels surface"
    );
    assert!(
        !route.contains("value: \"channels\""),
        "/extensions must not keep a channels tab after channels became primary"
    );
}

#[test]
fn migration_versions_are_unique() {
    let migrations_dir = repo_root().join("migrations");
    let entries = std::fs::read_dir(&migrations_dir).expect("read migrations dir");
    let mut versions: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for entry in entries {
        let entry = entry.expect("read migrations dir entry");
        let Some(file_name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some((version, _rest)) = file_name.split_once('_') else {
            continue;
        };
        if !version.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        versions
            .entry(version.to_owned())
            .or_default()
            .push(file_name);
    }

    let duplicates: Vec<String> = versions
        .into_iter()
        .filter_map(|(version, files)| {
            (files.len() > 1).then(|| format!("{version}: {}", files.join(", ")))
        })
        .collect();

    assert!(
        duplicates.is_empty(),
        "migration version prefixes must be unique; duplicates: {}",
        duplicates.join("; ")
    );
}

#[test]
fn daemon_heartbeat_memory_metrics_stays_on_the_parent_async_runtime() {
    let content = read_repo_file("src/platform/daemon/heartbeat_worker/memory_metrics.rs");
    assert!(
        content.contains("pub(super) async fn run_memory_hygiene_tick"),
        "heartbeat memory metrics should run as an async worker task"
    );
    assert!(
        !content.contains("tokio::runtime::Runtime::new()"),
        "heartbeat memory metrics must not create a nested Tokio runtime inside the daemon worker"
    );
    assert!(
        !content.contains("block_on_sync("),
        "heartbeat memory metrics must not synchronously bridge SQL work inside the daemon worker"
    );
}

#[test]
fn heartbeat_worker_routes_agent_tasks_through_runtime_surface() {
    let content = read_repo_file("src/platform/daemon/heartbeat_worker.rs");
    assert!(
        content.contains("crate::runtime::services::run_agent_surface("),
        "heartbeat worker must route agent tasks through the runtime-owned agent surface"
    );
    assert!(
        !content.contains("crate::core::agent::run("),
        "heartbeat worker must not bypass runtime-owned agent surface composition"
    );
}

#[test]
fn packet_d_removes_legacy_sync_store_files() {
    for file in [
        "src/core/planner/lifecycle/db.rs",
        "src/simulation/store/db.rs",
    ] {
        assert!(
            !repo_root().join(file).exists(),
            "Packet D should remove legacy sync store file: {file}"
        );
    }
}

#[test]
fn references_cross_reference_index_excludes_deleted_packet_d_modules() {
    let references = read_repo_file("docs/src/content/docs/reference/references.md");
    let current_module_index = markdown_section(&references, "## 9. Cross-Reference Index");

    for deleted in [
        "src/simulation/",
        "src/core/planner/",
        "src/core/eval/plan_holdout_",
        "src/core/eval/plan_outcome_",
        "src/contracts/simulation.rs",
        "src/runtime/diagnostics/control_plane_read_models/eval.rs",
    ] {
        assert!(
            !current_module_index.contains(deleted),
            "Cross-Reference Index must not map deleted Packet D module as current code: {deleted}"
        );
    }
}

#[test]
fn integration_tests_do_not_mutate_env_with_inline_unsafe_blocks() {
    for file in [
        "tests/cli_process_smoke.rs",
        "tests/agent/autonomy_cross_layer_flow.rs",
        "tests/persona/self_task_flow.rs",
    ] {
        let content = read_repo_file(file);
        assert!(
            !content.contains("unsafe { std::env::set_var"),
            "{file} should use shared test env helpers instead of inline unsafe env mutation"
        );
    }
}
