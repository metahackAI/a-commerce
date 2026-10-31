//! `get_cs_overview` (docs/api/analytics.md#service): customer service figures of a period,
//! counted from the inquiry read model (`analytics.inquiry_summary`), never who asked.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, ToolError,
};
use acom_store::Cx;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::repo::{self, InquiryFigures};
use crate::{INVALID_PERIOD, decompose, number, rate, span};

/// A customer service figure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceMetric {
    /// Inquiries that came in during the period.
    InquiryCount,
    /// Of those, the ones not resolved yet.
    UnresolvedCount,
    /// Average minutes from an inquiry coming in to the first answer by staff or the agent,
    /// over the answered ones; null when none was answered.
    AverageResponseTime,
    /// Resolved inquiries out of those that came in, 0–1.
    ResolutionRate,
}

impl ServiceMetric {
    const ALL: [Self; 4] = [
        Self::InquiryCount,
        Self::UnresolvedCount,
        Self::AverageResponseTime,
        Self::ResolutionRate,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::InquiryCount => "inquiry_count",
            Self::UnresolvedCount => "unresolved_count",
            Self::AverageResponseTime => "average_response_time",
            Self::ResolutionRate => "resolution_rate",
        }
    }

    fn value(self, f: &InquiryFigures) -> Value {
        match self {
            Self::InquiryCount => json!(f.received),
            Self::UnresolvedCount => json!(f.unresolved),
            Self::AverageResponseTime => f
                .response_minutes
                .map_or(Value::Null, |m| number(decompose::round(m, MINUTE_PLACES))),
            Self::ResolutionRate => rate(Decimal::from(f.resolved), Decimal::from(f.received)),
        }
    }
}

/// Minutes are given to one decimal place.
const MINUTE_PLACES: u32 = 1;

/// What service figures can be split by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceDimension {
    /// What the inquiry is about (order_status, return, product_question, account, callback,
    /// other).
    InquiryType,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCsOverviewInput {
    /// First store day of the period.
    #[schemars(example = &"2026-10-07")]
    pub valid_from: NaiveDate,
    /// Last store day, included; today at the latest.
    #[schemars(example = &"2026-11-06")]
    pub valid_through: NaiveDate,
    /// Figures to give. Default all four.
    #[serde(rename = "acom:metrics", alias = "metrics")]
    #[schemars(example = ["inquiry_count", "average_response_time"])]
    pub metrics: Option<Vec<ServiceMetric>>,
    /// `inquiry_type` gives each figure per type of inquiry.
    #[serde(rename = "acom:dimensions", alias = "dimensions")]
    #[schemars(example = ["inquiry_type"])]
    pub dimensions: Option<Vec<ServiceDimension>>,
}

/// Customer service figures of a period: inquiries, unresolved ones, first response time and
/// resolution rate.
#[derive(Debug)]
pub struct GetCsOverview;

impl Operation for GetCsOverview {
    const META: OpMeta = OpMeta {
        address: "analytics.service.get_cs_overview",
        tool: "get_cs_overview",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Post,
        path: "/v1/analytics/service/overview",
        summary: "Customer service figures for a period: inquiries that came in, how many are unresolved, average minutes to the first answer and the resolution rate, optionally by inquiry type. Counts only, never who.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[INVALID_PERIOD],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetCsOverviewInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetCsOverview {
    async fn read(&self, cx: &Cx, input: GetCsOverviewInput) -> Result<Value, ToolError> {
        let period = span(cx, input.valid_from, input.valid_through, "get_cs_overview")?;
        let mut metrics = input.metrics.unwrap_or_else(|| ServiceMetric::ALL.to_vec());
        metrics.sort();
        metrics.dedup();
        let by_type = input
            .dimensions
            .is_some_and(|d| d.contains(&ServiceDimension::InquiryType));
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let mut rows = repo::inquiry_figures(&mut tx, period.from, period.through, by_type).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if rows.is_empty() && !by_type {
            rows.push(InquiryFigures {
                inquiry_type: None,
                received: 0,
                unresolved: 0,
                resolved: 0,
                response_minutes: None,
            });
        }
        let mut items = Vec::new();
        for m in metrics {
            for f in &rows {
                let dimension = f
                    .inquiry_type
                    .as_ref()
                    .map_or(Value::Null, |t| json!({"inquiry_type": t}));
                items.push(json!({"acom:metric": m.as_str(), "value": m.value(f),
                                  "acom:dimension": dimension}));
            }
        }
        let total = items.len();
        Ok(json!({
            "items": items,
            "total": total,
            "valid_from": period.from,
            "valid_through": period.through,
            "unit": {"average_response_time": "minutes"},
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figures_read_from_one_row() {
        let f = InquiryFigures {
            inquiry_type: Some("return".into()),
            received: 3,
            unresolved: 2,
            resolved: 1,
            response_minutes: Some(Decimal::new(29333, 3)),
        };
        assert_eq!(ServiceMetric::InquiryCount.value(&f), json!(3));
        assert_eq!(ServiceMetric::UnresolvedCount.value(&f), json!(2));
        assert_eq!(
            ServiceMetric::AverageResponseTime.value(&f).to_string(),
            "29.3"
        );
        assert_eq!(
            ServiceMetric::ResolutionRate.value(&f).to_string(),
            "0.3333"
        );
        let none = InquiryFigures {
            received: 0,
            resolved: 0,
            response_minutes: None,
            ..f
        };
        assert_eq!(ServiceMetric::ResolutionRate.value(&none), Value::Null);
        assert_eq!(ServiceMetric::AverageResponseTime.value(&none), Value::Null);
    }
}
