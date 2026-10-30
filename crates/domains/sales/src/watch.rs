//! Watcher `stock_out_forecast` (`policies/platform.yml watchers`): products that, at the rate
//! they sold over the last whole store days, run out within the policy's number of days. The
//! sales rate is this domain's data (order lines); the stock comes from catalog's exported
//! `product_stock`. One finding per product, so each gets its own task.

use std::collections::BTreeMap;

use acom_catalog::service::for_sales::{ProductStock, product_stock};
use acom_contract::{EntityRef, ErrorCode, Job, JobMeta, JobOutcome, TaskDraft, ToolError};
use acom_store::{Cx, StoreError, TxCx};
use chrono::Days;
use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::{Value, json};

#[derive(Debug)]
pub struct StockOutForecast;

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

fn internal(message: String) -> ToolError {
    ToolError::new(ErrorCode::INTERNAL, message)
}

impl Job<Cx> for StockOutForecast {
    const META: JobMeta = JobMeta {
        kind: "watcher.stock_out_forecast",
        summary: "Products that run out within a few days at their recent sales rate.",
        watcher: Some("stock_out_forecast"),
    };

    async fn run(&self, tx: &mut TxCx, _: &Value) -> Result<JobOutcome, ToolError> {
        let w = tx
            .policy
            .watchers()
            .map_err(|e| internal(e.to_string()))?
            .stock_out_forecast;
        // The last `lookback_days` whole store days, today left out.
        let today = tx.clock.now().with_timezone(&tx.time.zone).date_naive();
        let from = today
            .checked_sub_days(Days::new(u64::from(w.lookback_days)))
            .ok_or_else(|| internal("the lookback reaches before the calendar".into()))?;
        let day_start = |d: chrono::NaiveDate| {
            tx.time
                .parse(&format!("{d}T00:00"))
                .map_err(|e| internal(e.to_string()))
        };
        let (from, through) = (day_start(from)?, day_start(today)?);
        let stock = product_stock(tx, &w.exclude_availability).await?;
        let variants: Vec<String> = stock
            .iter()
            .flat_map(|p| p.variant_ids.iter().cloned())
            .collect();
        let sold: BTreeMap<String, i64> = sqlx::query_as::<_, (String, i64)>(
            "SELECT i.product_id, sum(i.order_quantity)::bigint
             FROM sales.order_item i
             JOIN sales.sales_order o ON o.tenant_id = i.tenant_id AND o.id = i.order_id
             WHERE i.tenant_id = $1 AND o.order_date >= $2 AND o.order_date < $3
               AND i.order_item_status <> 'cancelled' AND i.product_id = ANY($4)
             GROUP BY i.product_id",
        )
        .bind(tx.tenant.as_str())
        .bind(from)
        .bind(through)
        .bind(&variants)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?
        .into_iter()
        .collect();
        let drafts = stock
            .iter()
            .filter_map(|p| {
                let units = p.variant_ids.iter().filter_map(|v| sold.get(v)).sum();
                finding(p, units, w.lookback_days, w.days_until_stockout_lte)
            })
            .collect();
        Ok(JobOutcome::Watched(drafts))
    }
}

/// A JSON number with exactly the digits of `d`.
fn num(d: Decimal) -> Value {
    d.to_string()
        .parse::<serde_json::Number>()
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

/// Days of stock left at the rate of `sold` units over `lookback_days`, to one decimal; `None`
/// when nothing sold.
pub(crate) fn days_left(stock: i64, sold: i64, lookback_days: u32) -> Option<Decimal> {
    (sold > 0).then(|| {
        (Decimal::from(stock) * Decimal::from(lookback_days) / Decimal::from(sold))
            .round_dp_with_strategy(1, RoundingStrategy::MidpointAwayFromZero)
    })
}

/// What the watcher reports for one product: `None` unless it sells out within `lte` days.
/// Compared exactly (stock × days ≤ limit × sold); only the reported number is rounded.
pub(crate) fn finding(
    p: &ProductStock,
    sold: i64,
    lookback_days: u32,
    lte: Decimal,
) -> Option<TaskDraft> {
    if sold <= 0 {
        return None;
    }
    let over = Decimal::from(p.inventory_level) * Decimal::from(lookback_days);
    if over > lte * Decimal::from(sold) {
        return None;
    }
    let days = days_left(p.inventory_level, sold, lookback_days)?;
    let whole = days
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        .normalize();
    let title = if whole.is_zero() {
        format!("{} sells out within a day", p.name)
    } else if whole == Decimal::ONE {
        format!("{} sells out in about 1 day", p.name)
    } else {
        format!("{} sells out in about {whole} days", p.name)
    };
    Some(TaskDraft {
        cause: Some(EntityRef::new("product", &p.product_id)),
        title,
        summary: format!(
            "{} left; {sold} sold in the last {lookback_days} days",
            p.inventory_level
        ),
        tag: "inventory".into(),
        request_text: format!(
            "{} is selling fast. Draft a reorder for the sizes that will run out.",
            p.name
        ),
        observation: json!({
            "threshold": {"policy_key": "watchers.stock_out_forecast.days_until_stockout_lte",
                          "value": num(lte.normalize())},
            "observed": {
                "product_id": p.product_id,
                "inventory_level": p.inventory_level,
                format!("sold_last_{lookback_days}_days"): sold,
                "days_until_stockout": num(days),
            },
        }),
        related: vec![EntityRef::new("product", &p.product_id)],
        fix: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linen(stock: i64) -> ProductStock {
        ProductStock {
            product_id: "prd_linen_shirt".into(),
            name: "Linen shirt".into(),
            inventory_level: stock,
            variant_ids: vec!["var_linen_shirt_m_white".into()],
        }
    }

    #[test]
    fn the_linen_shirt_sells_out_in_about_four_days() {
        let d = finding(&linen(38), 61, 7, Decimal::from(5)).unwrap();
        assert_eq!(d.title, "Linen shirt sells out in about 4 days");
        assert_eq!(
            d.observation.to_string(),
            r#"{"threshold":{"policy_key":"watchers.stock_out_forecast.days_until_stockout_lte","value":5},"observed":{"product_id":"prd_linen_shirt","inventory_level":38,"sold_last_7_days":61,"days_until_stockout":4.4}}"#
        );
        assert_eq!(
            d.request_text,
            "Linen shirt is selling fast. Draft a reorder for the sizes that will run out."
        );
        assert_eq!(d.cause, Some(EntityRef::new("product", "prd_linen_shirt")));
    }

    #[test]
    fn trips_at_the_limit_and_not_past_it() {
        // 50 units at 70 a week: exactly 5 days.
        assert!(finding(&linen(50), 70, 7, Decimal::from(5)).is_some());
        assert!(finding(&linen(51), 70, 7, Decimal::from(5)).is_none());
        assert!(finding(&linen(0), 0, 7, Decimal::from(5)).is_none());
        assert_eq!(
            finding(&linen(1), 70, 7, Decimal::from(5)).unwrap().title,
            "Linen shirt sells out within a day"
        );
        assert_eq!(days_left(38, 61, 7), Some(Decimal::new(44, 1)));
        assert_eq!(days_left(5, 0, 7), None);
    }
}
