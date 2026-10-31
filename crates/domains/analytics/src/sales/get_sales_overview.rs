//! `get_sales_overview` (docs/api/analytics.md#sales): the headline figures of a period.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, ToolError,
};
use acom_store::Cx;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::decompose;
use crate::period::Span;
use crate::repo::{self, Bucket, Cut, Split};
use crate::{DIMENSION_NOT_SUPPORTED, INVALID_PERIOD, money_json, number, rate, span};

use super::Dimension;
use super::get_sales_trend::{TrendMetric, empty_bucket, figure};

/// A headline figure of a period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OverviewMetric {
    /// Paid, not cancelled orders' totals, in the store's currency.
    Revenue,
    /// Number of paid, not cancelled orders.
    OrderCount,
    /// Revenue divided by orders.
    AverageOrderValue,
    /// Cancelled orders out of paid orders, 0–1.
    CancellationRate,
    /// Orders with a return asked for (not declined or withdrawn) out of paid, not cancelled
    /// orders, 0–1.
    ReturnRate,
}

impl OverviewMetric {
    const ALL: [Self; 5] = [
        Self::Revenue,
        Self::OrderCount,
        Self::AverageOrderValue,
        Self::CancellationRate,
        Self::ReturnRate,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::Revenue => "revenue",
            Self::OrderCount => "order_count",
            Self::AverageOrderValue => "average_order_value",
            Self::CancellationRate => "cancellation_rate",
            Self::ReturnRate => "return_rate",
        }
    }

    /// Whether the figure can be split by `d`: rates belong to whole orders, which a category
    /// split cuts into lines.
    fn splits_by(self, d: Dimension) -> bool {
        d == Dimension::MemberTier
            || matches!(
                self,
                Self::Revenue | Self::OrderCount | Self::AverageOrderValue
            )
    }

    /// The figure as an exact decimal (for change rates), or `None` when it has no value.
    fn exact(self, b: &Bucket) -> Option<Decimal> {
        match self {
            Self::Revenue => Some(b.revenue_value),
            Self::OrderCount => Some(Decimal::from(b.order_count)),
            Self::AverageOrderValue => {
                (b.order_count > 0).then(|| b.revenue_value / Decimal::from(b.order_count))
            }
            Self::CancellationRate => {
                (b.placed > 0).then(|| Decimal::from(b.cancelled) / Decimal::from(b.placed))
            }
            Self::ReturnRate => (b.order_count > 0)
                .then(|| Decimal::from(b.returned) / Decimal::from(b.order_count)),
        }
    }

    fn value(self, cx: &Cx, b: &Bucket) -> Value {
        match self {
            Self::Revenue => money_json(cx, b.revenue_value),
            Self::OrderCount => json!(b.order_count),
            Self::AverageOrderValue => figure(cx, TrendMetric::AverageOrderValue, b),
            Self::CancellationRate => rate(Decimal::from(b.cancelled), Decimal::from(b.placed)),
            Self::ReturnRate => rate(Decimal::from(b.returned), Decimal::from(b.order_count)),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSalesOverviewInput {
    /// First store day of the period.
    #[schemars(example = &"2026-11-05")]
    pub valid_from: NaiveDate,
    /// Last store day, included; today at the latest.
    #[schemars(example = &"2026-11-05")]
    pub valid_through: NaiveDate,
    /// Figures to give. Default all five.
    #[serde(rename = "acom:metrics", alias = "metrics")]
    #[schemars(example = ["revenue", "order_count"])]
    pub metrics: Option<Vec<OverviewMetric>>,
    /// Splits the figures by these, one list of items per dimension. Rates cannot be split by
    /// category.
    #[serde(rename = "acom:dimensions", alias = "dimensions")]
    #[schemars(example = ["member_tier"])]
    pub dimensions: Option<Vec<Dimension>>,
    /// First day of a period to compare with; gives each figure an `acom:change_rate`.
    #[schemars(example = &"2026-10-29")]
    pub compare_from: Option<NaiveDate>,
    /// Last day of the comparison, included. Needed with `compare_from`.
    #[schemars(example = &"2026-10-29")]
    pub compare_through: Option<NaiveDate>,
}

/// The headline sales figures of a period.
#[derive(Debug)]
pub struct GetSalesOverview;

impl Operation for GetSalesOverview {
    const META: OpMeta = OpMeta {
        address: "analytics.sales.get_sales_overview",
        tool: "get_sales_overview",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Post,
        path: "/v1/analytics/sales/overview",
        summary: "The key sales figures of a period at once: revenue, orders, average order value, cancellation and return rates, optionally by category or member tier and against a comparison period.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[INVALID_PERIOD, DIMENSION_NOT_SUPPORTED],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetSalesOverviewInput;
    type Output = Value;
}

fn whole(rows: Vec<Bucket>) -> Bucket {
    rows.into_iter().next().unwrap_or(Bucket {
        observation_date: None,
        ..empty_bucket(NaiveDate::MIN)
    })
}

impl ReadOp<Cx> for GetSalesOverview {
    async fn read(&self, cx: &Cx, input: GetSalesOverviewInput) -> Result<Value, ToolError> {
        const NEXT: &str = "get_sales_overview";
        let period = span(cx, input.valid_from, input.valid_through, NEXT)?;
        let compare: Option<Span> = match (input.compare_from, input.compare_through) {
            (None, None) => None,
            (Some(f), Some(t)) => Some(span(cx, f, t, NEXT)?),
            (_, _) => {
                return Err(ToolError::new(
                    INVALID_PERIOD,
                    "A comparison needs both compare_from and compare_through.",
                )
                .next(NEXT)
                .detail(json!({"field": "compare_through", "reason": "missing"})));
            }
        };
        let explicit = input.metrics.is_some();
        let mut metrics = input
            .metrics
            .unwrap_or_else(|| OverviewMetric::ALL.to_vec());
        metrics.dedup();
        let dimensions: Vec<Option<Dimension>> = match input.dimensions {
            Some(d) if !d.is_empty() => d.into_iter().map(Some).collect(),
            _ => vec![None],
        };
        for d in dimensions.iter().flatten() {
            if let Some(m) = metrics.iter().find(|m| !m.splits_by(*d))
                && explicit
            {
                return Err(ToolError::new(
                    DIMENSION_NOT_SUPPORTED,
                    format!(
                        "{} cannot be split by {}: it counts whole orders. Split it by member_tier, or leave it out.",
                        m.as_str(),
                        d.key()
                    ),
                )
                .next(NEXT)
                .detail(json!({"field": "acom:dimensions", "reason": "enum", "allowed": ["member_tier"]})));
            }
        }

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let mut items = Vec::new();
        for d in dimensions {
            let split = d.map_or(Split::None, Dimension::split);
            let now =
                repo::buckets(&mut tx, period.from, period.through, Cut::Whole, split).await?;
            let before = match &compare {
                Some(c) => {
                    Some(repo::buckets(&mut tx, c.from, c.through, Cut::Whole, split).await?)
                }
                None => None,
            };
            let now = if d.is_none() { vec![whole(now)] } else { now };
            for b in &now {
                let base = before.as_ref().and_then(|rows| {
                    rows.iter()
                        .find(|r| r.dimension == b.dimension)
                        .cloned()
                        .or_else(|| d.is_none().then(|| whole(rows.clone())))
                });
                for m in metrics.iter().filter(|m| d.is_none_or(|d| m.splits_by(d))) {
                    let change = match (&base, m.exact(b)) {
                        (Some(base), Some(v)) => match m.exact(base) {
                            Some(was) if !was.is_zero() => {
                                number(decompose::round((v - was) / was, decompose::RATE_PLACES))
                            }
                            _ => Value::Null,
                        },
                        _ => Value::Null,
                    };
                    items.push(json!({
                        "acom:metric": m.as_str(),
                        "value": m.value(cx, b),
                        "acom:dimension": d.map(|d| json!({d.key(): b.dimension})),
                        "acom:change_rate": change,
                    }));
                }
            }
        }
        tx.commit().await.map_err(ToolError::from)?;
        let total = items.len();
        Ok(json!({
            "items": items,
            "total": total,
            "valid_from": period.from,
            "valid_through": period.through,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_do_not_split_by_category() {
        assert!(OverviewMetric::Revenue.splits_by(Dimension::Category));
        assert!(!OverviewMetric::ReturnRate.splits_by(Dimension::Category));
        assert!(OverviewMetric::ReturnRate.splits_by(Dimension::MemberTier));
    }
}
