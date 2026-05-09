use anyhow::Result;

use asterel::CronCommands;
use asterel::config::Config;
use asterel::platform::cron::{add_job, list_jobs, remove_job, validate_main_runtime_cron_command};
use asterel::runtime::services::{RuntimeCapabilityStatus, load_runtime_operational_snapshot};
use asterel::ui::style as ui;

pub fn handle_cron_command(command: CronCommands, config: &Config) -> Result<()> {
    let cron = load_runtime_operational_snapshot(config).cron;
    match cron.status {
        RuntimeCapabilityStatus::Unsupported => {
            if matches!(command, CronCommands::List) {
                println!();
                println!("  {}", ui::section("Scheduled Jobs"));
                println!(
                    "{}",
                    ui::note_line(format!(
                        "Cron is unavailable in this setup: {}",
                        cron.reason
                            .as_deref()
                            .unwrap_or("requires PostgreSQL-backed scheduler state")
                    ))
                );
                return Ok(());
            }
            anyhow::bail!(
                "Cron is unavailable in this setup: {}",
                cron.reason
                    .as_deref()
                    .unwrap_or("requires PostgreSQL-backed scheduler state")
            );
        }
        RuntimeCapabilityStatus::Degraded => {
            anyhow::bail!(
                "Cron configuration is incomplete: {}",
                cron.reason
                    .as_deref()
                    .unwrap_or("requires PostgreSQL-backed scheduler state")
            );
        }
        RuntimeCapabilityStatus::Supported => {}
    }

    match command {
        CronCommands::List => render_cron_list(config),
        CronCommands::Add {
            expression,
            command,
        } => add_cron_job(config, &expression, &command),
        CronCommands::Remove { id } => remove_cron_job(config, &id),
    }
}

fn render_cron_list(config: &Config) -> Result<()> {
    let jobs = list_jobs(config)?;
    if jobs.is_empty() {
        println!();
        println!("  {}", ui::section("Scheduled Jobs"));
        println!("{}", ui::note_line("No scheduled tasks yet."));
        println!("{}", ui::note_line("Create one with:"));
        println!(
            "{}",
            ui::command_line("asterel cron add '0 9 * * *' 'agent -m \"Good morning!\"'")
        );
        return Ok(());
    }

    println!();
    println!("  {}", ui::section("Scheduled Jobs"));
    println!("{}", ui::field_line("Count", jobs.len()));
    println!();
    for job in jobs {
        let last_run: String = job
            .last_run
            .map_or_else(|| "never".into(), |d| d.to_rfc3339());
        let last_status = job.last_status.unwrap_or_else(|| "n/a".into());
        println!(
            "  {} {} {}",
            if job.enabled {
                ui::ok_badge("enabled")
            } else {
                ui::muted_badge("disabled")
            },
            ui::header(&job.id),
            ui::dim(job.expression.as_str())
        );
        println!("{}", ui::field_line("Next run", job.next_run.to_rfc3339()));
        println!("{}", ui::field_line("Last run", last_run));
        println!("{}", ui::field_line("Last status", last_status));
        println!("{}", ui::field_line("Command", job.command));
        println!();
    }
    Ok(())
}

fn add_cron_job(config: &Config, expression: &str, command: &str) -> Result<()> {
    validate_main_runtime_cron_command(command).map_err(anyhow::Error::new)?;
    let job = add_job(config, expression, command)?;
    println!();
    println!("  {}", ui::section("Add Scheduled Job"));
    println!("{}", ui::field_line("Result", ui::ok_badge("added")));
    println!("{}", ui::field_line("Job", job.id));
    println!("{}", ui::field_line("Expression", job.expression));
    println!("{}", ui::field_line("Next run", job.next_run.to_rfc3339()));
    println!("{}", ui::field_line("Command", job.command));
    Ok(())
}

fn remove_cron_job(config: &Config, id: &str) -> Result<()> {
    remove_job(config, id)?;
    println!();
    println!("  {}", ui::section("Remove Scheduled Job"));
    println!("{}", ui::field_line("Job", id));
    println!("{}", ui::field_line("Result", ui::ok_badge("removed")));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::handle_cron_command;
    use asterel::CronCommands;
    use asterel::config::{Config, MemoryBackend};
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        }
    }

    #[test]
    fn cron_list_reports_unavailable_for_markdown_setup_without_error() {
        let tmp = TempDir::new().expect("tempdir");
        let mut config = test_config(&tmp);
        config.memory.backend = MemoryBackend::Markdown;

        handle_cron_command(CronCommands::List, &config)
            .expect("unsupported cron list should render guidance instead of failing");
    }

    #[test]
    fn cron_add_rejects_unsupported_runtime_with_explicit_error() {
        let tmp = TempDir::new().expect("tempdir");
        let mut config = test_config(&tmp);
        config.memory.backend = MemoryBackend::Markdown;

        let error = handle_cron_command(
            CronCommands::Add {
                expression: "0 9 * * *".to_string(),
                command: "agent -m \"hello\"".to_string(),
            },
            &config,
        )
        .expect_err("unsupported cron add should fail explicitly");

        assert!(
            error
                .to_string()
                .contains("Cron is unavailable in this setup")
        );
    }
}
