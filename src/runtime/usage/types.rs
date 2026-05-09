//! Usage domain types: `UsageRecord`, `UsageSummary`, and
//! `ModelPricing` for token tracking and cost estimation.

use serde::{Deserialize, Serialize};

use crate::contracts::ids::SessionId;

/// A single token-usage record for one LLM request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    /// Unique identifier for this record.
    pub id: String,
    /// Session that generated this usage, if known.
    pub session_id: Option<SessionId>,
    /// LLM provider name (e.g. "openrouter").
    pub provider: String,
    /// Model identifier used for the request.
    pub model: String,
    /// Number of input (prompt) tokens consumed, if known.
    pub input_tokens: Option<u64>,
    /// Number of output (completion) tokens consumed, if known.
    pub output_tokens: Option<u64>,
    /// Estimated cost in micro-currency units, if known.
    pub estimated_cost_micros: Option<i64>,
    /// RFC 3339 timestamp when the record was created.
    pub created_at: String,
}

/// Aggregated usage summary over a set of records.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageSummary {
    /// Sum of input tokens across all records.
    pub total_input_tokens: u64,
    /// Sum of output tokens across all records.
    pub total_output_tokens: u64,
    /// Sum of estimated costs in micro-currency units.
    pub total_estimated_cost_micros: i64,
    /// Number of records included in this summary.
    pub record_count: u64,
}

/// Pricing definition for a model pattern used in cost estimation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    /// Substring pattern matched against model identifiers.
    pub model_pattern: String,
    /// Cost per million input tokens (in the base currency unit).
    pub input_cost_per_million: f64,
    /// Cost per million output tokens (in the base currency unit).
    pub output_cost_per_million: f64,
}

impl ModelPricing {
    /// Estimate the cost in micro-currency units for the given token counts.
    #[must_use]
    pub fn estimate_cost_micros(&self, input_tokens: u64, output_tokens: u64) -> i64 {
        let input_micros_per_million = micros_per_million(self.input_cost_per_million);
        let output_micros_per_million = micros_per_million(self.output_cost_per_million);

        let input_cost = i128::from(input_tokens) * i128::from(input_micros_per_million)
            / i128::from(1_000_000_i64);
        let output_cost = i128::from(output_tokens) * i128::from(output_micros_per_million)
            / i128::from(1_000_000_i64);
        let total = input_cost + output_cost;

        i64::try_from(total).unwrap_or(i64::MAX)
    }
}

fn micros_per_million(cost_per_million: f64) -> i64 {
    let scaled = (cost_per_million * 1_000_000.0).round();
    let text = format!("{scaled:.0}");
    text.parse::<i64>().unwrap_or_default()
}

/// Return the built-in pricing table for common LLM models.
#[must_use]
pub fn default_pricing() -> Vec<ModelPricing> {
    vec![
        ModelPricing {
            model_pattern: "claude-3-5-sonnet".into(),
            input_cost_per_million: 3.0,
            output_cost_per_million: 15.0,
        },
        ModelPricing {
            model_pattern: "claude-sonnet-4".into(),
            input_cost_per_million: 3.0,
            output_cost_per_million: 15.0,
        },
        ModelPricing {
            model_pattern: "claude-3-5-haiku".into(),
            input_cost_per_million: 0.8,
            output_cost_per_million: 4.0,
        },
        ModelPricing {
            model_pattern: "claude-3-opus".into(),
            input_cost_per_million: 15.0,
            output_cost_per_million: 75.0,
        },
        ModelPricing {
            model_pattern: "gpt-4o".into(),
            input_cost_per_million: 2.5,
            output_cost_per_million: 10.0,
        },
        ModelPricing {
            model_pattern: "gpt-4o-mini".into(),
            input_cost_per_million: 0.15,
            output_cost_per_million: 0.6,
        },
        ModelPricing {
            model_pattern: "gemini-2.0-flash".into(),
            input_cost_per_million: 0.1,
            output_cost_per_million: 0.4,
        },
        ModelPricing {
            model_pattern: "gemini-1.5-pro".into(),
            input_cost_per_million: 1.25,
            output_cost_per_million: 5.0,
        },
    ]
}

/// Find the first pricing entry whose `model_pattern` is a substring
/// of the given model identifier.
#[must_use]
pub fn lookup_pricing<'a>(
    model: &str,
    pricing_table: &'a [ModelPricing],
) -> Option<&'a ModelPricing> {
    pricing_table
        .iter()
        .find(|pricing| model.contains(&pricing.model_pattern))
}

#[cfg(test)]
mod tests {
    use super::{ModelPricing, default_pricing, lookup_pricing};

    #[test]
    fn default_pricing_returns_non_empty_list() {
        let pricing = default_pricing();
        assert!(!pricing.is_empty());
    }

    #[test]
    fn lookup_pricing_finds_matching_model() {
        let pricing = default_pricing();
        let found = lookup_pricing("anthropic/claude-sonnet-4.6", &pricing);
        assert!(found.is_some());
    }

    #[test]
    fn lookup_pricing_returns_none_for_unknown_model() {
        let pricing = default_pricing();
        let found = lookup_pricing("unknown/provider-model", &pricing);
        assert!(found.is_none());
    }

    #[test]
    fn estimate_cost_micros_calculates_correctly() {
        let pricing = ModelPricing {
            model_pattern: "test".to_string(),
            input_cost_per_million: 3.0,
            output_cost_per_million: 15.0,
        };
        let cost = pricing.estimate_cost_micros(1_000_000, 1_000_000);
        assert_eq!(cost, 18_000_000);
    }
}
