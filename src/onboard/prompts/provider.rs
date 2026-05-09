//! Interactive CLI prompts for LLM provider and API key setup.
//!
//! Guides the user through provider selection, API key entry,
//! optional base-URL override, and OAuth browser authentication.

use anyhow::Result;

use super::super::domain::{
    ProviderAuthMethod, model_choices_for_provider, oauth_login_provider, provider_after_oauth,
    provider_api_key_url, provider_auth_method, provider_choice_for_selection,
    provider_choices_for_tier, provider_env_var, validate_base_url,
};
use super::super::view::print_bullet;
use crate::onboard::api_verify::VerifyResult;
use crate::runtime::services::{
    DiscoveredModel, DiscoveredModelCapabilityHints, ProviderDiscoveryRequest,
    resolve_provider_discovery,
};
use crate::ui::style as ui;

fn gemini_vertex_project_env() -> Option<String> {
    [
        "VERTEX_AI_PROJECT",
        "GOOGLE_CLOUD_PROJECT",
        "GCLOUD_PROJECT",
    ]
    .iter()
    .find_map(|key| std::env::var(key).ok())
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty())
}

fn gemini_vertex_location_env() -> Option<String> {
    [
        "VERTEX_AI_LOCATION",
        "GOOGLE_CLOUD_LOCATION",
        "GOOGLE_CLOUD_REGION",
    ]
    .iter()
    .find_map(|key| std::env::var(key).ok())
    .map(|value| value.trim().to_string())
    .filter(|value| !value.is_empty())
}

fn gemini_vertex_default_location() -> String {
    gemini_vertex_location_env().unwrap_or_else(|| "global".to_string())
}

fn build_gemini_vertex_selector(project: &str, location: &str) -> String {
    format!("gemini-vertex:{project}/{location}")
}

fn prompt_gemini_vertex_selector() -> Result<String> {
    print_bullet(&t!("onboard.provider.gemini_vertex_project_hint"));
    print_bullet(&t!("onboard.provider.gemini_vertex_location_hint"));
    let project: String = if let Some(project) = gemini_vertex_project_env() {
        cliclack::input(format!(
            "  {}",
            t!("onboard.provider.gemini_vertex_project_prompt")
        ))
        .default_input(&project)
        .interact()?
    } else {
        cliclack::input(format!(
            "  {}",
            t!("onboard.provider.gemini_vertex_project_prompt")
        ))
        .interact()?
    };
    let project = project.trim().to_string();
    if project.is_empty() {
        anyhow::bail!("Google Cloud project ID cannot be empty");
    }

    let default_location = gemini_vertex_default_location();
    let location: String = cliclack::input(format!(
        "  {}",
        t!("onboard.provider.gemini_vertex_location_prompt")
    ))
    .default_input(&default_location)
    .interact()?;
    let location = location.trim().to_string();
    if location.is_empty() {
        anyhow::bail!("Vertex AI location cannot be empty");
    }

    Ok(build_gemini_vertex_selector(&project, &location))
}

fn vertex_adc_credentials_present() -> bool {
    if std::env::var("GOOGLE_APPLICATION_CREDENTIALS")
        .ok()
        .map(|value| value.trim().to_string())
        .is_some_and(|value| !value.is_empty())
    {
        return true;
    }

    directories::UserDirs::new().is_some_and(|dirs| {
        dirs.home_dir()
            .join(".config")
            .join("gcloud")
            .join("application_default_credentials.json")
            .exists()
    })
}

async fn prompt_api_key_for_provider(provider_name: &str) -> Result<String> {
    println!();
    if let Some(key_url) = provider_api_key_url(provider_name) {
        print_bullet(&t!("onboard.provider.api_key_url", url = ui::url(key_url)));
    }
    print_bullet(&t!("onboard.provider.api_key_later"));
    println!();

    loop {
        let key: String = cliclack::input(format!("  {}", t!("onboard.provider.paste_key")))
            .required(false)
            .interact()?;

        if key.is_empty() {
            let env_var = provider_env_var(provider_name);
            print_bullet(&t!(
                "onboard.provider.key_skipped",
                env_var = ui::yellow(env_var)
            ));
            return Ok(key);
        }

        let sp = cliclack::spinner();
        sp.start("Verifying API key...");
        match crate::onboard::api_verify::verify_api_key(provider_name, &key).await {
            Ok(VerifyResult::Valid { detail }) => {
                sp.stop(format!("✓ {detail}"));
                return Ok(key);
            }
            Ok(VerifyResult::Invalid { reason }) => {
                sp.stop(format!("✗ {reason}"));
                cliclack::log::warning(format!("API key rejected: {reason}"))?;
                // Loop to re-prompt.
            }
            Ok(VerifyResult::Skipped) | Err(_) => {
                sp.stop("Skipped verification");
                return Ok(key);
            }
        }
    }
}

