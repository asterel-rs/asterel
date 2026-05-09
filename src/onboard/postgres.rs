//! Local Postgres setup and revival helpers for onboarding flows.
//!
//! Keeps provisioning/runtime concerns separate from the interactive wizard
//! orchestration in `flow.rs`.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};
use rand::distr::{Alphanumeric, SampleString};

use crate::config::{Config, MemoryConfig};
use crate::ui::style as ui;

const POSTGRES_URL_ENV_VAR: &str = "ASTEREL_POSTGRES_URL";
const POSTGRES_SETUP_MODE_ENV_VAR: &str = "ASTEREL_POSTGRES_SETUP";
const POSTGRES_DOCKER_IMAGE: &str = "pgvector/pgvector:pg16";
const POSTGRES_CONTAINER_NAME: &str = "asterel-postgres";
const POSTGRES_HOST: &str = "127.0.0.1";
const POSTGRES_PORT: &str = "6543";
const POSTGRES_USER: &str = "asterel";
const POSTGRES_LEGACY_PASSWORD: &str = "asterel";
const POSTGRES_DB: &str = "asterel";
const POSTGRES_PASSWORD_FILE_NAME: &str = ".postgres_password";
const POSTGRES_NATIVE_DIR_NAME: &str = "postgres";
const POSTGRES_NATIVE_DATA_DIR_NAME: &str = "data";
const POSTGRES_NATIVE_LOG_FILE_NAME: &str = "server.log";
const POSTGRES_WAIT_ATTEMPTS: u32 = 40;
const POSTGRES_WAIT_DURATION: Duration = Duration::from_millis(500);
const POSTGRES_DEFAULT_DB: &str = "postgres";
const POSTGRES_PASSWORD_LENGTH: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PostgresSetupMode {
    Auto,
    Native,
    Docker,
}

pub(super) fn resolve_postgres_setup_mode(cli_mode: Option<&str>) -> Result<PostgresSetupMode> {
    if let Some(mode) = normalize_non_empty(cli_mode) {
        return parse_postgres_setup_mode(Some(&mode));
    }
    let env_mode = std::env::var(POSTGRES_SETUP_MODE_ENV_VAR).ok();
    parse_postgres_setup_mode(env_mode.as_deref())
}

fn parse_postgres_setup_mode(raw: Option<&str>) -> Result<PostgresSetupMode> {
    let normalized = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("auto")
        .to_ascii_lowercase();

    match normalized.as_str() {
        "auto" => Ok(PostgresSetupMode::Auto),
        "native" => Ok(PostgresSetupMode::Native),
        "docker" => Ok(PostgresSetupMode::Docker),
        _ => anyhow::bail!(
            "invalid postgres setup mode '{normalized}'. Use auto, native, or docker."
        ),
    }
}

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .map(ToOwned::to_owned)
}

fn resolve_postgres_url(memory_url: Option<&str>, env_url: Option<&str>) -> Option<String> {
    normalize_non_empty(memory_url).or_else(|| normalize_non_empty(env_url))
}

fn postgres_password_file_path() -> Result<std::path::PathBuf> {
    let home = crate::utils::dirs::asterel_home_dir()?;
    #[cfg(test)]
    {
        return Ok(home.join(format!(
            "{POSTGRES_PASSWORD_FILE_NAME}.test-{}",
            std::process::id()
        )));
    }
    #[cfg(not(test))]
    {
        Ok(home.join(POSTGRES_PASSWORD_FILE_NAME))
    }
}

fn generate_postgres_password() -> String {
    Alphanumeric.sample_string(&mut rand::rng(), POSTGRES_PASSWORD_LENGTH)
}

fn read_stored_postgres_password() -> Result<Option<String>> {
    let password_path = postgres_password_file_path()?;
    let Ok(contents) = std::fs::read_to_string(&password_path) else {
        return Ok(None);
    };

    Ok(normalize_non_empty(Some(contents.as_str())))
}

fn load_managed_postgres_password() -> Result<String> {
    Ok(read_stored_postgres_password()?.unwrap_or_else(|| POSTGRES_LEGACY_PASSWORD.to_string()))
}

