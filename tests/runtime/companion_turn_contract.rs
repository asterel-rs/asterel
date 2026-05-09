use asterel::runtime::services::{
    CompanionTurnContractExclusionRules, CompanionTurnContractFixture, derive_contract_semantics,
    semantics_match_with_exclusions,
};

fn gateway_http_fixture(user_message: &str, tenant_id: &str) -> CompanionTurnContractFixture {
    CompanionTurnContractFixture {
        transport_route: "gateway_http".to_string(),
        session_surface: Some("gateway_http".to_string()),
        user_message: user_message.to_string(),
        policy_tenant_id: Some(tenant_id.to_string()),
        session_owner_scope: Some(format!("tenant::{tenant_id}::source-42")),
        history_session_key: Some("source-42".to_string()),
        history_channel_name: "gateway_http".to_string(),
        history_richness_tokens: 1_536,
        channel_context_hint: None,
        image_content_present: false,
    }
}

fn gateway_ws_fixture(user_message: &str, tenant_id: &str) -> CompanionTurnContractFixture {
    CompanionTurnContractFixture {
        transport_route: "gateway_ws".to_string(),
        session_surface: Some("gateway_ws".to_string()),
        user_message: user_message.to_string(),
        policy_tenant_id: Some(tenant_id.to_string()),
        session_owner_scope: Some(format!("tenant::{tenant_id}::source-42")),
        history_session_key: Some("source-42".to_string()),
        history_channel_name: "gateway_ws".to_string(),
        history_richness_tokens: 1_024,
        channel_context_hint: None,
        image_content_present: false,
    }
}

fn channel_handler_fixture(user_message: &str, tenant_id: &str) -> CompanionTurnContractFixture {
    CompanionTurnContractFixture {
        transport_route: "channel_handler".to_string(),
        session_surface: Some("discord".to_string()),
        user_message: user_message.to_string(),
        policy_tenant_id: Some(tenant_id.to_string()),
        session_owner_scope: Some(format!(
            "tenant::{tenant_id}::conversation::discord::room-1"
        )),
        history_session_key: Some("conversation::discord::room-1".to_string()),
        history_channel_name: "discord".to_string(),
        history_richness_tokens: 4_096,
        channel_context_hint: None,
        image_content_present: false,
    }
}

fn channel_handler_fixture_with_hint(
    user_message: &str,
    tenant_id: &str,
    hint: &str,
) -> CompanionTurnContractFixture {
    CompanionTurnContractFixture {
        channel_context_hint: Some(hint.to_string()),
        ..channel_handler_fixture(user_message, tenant_id)
    }
}

#[test]
fn companion_turn_contract_semantics_match_across_transport_paths() {
    let user_message = "Can you summarize this thread in three bullets?";
    let tenant_id = "tenant-alpha";

    let fixtures = vec![
        gateway_http_fixture(user_message, tenant_id),
        gateway_ws_fixture(user_message, tenant_id),
        channel_handler_fixture(user_message, tenant_id),
    ];

    let rules = CompanionTurnContractExclusionRules::default();

    for left in &fixtures {
        for right in &fixtures {
            assert!(
                semantics_match_with_exclusions(left, right, &rules),
                "semantic contract mismatch: left={:?} right={:?}",
                derive_contract_semantics(left),
                derive_contract_semantics(right),
            );
        }
    }
}

#[test]
fn exclusion_rules_explicitly_allow_history_richness_diffs() {
    let user_message = "same message";
    let tenant_id = "tenant-alpha";
    let mut http = gateway_http_fixture(user_message, tenant_id);
    let mut ws = gateway_ws_fixture(user_message, tenant_id);

    http.history_richness_tokens = 512;
    ws.history_richness_tokens = 8_192;

    let rules = CompanionTurnContractExclusionRules::default();

    assert_eq!(
        rules.excluded_fields,
        vec!["history_richness_tokens", "transport_route"]
    );
    assert!(semantics_match_with_exclusions(&http, &ws, &rules));
}

#[test]
fn channel_pickup_semantics_distinguish_direct_from_ambient_context() {
    let direct = channel_handler_fixture_with_hint(
        "help me debug this",
        "tenant-alpha",
        "[Channel Context: Direct mention — concise, relevant to channel topic]",
    );
    let context_menu = channel_handler_fixture_with_hint(
        "Please summarize this message",
        "tenant-alpha",
        "discord:context_menu:summarize",
    );
    let thread = channel_handler_fixture_with_hint(
        "continuing the thread",
        "tenant-alpha",
        "[Channel Context: Thread continuation — stay on topic, build on prior context]",
    );
    let ambient = channel_handler_fixture_with_hint(
        "anyone know why this failed?",
        "tenant-alpha",
        "[Channel Context: Ambient pickup — brief, useful, and easy to ignore]",
    );
    let passive = channel_handler_fixture_with_hint("passive event", "tenant-alpha", "passive");

    assert_eq!(derive_contract_semantics(&direct).pickup, "direct");
    assert_eq!(derive_contract_semantics(&context_menu).pickup, "direct");
    assert_eq!(derive_contract_semantics(&thread).pickup, "ambient");
    assert_eq!(derive_contract_semantics(&ambient).pickup, "ambient");
    assert_eq!(derive_contract_semantics(&passive).pickup, "ambient");
}
