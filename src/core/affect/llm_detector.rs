//! LLM-based affect detection with rule-based fallback.
//!
//! # Architecture
//!
//! This module defines the [`AffectDetector`] trait and two implementations:
//!
//! - [`LlmAffectDetector`] — sends the user message to an LLM with a
//!   compact system prompt (~150 tokens) that requests a JSON object
//!   containing `label`, `valence`, `arousal`, `dominance`, and `confidence`.
//!   Has a 2-second timeout and a 5-second same-message cache to avoid
//!   redundant calls when pre-answer and post-answer analysis run in the
//!   same turn. Falls back to rule-based on timeout, parse error, or
//!   provider failure.
//!
//! - [`RuleBasedAffectDetector`] — async wrapper around the synchronous
//!   [`RuleBasedDetector`] for uniform dispatch. Zero network cost.
//!
//! # Gating
//!
//! Gated by `PersonaConfig::enable_llm_affect`. When disabled (or when no
//! provider is configured), [`build_affect_detector`] returns a
//! `RuleBasedAffectDetector` so all call-sites remain unchanged.
//!
//! # Response format
//!
//! The LLM is prompted to return only a JSON object. The parser handles both
//! raw JSON and code-fenced JSON (` ```json ... ``` `) to tolerate common LLM
//! formatting quirks. Unknown labels cause a fallback rather than an error.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use super::detector::RuleBasedDetector;
use super::types::{AffectLabel, AffectReading};
use crate::contracts::scores::Confidence;
use crate::core::providers::Provider;

// ── Trait ──────────────────────────────────────────────────────

/// Async, object-safe affect detection trait.
///
/// Implementations must be `Send + Sync` for use behind `Arc`.
pub(crate) trait AffectDetector: Send + Sync {
    /// Detect the user's affective state from a text message.
    fn detect<'a>(
        &'a self,
        user_message: &'a str,
    ) -> Pin<Box<dyn Future<Output = AffectReading> + Send + 'a>>;
}

// ── Rule-based wrapper ────────────────────────────────────────

/// Async wrapper around `RuleBasedDetector` for uniform dispatch.
pub(crate) struct RuleBasedAffectDetector;

impl AffectDetector for RuleBasedAffectDetector {
    fn detect<'a>(
        &'a self,
        user_message: &'a str,
    ) -> Pin<Box<dyn Future<Output = AffectReading> + Send + 'a>> {
        Box::pin(async move { RuleBasedDetector::new().detect(user_message) })
    }
}

// ── LLM-based detector ───────────────────────────────────────

/// System prompt for affect analysis (~150 tokens).
const AFFECT_SYSTEM_PROMPT: &str = "\
You are an affect analysis module. Given a user message, output \
a JSON object with these fields:
- \"label\": one of \"neutral\",\"confused\",\"frustrated\",\"anxious\",\
\"sad\",\"angry\",\"excited\",\"grateful\",\"curious\",\"overwhelmed\"
- \"valence\": float in [-1.0, 1.0] (pleasure/displeasure)
- \"arousal\": float in [0.0, 1.0] (activation)
- \"dominance\": float in [0.0, 1.0] (control)
- \"confidence\": float in [0.0, 1.0]
Output ONLY the JSON object, no other text.";

/// LLM call timeout for affect detection.
const AFFECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Cache TTL: avoids redundant LLM calls when the same message
/// is analysed by both pre-answer and post-answer within one turn.
const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(5);

/// LLM-based affect detector with rule-based fallback.
pub(crate) struct LlmAffectDetector {
    provider: Arc<dyn Provider>,
    model: String,
    cache: tokio::sync::Mutex<Option<(Instant, String, AffectReading)>>,
}

impl LlmAffectDetector {
    /// Create a new LLM affect detector.
    pub(crate) fn new(provider: Arc<dyn Provider>, model: String) -> Self {
        Self {
            provider,
            model,
            cache: tokio::sync::Mutex::new(None),
        }
    }
}

impl AffectDetector for LlmAffectDetector {
    fn detect<'a>(
        &'a self,
        user_message: &'a str,
    ) -> Pin<Box<dyn Future<Output = AffectReading> + Send + 'a>> {
        Box::pin(async move {
            // ── Cache check ──────────────────────────────────
            {
                let guard = self.cache.lock().await;
                if let Some((ts, ref cached_msg, ref reading)) = *guard
                    && ts.elapsed() < CACHE_TTL
                    && cached_msg == user_message
                {
                    return reading.clone();
                }
            }

            // ── LLM call with timeout ────────────────────────
            let result = tokio::time::timeout(
                AFFECT_TIMEOUT,
                self.provider.chat_with_system(
                    Some(AFFECT_SYSTEM_PROMPT),
                    user_message,
                    &self.model,
                    0.0,
                ),
            )
            .await;

            let reading = match result {
                Ok(Ok(response)) => {
                    if let Some(r) = parse_affect_response(&response) {
                        r
                    } else {
                        tracing::debug!("LLM affect response parse failed, falling back");
                        RuleBasedDetector::new().detect(user_message)
                    }
                }
                Ok(Err(error)) => {
                    tracing::debug!(%error, "LLM affect call failed, falling back");
                    RuleBasedDetector::new().detect(user_message)
                }
                Err(_) => {
                    tracing::debug!("LLM affect call timed out, falling back");
                    RuleBasedDetector::new().detect(user_message)
                }
            };

            // ── Update cache ─────────────────────────────────
            {
                let mut guard = self.cache.lock().await;
                *guard = Some((Instant::now(), user_message.to_string(), reading.clone()));
            }

            reading
        })
    }
}

