use asterel::core::providers::factory::create_provider;

fn resolve_real_provider() -> Option<(String, String)> {
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        let trimmed = key.trim();
        if !trimmed.is_empty() {
            return Some(("openai".to_string(), trimmed.to_string()));
        }
    }

    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        let trimmed = key.trim();
        if !trimmed.is_empty() {
            return Some(("anthropic".to_string(), trimmed.to_string()));
        }
    }

    if let Ok(key) = std::env::var("ASTEREL_API_KEY") {
        let trimmed = key.trim();
        if trimmed.is_empty() {
            return None;
        }

        if let Ok(provider) = std::env::var("ASTEREL_PROVIDER") {
            let normalized = provider.trim();
            if !normalized.is_empty() {
                return Some((normalized.to_string(), trimmed.to_string()));
            }
        }

        if trimmed.starts_with("sk-ant") {
            return Some(("anthropic".to_string(), trimmed.to_string()));
        }

        return Some(("openai".to_string(), trimmed.to_string()));
    }

    None
}

fn smoke_model_for(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "claude-3-5-haiku-latest",
        "openrouter" => "openai/gpt-4o-mini",
        _ => "gpt-4o-mini",
    }
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY, ANTHROPIC_API_KEY, or ASTEREL_API_KEY"]
async fn real_llm_returns_nonempty_response() {
    let Some((provider_name, api_key)) = resolve_real_provider() else {
        eprintln!(
            "skipping real LLM smoke test: OPENAI_API_KEY, ANTHROPIC_API_KEY, or ASTEREL_API_KEY is not set"
        );
        return;
    };

    let model = std::env::var("ASTEREL_MODEL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| smoke_model_for(&provider_name).to_string());

    let provider = match create_provider(&provider_name, Some(api_key.as_str())) {
        Ok(provider) => provider,
        Err(error) => {
            eprintln!(
                "skipping real LLM smoke test: failed to create provider {provider_name}: {error}"
            );
            return;
        }
    };

    match provider
        .chat_with_system(
            Some("You are a test assistant. Reply briefly."),
            "Say hello in one short sentence.",
            &model,
            0.0,
        )
        .await
    {
        Ok(text) => {
            assert!(
                !text.trim().is_empty(),
                "LLM smoke response should not be empty"
            );
            println!("LLM smoke test response: {text}");
        }
        Err(error) => {
            panic!("LLM smoke test failed: API call returned error: {error}");
        }
    }
}