fn select_provider() -> Result<Option<&'static str>> {
    let tier_idx: usize = cliclack::select(format!("  {}", t!("onboard.provider.select_category")))
        .item(0usize, t!("onboard.provider.tier_recommended").as_ref(), "")
        .item(1usize, t!("onboard.provider.tier_fast").as_ref(), "")
        .item(2usize, t!("onboard.provider.tier_gateway").as_ref(), "")
        .item(3usize, t!("onboard.provider.tier_specialized").as_ref(), "")
        .item(4usize, t!("onboard.provider.tier_local").as_ref(), "")
        .item(5usize, t!("onboard.provider.tier_custom").as_ref(), "")
        .interact()?;

    let providers = provider_choices_for_tier(tier_idx);

    if providers.is_empty() {
        return Ok(None);
    }

    let mut provider_select =
        cliclack::select(format!("  {}", t!("onboard.provider.select_provider")));
    for (i, choice) in providers.iter().enumerate() {
        provider_select = provider_select.item(i, choice.label, "");
    }
    let provider_idx: usize = provider_select.interact()?;

    Ok(provider_choice_for_selection(tier_idx, provider_idx).map(|choice| choice.id))
}

fn setup_custom_provider() -> Result<(String, String, String, Option<String>)> {
    println!();
    println!(
        "  {} {}",
        ui::header(t!("onboard.provider.custom_title")),
        ui::dim(format!("— {}", t!("onboard.provider.custom_subtitle")))
    );
    print_bullet(&t!("onboard.provider.custom_desc"));
    print_bullet(&t!("onboard.provider.custom_examples"));
    println!();

    let base_url: String =
        cliclack::input(format!("  {}", t!("onboard.provider.base_url_prompt"))).interact()?;

    let base_url = validate_base_url(&base_url)?;

    let api_key: String = cliclack::input(format!("  {}", t!("onboard.provider.api_key_prompt")))
        .required(false)
        .interact()?;

    let model: String = cliclack::input(format!("  {}", t!("onboard.provider.model_prompt")))
        .default_input("default")
        .interact()?;

    let provider_name = format!("custom:{base_url}");

    println!(
        "  {} {}",
        ui::success("✓"),
        t!(
            "onboard.provider.confirm",
            provider = ui::value(&provider_name),
            model = ui::value(&model)
        )
    );

    Ok((provider_name, api_key, model, None))
}

