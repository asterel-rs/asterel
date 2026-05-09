use std::fs;
use std::path::Path;
use std::sync::Arc;

use asterel::config::PersonaConfig;
use asterel::core::memory::{MarkdownMemory, Memory};
use asterel::core::persona::state_header::StateHeader;
use asterel::core::persona::state_persistence::BackendHeaderPersist;
use tempfile::TempDir;

fn state_for_turn(turn: u8) -> StateHeader {
    StateHeader {
        identity_principles_hash: "identity-v1-abcd1234".to_string(),
        safety_posture: "strict".to_string(),
        current_objective: format!("Ship task 7 continuity turn {turn}"),
        open_loops: vec![format!("keep continuity across restart turn {turn}")],
        next_actions: vec!["run integration tests".to_string()],
        commitments: vec!["preserve canonical backend state".to_string()],
        recent_context_summary: format!("deterministic continuity integration turn {turn}"),
        last_updated_at: format!("2026-02-16T12:00:0{turn}Z"),
    }
}

fn persistence_for_workspace(workspace_dir: &Path) -> BackendHeaderPersist {
    let memory: Arc<dyn Memory> = Arc::new(MarkdownMemory::new(workspace_dir));
    let persona = PersonaConfig {
        state_mirror_filename: "STATE.md".to_string(),
        ..PersonaConfig::default()
    };
    BackendHeaderPersist::new(memory, workspace_dir.to_path_buf(), persona, "person-test")
}

#[tokio::test]
async fn persona_bootstrap_seeds_minimal_state() {
    let workspace = TempDir::new().unwrap();
    let persistence = persistence_for_workspace(workspace.path());

    let seeded = persistence
        .reconcile_mirror_from_backend_on_startup()
        .await
        .unwrap()
        .unwrap();

    assert!(!seeded.identity_principles_hash.trim().is_empty());
    assert!(!seeded.safety_posture.trim().is_empty());
    assert!(!seeded.current_objective.trim().is_empty());
    assert!(!seeded.recent_context_summary.trim().is_empty());

    let backend = persistence.load_backend_state().await.unwrap().unwrap();
    assert_eq!(backend, seeded);

    let mirror = persistence.read_mirror_state().unwrap().unwrap();
    assert_eq!(mirror, seeded);
}

#[tokio::test]
async fn continuity_across_restart_preserves_latest_backend_state_and_repairs_mirror() {
    let workspace = TempDir::new().unwrap();

    {
        let persistence = persistence_for_workspace(workspace.path());
        let turn1 = state_for_turn(1);
        persistence.persist_backend_sync(&turn1).await.unwrap();

        let turn2 = state_for_turn(2);
        persistence.persist_backend_sync(&turn2).await.unwrap();
    }

    let mirror_path = workspace.path().join("STATE.md");
    fs::write(
        &mirror_path,
        "# Persona State Header\n\n```json\n{\"current_objective\":\"stale\"}\n```\n",
    )
    .unwrap();

    let restarted = persistence_for_workspace(workspace.path());
    let recovered = restarted
        .reconcile_mirror_from_backend_on_startup()
        .await
        .unwrap()
        .unwrap();

    let expected_latest = state_for_turn(2);
    assert_eq!(recovered, expected_latest);

    let backend_after_restart = restarted.load_backend_state().await.unwrap().unwrap();
    assert_eq!(backend_after_restart, expected_latest);

    let mirror_after_restart = restarted.read_mirror_state().unwrap().unwrap();
    assert_eq!(mirror_after_restart, expected_latest);
}
