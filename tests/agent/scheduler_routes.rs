use asterel::config::Config;
use asterel::core::memory::{
    MemoryEventInput, MemoryEventType, MemorySource, PrivacyLevel, create_memory,
};
use asterel::platform::cron::{CronJob, CronJobKind, CronJobOrigin};
use asterel::security::SecurityPolicy;
use chrono::Utc;
use tempfile::TempDir;

fn build_job(command: &str, job_kind: CronJobKind, origin: CronJobOrigin) -> CronJob {
    CronJob {
        id: format!("job-{job_kind:?}-{origin:?}"),
        enabled: true,
        expression: "* * * * *".to_string(),
        command: command.to_string(),
        next_run: Utc::now(),
        last_run: None,
        last_status: None,
        job_kind,
        origin,
        expires_at: None,
        max_attempts: 1,
        consecutive_failures: 0,
        breaker_open_until: None,
    }
}

fn config_with_workspace(workspace_dir: &std::path::Path) -> Config {
    Config {
        workspace_dir: workspace_dir.to_path_buf(),
        memory: asterel::config::MemoryConfig {
            backend: asterel::config::MemoryBackend::Markdown,
            ..asterel::config::MemoryConfig::default()
        },
        ..Config::default()
    }
}

#[tokio::test]
async fn scheduler_routes_user_vs_agent() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let mut config = config_with_workspace(&workspace);
    config.autonomy.allowed_commands = vec!["sh".to_string()];
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    // Intentional CronJobKind/CronJobOrigin cross-wire: routing is determined
    // by CronJobOrigin (User → user-direct-shell), NOT by CronJobKind.
    // Kind::Agent on a User-origin job verifies that Kind alone does not
    // influence the routing decision.
    let user_marker_file = "route-user-origin-marker.txt";
    let user_job = build_job(
        &format!("sh -c 'touch {user_marker_file}'"),
        CronJobKind::Agent, // intentionally mismatched — see comment above
        CronJobOrigin::User,
    );
    let (user_success, user_output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &user_job).await;

    assert!(user_success, "{user_output}");
    assert!(user_output.contains("route=user-direct-shell"));
    assert!(user_output.contains("status=exit status: 0"));
    assert!(workspace.join(user_marker_file).exists());

    let agent_marker_file = "route-agent-origin-marker.txt";
    let agent_job = build_job(
        &format!("sh -c 'touch {agent_marker_file}'"),
        CronJobKind::User, // intentionally mismatched — routing uses Origin, not Kind
        CronJobOrigin::Agent,
    );
    let (agent_success, agent_output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &agent_job).await;

    assert!(!agent_success);
    assert!(agent_output.contains("route=agent-no-direct-shell"));
    assert!(
        agent_output
            .contains("agent-origin cron commands are quarantined from the primary runtime")
    );
    assert!(!workspace.join(agent_marker_file).exists());
}

#[tokio::test]
async fn autonomy_policy_applies_all_routes() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let mut config = config_with_workspace(&workspace);
    config.autonomy.allowed_commands = vec!["echo".to_string()];
    config.autonomy.max_actions_per_hour = 1;
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let user_first = build_job("echo first", CronJobKind::User, CronJobOrigin::User);
    let (user_first_success, user_first_output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &user_first).await;
    assert!(user_first_success, "{user_first_output}");
    assert!(user_first_output.contains("route=user-direct-shell"));

    let agent_second = build_job("echo second", CronJobKind::Agent, CronJobOrigin::Agent);
    let (agent_second_success, agent_second_output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &agent_second).await;

    assert!(!agent_second_success);
    assert!(agent_second_output.contains("route=agent-no-direct-shell"));
    assert!(agent_second_output.contains("blocked by security policy: action limit exceeded"));
}