fn write_managed_postgres_password(password: &str) -> Result<()> {
    let password_path = postgres_password_file_path()?;
    if let Some(parent) = password_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("create postgres password directory at {}", parent.display())
        })?;
    }

    std::fs::write(&password_path, format!("{password}\n")).with_context(|| {
        format!(
            "write managed postgres password file at {}",
            password_path.display()
        )
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&password_path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| {
                format!(
                    "set managed postgres password permissions at {}",
                    password_path.display()
                )
            })?;
    }

    Ok(())
}

fn generate_and_store_postgres_password() -> Result<String> {
    let password = generate_postgres_password();
    write_managed_postgres_password(&password)?;
    Ok(password)
}

fn postgres_connection_url(password: &str) -> String {
    format!("postgres://{POSTGRES_USER}:{password}@{POSTGRES_HOST}:{POSTGRES_PORT}/{POSTGRES_DB}")
}

fn is_managed_local_postgres_url(raw: &str) -> bool {
    let Ok(parsed) = url::Url::parse(raw) else {
        return false;
    };

    if !matches!(parsed.scheme(), "postgres" | "postgresql") {
        return false;
    }
    if parsed.host_str() != Some(POSTGRES_HOST) {
        return false;
    }
    if parsed.port() != Some(6543) {
        return false;
    }
    if parsed.username() != POSTGRES_USER {
        return false;
    }

    let Ok(configured_password) = load_managed_postgres_password() else {
        return false;
    };
    let password = parsed.password().unwrap_or_default();
    if password != configured_password && password != POSTGRES_LEGACY_PASSWORD {
        return false;
    }

    parsed.path().trim_start_matches('/') == POSTGRES_DB
}

fn apply_postgres_password<'a>(command: &'a mut Command, password: &str) -> &'a mut Command {
    command.env("PGPASSWORD", password)
}

fn native_postgres_is_ready() -> bool {
    let Ok(status) = Command::new("pg_isready")
        .args([
            "-h",
            POSTGRES_HOST,
            "-p",
            POSTGRES_PORT,
            "-U",
            POSTGRES_USER,
            "-d",
            POSTGRES_DEFAULT_DB,
        ])
        .status()
    else {
        return false;
    };

    status.success()
}

/// Best-effort runtime self-healing for local managed Postgres.
///
/// Intended for daemon startup after host reboot. This function only attempts
/// revival when config points to the managed local URL provisioned by onboarding.
/// External Postgres URLs are never modified or restarted.
///
/// # Errors
/// Returns an error when the managed local runtime was selected for auto-revive
/// but restart/wait/extension preparation failed.
pub(crate) fn try_revive_managed_local_postgres(config: &Config) -> Result<()> {
    if config.memory.backend != crate::config::MemoryBackend::Postgres {
        return Ok(());
    }

    let Some(postgres_url) = normalize_non_empty(config.memory.postgres_url.as_deref()) else {
        return Ok(());
    };

    if !is_managed_local_postgres_url(&postgres_url) {
        return Ok(());
    }

    if native_postgres_is_ready() {
        tracing::info!("managed local postgres already ready; skipping auto-revive");
        return Ok(());
    }

    let native_root = crate::utils::dirs::asterel_home_dir()?.join(POSTGRES_NATIVE_DIR_NAME);
    let data_dir = native_root.join(POSTGRES_NATIVE_DATA_DIR_NAME);
    let log_path = native_root.join(POSTGRES_NATIVE_LOG_FILE_NAME);

    if data_dir.join("PG_VERSION").exists() {
        let password = load_managed_postgres_password()?;
        start_native_postgres_cluster(&data_dir, &log_path)
            .context("start managed native postgres")?;
        wait_for_native_postgres_ready().context("wait for managed native postgres readiness")?;
        ensure_native_postgres_database(&password)
            .context("ensure managed native postgres database")?;
        ensure_native_postgres_extensions(&password)
            .context("ensure managed native postgres extensions")?;
        tracing::info!(path = %data_dir.display(), "auto-revived managed native postgres");
        return Ok(());
    }

    match check_docker_ready() {
        Ok(()) => match detect_postgres_container_state()? {
            PostgresDockerState::Missing => {
                tracing::info!("managed docker postgres container not found; skipping auto-revive");
            }
            PostgresDockerState::Running => {
                wait_for_postgres_ready().context("wait for managed docker postgres readiness")?;
                tracing::info!("managed docker postgres already running");
            }
            PostgresDockerState::Stopped(status) => {
                tracing::info!(%status, "starting managed docker postgres container");
                start_postgres_container().context("start managed docker postgres container")?;
                wait_for_postgres_ready().context("wait for managed docker postgres readiness")?;
                tracing::info!("auto-revived managed docker postgres");
            }
        },
        Err(reason) => {
            tracing::info!(%reason, "docker unavailable; skipping managed postgres auto-revive");
        }
    }

    Ok(())
}

