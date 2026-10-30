//! Per-call usage records: what each model call cost in tokens, time, and money
//! (docs/backend/agent.md §8.4). The runtime stores them in `agent.model_call`.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::chat::{ChatResponse, LlmError, ModelInfo, Usage};

/// A model's price list, from `agents/models.yml`. Amounts are decimal strings so no float
/// ever touches money.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Price {
    /// Price of one million input tokens.
    pub input_per_million_tokens: Decimal,
    /// Price of one million output tokens.
    pub output_per_million_tokens: Decimal,
    /// ISO 4217 code of both prices.
    pub currency: String,
}

impl Price {
    pub fn cost(&self, usage: Usage) -> Decimal {
        let million = Decimal::from(1_000_000u32);
        (Decimal::from(usage.input_tokens) * self.input_per_million_tokens
            + Decimal::from(usage.output_tokens) * self.output_per_million_tokens)
            / million
    }
}

/// One model call, as recorded.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageRecord {
    /// The provider's model id.
    pub model: String,
    /// The alias the call went through (`glimmer`, `spark`).
    pub alias: String,
    /// Why it was called: `route`, `plan`, `arguments`, `reply`.
    pub purpose: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub latency_ms: u64,
    /// `None` when the model has no price in `agents/models.yml`.
    pub cost: Option<Decimal>,
    pub cost_currency: Option<String>,
    /// False when the call errored or its answer failed validation.
    pub succeeded: bool,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
}

impl UsageRecord {
    pub fn new(
        info: &ModelInfo,
        purpose: &str,
        started_at: DateTime<Utc>,
        latency_ms: u64,
        result: &Result<ChatResponse, LlmError>,
    ) -> Self {
        let usage = result.as_ref().map(|r| r.usage).unwrap_or_default();
        Self {
            model: info.model.clone(),
            alias: info.alias.clone(),
            purpose: purpose.to_owned(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            latency_ms,
            cost: info.price.as_ref().map(|p| p.cost(usage)),
            cost_currency: info.price.as_ref().map(|p| p.currency.clone()),
            succeeded: result.is_ok(),
            error: result.as_ref().err().map(ToString::to_string),
            started_at,
        }
    }

    /// Marks a call whose answer could not be used (no JSON, invalid plan).
    pub fn failed_validation(mut self, why: impl Into<String>) -> Self {
        self.succeeded = false;
        self.error = Some(why.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn cost_is_exact_decimal_arithmetic() {
        let price = Price {
            input_per_million_tokens: Decimal::from_str("0.20").unwrap(),
            output_per_million_tokens: Decimal::from_str("0.60").unwrap(),
            currency: "USD".into(),
        };
        let cost = price.cost(Usage {
            input_tokens: 2_500,
            output_tokens: 500,
        });
        assert_eq!(cost, Decimal::from_str("0.0008").unwrap());

        let info = ModelInfo {
            alias: "glimmer".into(),
            model: "muse-glimmer-30b".into(),
            display_name: "Muse Glimmer 30B".into(),
            price: Some(price),
        };
        let ok = Ok(ChatResponse {
            usage: Usage {
                input_tokens: 2_500,
                output_tokens: 500,
            },
            ..Default::default()
        });
        let r = UsageRecord::new(&info, "plan", Utc::now(), 120, &ok);
        assert!(r.succeeded);
        assert_eq!(r.cost, Some(Decimal::from_str("0.0008").unwrap()));
        let r = UsageRecord::new(&info, "plan", Utc::now(), 120, &Err(LlmError::Timeout(30)));
        assert!(!r.succeeded);
        assert_eq!(r.input_tokens, 0);
    }
}
