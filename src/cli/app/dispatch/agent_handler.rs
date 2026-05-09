use std::sync::Arc;

use anyhow::Result;
use asterel::config::Config;

pub(super) async fn dispatch_agent(
    config: Arc<Config>,
    message: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    temperature: f64,
) -> Result<()> {
    let approval_broker = message.is_none().then(|| {
        Arc::new(asterel::security::approval::CliApprovalBroker::default_timeout())
            as Arc<dyn asterel::security::ApprovalBroker>
    });

    let model_selection = config.resolve_model(provider.as_deref(), model.as_deref());
    let security = asterel::security::SecurityPolicy::from_config_runtime(
        &config.autonomy,
        &config.runtime,
        &config.workspace_dir,
    );
    let system_prompt = crate::app::prompt::build_agent_system_prompt(
        &config,
        model_selection.model.as_str(),
        &security,
    );

    let (cli_input_rx, cli_listen_handle) = if message.is_none() {
        let (cli_tx, cli_rx) = tokio::sync::mpsc::channel(32);
        let handle = tokio::spawn(async move {
            if let Err(error) = asterel::transport::channels::cli::listen_for_messages(cli_tx).await
            {
                tracing::error!(%error, "CLI input listener failed");
            }
        });
        (Some(cli_rx), Some(handle))
    } else {
        (None, None)
    };

    let run_result = asterel::runtime::services::run_agent_surface(
        Arc::clone(&config),
        asterel::core::agent::RunRequest {
            message,
            provider_override: provider,
            model_override: model,
            temperature,
            system_prompt,
            stream_sink: None,
            interactive_input_tx: None,
            approval_broker,
            execution_audit_sink: None,
            cli_input_rx,
        },
    )
    .await;
    if let Some(handle) = cli_listen_handle {
        handle.abort();
    }
    run_result
}