#[tokio::test]
async fn autonomy_policy_blocks_bypass() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let mut config = config_with_workspace(&workspace);
    config.autonomy.allowed_commands = vec!["cat".to_string()];
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let agent_disallowed = build_job(
        "sh -c 'touch bypass.txt'",
        CronJobKind::Agent,
        CronJobOrigin::Agent,
    );
    let (agent_disallowed_success, agent_disallowed_output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &agent_disallowed)
            .await;

    assert!(!agent_disallowed_success);
    assert!(agent_disallowed_output.contains("route=agent-no-direct-shell"));
    assert!(agent_disallowed_output.contains("blocked by security policy"));
    assert!(agent_disallowed_output.contains("command not allowed"));

    let user_forbidden_path = build_job("cat /etc/passwd", CronJobKind::User, CronJobOrigin::User);
    let (user_forbidden_path_success, user_forbidden_path_output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &user_forbidden_path)
            .await;

    assert!(!user_forbidden_path_success);
    assert!(user_forbidden_path_output.contains("route=user-direct-shell"));
    assert!(user_forbidden_path_output.contains("forbidden path argument"));

    let agent_forbidden_path =
        build_job("cat /etc/passwd", CronJobKind::Agent, CronJobOrigin::Agent);
    let (agent_forbidden_path_success, agent_forbidden_path_output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &agent_forbidden_path)
            .await;

    assert!(!agent_forbidden_path_success);
    assert!(agent_forbidden_path_output.contains("route=agent-no-direct-shell"));
    assert!(agent_forbidden_path_output.contains("forbidden path argument"));
}

#[tokio::test]
async fn scheduler_routes_ingestion_pipeline_paths_end_to_end() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let config = config_with_workspace(&workspace);
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let entity_id = format!(
        "person:ingest.{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );

    let api_job = build_job(
        &format!("ingest:api {entity_id} api-e2e-1 release pulse signal"),
        CronJobKind::User,
        CronJobOrigin::User,
    );
    let (api_success, api_output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &api_job).await;
    assert!(api_success, "{api_output}");
    assert!(api_output.contains("route=user-ingestion-pipeline"));
    assert!(api_output.contains("accepted=true"));

    let api_repeat = build_job(
        &format!("ingest:api {entity_id} api-e2e-1 release pulse signal"),
        CronJobKind::User,
        CronJobOrigin::User,
    );
    let (api_repeat_success, api_repeat_output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &api_repeat).await;
    assert!(!api_repeat_success, "{api_repeat_output}");
    assert!(api_repeat_output.contains("accepted=false"));
    assert!(
        api_repeat_output.contains("reason=rate_limited")
            || api_repeat_output.contains("reason=duplicate")
            || api_repeat_output.contains("reason=dedup:"),
        "expected repeat ingestion to be rejected by rate limiting or duplicate detection, got: {api_repeat_output}"
    );

    let rss_job = build_job(
        &format!("ingest:rss {entity_id} rss-e2e-1 release pulse signal from rss"),
        CronJobKind::User,
        CronJobOrigin::User,
    );
    let (rss_success, rss_output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &rss_job).await;
    assert!(rss_success, "{rss_output}");
    assert!(rss_output.contains("route=user-ingestion-pipeline"));
    assert!(rss_output.contains("accepted=true"));

    let x_job = build_job(
        &format!("ingest:x {entity_id} tweet-e2e-1 release pulse signal on x"),
        CronJobKind::User,
        CronJobOrigin::User,
    );
    let (x_success, x_output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &x_job).await;
    assert!(x_success, "{x_output}");
    assert!(x_output.contains("route=user-ingestion-pipeline"));
    assert!(x_output.contains("accepted=true"));

    let memory = create_memory(&config.memory, &config.workspace_dir, None)
        .await
        .expect("memory backend should initialize");
    memory
        .append_event(MemoryEventInput::new(
            &entity_id,
            "external.api.seed-release-a",
            MemoryEventType::FactAdded,
            "release pulse signal seeded A",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        ))
        .await
        .expect("seed external candidate A");
    memory
        .append_event(MemoryEventInput::new(
            &entity_id,
            "external.news.seed-release-b",
            MemoryEventType::FactAdded,
            "release pulse signal seeded B",
            MemorySource::ExplicitUser,
            PrivacyLevel::Private,
        ))
        .await
        .expect("seed external candidate B");

    let trend_job = build_job(
        &format!("ingest:trend {entity_id} release release pulse"),
        CronJobKind::User,
        CronJobOrigin::User,
    );
    let (trend_success, trend_output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &trend_job).await;
    assert!(trend_success, "{trend_output}");
    assert!(trend_output.contains("route=user-trend-aggregation"));
    assert!(trend_output.contains("accepted=true"), "{trend_output}");
    assert!(trend_output.contains("slot_key=trend.snapshot.release"));

    let trend_slot = memory
        .resolve_slot(&entity_id, "trend.snapshot.release")
        .await
        .expect("resolve slot should succeed")
        .expect("trend snapshot should be written");
    assert!(trend_slot.value.contains("trend topic=release"));
}

