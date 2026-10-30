//! What `analytics` reads from sales that its views cannot give without naming people: how many
//! members bought in a period, and how many of them bought more than once. Counts only.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};

/// Members who placed paid, not cancelled orders in a period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemberActivity {
    /// Members with at least one such order.
    pub active: i64,
    /// Members with two or more.
    pub repeat: i64,
}

/// Counts over orders placed in `[from, until)`; guest orders are left out.
pub async fn member_activity(
    tx: &mut TxCx,
    from: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Result<MemberActivity, ToolError> {
    let (active, repeat): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE orders >= 2) FROM (
           SELECT customer_id, count(*) AS orders FROM sales.sales_order
           WHERE tenant_id = $1 AND customer_id IS NOT NULL
             AND order_status NOT IN ('payment_due', 'cancelled')
             AND order_date >= $2 AND order_date < $3
           GROUP BY customer_id) AS m",
    )
    .bind(tx.tenant.as_str())
    .bind(from)
    .bind(until)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(|e| ToolError::from(StoreError::from(e)))?;
    Ok(MemberActivity { active, repeat })
}