/// Ensure Postgres memory backend has a usable connection URL.
///
/// Resolution order:
/// 1. Existing `memory.postgres_url`
/// 2. `ASTEREL_POSTGRES_URL`
/// 3. Auto-provision local Postgres (native or Docker)
///
/// Returns `true` if the memory config was updated.
pub(super) fn ensure_postgres_memory_ready(
    memory_config: &mut MemoryConfig,
    setup_mode: PostgresSetupMode,
) -> Result<bool> {
    if memory_config.backend != crate::config::MemoryBackend::Postgres {
        return Ok(false);
    }

    let env_url = std::env::var(POSTGRES_URL_ENV_VAR).ok();
    if let Some(url) =
        resolve_postgres_url(memory_config.postgres_url.as_deref(), env_url.as_deref())
    {
        let changed = memory_config.postgres_url.as_deref() != Some(url.as_str());
        if changed {
            memory_config.postgres_url = Some(url);
            println!(
                "  {} Using {} for Postgres memory backend.",
                ui::dim("›"),
                POSTGRES_URL_ENV_VAR
            );
        }
        return Ok(changed);
    }

    println!(
        "  {} Postgres backend selected. Auto-provisioning local instance ({})...",
        ui::dim("›"),
        match setup_mode {
            PostgresSetupMode::Auto => "auto",
            PostgresSetupMode::Native => "native",
            PostgresSetupMode::Docker => "docker",
        }
    );
    let url = match setup_mode {
        PostgresSetupMode::Auto => {
            let auto_result = match plan_auto_postgres_setup() {
                AutoPostgresPlan::TryNativeThenDocker => auto_setup_local_postgres_auto(),
                AutoPostgresPlan::UseDockerOnly { native_reason } => {
                    println!(
                        "  {} Native Postgres unavailable ({}). Using Docker directly.",
                        ui::dim("›"),
                        native_reason
                    );
                    auto_setup_local_postgres_docker()
                }
                AutoPostgresPlan::FallbackMemoryNone {
                    native_reason,
                    docker_reason,
                } => {
                    println!(
                        "  {} Local Postgres auto-setup is unavailable in this environment.",
                        ui::dim("›")
                    );
                    println!("    • native: {native_reason}");
                    println!("    • docker: {docker_reason}");
                    fallback_to_memory_none(memory_config);
                    return Ok(true);
                }
            };
            match auto_result {
                Ok(url) => url,
                Err(error) => {
                    println!(
                        "  {} Postgres auto-setup failed during startup: {}",
                        ui::dim("›"),
                        error
                    );
                    fallback_to_memory_none(memory_config);
                    return Ok(true);
                }
            }
        }
        PostgresSetupMode::Native => auto_setup_local_postgres_native()?,
        PostgresSetupMode::Docker => auto_setup_local_postgres_docker()?,
    };
    memory_config.postgres_url = Some(url.clone());
    println!(
        "  {} Postgres ready at {}",
        ui::success("✓"),
        ui::value(&url)
    );
    Ok(true)
}