#[tokio::test]
async fn scheduler_routes_trend_without_candidates_reports_no_external_candidates() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let config = config_with_workspace(&workspace);
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let entity_id = format!(
        "person:ingest.no-candidates.{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let trend_job = build_job(
        &format!("ingest:trend {entity_id} release release pulse"),
        CronJobKind::User,
        CronJobOrigin::User,
    );

    let (trend_success, trend_output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &trend_job).await;
    assert!(trend_success, "{trend_output}");
    assert!(trend_output.contains("route=user-trend-aggregation"));
    assert!(trend_output.contains("accepted=false"));
    assert!(trend_output.contains("reason=no_external_candidates"));
}

#[tokio::test]
async fn scheduler_routes_trend_missing_query_is_blocked_by_policy_allowlist() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let config = config_with_workspace(&workspace);
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let entity_id = format!(
        "person:trend.invalid.{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let job = build_job(
        &format!("ingest:trend {entity_id} release"),
        CronJobKind::User,
        CronJobOrigin::User,
    );

    let (success, output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &job).await;
    assert!(!success, "{output}");
    assert!(output.contains("route=invalid-routed-cron"), "{output}");
    assert!(output.contains("malformed reserved route"), "{output}");
}

#[tokio::test]
async fn scheduler_routes_rss_poll_invalid_url_reports_route_failure() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let config = config_with_workspace(&workspace);
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let entity_id = format!(
        "person:rss.invalid.{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let job = build_job(
        &format!("ingest:rss-poll {entity_id} not-a-valid-url"),
        CronJobKind::User,
        CronJobOrigin::User,
    );

    let (success, output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &job).await;
    assert!(!success, "{output}");
    assert!(output.contains("route=user-rss-poll"), "{output}");
    assert!(
        output.contains("invalid rss url") || output.contains("request failed"),
        "{output}",
    );
}

#[tokio::test]
async fn scheduler_routes_rss_poll_missing_url_is_blocked_by_policy_allowlist() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let config = config_with_workspace(&workspace);
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let entity_id = format!(
        "person:rss.missing-url.{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let job = build_job(
        &format!("ingest:rss-poll {entity_id}"),
        CronJobKind::User,
        CronJobOrigin::User,
    );

    let (success, output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &job).await;
    assert!(!success, "{output}");
    assert!(output.contains("route=invalid-routed-cron"), "{output}");
    assert!(output.contains("malformed reserved route"), "{output}");
}

#[tokio::test]
async fn scheduler_routes_x_poll_missing_query_is_blocked_by_policy_allowlist() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let config = config_with_workspace(&workspace);
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let entity_id = format!(
        "person:xpoll.invalid.{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let job = build_job(
        &format!("ingest:x-poll {entity_id}"),
        CronJobKind::User,
        CronJobOrigin::User,
    );

    let (success, output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &job).await;
    assert!(!success, "{output}");
    assert!(output.contains("route=invalid-routed-cron"), "{output}");
    assert!(output.contains("malformed reserved route"), "{output}");
}

#[tokio::test]
async fn scheduler_routes_x_poll_without_token_reports_missing_bearer_token() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let config = config_with_workspace(&workspace);
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let entity_id = format!(
        "person:xpoll.no-token.{}",
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let job = build_job(
        &format!("ingest:x-poll {entity_id} rustlang from:rustlang"),
        CronJobKind::User,
        CronJobOrigin::User,
    );

    let (success, output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &job).await;
    assert!(!success, "{output}");
    assert!(output.contains("route=user-x-poll"), "{output}");
    assert!(output.contains("missing X_BEARER_TOKEN"), "{output}");
}

#[tokio::test]
async fn scheduler_routes_agent_plan_respects_retry_limit_budget() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let config = config_with_workspace(&workspace);
    let security = SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir);

    let plan = serde_json::json!({
        "id": "retry-budget-route-test",
        "description": "agent planner route retry budget",
        "steps": [
            {
                "id": "step_1",
                "description": "unknown tool should fail deterministically",
                "action": {
                    "kind": "tool_call",
                    "tool_name": "tool_does_not_exist",
                    "args": {}
                },
                "depends_on": []
            }
        ]
    });

    let mut agent_job = build_job(
        &format!("plan:{}", serde_json::to_string(&plan).unwrap()),
        CronJobKind::Agent,
        CronJobOrigin::Agent,
    );
    agent_job.max_attempts = 2;

    let (success, output) =
        asterel::platform::cron::scheduler::run_job_once(&config, &security, &agent_job).await;

    assert!(!success, "{output}");
    assert!(output.contains("route=agent-no-direct-shell"), "{output}");
    assert!(
        output.contains("legacy planner cron commands are no longer accepted"),
        "{output}"
    );
}
