//! Enhanced Theory of Mind: LLM-based user model inference with
//! belief tracking and dialogue foresight.
//!
//! Extends the rule-based `UserMentalModel` (Phase 1-4) with:
//! - **Beliefs about agent**: what the user thinks the agent can do.
//! - **Likely next question**: anticipatory dialogue planning.
//!
//! Inspired by `ToMAgent` (2025): structured belief inference enables
//! proactive response shaping.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::user_model::{UserMentalModel, infer_user_model};
use crate::contracts::affect::AffectReading;
use crate::contracts::observability::{Observer, ObserverEvent};
use crate::core::memory::MemoryRecallEntry;
use crate::core::providers::Provider;
use crate::security::scrub::sanitize_api_error;

// ── Constants ──────────────────────────────────────────────────

/// System prompt for LLM user model inference.
const SYSTEM_PROMPT: &str = "\
You are a theory-of-mind module. Given a user message and context, \
infer the user's mental model. Output ONLY a JSON object with:\n\
- \"beliefs_about_agent\": string (what the user likely thinks the \
  agent can/cannot do)\n\
- \"likely_next_question\": string (what the user will probably \
  ask next)\n\
Output ONLY the JSON object, no other text.";

// ── Types ──────────────────────────────────────────────────────

/// Enhanced user model combining rule-based inference with LLM
/// belief and foresight predictions.
#[derive(Debug, Clone)]
pub(crate) struct EnhancedUserModel {
    /// Base rule-based mental model.
    pub base: UserMentalModel,
    /// What the user likely believes about the agent's capabilities.
    pub beliefs_about_agent: String,
    /// Predicted next question or follow-up from the user.
    pub likely_next_question: String,
}

/// Raw JSON response from the LLM user model inference.
#[derive(Debug, Deserialize)]
struct LlmUserModelResponse {
    beliefs_about_agent: Option<String>,
    likely_next_question: Option<String>,
}

// ── Inference ──────────────────────────────────────────────────

/// Infer an enhanced user model using LLM inference with rule-based
/// fallback.
///
/// When the provider is `None` or the LLM call fails/times out,
/// falls back to the rule-based model with empty belief/foresight
/// fields.
pub(crate) fn infer_enhanced_user_model<'a>(
    provider: Option<&'a Arc<dyn Provider>>,
    observer: Option<&'a dyn Observer>,
    model_name: &'a str,
    timeout_budget: Duration,
    user_message: &'a str,
    affect: &'a AffectReading,
    user_memories: &'a [MemoryRecallEntry],
) -> Pin<Box<dyn Future<Output = EnhancedUserModel> + Send + 'a>> {
    Box::pin(async move {
        let base = infer_user_model(user_message, affect, user_memories);

        let Some(provider) = provider else {
            return EnhancedUserModel {
                base,
                beliefs_about_agent: String::new(),
                likely_next_question: String::new(),
            };
        };

        let prompt = build_inference_prompt(user_message, &base);
        let timeout_budget = timeout_budget.max(Duration::from_secs(1));
        let started_at = Instant::now();

        let result = tokio::time::timeout(
            timeout_budget,
            provider.chat_with_system(Some(SYSTEM_PROMPT), &prompt, model_name, 0.3),
        )
        .await;

        match result {
            Ok(Ok(response)) => {
                let parsed = parse_llm_response_with_observer(&response, observer);
                EnhancedUserModel {
                    base,
                    beliefs_about_agent: parsed.0,
                    likely_next_question: parsed.1,
                }
            }
            Ok(Err(error)) => {
                record_inference_failure(
                    observer,
                    "provider_error",
                    &sanitize_api_error(&error.to_string()),
                );
                tracing::warn!(
                    %error,
                    elapsed_ms = started_at.elapsed().as_millis(),
                    timeout_ms = timeout_budget.as_millis(),
                    model = model_name,
                    "LLM user model inference failed, using rule-based fallback"
                );
                EnhancedUserModel {
                    base,
                    beliefs_about_agent: String::new(),
                    likely_next_question: String::new(),
                }
            }
            Err(_) => {
                record_inference_failure(
                    observer,
                    "timeout",
                    &format!("timed out after {}ms", timeout_budget.as_millis()),
                );
                tracing::warn!(
                    elapsed_ms = started_at.elapsed().as_millis(),
                    timeout_ms = timeout_budget.as_millis(),
                    model = model_name,
                    "LLM user model inference timed out, using rule-based fallback"
                );
                EnhancedUserModel {
                    base,
                    beliefs_about_agent: String::new(),
                    likely_next_question: String::new(),
                }
            }
        }
    })
}