fn fallback_to_memory_none(memory_config: &mut MemoryConfig) {
    println!(
        "  {} Falling back to memory backend '{}'.",
        ui::success("✓"),
        ui::value("none")
    );
    println!("    Next options:");
    println!("    1) continue now (no DB): asterel onboard --memory none");
    println!("    2) use external DB: export {POSTGRES_URL_ENV_VAR}='postgres://...'");
    println!("    3) install postgres tools or start Docker, then rerun onboarding");
    memory_config.backend = crate::config::MemoryBackend::None;
    memory_config.auto_save = false;
    memory_config.postgres_url = None;
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PostgresDockerState {
    Missing,
    Running,
    Stopped(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AutoPostgresPlan {
    TryNativeThenDocker,
    UseDockerOnly {
        native_reason: String,
    },
    FallbackMemoryNone {
        native_reason: String,
        docker_reason: String,
    },
}

fn plan_auto_postgres_setup() -> AutoPostgresPlan {
    let native = check_native_postgres_prerequisites();
    let docker = check_docker_ready();
    decide_auto_postgres_plan(native, docker)
}

fn decide_auto_postgres_plan(
    native: std::result::Result<(), String>,
    docker: std::result::Result<(), String>,
) -> AutoPostgresPlan {
    match (native, docker) {
        (Ok(()), _) => AutoPostgresPlan::TryNativeThenDocker,
        (Err(native_reason), Ok(())) => AutoPostgresPlan::UseDockerOnly { native_reason },
        (Err(native_reason), Err(docker_reason)) => AutoPostgresPlan::FallbackMemoryNone {
            native_reason,
            docker_reason,
        },
    }
}

fn check_native_postgres_prerequisites() -> std::result::Result<(), String> {
    for command in ["initdb", "pg_ctl", "createdb", "psql", "pg_isready"] {
        check_command_available(command)?;
    }
    Ok(())
}

fn check_docker_ready() -> std::result::Result<(), String> {
    let output = Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .output()
        .map_err(|error| format!("docker not installed or not in PATH: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("docker daemon unavailable: {}", stderr.trim()))
}

fn check_command_available(command: &str) -> std::result::Result<(), String> {
    let output = Command::new(command)
        .arg("--version")
        .output()
        .map_err(|error| format!("{command} not found: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!("{command} unavailable: {}", stderr.trim()))
}

fn auto_setup_local_postgres_auto() -> Result<String> {
    if cfg!(unix) {
        match auto_setup_local_postgres_native() {
            Ok(url) => return Ok(url),
            Err(native_err) => {
                let native_err_message = native_err.to_string();
                println!(
                    "  {} Native Postgres setup failed ({}). Falling back to Docker.",
                    ui::dim("›"),
                    native_err_message
                );
                return auto_setup_local_postgres_docker().with_context(|| {
                    format!(
                        "native setup failed ({native_err_message}); docker fallback also failed"
                    )
                });
            }
        }
    }

    auto_setup_local_postgres_docker()
}

fn auto_setup_local_postgres_docker() -> Result<String> {
    ensure_docker_available()?;
    let password = match detect_postgres_container_state()? {
        PostgresDockerState::Missing => {
            let password = generate_and_store_postgres_password()?;
            create_postgres_container(&password)?;
            password
        }
        PostgresDockerState::Running => {
            println!(
                "  {} Reusing running Docker container '{}'.",
                ui::dim("›"),
                POSTGRES_CONTAINER_NAME
            );
            load_managed_postgres_password()?
        }
        PostgresDockerState::Stopped(status) => {
            println!(
                "  {} Starting existing Docker container '{}' (status: {}).",
                ui::dim("›"),
                POSTGRES_CONTAINER_NAME,
                status
            );
            start_postgres_container()?;
            load_managed_postgres_password()?
        }
    };
    wait_for_postgres_ready()?;
    Ok(postgres_connection_url(&password))
}

fn auto_setup_local_postgres_native() -> Result<String> {
    if !cfg!(unix) {
        anyhow::bail!(
            "native postgres setup is only supported on Unix. Use --postgres-setup docker"
        );
    }

    ensure_native_postgres_available()?;

    let native_root = crate::utils::dirs::asterel_home_dir()?.join(POSTGRES_NATIVE_DIR_NAME);
    let data_dir = native_root.join(POSTGRES_NATIVE_DATA_DIR_NAME);
    let log_path = native_root.join(POSTGRES_NATIVE_LOG_FILE_NAME);

    let password = if data_dir.join("PG_VERSION").exists() {
        load_managed_postgres_password()?
    } else {
        let password = generate_and_store_postgres_password()?;
        initialize_native_postgres_cluster(&native_root, &data_dir)?;
        password
    };

    start_native_postgres_cluster(&data_dir, &log_path)?;
    wait_for_native_postgres_ready()?;
    ensure_native_postgres_database(&password)?;
    ensure_native_postgres_extensions(&password)?;

    Ok(postgres_connection_url(&password))
}

fn ensure_native_postgres_available() -> Result<()> {
    for command in ["initdb", "pg_ctl", "createdb", "psql", "pg_isready"] {
        check_command_available(command).map_err(|reason| {
            anyhow::anyhow!("{reason}. Install PostgreSQL binaries or use --postgres-setup docker")
        })?;
    }
    Ok(())
}

fn initialize_native_postgres_cluster(native_root: &Path, data_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(native_root).with_context(|| {
        format!(
            "create native postgres directory at {}",
            native_root.display()
        )
    })?;

    let password_path = postgres_password_file_path()?;

    let output = Command::new("initdb")
        .args([
            "-D",
            &data_dir.display().to_string(),
            "-U",
            POSTGRES_USER,
            "--auth-local=trust",
            "--auth-host=scram-sha-256",
            "--pwfile",
            &password_path.display().to_string(),
        ])
        .output()
        .context("initialize native postgres cluster via initdb")?;

    if output.status.success() {
        println!(
            "  {} Initialized native Postgres cluster at {}.",
            ui::dim("›"),
            data_dir.display()
        );
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!("initdb failed: {}", stderr.trim());
}

fn start_native_postgres_cluster(data_dir: &Path, log_path: &Path) -> Result<()> {
    let status_output = Command::new("pg_ctl")
        .args(["-D", &data_dir.display().to_string(), "status"])
        .output()
        .context("check native postgres status")?;
    if status_output.status.success() {
        println!(
            "  {} Reusing running native Postgres cluster at {}.",
            ui::dim("›"),
            data_dir.display()
        );
        return Ok(());
    }

    let server_opts = format!(
        "-h {POSTGRES_HOST} -p {POSTGRES_PORT} -k {}",
        data_dir.display()
    );
    let output = Command::new("pg_ctl")
        .args([
            "-D",
            &data_dir.display().to_string(),
            "-l",
            &log_path.display().to_string(),
            "-o",
            &server_opts,
            "-w",
            "start",
        ])
        .output()
        .context("start native postgres cluster")?;

    if output.status.success() {
        println!(
            "  {} Started native Postgres cluster at {}.",
            ui::dim("›"),
            data_dir.display()
        );
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!("pg_ctl start failed: {}", stderr.trim());
}

fn wait_for_native_postgres_ready() -> Result<()> {
    for _ in 0..POSTGRES_WAIT_ATTEMPTS {
        let status = Command::new("pg_isready")
            .args([
                "-h",
                POSTGRES_HOST,
                "-p",
                POSTGRES_PORT,
                "-U",
                POSTGRES_USER,
                "-d",
                POSTGRES_DEFAULT_DB,
            ])
            .status()
            .context("check native postgres readiness with pg_isready")?;

        if status.success() {
            return Ok(());
        }
        std::thread::sleep(POSTGRES_WAIT_DURATION);
    }

    anyhow::bail!(
        "timed out waiting for native postgres on {POSTGRES_HOST}:{POSTGRES_PORT}; \
         check logs or set {POSTGRES_URL_ENV_VAR} manually"
    );
}

fn ensure_native_postgres_database(password: &str) -> Result<()> {
    if native_postgres_database_exists(password)? {
        return Ok(());
    }

    let mut command = Command::new("createdb");
    let output = apply_postgres_password(&mut command, password)
        .args([
            "-h",
            POSTGRES_HOST,
            "-p",
            POSTGRES_PORT,
            "-U",
            POSTGRES_USER,
            POSTGRES_DB,
        ])
        .output()
        .context("create native postgres database")?;

    if output.status.success() {
        return Ok(());
    }

    if native_postgres_database_exists(password)? {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!("createdb failed: {}", stderr.trim());
}

fn native_postgres_database_exists(password: &str) -> Result<bool> {
    let mut command = Command::new("psql");
    let output = apply_postgres_password(&mut command, password)
        .args([
            "-h",
            POSTGRES_HOST,
            "-p",
            POSTGRES_PORT,
            "-U",
            POSTGRES_USER,
            "-d",
            POSTGRES_DEFAULT_DB,
            "-tAc",
            &format!("SELECT 1 FROM pg_database WHERE datname='{POSTGRES_DB}'"),
        ])
        .output()
        .context("check native postgres database existence")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("psql database existence check failed: {}", stderr.trim());
    }

    Ok(psql_stdout_contains_exists_marker(&output.stdout))
}

fn psql_stdout_contains_exists_marker(stdout: &[u8]) -> bool {
    String::from_utf8_lossy(stdout)
        .lines()
        .any(|line| line.trim() == "1")
}

fn ensure_native_postgres_extensions(password: &str) -> Result<()> {
    let mut command = Command::new("psql");
    let output = apply_postgres_password(&mut command, password)
        .args([
            "-h",
            POSTGRES_HOST,
            "-p",
            POSTGRES_PORT,
            "-U",
            POSTGRES_USER,
            "-d",
            POSTGRES_DB,
            "-v",
            "ON_ERROR_STOP=1",
            "-c",
            "CREATE EXTENSION IF NOT EXISTS vector;",
            "-c",
            "CREATE EXTENSION IF NOT EXISTS pg_trgm;",
        ])
        .output()
        .context("prepare native postgres extensions")?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!(
        "psql extension setup failed: {}. Install pgvector for native mode or use --postgres-setup docker",
        stderr.trim()
    );
}

fn ensure_docker_available() -> Result<()> {
    let output = Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .output()
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to run docker: {error}. Install Docker Desktop/Engine or set {POSTGRES_URL_ENV_VAR}"
            )
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!(
        "docker is not ready: {}. Start Docker or set {}",
        stderr.trim(),
        POSTGRES_URL_ENV_VAR
    );
}

fn detect_postgres_container_state() -> Result<PostgresDockerState> {
    let output = Command::new("docker")
        .args([
            "inspect",
            "--format",
            "{{.State.Status}}",
            POSTGRES_CONTAINER_NAME,
        ])
        .output()
        .context("inspect postgres docker container state")?;

    if output.status.success() {
        let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if status == "running" {
            Ok(PostgresDockerState::Running)
        } else {
            Ok(PostgresDockerState::Stopped(status))
        }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr_lower = stderr.to_ascii_lowercase();
        if stderr_lower.contains("no such object") || stderr_lower.contains("no such container") {
            Ok(PostgresDockerState::Missing)
        } else {
            anyhow::bail!("docker inspect failed: {}", stderr.trim());
        }
    }
}

fn create_postgres_container(password: &str) -> Result<()> {
    let port_mapping = format!("{POSTGRES_HOST}:{POSTGRES_PORT}:5432");
    let output = Command::new("docker")
        .arg("run")
        .arg("-d")
        .arg("--name")
        .arg(POSTGRES_CONTAINER_NAME)
        .arg("-e")
        .arg(format!("POSTGRES_USER={POSTGRES_USER}"))
        .arg("-e")
        .arg(format!("POSTGRES_PASSWORD={password}"))
        .arg("-e")
        .arg(format!("POSTGRES_DB={POSTGRES_DB}"))
        .arg("-p")
        .arg(port_mapping)
        .arg(POSTGRES_DOCKER_IMAGE)
        .output()
        .context("create postgres docker container")?;

    if output.status.success() {
        println!(
            "  {} Created Docker container '{}'.",
            ui::dim("›"),
            POSTGRES_CONTAINER_NAME
        );
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("docker run failed: {}", stderr.trim());
    }
}

fn start_postgres_container() -> Result<()> {
    let output = Command::new("docker")
        .args(["start", POSTGRES_CONTAINER_NAME])
        .output()
        .context("start postgres docker container")?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("docker start failed: {}", stderr.trim());
    }
}

fn wait_for_postgres_ready() -> Result<()> {
    for _ in 0..POSTGRES_WAIT_ATTEMPTS {
        let status = Command::new("docker")
            .args([
                "exec",
                POSTGRES_CONTAINER_NAME,
                "pg_isready",
                "-U",
                POSTGRES_USER,
                "-d",
                POSTGRES_DB,
            ])
            .status()
            .context("run pg_isready in postgres container")?;
        if status.success() {
            return Ok(());
        }
        std::thread::sleep(POSTGRES_WAIT_DURATION);
    }

    anyhow::bail!(
        "timed out waiting for postgres container '{POSTGRES_CONTAINER_NAME}'; \
         check docker logs and set {POSTGRES_URL_ENV_VAR} manually"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{LazyLock, Mutex, MutexGuard};

    static POSTGRES_PASSWORD_FILE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    struct PasswordFileGuard {
        _lock: MutexGuard<'static, ()>,
        path: std::path::PathBuf,
        original: Option<String>,
    }

    impl PasswordFileGuard {
        fn capture() -> Self {
            let lock = POSTGRES_PASSWORD_FILE_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let path = postgres_password_file_path().unwrap();
            let original = std::fs::read_to_string(&path).ok();
            Self {
                _lock: lock,
                path,
                original,
            }
        }
    }

    impl Drop for PasswordFileGuard {
        fn drop(&mut self) {
            if let Some(contents) = &self.original {
                if let Some(parent) = self.path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&self.path, contents);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;

                    let _ = std::fs::set_permissions(
                        &self.path,
                        std::fs::Permissions::from_mode(0o600),
                    );
                }
            } else {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }

    #[test]
    fn parse_postgres_setup_mode_defaults_to_auto() {
        assert_eq!(
            parse_postgres_setup_mode(None).unwrap(),
            PostgresSetupMode::Auto
        );
        assert_eq!(
            parse_postgres_setup_mode(Some("")).unwrap(),
            PostgresSetupMode::Auto
        );
    }

    #[test]
    fn parse_postgres_setup_mode_accepts_native_and_docker() {
        assert_eq!(
            parse_postgres_setup_mode(Some("native")).unwrap(),
            PostgresSetupMode::Native
        );
        assert_eq!(
            parse_postgres_setup_mode(Some("docker")).unwrap(),
            PostgresSetupMode::Docker
        );
    }

    #[test]
    fn parse_postgres_setup_mode_rejects_unknown_value() {
        let err = parse_postgres_setup_mode(Some("invalid-mode")).unwrap_err();
        assert!(
            err.to_string()
                .contains("invalid postgres setup mode 'invalid-mode'")
        );
    }

    #[test]
    fn auto_postgres_plan_prefers_native_when_native_ready() {
        let plan = decide_auto_postgres_plan(Ok(()), Err("docker unavailable".to_string()));
        assert_eq!(plan, AutoPostgresPlan::TryNativeThenDocker);
    }

    #[test]
    fn auto_postgres_plan_uses_docker_when_native_missing() {
        let plan = decide_auto_postgres_plan(Err("initdb not found".to_string()), Ok(()));
        assert_eq!(
            plan,
            AutoPostgresPlan::UseDockerOnly {
                native_reason: "initdb not found".to_string()
            }
        );
    }

    #[test]
    fn auto_postgres_plan_falls_back_to_none_when_no_runtime_available() {
        let plan = decide_auto_postgres_plan(
            Err("initdb not found".to_string()),
            Err("docker.sock missing".to_string()),
        );
        assert_eq!(
            plan,
            AutoPostgresPlan::FallbackMemoryNone {
                native_reason: "initdb not found".to_string(),
                docker_reason: "docker.sock missing".to_string()
            }
        );
    }

    #[test]
    fn resolve_postgres_url_prefers_config_value() {
        let resolved = resolve_postgres_url(
            Some("postgres://config-user:config-pass@localhost:5432/db"),
            Some("postgres://env-user:env-pass@localhost:5432/db"),
        );
        assert_eq!(
            resolved.as_deref(),
            Some("postgres://config-user:config-pass@localhost:5432/db")
        );
    }

    #[test]
    fn resolve_postgres_url_uses_env_when_config_missing() {
        let resolved =
            resolve_postgres_url(None, Some("postgres://env-user:env-pass@localhost:5432/db"));
        assert_eq!(
            resolved.as_deref(),
            Some("postgres://env-user:env-pass@localhost:5432/db")
        );
    }

    #[test]
    fn postgres_connection_url_uses_expected_local_defaults() {
        let password = "generated-secret";
        assert_eq!(
            postgres_connection_url(password),
            "postgres://asterel:generated-secret@127.0.0.1:6543/asterel".to_string()
        );
    }

    #[test]
    fn psql_stdout_contains_exists_marker_detects_existing_database() {
        assert!(psql_stdout_contains_exists_marker(b"1\n"));
        assert!(psql_stdout_contains_exists_marker(b" \n 1 \n"));
    }

    #[test]
    fn psql_stdout_contains_exists_marker_handles_absent_database() {
        assert!(!psql_stdout_contains_exists_marker(b""));
        assert!(!psql_stdout_contains_exists_marker(b"0\n"));
        assert!(!psql_stdout_contains_exists_marker(b" \n"));
    }

    #[test]
    fn managed_local_postgres_url_detection_accepts_default_local_url() {
        let _guard = PasswordFileGuard::capture();
        write_managed_postgres_password("generated-secret").unwrap();
        assert!(is_managed_local_postgres_url(
            "postgres://asterel:generated-secret@127.0.0.1:6543/asterel"
        ));
    }

    #[test]
    fn managed_local_postgres_url_detection_accepts_legacy_local_url_without_password_file() {
        let _guard = PasswordFileGuard::capture();
        let password_path = postgres_password_file_path().unwrap();
        let _ = std::fs::remove_file(password_path);

        assert!(is_managed_local_postgres_url(
            "postgres://asterel:asterel@127.0.0.1:6543/asterel"
        ));
    }

    #[test]
    fn managed_local_postgres_url_detection_rejects_external_urls() {
        assert!(!is_managed_local_postgres_url(
            "postgres://asterel:asterel@db.example.com:6543/asterel"
        ));
        assert!(!is_managed_local_postgres_url(
            "postgres://asterel:asterel@127.0.0.1:5432/asterel"
        ));
        assert!(!is_managed_local_postgres_url(
            "postgres://external:secret@127.0.0.1:6543/asterel"
        ));
    }

    #[test]
    fn revive_managed_local_postgres_noops_for_non_postgres_backend() {
        let mut config = Config::default();
        config.memory.backend = crate::config::MemoryBackend::Markdown;
        config.memory.postgres_url =
            Some("postgres://asterel:asterel@127.0.0.1:6543/asterel".to_string());
        assert!(try_revive_managed_local_postgres(&config).is_ok());
    }

    #[test]
    fn revive_managed_local_postgres_noops_for_external_postgres_url() {
        let mut config = Config::default();
        config.memory.backend = crate::config::MemoryBackend::Postgres;
        config.memory.postgres_url =
            Some("postgres://user:pass@db.example.com:5432/asterel".to_string());
        assert!(try_revive_managed_local_postgres(&config).is_ok());
    }

    #[test]
    fn fallback_to_memory_none_disables_auto_save_and_clears_postgres_url() {
        let mut memory = MemoryConfig {
            backend: crate::config::MemoryBackend::Postgres,
            auto_save: true,
            postgres_url: Some("postgres://asterel:asterel@127.0.0.1:6543/asterel".to_string()),
            ..MemoryConfig::default()
        };

        fallback_to_memory_none(&mut memory);

        assert_eq!(memory.backend, crate::config::MemoryBackend::None);
        assert!(!memory.auto_save);
        assert!(memory.postgres_url.is_none());
    }

    #[test]
    fn load_managed_postgres_password_falls_back_to_legacy_password() {
        let _guard = PasswordFileGuard::capture();
        let password_path = postgres_password_file_path().unwrap();
        let _ = std::fs::remove_file(password_path);

        assert_eq!(
            load_managed_postgres_password().unwrap(),
            POSTGRES_LEGACY_PASSWORD
        );
    }

    #[test]
    fn load_managed_postgres_password_reads_stored_password() {
        let _guard = PasswordFileGuard::capture();
        write_managed_postgres_password("stored-secret").unwrap();

        assert_eq!(load_managed_postgres_password().unwrap(), "stored-secret");
    }
}
