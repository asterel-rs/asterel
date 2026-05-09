use asterel::transport::channels::build_system_prompt;
#[cfg(feature = "whatsapp")]
use asterel::transport::gateway::verify_wa_signature;
use asterel::transport::gateway::{MAX_BODY_SIZE, REQUEST_TIMEOUT_SECS, WebhookBody};
use tempfile::TempDir;

fn write_workspace_fixture(path: &std::path::Path) {
    std::fs::write(path.join("AGENTS.md"), "# Agents\nKeep runtime stable.").unwrap();
    std::fs::write(path.join("SOUL.md"), "# Soul\nDeterministic.").unwrap();
    std::fs::write(path.join("TOOLS.md"), "# Tools\nUse safe tools.").unwrap();
    std::fs::write(path.join("CHARACTER.md"), "# Voice\nWarm and direct.").unwrap();
    std::fs::write(path.join("USER.md"), "# User\nRegression suite").unwrap();
    std::fs::write(path.join("HEARTBEAT.md"), "# Heartbeat\nOn").unwrap();
    std::fs::write(path.join("MEMORY.md"), "# Memory\nKnown preference").unwrap();
}

#[test]
fn channel_prompt_default_path_does_not_inject_state_mirror() {
    let workspace = TempDir::new().unwrap();
    write_workspace_fixture(workspace.path());
    std::fs::write(
        workspace.path().join("STATE.md"),
        "# State Header\n\ncurrent_objective: must stay main-session only",
    )
    .unwrap();

    let prompt = build_system_prompt(workspace.path(), "test-model", &[], &[], None);

    assert!(!prompt.contains("### State Header Mirror"));
    assert!(!prompt.contains("must stay main-session only"));
    assert!(prompt.contains("### MEMORY.md"));
}

#[test]
fn gateway_contract_regression_stays_stable() {
    assert_eq!(MAX_BODY_SIZE, 65_536);
    assert_eq!(REQUEST_TIMEOUT_SECS, 30);

    let valid: Result<WebhookBody, _> = serde_json::from_str(r#"{"message":"hello"}"#);
    assert!(valid.is_ok());

    let missing: Result<WebhookBody, _> = serde_json::from_str(r#"{"persona_state":"x"}"#);
    assert!(missing.is_err());
}

#[cfg(feature = "whatsapp")]
#[test]
fn gateway_signature_verification_behavior_is_unchanged() {
    let secret = "task7-secret";
    let body = br#"{"message":"hi"}"#;

    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let valid_header = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    assert!(verify_wa_signature(secret, body, &valid_header));
    assert!(!verify_wa_signature(secret, body, "sha256=deadbeef"));
}
