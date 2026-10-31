//! `get_sales_trend` (docs/api/analytics.md#sales): sales per day, week or month.

use std::collections::BTreeMap;

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, ToolError,
};
use acom_store::Cx;
use chrono::{Datelike, Days, Months, NaiveDate};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::decompose;
use crate::repo::{self, Bucket, Cut, Split};
use crate::{INVALID_GRANULARITY, INVALID_PERIOD, money_json, span};

use super::Dimension;

/// How long one point of a trend is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Granularity {
    /// One store day (periods of 92 days at most).
    Day,
    /// A week starting Monday; the point's date is the Monday.
    Week,
    /// A calendar month; the point's date is the 1st.
    Month,
}

/// A figure of the trend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrendMetric {
    /// Paid, not cancelled orders' totals, in the store's currency.
    Revenue,
    /// Number of paid, not cancelled orders.
    OrderCount,
    /// Revenue divided by orders; null for a point without orders.
    AverageOrderValue,
}

impl TrendMetric {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Revenue => "revenue",
            Self::OrderCount => "order_count",
            Self::AverageOrderValue => "average_order_value",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSalesTrendInput {
    /// First store day of the trend.
    #[schemars(example = &"2026-10-23")]
    pub valid_from: NaiveDate,
    /// Last store day, included; today at the latest.
    #[schemars(example = &"2026-11-05")]
    pub valid_through: NaiveDate,
    /// Length of one point: `day`, `week` (starting Monday) or `month`.
    #[serde(rename = "acom:granularity", alias = "granularity")]
    #[schemars(example = &"day")]
    pub granularity: Granularity,
    /// Figures per point. Default `["revenue"]`.
    #[serde(rename = "acom:metrics", alias = "metrics")]
    #[schemars(example = ["revenue", "order_count"])]
    pub metrics: Option<Vec<TrendMetric>>,
    /// Splits every point by category or member tier.
    #[serde(rename = "acom:dimension", alias = "dimension")]
    #[schemars(example = &"member_tier")]
    pub dimension: Option<Dimension>,
}

/// Sales per day, week or month.
#[derive(Debug)]
pub struct GetSalesTrend;

impl Operation for GetSalesTrend {
    const META: OpMeta = OpMeta {
        address: "analytics.sales.get_sales_trend",
        tool: "get_sales_trend",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Post,
        path: "/v1/analytics/sales/trend",
        summary: "Sales over time: revenue, orders or average order value per day, week or month, optionally split by category or member tier. For daily revenue charts.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[INVALID_PERIOD, INVALID_GRANULARITY],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetSalesTrendInput;
    type Output = Value;
}

/// The longest period shown day by day.
const MAX_DAY_POINTS: i64 = 92;

/// The value of one figure in one bucket.
pub(super) fn figure(cx: &Cx, m: TrendMetric, b: &Bucket) -> Value {
    match m {
        TrendMetric::Revenue => money_json(cx, b.revenue_value),
        TrendMetric::OrderCount => json!(b.order_count),
        TrendMetric::AverageOrderValue if b.order_count == 0 => Value::Null,
        TrendMetric::AverageOrderValue => money_json(
            cx,
            decompose::round(
                b.revenue_value / Decimal::from(b.order_count),
                cx.currency.minor_units(),
            ),
        ),
    }
}

/// Starts of the buckets that cover `[from, through]`.
fn bucket_starts(from: NaiveDate, through: NaiveDate, g: Granularity) -> Vec<NaiveDate> {
    let first = match g {
        Granularity::Day => from,
        Granularity::Week => from - Days::new(u64::from(from.weekday().num_days_from_monday())),
        Granularity::Month => from.with_day(1).unwrap_or(from),
    };
    let mut out = Vec::new();
    let mut d = first;
    while d <= through {
        out.push(d);
        let next = match g {
            Granularity::Day => d.checked_add_days(Days::new(1)),
            Granularity::Week => d.checked_add_days(Days::new(7)),
            Granularity::Month => d.checked_add_months(Months::new(1)),
        };
        match next {
            Some(n) => d = n,
            None => break,
        }
    }
    out
}

pub(super) fn empty_bucket(date: NaiveDate) -> Bucket {
    Bucket {
        observation_date: Some(date),
        dimension: None,
        placed: 0,
        cancelled: 0,
        order_count: 0,
        revenue_value: Decimal::ZERO,
        returned: 0,
    }
}

impl ReadOp<Cx> for GetSalesTrend {
    async fn read(&self, cx: &Cx, input: GetSalesTrendInput) -> Result<Value, ToolError> {
        let period = span(cx, input.valid_from, input.valid_through, "get_sales_trend")?;
        if input.granularity == Granularity::Day && period.day_count() > MAX_DAY_POINTS {
            return Err(ToolError::new(
                INVALID_GRANULARITY,
                format!(
                    "{} days is too many to show day by day ({MAX_DAY_POINTS} at most); use week or month.",
                    period.day_count()
                ),
            )
            .next("get_sales_trend")
            .detail(json!({"field": "acom:granularity", "reason": "range", "allowed": ["week", "month"]})));
        }
        let mut metrics = input.metrics.unwrap_or_else(|| vec![TrendMetric::Revenue]);
        metrics.dedup();
        if metrics.is_empty() {
            metrics.push(TrendMetric::Revenue);
        }
        let cut = match input.granularity {
            Granularity::Day => Cut::Day,
            Granularity::Week => Cut::Week,
            Granularity::Month => Cut::Month,
        };
        let split = input.dimension.map_or(Split::None, Dimension::split);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let mut rows = repo::buckets(&mut tx, period.from, period.through, cut, split).await?;
        tx.commit().await.map_err(ToolError::from)?;
        if input.dimension.is_none() {
            // Every point is there, with zeros where nothing sold.
            let have: BTreeMap<Option<NaiveDate>, Bucket> =
                rows.into_iter().map(|b| (b.observation_date, b)).collect();
            rows = bucket_starts(period.from, period.through, input.granularity)
                .into_iter()
                .map(|d| {
                    have.get(&Some(d))
                        .cloned()
                        .unwrap_or_else(|| empty_bucket(d))
                })
                .collect();
        }
        let mut items = Vec::new();
        for b in &rows {
            for m in &metrics {
                let mut item = json!({
                    "observation_date": b.observation_date,
                    "acom:metric": m.as_str(),
                    "value": figure(cx, *m, b),
                });
                if let Some(d) = input.dimension {
                    item["acom:dimension"] = json!({d.key(): b.dimension});
                }
                items.push(item);
            }
        }
        let total = items.len();
        Ok(json!({"items": items, "total": total}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    #[test]
    fn buckets_cover_the_period() {
        assert_eq!(
            bucket_starts(d(10, 23), d(11, 5), Granularity::Day).len(),
            14
        );
        assert_eq!(
            bucket_starts(d(10, 23), d(11, 5), Granularity::Week),
            [d(10, 19), d(10, 26), d(11, 2)]
        );
        assert_eq!(
            bucket_starts(d(10, 23), d(11, 5), Granularity::Month),
            [d(10, 1), d(11, 1)]
        );
    }
}
