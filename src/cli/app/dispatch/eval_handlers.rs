use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use asterel::cli::commands::EvalCommands;
use asterel::config::Config;

pub(super) async fn dispatch_eval(config: &Config, eval_command: EvalCommands) -> Result<()> {
    match eval_command {
        EvalCommands::Baseline {
            seed,
            evidence_slug,
        } => dispatch_eval_baseline(config, seed, evidence_slug.as_deref()),
        EvalCommands::Replay {
            input,
            suite,
            evidence_slug,
        } => dispatch_eval_replay(config, &input, &suite, evidence_slug.as_deref()),
        EvalCommands::MemoryBench {
            config: bench_config,
            evidence_slug,
        } => dispatch_eval_memory_bench(config, &bench_config, evidence_slug.as_deref()).await,
        EvalCommands::Harness {
            fixtures,
            model_backed,
            provider,
            model,
            temperature,
            output,
            evidence_slug,
        } => {
            dispatch_eval_harness(
                config,
                &fixtures,
                model_backed,
                provider.as_deref(),
                model.as_deref(),
                &temperature,
                output.as_deref(),
                evidence_slug.as_deref(),
            )
            .await
        }
    }
}

fn dispatch_eval_baseline(config: &Config, seed: u64, evidence_slug: Option<&str>) -> Result<()> {
    println!(
        "note: `eval` emits synthetic baseline metrics; use scripts/release/human_like_release_gate.sh for behavioral release gating"
    );
    let suites = asterel::core::eval::default_baseline_suites();
    let harness = asterel::core::eval::EvalHarness::new(seed);
    let report = harness.run(&suites);

    if let Some(slug) = evidence_slug {
        let files =
            asterel::core::eval::write_evidence_files(&config.workspace_dir, &report, slug, None)?;
        println!("wrote evidence files:");
        for path in files {
            println!("- {}", path.display());
        }
    }

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn dispatch_eval_harness(
    config: &Config,
    fixtures: &str,
    model_backed: bool,
    provider: Option<&str>,
    model: Option<&str>,
    temperature: &str,
    output: Option<&str>,
    evidence_slug: Option<&str>,
) -> Result<()> {
    let fixtures_path = std::path::Path::new(fixtures);
    let report = if model_backed {
        let temperature = temperature
            .parse::<f64>()
            .context("parse harness ablation temperature")?;
        let provider_selector = provider_selector_for_harness(config, provider, model);
        asterel::core::eval::run_model_backed_harness_ablation(
            asterel::core::eval::ModelBackedHarnessAblationRequest {
                config,
                fixtures_path,
                provider_override: provider,
                model_override: model,
                provider_selector_override: Some(provider_selector.as_str()),
                temperature,
            },
        )
        .await?
    } else {
        asterel::core::eval::run_harness_ablation(fixtures_path)?
    };

    println!(
        "Harness ablation: {} fixtures | method={} | off violations={} | on violations={}",
        report.off.fixtures,
        report.methodology,
        report.off.total_constraint_violations,
        report.on.total_constraint_violations
    );

    if let Some(output) = output {
        let output_path = std::path::Path::new(output);
        if let Some(parent) = output_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).context("create harness ablation output directory")?;
        }
        std::fs::write(output_path, serde_json::to_string_pretty(&report)?)
            .context("write harness ablation output")?;
        println!("wrote harness ablation report: {}", output_path.display());
    }

    if let Some(slug) = evidence_slug {
        let files = asterel::core::eval::write_harness_ablation_evidence(
            &config.workspace_dir,
            &report,
            slug,
        )?;
        println!("wrote harness ablation evidence files:");
        for path in files {
            println!("- {}", path.display());
        }
    }

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn dispatch_eval_replay(
    config: &Config,
    input: &str,
    suite: &str,
    evidence_slug: Option<&str>,
) -> Result<()> {
    let input_path = std::path::Path::new(input);
    let report = asterel::core::eval::run_replay(input_path, suite)?;

    if let Some(slug) = evidence_slug {
        let files =
            asterel::core::eval::write_replay_evidence_files(&config.workspace_dir, &report, slug)?;
        println!("wrote replay evidence files:");
        for path in files {
            println!("- {}", path.display());
        }
    }

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn provider_selector_for_harness(
    config: &Config,
    provider: Option<&str>,
    model: Option<&str>,
) -> String {
    let model_selection = config.resolve_model(provider, model);
    asterel::runtime::services::provider_selector_with_api_base(
        &model_selection.provider,
        model_selection.api_base.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use asterel::config::{Config, ModelListEntry};

    use super::{memory_bench_evidence_dir, provider_selector_for_harness};

    #[test]
    fn harness_provider_selector_preserves_model_list_api_base() {
        let config = Config {
            default_model: Some("proxy-model".to_string()),
            model_list: vec![ModelListEntry {
                model_name: "proxy-model".to_string(),
                model: "openai/gpt-4.1-mini".to_string(),
                api_key: None,
                api_base: Some("https://proxy.example/v1".to_string()),
            }],
            ..Config::default()
        };

        assert_eq!(
            provider_selector_for_harness(&config, None, None),
            "custom:https://proxy.example/v1"
        );
    }

    #[test]
    fn memory_bench_evidence_slug_is_sanitized() {
        let temp = tempfile::TempDir::new().unwrap();
        let config = Config {
            workspace_dir: temp.path().to_path_buf(),
            ..Config::default()
        };

        let evidence_dir = memory_bench_evidence_dir(&config, "../release,=cmd");

        assert_eq!(
            evidence_dir,
            temp.path().join("evidence").join("release-cmd")
        );
    }
}

fn memory_bench_evidence_dir(config: &Config, slug: &str) -> PathBuf {
    let slug = asterel::core::eval::memory_bench::sanitize_memory_bench_evidence_slug(slug);
    config.workspace_dir.join("evidence").join(slug)
}

async fn dispatch_eval_memory_bench(
    config: &Config,
    bench_config_path: &str,
    evidence_slug: Option<&str>,
) -> Result<()> {
    let bench_json =
        std::fs::read_to_string(bench_config_path).context("read memory bench config")?;
    let bench_config: asterel::core::eval::memory_bench::MemoryBenchConfig =
        serde_json::from_str(&bench_json).context("parse memory bench config")?;

    println!(
        "Memory bench: {} facts, {} probes",
        bench_config.planted_facts.len(),
        bench_config.recall_probes.len(),
    );

    let bench_dir = config.workspace_dir.join("eval").join("memory-bench");
    std::fs::create_dir_all(&bench_dir).context("create memory bench directory")?;
    let memory = Arc::new(asterel::core::memory::MarkdownMemory::new(&bench_dir));

    let report =
        asterel::core::eval::memory_bench::run_memory_bench(memory.as_ref(), &bench_config)
            .await
            .context("run memory bench")?;

    println!(
        "Fact Recall Rate (FRR): {:.1}%",
        report.fact_recall_rate * 100.0
    );
    println!("Average Recall Rank: {:.1}", report.avg_recall_rank);
    println!("Average Recall Latency: {}ms", report.avg_recall_latency_ms);

    if let Some(slug) = evidence_slug {
        let evidence_dir = memory_bench_evidence_dir(config, slug);
        std::fs::create_dir_all(&evidence_dir).context("create evidence directory")?;
        let evidence_path = evidence_dir.join("memory-bench.json");
        let report_json =
            serde_json::to_string_pretty(&report).context("serialize bench report")?;
        std::fs::write(&evidence_path, report_json).context("write bench evidence")?;
        println!("wrote evidence: {}", evidence_path.display());
    }

    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
