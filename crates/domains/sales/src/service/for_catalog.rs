//! What catalog needs from sales: the lines of a shopper's own order, so a review can be
//! accepted only for something they received (`catalog.yml review.verified_purchase_only`); and
//! the units each variant sold lately, so a reorder can be sized (`suggest_reorder`).

use std::collections::BTreeMap;

use acom_contract::{ErrorCode, ToolError};
use acom_store::{StoreError, TxCx};
use chrono::{Days, NaiveDate};

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// One line of an order (`sales.order_item`).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct OrderLine {
    pub id: String,
    /// The variant bought (`var_`).
    pub product_id: String,
    pub order_item_status: String,
}

/// The lines of `order_id` when it is this customer's order; empty otherwise.
pub async fn order_lines(
    tx: &mut TxCx,
    customer_id: &str,
    order_id: &str,
) -> Result<Vec<OrderLine>, ToolError> {
    sqlx::query_as::<_, OrderLine>(
        "SELECT i.id, i.product_id, i.order_item_status
         FROM sales.order_item i
         JOIN sales.sales_order o ON o.tenant_id = i.tenant_id AND o.id = i.order_id
         WHERE i.tenant_id = $1 AND i.order_id = $2 AND o.customer_id = $3
         ORDER BY i.position",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .bind(customer_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The last whole store days, today left out, over which units sold are counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SalesWindow {
    pub valid_from: NaiveDate,
    /// The last day counted (yesterday).
    pub valid_through: NaiveDate,
    pub days: u32,
}

/// Units each of `variant_ids` sold on orders placed in the last `days` whole store days (not
/// cancelled lines), with the window counted. Variants that sold nothing are left out. The
/// stock-out watcher reads the same numbers, so a reorder suggestion and the task that asked for
/// it agree.
pub async fn units_sold(
    tx: &mut TxCx,
    variant_ids: &[String],
    days: u32,
) -> Result<(SalesWindow, BTreeMap<String, i64>), ToolError> {
    let internal = |m: String| ToolError::new(ErrorCode::INTERNAL, m);
    let today = tx.clock.now().with_timezone(&tx.time.zone).date_naive();
    let from = today
        .checked_sub_days(Days::new(u64::from(days)))
        .ok_or_else(|| internal("the sales window reaches before the calendar".into()))?;
    let day_start = |d: NaiveDate| {
        tx.time
            .parse(&format!("{d}T00:00"))
            .map_err(|e| internal(e.to_string()))
    };
    let (start, end) = (day_start(from)?, day_start(today)?);
    let sold = sqlx::query_as::<_, (String, i64)>(
        "SELECT i.product_id, sum(i.order_quantity)::bigint
         FROM sales.order_item i
         JOIN sales.sales_order o ON o.tenant_id = i.tenant_id AND o.id = i.order_id
         WHERE i.tenant_id = $1 AND o.order_date >= $2 AND o.order_date < $3
           AND i.order_item_status <> 'cancelled' AND i.product_id = ANY($4)
         GROUP BY i.product_id",
    )
    .bind(tx.tenant.as_str())
    .bind(start)
    .bind(end)
    .bind(variant_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?
    .into_iter()
    .collect();
    let window = SalesWindow {
        valid_from: from,
        valid_through: today.pred_opt().unwrap_or(today),
        days,
    };
    Ok((window, sold))
}