fn acquire_api_key_gemini() -> Result<String> {
    if crate::core::providers::gemini::GeminiProvider::has_cli_credentials() {
        print_bullet(&format!(
            "{} {}",
            ui::success("✓"),
            t!("onboard.provider.gemini_cli_detected")
        ));
        print_bullet(&t!("onboard.provider.gemini_cli_reuse"));
        println!();

        let use_cli: bool =
            cliclack::confirm(format!("  {}", t!("onboard.provider.gemini_use_cli")))
                .initial_value(true)
                .interact()?;

        if use_cli {
            println!(
                "  {} {}",
                ui::success("✓"),
                t!("onboard.provider.gemini_using_cli")
            );
            return Ok(String::new());
        }

        print_bullet(&t!("onboard.provider.gemini_api_key_url"));
        let key: String = cliclack::input(format!(
            "  {}",
            t!("onboard.provider.gemini_api_key_prompt")
        ))
        .required(false)
        .interact()?;
        return Ok(key);
    }

    let has_gemini_env_key = std::env::var("GEMINI_API_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .is_some_and(|value| !value.is_empty());
    let has_google_env_key = std::env::var("GOOGLE_API_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .is_some_and(|value| !value.is_empty());
    if has_gemini_env_key || has_google_env_key {
        print_bullet(&format!(
            "{} {}",
            ui::success("✓"),
            t!("onboard.provider.gemini_env_detected")
        ));
        return Ok(String::new());
    }

    print_bullet(&t!("onboard.provider.gemini_api_key_url"));
    print_bullet(&t!("onboard.provider.gemini_cli_hint"));
    println!();

    let key: String = cliclack::input(format!(
        "  {}",
        t!("onboard.provider.gemini_api_key_skip_prompt")
    ))
    .required(false)
    .interact()?;
    Ok(key)
}

async fn acquire_api_key_oauth(provider_name: &str) -> Result<(String, Option<String>)> {
    let second_label = match provider_auth_method(provider_name) {
        ProviderAuthMethod::ApiKeyOrSetupToken => {
            t!("onboard.provider.auth_method_setup_token").to_string()
        }
        _ => t!("onboard.provider.auth_method_oauth").to_string(),
    };

    let selected_auth: usize =
        cliclack::select(format!("  {}", t!("onboard.provider.auth_method_prompt")))
            .item(
                0usize,
                t!("onboard.provider.auth_method_api_key").as_ref(),
                "",
            )
            .item(1usize, second_label.as_str(), "")
            .interact()?;

    if selected_auth == 0 {
        let key = prompt_api_key_for_provider(provider_name).await?;
        return Ok((key, None));
    }

    if !matches!(
        provider_auth_method(provider_name),
        ProviderAuthMethod::ApiKeyOrSetupToken
    ) {
        print_bullet(&t!("onboard.provider.oauth_import_start"));
    }
    // Always run the interactive OAuth login when user explicitly
    // selects OAuth — don't silently reuse potentially stale cache.
    match crate::security::auth::run_interactive_oauth_for_provider(&oauth_login_provider(
        provider_name,
    )) {
        Ok(Some((token, source))) => {
            let oauth_source = Some(source);
            print_bullet(&t!(
                "onboard.provider.oauth_import_success",
                source = ui::value(oauth_source.as_deref().unwrap_or("oauth"))
            ));
            Ok((token, oauth_source))
        }
        Ok(None) => {
            print_bullet(&t!("onboard.provider.oauth_import_unavailable"));
            let key = prompt_api_key_for_provider(provider_name).await?;
            Ok((key, None))
        }
        Err(err) => {
            print_bullet(&t!(
                "onboard.provider.oauth_import_failed",
                error = ui::yellow(err.to_string())
            ));
            let key = prompt_api_key_for_provider(provider_name).await?;
            Ok((key, None))
        }
    }
}

async fn acquire_api_key_cloud_auth(provider_name: &str) -> Result<(String, Option<String>)> {
    let selected_auth: usize =
        cliclack::select(format!("  {}", t!("onboard.provider.auth_method_prompt")))
            .item(
                0usize,
                t!("onboard.provider.auth_method_api_key").as_ref(),
                "",
            )
            .item(1usize, t!("onboard.provider.auth_method_adc").as_ref(), "")
            .interact()?;

    if selected_auth == 0 {
        let key = prompt_api_key_for_provider(provider_name).await?;
        return Ok((key, None));
    }

    if vertex_adc_credentials_present() {
        print_bullet(&format!(
            "{} {}",
            ui::success("✓"),
            t!("onboard.provider.gemini_vertex_adc_detected")
        ));
        let use_adc: bool = cliclack::confirm(format!(
            "  {}",
            t!("onboard.provider.gemini_vertex_use_adc")
        ))
        .initial_value(true)
        .interact()?;

        if use_adc {
            print_bullet(&format!(
                "{} {}",
                ui::success("✓"),
                t!("onboard.provider.gemini_vertex_using_adc")
            ));
            return Ok((String::new(), None));
        }
    }

    let key = prompt_api_key_for_provider(provider_name).await?;
    Ok((key, None))
}

async fn acquire_api_key(provider_name: &str) -> Result<(String, Option<String>)> {
    match provider_auth_method(provider_name) {
        ProviderAuthMethod::NoKeyRequired => {
            print_bullet(&t!("onboard.provider.ollama_no_key"));
            return Ok((String::new(), None));
        }
        ProviderAuthMethod::ApiKeyOrOAuth | ProviderAuthMethod::ApiKeyOrSetupToken => {
            return acquire_api_key_oauth(provider_name).await;
        }
        ProviderAuthMethod::ApiKeyOrApplicationDefaultCredentials => {
            return acquire_api_key_cloud_auth(provider_name).await;
        }
        ProviderAuthMethod::ApiKeyOnly => {}
    }

    if provider_name == "gemini" {
        let key = acquire_api_key_gemini()?;
        return Ok((key, None));
    }

    let key = prompt_api_key_for_provider(provider_name).await?;
    Ok((key, None))
}

fn workspace_dir_for_provider_discovery() -> std::path::PathBuf {
    crate::utils::dirs::asterel_home_dir_or_local().join("workspace")
}

fn merge_curated_and_discovered_model_options(
    mut curated: Vec<(String, String)>,
    discovered: &[DiscoveredModel],
) -> Vec<(String, String)> {
    let mut seen = curated
        .iter()
        .map(|(model, _)| model.clone())
        .collect::<std::collections::HashSet<_>>();

    for model in discovered {
        if !seen.insert(model.model_id.clone()) {
            continue;
        }
        let label = discovered_model_option_label(model);
        curated.push((model.model_id.clone(), label));
    }

    curated
}

fn discovered_capability_note(capabilities: &DiscoveredModelCapabilityHints) -> Option<String> {
    let mut notes = Vec::new();
    if capabilities.supports_tools == Some(true) {
        notes.push("tools");
    }
    if capabilities.supports_vision == Some(true) {
        notes.push("vision");
    }
    if capabilities.supports_reasoning == Some(true) {
        notes.push("reasoning");
    }
    if capabilities.supports_streaming == Some(true) {
        notes.push("streaming");
    }

    (!notes.is_empty()).then(|| format!("native: {}", notes.join(", ")))
}

fn discovered_model_option_label(model: &DiscoveredModel) -> String {
    let capability_note = discovered_capability_note(&model.capabilities);
    let suffix = capability_note
        .as_deref()
        .map_or_else(String::new, |note| format!("; {note}"));

    model
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|display| !display.is_empty())
        .filter(|display| *display != model.model_id)
        .map_or_else(
            || format!("{} (discovered{suffix})", model.model_id),
            |display| format!("{display} ({}{suffix})", model.model_id),
        )
}

fn should_skip_live_discovery(provider_name: &str, oauth_source: Option<&str>) -> bool {
    (oauth_source.is_some() && matches!(provider_name, "openai" | "openai-codex"))
        || provider_name.starts_with("gemini-vertex")
}

async fn models_for_provider(
    provider_name: &str,
    api_key: &str,
    oauth_source: Option<&str>,
) -> Vec<(String, String)> {
    let curated = model_choices_for_provider(provider_name)
        .into_iter()
        .map(|choice| (choice.model.to_string(), choice.label.to_string()))
        .collect::<Vec<_>>();
    if should_skip_live_discovery(provider_name, oauth_source) {
        return curated;
    }
    let workspace_dir = workspace_dir_for_provider_discovery();

    let discovery = resolve_provider_discovery(ProviderDiscoveryRequest {
        workspace_dir: workspace_dir.as_path(),
        provider: provider_name,
        api_key: (!api_key.trim().is_empty()).then_some(api_key),
        api_base: None,
        force_refresh: false,
    })
    .await;

    match discovery {
        Ok(result) => merge_curated_and_discovered_model_options(curated, &result.models),
        Err(error) => {
            tracing::debug!(
                provider = %provider_name,
                error = %error,
                "provider discovery unavailable during onboarding model selection"
            );
            curated
        }
    }
}

async fn select_model(
    provider_name: &str,
    api_key: &str,
    oauth_source: Option<&str>,
) -> Result<String> {
    let models = models_for_provider(provider_name, api_key, oauth_source).await;

    let mut model_select = cliclack::select(format!("  {}", t!("onboard.provider.select_model")));
    for (i, (model_id, label)) in models.iter().enumerate() {
        model_select = model_select.item(i, label.as_str(), model_id.as_str());
    }
    let model_idx: usize = model_select.interact()?;

    let model = models[model_idx].0.clone();

    println!(
        "  {} {}",
        ui::success("✓"),
        t!(
            "onboard.provider.confirm",
            provider = ui::value(provider_name),
            model = ui::value(&model)
        )
    );

    Ok(model)
}

/// # Errors
///
/// Returns an error when interactive prompt input fails or selected provider
/// setup cannot be validated.
pub(crate) async fn setup_provider() -> Result<(String, String, String, Option<String>)> {
    let Some(provider_name) = select_provider()? else {
        return setup_custom_provider();
    };

    let provider_selector = if provider_name == "gemini-vertex" {
        prompt_gemini_vertex_selector()?
    } else {
        provider_name.to_string()
    };

    let (api_key, oauth_source) = acquire_api_key(&provider_selector).await?;
    let resolved_provider = provider_after_oauth(&provider_selector, oauth_source.as_deref());
    let model = select_model(&resolved_provider, &api_key, oauth_source.as_deref()).await?;

    Ok((resolved_provider, api_key, model, oauth_source))
}

#[cfg(test)]
mod tests {
    use super::{merge_curated_and_discovered_model_options, should_skip_live_discovery};
    use crate::runtime::services::{DiscoveredModel, DiscoveredModelCapabilityHints};

    #[test]
    fn merge_curated_and_discovered_model_options_appends_new_models_only() {
        let merged = merge_curated_and_discovered_model_options(
            vec![
                ("gpt-5".to_string(), "GPT-5".to_string()),
                ("gpt-5-mini".to_string(), "GPT-5 Mini".to_string()),
            ],
            &[
                DiscoveredModel {
                    model_id: "gpt-5".to_string(),
                    display_name: Some("GPT-5".to_string()),
                    capabilities: DiscoveredModelCapabilityHints::default(),
                },
                DiscoveredModel {
                    model_id: "o4-mini".to_string(),
                    display_name: Some("o4 Mini".to_string()),
                    capabilities: DiscoveredModelCapabilityHints::default(),
                },
            ],
        );

        assert_eq!(merged.len(), 3);
        assert_eq!(
            merged[2],
            ("o4-mini".to_string(), "o4 Mini (o4-mini)".to_string())
        );
    }

    #[test]
    fn discovered_model_options_label_native_capability_hints_only_when_known_true() {
        let merged = merge_curated_and_discovered_model_options(
            vec![("gpt-5".to_string(), "GPT-5".to_string())],
            &[DiscoveredModel {
                model_id: "gpt-4o".to_string(),
                display_name: Some("GPT-4o".to_string()),
                capabilities: DiscoveredModelCapabilityHints {
                    supports_tools: Some(true),
                    supports_vision: Some(true),
                    supports_reasoning: Some(false),
                    supports_streaming: None,
                },
            }],
        );

        assert_eq!(
            merged[1],
            (
                "gpt-4o".to_string(),
                "GPT-4o (gpt-4o; native: tools, vision)".to_string()
            )
        );
    }

    #[test]
    fn discovered_model_options_do_not_claim_unknown_or_false_capabilities() {
        let merged = merge_curated_and_discovered_model_options(
            Vec::new(),
            &[DiscoveredModel {
                model_id: "local-model".to_string(),
                display_name: None,
                capabilities: DiscoveredModelCapabilityHints {
                    supports_tools: Some(false),
                    supports_vision: None,
                    supports_reasoning: Some(false),
                    supports_streaming: None,
                },
            }],
        );

        assert_eq!(
            merged[0],
            (
                "local-model".to_string(),
                "local-model (discovered)".to_string()
            )
        );
    }

    #[test]
    fn should_skip_live_discovery_for_openai_oauth_routes() {
        assert!(should_skip_live_discovery("openai", Some("codex")));
        assert!(should_skip_live_discovery("openai-codex", Some("codex")));
        assert!(!should_skip_live_discovery("openai", None));
        assert!(!should_skip_live_discovery("anthropic", Some("claude")));
    }
}