// ── Response parsing ─────────────────────────────────────────

/// LLM output JSON structure.
#[derive(serde::Deserialize)]
struct AffectJson {
    label: String,
    valence: f64,
    arousal: f64,
    #[serde(default = "super::types::default_dominance")]
    dominance: f64,
    confidence: f64,
}

fn parse_affect_response(response: &str) -> Option<AffectReading> {
    // Try direct parse first, then look for JSON in code fences.
    let stripped = response.trim();
    let json_str = stripped
        .strip_prefix("```json")
        .map(str::trim)
        .and_then(|s| s.strip_suffix("```"))
        .map_or(stripped, str::trim);

    let parsed: AffectJson = serde_json::from_str(json_str).ok()?;

    let label = match parsed.label.as_str() {
        "neutral" => AffectLabel::Neutral,
        "confused" => AffectLabel::Confused,
        "frustrated" => AffectLabel::Frustrated,
        "anxious" => AffectLabel::Anxious,
        "sad" => AffectLabel::Sad,
        "angry" => AffectLabel::Angry,
        "excited" => AffectLabel::Excited,
        "grateful" => AffectLabel::Grateful,
        "curious" => AffectLabel::Curious,
        "overwhelmed" => AffectLabel::Overwhelmed,
        _ => return None,
    };

    Some(AffectReading {
        label,
        valence: parsed.valence.clamp(-1.0, 1.0),
        arousal: parsed.arousal.clamp(0.0, 1.0),
        dominance: parsed.dominance.clamp(0.0, 1.0),
        confidence: Confidence::new(parsed.confidence),
    })
}

/// Build the appropriate affect detector based on config and provider availability.
///
/// Returns an [`LlmAffectDetector`] when `enable_llm` is `true` **and** a
/// provider is available. Returns a [`RuleBasedAffectDetector`] otherwise,
/// with a warning log when LLM was requested but no provider was found.
pub(crate) fn build_affect_detector(
    enable_llm: bool,
    provider: Option<Arc<dyn Provider>>,
    model: String,
) -> Arc<dyn AffectDetector> {
    if enable_llm {
        if let Some(p) = provider {
            return Arc::new(LlmAffectDetector::new(p, model));
        }
        tracing::warn!(
            "enable_llm_affect=true but no provider available; \
             falling back to rule-based"
        );
    }
    Arc::new(RuleBasedAffectDetector)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_json() {
        let json = r#"{"label":"frustrated","valence":-0.5,"arousal":0.7,"dominance":0.4,"confidence":0.85}"#;
        let reading = parse_affect_response(json).unwrap();
        assert_eq!(reading.label, AffectLabel::Frustrated);
        assert!((reading.valence - (-0.5)).abs() < f64::EPSILON);
        assert!((reading.arousal - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_code_fenced_json() {
        let response = "```json\n{\"label\":\"excited\",\"valence\":0.8,\"arousal\":0.9,\"dominance\":0.6,\"confidence\":0.9}\n```";
        let reading = parse_affect_response(response).unwrap();
        assert_eq!(reading.label, AffectLabel::Excited);
    }

    #[test]
    fn parse_invalid_label_returns_none() {
        let json =
            r#"{"label":"bored","valence":0.0,"arousal":0.2,"dominance":0.5,"confidence":0.5}"#;
        assert!(parse_affect_response(json).is_none());
    }

    #[test]
    fn parse_missing_fields_returns_none() {
        let json = r#"{"label":"neutral"}"#;
        assert!(parse_affect_response(json).is_none());
    }

    #[test]
    fn parse_clamps_out_of_range_values() {
        let json =
            r#"{"label":"angry","valence":-2.0,"arousal":1.5,"dominance":-0.5,"confidence":3.0}"#;
        let reading = parse_affect_response(json).unwrap();
        assert_eq!(reading.valence, -1.0);
        assert_eq!(reading.arousal, 1.0);
        assert_eq!(reading.dominance, 0.0);
        assert_eq!(reading.confidence, Confidence::new(1.0));
    }

    #[test]
    fn parse_dominance_defaults() {
        let json = r#"{"label":"neutral","valence":0.0,"arousal":0.3,"confidence":0.8}"#;
        let reading = parse_affect_response(json).unwrap();
        assert!((reading.dominance - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_new_labels() {
        for (label_str, expected) in [
            ("grateful", AffectLabel::Grateful),
            ("curious", AffectLabel::Curious),
            ("overwhelmed", AffectLabel::Overwhelmed),
        ] {
            let json = format!(
                r#"{{"label":"{label_str}","valence":0.0,"arousal":0.5,"dominance":0.5,"confidence":0.7}}"#
            );
            let reading = parse_affect_response(&json).unwrap();
            assert_eq!(reading.label, expected);
        }
    }

    #[tokio::test]
    async fn rule_based_wrapper_returns_reading() {
        let detector = RuleBasedAffectDetector;
        let reading = detector.detect("I'm so frustrated!").await;
        assert_eq!(reading.label, AffectLabel::Frustrated);
    }
}
