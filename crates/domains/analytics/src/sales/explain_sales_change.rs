//! `explain_sales_change` (docs/api/analytics.md#sales): a period's sales against a basis, the
//! difference split into causes found in the data.

use std::collections::{BTreeMap, BTreeSet};

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, ToolError,
};
use acom_store::Cx;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::decompose::{self, CauseFigures, Scale};
use crate::period::{self, CompareMode};
use crate::repo::{self, CampaignSend, StockOut};
use crate::{INVALID_PERIOD, NO_DATA_FOR_PERIOD, money_json, number, period_error, span};

/// The figure an explanation splits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExplainMetric {
    /// Paid, not cancelled orders' totals (total_payment_due), in the store's currency.
    #[default]
    Revenue,
    /// Number of paid, not cancelled orders.
    OrderCount,
}

impl ExplainMetric {
    fn as_str(self) -> &'static str {
        match self {
            Self::Revenue => "revenue",
            Self::OrderCount => "order_count",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExplainSalesChangeInput {
    /// First store day of the period to explain.
    #[schemars(example = &"2026-11-05")]
    pub valid_from: NaiveDate,
    /// Last store day of the period, included; today at the latest.
    #[schemars(example = &"2026-11-05")]
    pub valid_through: NaiveDate,
    /// What the period is compared with. `same_weekday_last_4_weeks`: the same weekdays in each
    /// of the last 4 weeks, averaged (periods of 7 days at most); `previous_period`: the days
    /// just before, as many as the period has; `custom`: `compare_from` to `compare_through`,
    /// scaled to the period's length.
    #[schemars(example = &"same_weekday_last_4_weeks")]
    pub compare_mode: CompareMode,
    /// First day of the comparison. Needed with `custom`, ignored otherwise.
    #[schemars(example = &"2026-10-01")]
    pub compare_from: Option<NaiveDate>,
    /// Last day of the comparison, included; before the period starts. Needed with `custom`.
    #[schemars(example = &"2026-10-31")]
    pub compare_through: Option<NaiveDate>,
    /// The figure to explain. Default `revenue`.
    #[serde(rename = "acom:metric", alias = "metric")]
    #[schemars(example = &"revenue")]
    pub metric: Option<ExplainMetric>,
    /// Causes to list at most, 1–10. Default 5; what the listed causes leave is unexplained.
    #[schemars(range(min = 1, max = 10), example = 5)]
    pub max_causes: Option<u32>,
}

/// Compare a period's sales with a basis and split the difference into causes found in the
/// data, plus what is left unexplained.
#[derive(Debug)]
pub struct ExplainSalesChange;

impl Operation for ExplainSalesChange {
    const META: OpMeta = OpMeta {
        address: "analytics.sales.explain_sales_change",
        tool: "explain_sales_change",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Post,
        path: "/v1/analytics/sales/explain",
        summary: "Explain why sales went up or down: compares a period with a basis (by default the same weekday over the last 4 weeks) and splits the difference into causes the data shows, such as sizes that sold out or a campaign email that was not sent, plus what is left unexplained.",
        consumes: &[],
        produces: &[],
        checks: &[],
        errors: &[INVALID_PERIOD, NO_DATA_FOR_PERIOD],
        audience: Audience::Operator,
        action_type: ActionType::ASSESS,
        person_scope: "",
    };
    type Input = ExplainSalesChangeInput;
    type Output = Value;
}

/// The default and the ceiling of `max_causes` (docs/api/analytics.md).
const DEFAULT_CAUSES: u32 = 5;
const MAX_CAUSES: u32 = 10;

/// Sum of one figure over day rows.
fn sum_by(rows: &[repo::DayTotal], metric: ExplainMetric) -> Decimal {
    rows.iter()
        .map(|r| match metric {
            ExplainMetric::Revenue => r.revenue_value,
            ExplainMetric::OrderCount => Decimal::from(r.order_count),
        })
        .sum()
}

/// One cause found in the data, before the arithmetic.
#[derive(Debug)]
enum Found {
    OutOfStock {
        variants: Vec<StockOut>,
        /// Product id → (period, compare sum) of the figure.
        by_product: BTreeMap<String, (Decimal, Decimal)>,
    },
    CampaignNotSent {
        missed: Vec<CampaignSend>,
        /// Sends of the same template on compare days.
        compared_with: Vec<String>,
    },
}

/// Sizes in the order people read them: XS before S before M; numbers ascending; the rest
/// alphabetically.
fn size_order(size: &str) -> (u8, i64, String) {
    const LETTERS: &[&str] = &["XXS", "XS", "S", "M", "L", "XL", "XXL", "XXXL"];
    if let Some(i) = LETTERS.iter().position(|l| l.eq_ignore_ascii_case(size)) {
        return (0, i as i64, String::new());
    }
    match size.parse::<i64>() {
        Ok(n) => (1, n, String::new()),
        Err(_) => (2, 0, size.to_owned()),
    }
}

/// "M and L", "S, M and L".
fn and_list(words: &[String]) -> String {
    match words {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn lower_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().chain(c).collect(),
        None => String::new(),
    }
}

fn sizes_of<'a>(variants: impl Iterator<Item = &'a StockOut>) -> Vec<String> {
    let set: BTreeSet<String> = variants.filter_map(|v| v.size.clone()).collect();
    let mut sizes: Vec<String> = set.into_iter().collect();
    sizes.sort_by_key(|s| size_order(s));
    sizes
}

impl ReadOp<Cx> for ExplainSalesChange {
    async fn read(&self, cx: &Cx, input: ExplainSalesChangeInput) -> Result<Value, ToolError> {
        const NEXT: &str = "explain_sales_change";
        let period = span(cx, input.valid_from, input.valid_through, NEXT)?;
        let compare = period::compare_days(
            &period,
            input.compare_mode,
            Some((input.compare_from, input.compare_through)),
        )
        .map_err(|p| period_error(p, NEXT))?;
        let metric = input.metric.unwrap_or_default();
        let max_causes = input
            .max_causes
            .unwrap_or(DEFAULT_CAUSES)
            .clamp(1, MAX_CAUSES) as usize;
        let scale = Scale {
            period_days: period.day_count(),
            compare_days: compare.len() as i64,
        };
        let (start, end) = period.instants(cx.time.zone);
        let period_days = period.dates();

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let p_totals = repo::daily_totals(&mut tx, &period_days).await?;
        let c_totals = repo::daily_totals(&mut tx, &compare).await?;

        // Campaign sends that were due in the period and reached nobody, by template: each
        // template's sends are one recurring campaign, so its other sends say what it brings.
        let mut missed_by_template: BTreeMap<String, Vec<CampaignSend>> = BTreeMap::new();
        for c in repo::missed_campaigns(&mut tx, start, end).await? {
            if let Some(t) = c.template_id.clone() {
                missed_by_template.entry(t).or_default().push(c);
            }
        }
        let mut found: Vec<(Found, CauseFigures)> = Vec::new();
        let mut campaign_ids: Vec<String> = Vec::new();
        for (template, missed) in missed_by_template {
            let sent = repo::sent_campaigns_of(&mut tx, &template).await?;
            let ids: Vec<String> = sent.iter().map(|c| c.campaign_id.clone()).collect();
            let actual = repo::campaign_sales(&mut tx, &ids, &period_days).await?;
            let expected = repo::campaign_sales(&mut tx, &ids, &compare).await?;
            let compared_with: Vec<String> = sent
                .iter()
                .filter(|c| {
                    c.date_sent.is_some_and(|t| {
                        compare.contains(&t.with_timezone(&cx.time.zone).date_naive())
                    })
                })
                .map(|c| c.campaign_id.clone())
                .collect();
            campaign_ids.extend(ids);
            campaign_ids.extend(missed.iter().map(|c| c.campaign_id.clone()));
            let figures = CauseFigures {
                actual: sum_by(&actual, metric),
                expected_sum: sum_by(&expected, metric),
            };
            found.push((
                Found::CampaignNotSent {
                    missed,
                    compared_with,
                },
                figures,
            ));
        }

        // Variants that were out of stock in the period: what they sold then against what they
        // sold on the compare days. Orders that came from the campaigns above are left out here,
        // so no sale is counted twice.
        let outs = repo::stock_outs(&mut tx, end).await?;
        if !outs.is_empty() {
            let variants: Vec<String> = outs.iter().map(|o| o.product_id.clone()).collect();
            let in_period =
                repo::variant_sales(&mut tx, &variants, &period_days, &campaign_ids).await?;
            let in_compare =
                repo::variant_sales(&mut tx, &variants, &compare, &campaign_ids).await?;
            let mut by_product: BTreeMap<String, (Decimal, Decimal)> = BTreeMap::new();
            let figures = match metric {
                ExplainMetric::Revenue => {
                    for r in &in_period {
                        by_product.entry(r.product_group_id.clone()).or_default().0 +=
                            r.revenue_value;
                    }
                    for r in &in_compare {
                        by_product.entry(r.product_group_id.clone()).or_default().1 +=
                            r.revenue_value;
                    }
                    CauseFigures {
                        actual: in_period.iter().map(|r| r.revenue_value).sum(),
                        expected_sum: in_compare.iter().map(|r| r.revenue_value).sum(),
                    }
                }
                ExplainMetric::OrderCount => {
                    for r in &in_period {
                        by_product.entry(r.product_group_id.clone()).or_default().0 +=
                            Decimal::from(r.order_count);
                    }
                    for r in &in_compare {
                        by_product.entry(r.product_group_id.clone()).or_default().1 +=
                            Decimal::from(r.order_count);
                    }
                    let count = |rows: Vec<(NaiveDate, i64)>| -> Decimal {
                        rows.iter().map(|(_, n)| Decimal::from(*n)).sum()
                    };
                    CauseFigures {
                        actual: count(
                            repo::variant_orders(&mut tx, &variants, &period_days, &campaign_ids)
                                .await?,
                        ),
                        expected_sum: count(
                            repo::variant_orders(&mut tx, &variants, &compare, &campaign_ids)
                                .await?,
                        ),
                    }
                }
            };
            found.insert(
                0,
                (
                    Found::OutOfStock {
                        variants: outs,
                        by_product,
                    },
                    figures,
                ),
            );
        }
        tx.commit().await.map_err(ToolError::from)?;

        if c_totals.is_empty() {
            return Err(ToolError::new(
                NO_DATA_FOR_PERIOD,
                format!(
                    "There are no sales on the comparison days ({} to {}), so there is nothing to compare with.",
                    compare.first().copied().unwrap_or(period.from),
                    compare.last().copied().unwrap_or(period.through)
                ),
            )
            .next("get_sales_trend")
            .detail(json!({"compare_days": compare})));
        }

        let places = match metric {
            ExplainMetric::Revenue => cx.currency.minor_units(),
            ExplainMetric::OrderCount => 2,
        };
        let figures: Vec<CauseFigures> = found.iter().map(|(_, f)| *f).collect();
        let e = decompose::explain(
            sum_by(&p_totals, metric),
            sum_by(&c_totals, metric),
            scale,
            &figures,
            places,
            max_causes,
        );
        let amount = |v: Decimal| match metric {
            ExplainMetric::Revenue => money_json(cx, v),
            ExplainMetric::OrderCount => number(v),
        };
        let at = |t: DateTime<Utc>, with_date: bool| {
            let local = t.with_timezone(&cx.time.zone);
            if with_date {
                local.format("%a %b %-d %H:%M").to_string()
            } else {
                local.format("%a %H:%M").to_string()
            }
        };

        let mut items = Vec::new();
        for c in &e.causes {
            let (kind, f) = &found[c.index];
            let mut item = Map::new();
            match kind {
                Found::OutOfStock {
                    variants,
                    by_product,
                } => {
                    // Products whose sold-out sizes sold in the period or on the compare days,
                    // the biggest loss first.
                    let mut products: Vec<(&String, Decimal, Decimal)> = by_product
                        .iter()
                        .map(|(id, (a, x))| (id, *a, decompose::round(scale.apply(*x), places)))
                        .filter(|(_, a, x)| !a.is_zero() || !x.is_zero())
                        .collect();
                    products.sort_by(|a, b| (b.2 - b.1).cmp(&(a.2 - a.1)).then(a.0.cmp(b.0)));
                    let ids: BTreeSet<&String> = products.iter().map(|p| p.0).collect();
                    let used: Vec<&StockOut> = variants
                        .iter()
                        .filter(|v| ids.contains(&v.product_group_id))
                        .collect();
                    let since = used.iter().map(|v| v.sold_out_at).min();
                    let sizes = sizes_of(used.iter().copied());
                    let bestsellers = used.iter().all(|v| v.is_bestseller);
                    let n = products.len();
                    let noun = match (bestsellers, n) {
                        (true, 1) => "bestseller",
                        (true, _) => "bestsellers",
                        (false, 1) => "product",
                        (false, _) => "products",
                    };
                    let what = if sizes.is_empty() {
                        "out of stock".to_owned()
                    } else {
                        format!("out of {}", and_list(&sizes))
                    };
                    let detail = match since {
                        Some(t) => format!("{n} {noun} {what} since {}", at(t, true)),
                        None => format!("{n} {noun} {what}"),
                    };
                    let list: Vec<Value> = products
                        .iter()
                        .map(|(id, a, x)| {
                            let vs: Vec<&&StockOut> =
                                used.iter().filter(|v| &v.product_group_id == *id).collect();
                            let back = vs.iter().filter_map(|v| v.availability_starts).max();
                            json!({
                                "product_id": id,
                                "name": vs.first().map(|v| v.product_name.clone()),
                                "sizes": sizes_of(vs.iter().map(|v| **v)),
                                "is_bestseller": vs.iter().all(|v| v.is_bestseller),
                                "sold_out_at": vs.iter().map(|v| v.sold_out_at).min().map(|t| cx.time.format(t)),
                                "availability_starts": back.map(|t| cx.time.format(t)),
                                "expected": amount(*x),
                                "actual": amount(*a),
                            })
                        })
                        .collect();
                    item.insert("cause".into(), json!("out_of_stock"));
                    item.insert("detail".into(), json!(detail));
                    item.insert("amount".into(), amount(c.amount));
                    item.insert(
                        "evidence".into(),
                        json!(products.iter().map(|p| p.0).collect::<Vec<_>>()),
                    );
                    item.insert("expected".into(), amount(c.expected));
                    item.insert("actual".into(), amount(decompose::round(f.actual, places)));
                    item.insert("since".into(), json!(since.map(|t| cx.time.format(t))));
                    item.insert("products".into(), Value::Array(list));
                }
                Found::CampaignNotSent {
                    missed,
                    compared_with,
                } => {
                    let first = &missed[0];
                    let reason = if first.campaign_status == "cancelled" {
                        "cancelled".to_owned()
                    } else {
                        first
                            .error_message
                            .as_deref()
                            .map_or_else(|| "the send failed".to_owned(), lower_first)
                    };
                    let when = first
                        .scheduled_time
                        .map(|t| format!(" {}", at(t, period.day_count() > 7)))
                        .unwrap_or_default();
                    item.insert("cause".into(), json!("campaign_not_sent"));
                    item.insert(
                        "detail".into(),
                        json!(format!("{} not sent: {reason}{when}", first.name)),
                    );
                    item.insert("amount".into(), amount(c.amount));
                    item.insert(
                        "evidence".into(),
                        json!(missed.iter().map(|m| &m.campaign_id).collect::<Vec<_>>()),
                    );
                    item.insert("expected".into(), amount(c.expected));
                    item.insert("actual".into(), amount(decompose::round(f.actual, places)));
                    item.insert(
                        "campaign".into(),
                        json!({
                            "campaign_id": first.campaign_id,
                            "name": first.name,
                            "channel": first.message_channel,
                            "campaign_status": first.campaign_status,
                            "scheduled_time": first.scheduled_time.map(|t| cx.time.format(t)),
                            "error_message": first.error_message,
                            "compared_with": compared_with,
                        }),
                    );
                }
            }
            items.push(Value::Object(item));
        }

        let p_revenue: Decimal = p_totals.iter().map(|r| r.revenue_value).sum();
        let p_orders: i64 = p_totals.iter().map(|r| r.order_count).sum();
        let c_revenue = sum_by(&c_totals, ExplainMetric::Revenue);
        let c_orders = sum_by(&c_totals, ExplainMetric::OrderCount);
        let total = items.len();
        Ok(json!({
            "items": items,
            "total": total,
            "acom:metric": metric.as_str(),
            "period": {
                "valid_from": period.from,
                "valid_through": period.through,
                "revenue": money_json(cx, p_revenue),
                "order_count": p_orders,
            },
            "compare_to": {
                "basis": input.compare_mode.as_str(),
                "observation_dates": compare,
                "revenue": money_json(cx, decompose::round(scale.apply(c_revenue), cx.currency.minor_units())),
                "order_count": number(decompose::round(scale.apply(c_orders), 2)),
            },
            "gap": amount(e.gap),
            "change_rate": e.change_rate.map(number),
            "unexplained": amount(e.unexplained),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_small_to_large_and_join_with_and() {
        let mut sizes = vec!["L".to_owned(), "M".to_owned(), "XS".to_owned()];
        sizes.sort_by_key(|s| size_order(s));
        assert_eq!(and_list(&sizes), "XS, M and L");
        assert_eq!(and_list(&sizes[1..]), "M and L");
        assert_eq!(and_list(&["9".to_owned()]), "9");
        assert!(size_order("10") > size_order("9"));
        assert_eq!(lower_first("List import failed"), "list import failed");
    }
}
