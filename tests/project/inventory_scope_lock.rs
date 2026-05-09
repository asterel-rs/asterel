use asterel::config::Config;
use asterel::plugins::integrations::inventory::{
    IntegrationCapabilityDrift, IntegrationCapabilityMatrix, IntegrationCapabilityMatrixEntry,
    InventoryTodoEntry, build_scope_lock_inventory, load_scope_lock_baseline_inventory,
    normalize_unimplemented_inventory_from_sources, parse_registry_coming_soon_count,
    validate_integration_status_against_matrix, validate_inventory_against_sources,
};
use asterel::plugins::integrations::registry;

fn baseline_matrix() -> Vec<IntegrationCapabilityMatrixEntry> {
    let matrix: IntegrationCapabilityMatrix = serde_json::from_str(include_str!(
        "../../src/plugins/integrations/integration_capability_matrix.json"
    ))
    .expect("integration capability matrix should parse");
    matrix.entries
}

#[test]
fn inventory_scope_lock() {
    let inventory = build_scope_lock_inventory().expect("scope-lock inventory should build");
    let artifact = inventory.to_json_pretty();
    let baseline = load_scope_lock_baseline_inventory().expect("baseline artifact should parse");
    let registry_source = include_str!("../../src/plugins/integrations/registry/catalog.rs");

    let expected_coming_soon_count = parse_registry_coming_soon_count(registry_source)
        .expect("registry coming-soon parser should work");

    println!("{artifact}");

    assert!(artifact.contains("\"coming_soon_count\""));
    assert_eq!(inventory, baseline);
    assert_eq!(inventory.coming_soon_count, expected_coming_soon_count);
}

#[test]
fn inventory_detects_registry_drift() {
    let inventory = build_scope_lock_inventory().expect("baseline inventory should build");
    let expected_coming_soon_count = inventory.coming_soon_count;
    let registry_source = include_str!("../../src/plugins/integrations/registry/catalog.rs");

    let drifted_registry = format!(
        "integration(\n    \"Drift Only\",\n    \"Temporary fixture\",\n    IntegrationCategory::Chat,\n    status::coming_soon,\n),\n{registry_source}"
    );
    let drifted_coming_soon_count =
        parse_registry_coming_soon_count(&drifted_registry).expect("drifted registry should parse");

    let drift_error = validate_inventory_against_sources(&inventory, &drifted_registry)
        .expect_err("drifted registry fixture should fail scope lock");

    println!("{drift_error}");

    let expected_mismatch = format!(
        "coming_soon_count mismatch: expected={}, actual={}",
        expected_coming_soon_count, drifted_coming_soon_count
    );

    assert!(drift_error.contains(&expected_mismatch));
}

#[test]
fn inventory_ignores_test_only_registry_coming_soon_tokens() {
    let registry_source = include_str!("../../src/plugins/integrations/registry/catalog.rs");
    let baseline_count =
        parse_registry_coming_soon_count(registry_source).expect("baseline registry should parse");

    let fixture_with_test_only_drift = format!(
        "{registry_source}\n#[cfg(test)]\nmod drift_only {{\n    fn test_only_fixture() {{\n        let _ = \"IntegrationStatus::ComingSoon\";\n        let _ = \"IntegrationStatus::ComingSoon\";\n    }}\n}}\n"
    );

    let parsed_count = parse_registry_coming_soon_count(&fixture_with_test_only_drift)
        .expect("fixture with test-only drift should parse");

    assert_eq!(
        parsed_count, baseline_count,
        "test-only drift tokens must not affect production-scope count"
    );
}

#[test]
fn normalize_unimplemented_inventory() {
    let registry_source = r#"
pub fn all_integrations() -> Vec<IntegrationEntry> {
    vec![
        IntegrationEntry {
            name: "Zeta",
            description: "Zeta integration",
            category: IntegrationCategory::AiModel,
            status_fn: status::coming_soon,
        },
        IntegrationEntry {
            name: "Alpha",
            description: "Alpha integration",
            category: IntegrationCategory::Chat,
            status_fn: status::coming_soon,
        },
        IntegrationEntry {
            name: "Active",
            description: "Already active",
            category: IntegrationCategory::Chat,
            status_fn: status::active,
        },
    ];
}
"#;

    let normalized: Vec<InventoryTodoEntry> =
        normalize_unimplemented_inventory_from_sources(registry_source)
            .expect("normalization should parse fixtures");

    assert_eq!(
        normalized,
        vec![
            InventoryTodoEntry {
                source: "integrations".to_string(),
                category: "AI Models".to_string(),
                status: "ComingSoon".to_string(),
                priority: 1,
                name: "Zeta".to_string(),
            },
            InventoryTodoEntry {
                source: "integrations".to_string(),
                category: "Chat Providers".to_string(),
                status: "ComingSoon".to_string(),
                priority: 1,
                name: "Alpha".to_string(),
            },
        ]
    );
}

#[test]
fn integrations_status_matches_capability_matrix() {
    let matrix_entries = baseline_matrix();
    let registry_entries = registry::all_integrations();
    let matrix = asterel::plugins::integrations::inventory::IntegrationCapabilityMatrix {
        schema_version: "1".to_string(),
        source_file: "src/plugins/integrations/registry.rs".to_string(),
        entries: matrix_entries,
    };

    validate_integration_status_against_matrix(&matrix, &registry_entries, &Config::default())
        .expect("status and matrix projection should match");

    assert!(
        matrix
            .entries
            .iter()
            .any(|entry| entry.name == "Shell" && entry.implemented),
        "baseline matrix must include implemented Shell"
    );
    assert!(
        registry_entries
            .iter()
            .all(|entry| entry.name != "WhatsApp"),
        "removed placeholder integrations must not remain in the registry"
    );
}

#[test]
fn integrations_rejects_unbacked_active_status() {
    let mut matrix = asterel::plugins::integrations::inventory::IntegrationCapabilityMatrix {
        schema_version: "1".to_string(),
        source_file: "src/plugins/integrations/registry.rs".to_string(),
        entries: baseline_matrix()
            .into_iter()
            .filter(|entry| entry.name != "Shell")
            .collect(),
    };
    matrix.entries.sort_by(|a, b| a.name.cmp(&b.name));

    let registry_entries = registry::all_integrations();
    let drifts =
        validate_integration_status_against_matrix(&matrix, &registry_entries, &Config::default())
            .expect_err("missing implemented capability for Shell should fail");

    assert!(
        drifts
            .iter()
            .any(|drift: &IntegrationCapabilityDrift| drift.name == "Shell"),
        "expected Shell mismatch to be reported: {drifts:?}"
    );
    assert!(
        drifts
            .iter()
            .any(|drift| drift.kind == "unsupported_status_claim"),
        "expected unsupported status claim classification for missing entry"
    );
}

#[test]
fn load_capability_matrix_and_validate() {
    let matrix = asterel::plugins::integrations::inventory::load_integration_capability_matrix()
        .expect("matrix should parse through loader helper");
    let registry_entries = registry::all_integrations();
    let result =
        validate_integration_status_against_matrix(&matrix, &registry_entries, &Config::default());
    assert!(
        result.is_ok(),
        "matrix projection should pass baseline validation: {result:?}"
    );
}