fn record_inference_failure(observer: Option<&dyn Observer>, status: &str, detail: &str) {
    if let Some(observer) = observer {
        observer.record_event(&ObserverEvent::Error {
            component: "llm_user_model_inference".to_string(),
            message: format!("{status}: {detail}"),
        });
    }
}

/// Build the user prompt for LLM inference.
fn build_inference_prompt(user_message: &str, base: &UserMentalModel) -> String {
    let truncated = crate::utils::text::truncate_ellipsis(user_message, 300);
    format!(
        "User message: \"{truncated}\"\n\
         Inferred intent: {:?}\n\
         Knowledge level: {:?}\n\
         Emotional need: {:?}\n\n\
         Based on this context, infer beliefs_about_agent and likely_next_question.",
        base.inferred_intent, base.knowledge_level, base.emotional_need,
    )
}

/// Parse the LLM JSON response, returning (beliefs, `next_question`).
///
/// Returns empty strings on parse failure.
#[cfg(test)]
fn parse_llm_response(response: &str) -> (String, String) {
    parse_llm_response_with_observer(response, None)
}

fn parse_llm_response_with_observer(
    response: &str,
    observer: Option<&dyn Observer>,
) -> (String, String) {
    let stripped = response.trim();
    let json_str = stripped
        .strip_prefix("```json")
        .map(str::trim)
        .and_then(|s| s.strip_suffix("```"))
        .map_or(stripped, str::trim);

    match serde_json::from_str::<LlmUserModelResponse>(json_str) {
        Ok(parsed) => (
            parsed.beliefs_about_agent.unwrap_or_default(),
            parsed.likely_next_question.unwrap_or_default(),
        ),
        Err(error) => {
            record_inference_failure(
                observer,
                "parse_error",
                &sanitize_api_error(&error.to_string()),
            );
            tracing::warn!(%error, "failed to parse LLM user model response");
            (String::new(), String::new())
        }
    }
}

// ── Rendering ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::affect::AffectReading;
    use crate::contracts::observability::ObserverMetric;
    use crate::core::persona::user_model::{
        EmotionalNeed, KnowledgeLevel, UserIntent, UserMentalModel,
    };
    use crate::core::providers::ProviderResult;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingObserver {
        events: Mutex<Vec<ObserverEvent>>,
    }

    impl Observer for RecordingObserver {
        fn record_event(&self, event: &ObserverEvent) {
            self.events.lock().unwrap().push(event.clone());
        }

        fn record_metric(&self, _metric: &ObserverMetric) {}

        fn name(&self) -> &str {
            "recording"
        }
    }

    fn neutral_reading() -> AffectReading {
        AffectReading::neutral()
    }

    fn sample_base_model() -> UserMentalModel {
        UserMentalModel {
            inferred_intent: UserIntent::Debug,
            knowledge_level: KnowledgeLevel::Advanced,
            emotional_need: EmotionalNeed::Solution,
            active_constraints: vec!["time pressure".to_string()],
        }
    }

    #[test]
    fn parse_valid_llm_response() {
        let json = r#"{"beliefs_about_agent":"can debug Rust code","likely_next_question":"How do I fix the borrow checker error?"}"#;
        let (beliefs, next_q) = parse_llm_response(json);
        assert_eq!(beliefs, "can debug Rust code");
        assert_eq!(next_q, "How do I fix the borrow checker error?");
    }

    #[test]
    fn parse_code_fenced_response() {
        let fenced =
            "```json\n{\"beliefs_about_agent\":\"helpful\",\"likely_next_question\":\"next\"}\n```";
        let (beliefs, next_q) = parse_llm_response(fenced);
        assert_eq!(beliefs, "helpful");
        assert_eq!(next_q, "next");
    }

    #[test]
    fn parse_invalid_response_returns_empty() {
        let (beliefs, next_q) = parse_llm_response("not json at all");
        assert!(beliefs.is_empty());
        assert!(next_q.is_empty());
    }

    #[test]
    fn render_enhanced_model_includes_base_and_extensions() {
        let model = EnhancedUserModel {
            base: sample_base_model(),
            beliefs_about_agent: "can help with Rust".to_string(),
            likely_next_question: "How to fix lifetimes?".to_string(),
        };
        let block = crate::core::persona::presenter::render_enhanced_user_model_block(&model);
        assert!(block.contains("[User Model]"));
        assert!(block.contains("Debug"));
        assert!(block.contains("can help with Rust"));
        assert!(block.contains("How to fix lifetimes?"));
    }

    #[test]
    fn render_enhanced_model_empty_extensions_omitted() {
        let model = EnhancedUserModel {
            base: sample_base_model(),
            beliefs_about_agent: String::new(),
            likely_next_question: String::new(),
        };
        let block = crate::core::persona::presenter::render_enhanced_user_model_block(&model);
        assert!(block.contains("[User Model]"));
        assert!(!block.contains("User believes:"));
        assert!(!block.contains("Likely follow-up:"));
    }

    #[tokio::test]
    async fn infer_without_provider_uses_fallback() {
        let reading = neutral_reading();
        let model = infer_enhanced_user_model(
            None,
            None,
            "",
            Duration::from_secs(8),
            "How does async work?",
            &reading,
            &[],
        )
        .await;
        assert_eq!(model.base.inferred_intent, UserIntent::Learn);
        assert!(model.beliefs_about_agent.is_empty());
        assert!(model.likely_next_question.is_empty());
    }

    struct DelayedProvider {
        response: String,
        delay: Duration,
    }

    impl Provider for DelayedProvider {
        fn chat_with_system<'a>(
            &'a self,
            _system_prompt: Option<&'a str>,
            _message: &'a str,
            _model: &'a str,
            _temperature: f64,
        ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
            Box::pin(async move {
                tokio::time::sleep(self.delay).await;
                Ok(self.response.clone())
            })
        }
    }

    #[tokio::test]
    async fn infer_with_slow_provider_within_timeout_budget_succeeds() {
        let reading = neutral_reading();
        let provider: Arc<dyn Provider> = Arc::new(DelayedProvider {
            response: r#"{"beliefs_about_agent":"can inspect code","likely_next_question":"Can you patch it?"}"#
                .to_string(),
            delay: Duration::from_secs(4),
        });

        let model = infer_enhanced_user_model(
            Some(&provider),
            None,
            "test-model",
            Duration::from_secs(8),
            "Can you review this Rust patch?",
            &reading,
            &[],
        )
        .await;

        assert_eq!(model.beliefs_about_agent, "can inspect code");
        assert_eq!(model.likely_next_question, "Can you patch it?");
    }

    #[tokio::test]
    async fn provider_error_records_observer_signal_before_rule_based_fallback() {
        struct ErrorProvider;

        impl Provider for ErrorProvider {
            fn chat_with_system<'a>(
                &'a self,
                _system_prompt: Option<&'a str>,
                _message: &'a str,
                _model: &'a str,
                _temperature: f64,
            ) -> Pin<Box<dyn Future<Output = ProviderResult<String>> + Send + 'a>> {
                Box::pin(
                    async move { Err(anyhow::anyhow!("provider failed sk-test-secret").into()) },
                )
            }
        }

        let reading = neutral_reading();
        let provider: Arc<dyn Provider> = Arc::new(ErrorProvider);
        let observer = RecordingObserver::default();

        let model = infer_enhanced_user_model(
            Some(&provider),
            Some(&observer),
            "test-model",
            Duration::from_secs(1),
            "Can you review this Rust patch?",
            &reading,
            &[],
        )
        .await;

        assert!(model.beliefs_about_agent.is_empty());
        let events = observer.events.lock().unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            ObserverEvent::Error { component, message }
                if component == "llm_user_model_inference"
                    && message.contains("provider_error")
                    && !message.contains("sk-test-secret")
        )));
    }

    #[test]
    fn parse_error_records_observer_signal() {
        let observer = RecordingObserver::default();

        let (beliefs, next_q) =
            parse_llm_response_with_observer("not json at all", Some(&observer));

        assert!(beliefs.is_empty());
        assert!(next_q.is_empty());
        let events = observer.events.lock().unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            ObserverEvent::Error { component, message }
                if component == "llm_user_model_inference" && message.contains("parse_error")
        )));
    }
}
