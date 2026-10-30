//! What customer reads from fulfillment: a member's return claims, for `get_return_history`.
//! Customer cannot depend on fulfillment (fulfillment depends on customer), so fulfillment
//! installs this reader into `acom_customer::service::providers` at start-up: call
//! [`install`] from this crate's `register`.

use acom_contract::{ErrorCode, ToolError};
use acom_core::{Currency, Money};
use acom_customer::service::providers::{self, Reading, ReturnClaim};
use acom_sales::service::for_fulfillment as sales;
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

/// Lets `get_return_history` read return claims.
pub fn install() {
    providers::provide_return_claims(return_claims);
}

#[derive(Debug, sqlx::FromRow)]
struct ClaimRow {
    id: String,
    claim_type: String,
    order_id: String,
    claim_reason: Option<String>,
    claim_status: String,
    amount_value: Decimal,
    amount_currency: String,
    start_time: DateTime<Utc>,
}

/// A customer's return claims requested at or after `since`, newest first, with the order
/// number and the names of the returned items read from sales.
pub fn return_claims<'a>(
    tx: &'a mut TxCx,
    customer_id: &'a str,
    since: DateTime<Utc>,
) -> Reading<'a, Vec<ReturnClaim>> {
    Box::pin(async move {
        let rows = sqlx::query_as::<_, ClaimRow>(
            "SELECT id, claim_type, order_id, claim_reason, claim_status, amount_value,
                    trim(amount_currency) AS amount_currency, start_time
             FROM fulfillment.claim
             WHERE tenant_id = $1 AND customer_id = $2 AND claim_type = 'return'
               AND start_time >= $3
             ORDER BY start_time DESC, id",
        )
        .bind(tx.tenant.as_str())
        .bind(customer_id)
        .bind(since)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(|e| ToolError::from(StoreError::from(e)))?;
        let claim_ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
        let lines: Vec<(String, String)> = sqlx::query_as(
            "SELECT claim_id, order_item_id FROM fulfillment.claim_item
             WHERE tenant_id = $1 AND claim_id = ANY($2) ORDER BY claim_id, order_item_id",
        )
        .bind(tx.tenant.as_str())
        .bind(&claim_ids)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(|e| ToolError::from(StoreError::from(e)))?;
        let order_ids: Vec<String> = rows.iter().map(|r| r.order_id.clone()).collect();
        let orders = sales::orders(tx, &order_ids).await?;
        let line_ids: Vec<String> = lines.iter().map(|(_, l)| l.clone()).collect();
        let items = sales::items(tx, &line_ids).await?;
        let item_names = |claim: &str| -> String {
            let names: Vec<&str> = lines
                .iter()
                .filter(|(c, _)| c == claim)
                .filter_map(|(_, l)| items.iter().find(|i| &i.id == l))
                .map(|i| i.name.as_str())
                .collect();
            names.join(" + ")
        };
        rows.into_iter()
            .map(|r| {
                let currency = Currency::parse(&r.amount_currency).map_err(|e| {
                    ToolError::new(ErrorCode::INTERNAL, format!("claim {}: {e}", r.id))
                })?;
                let order_number = orders
                    .iter()
                    .find(|o| o.id == r.order_id)
                    .map(|o| o.order_number.clone())
                    .unwrap_or_default();
                let item = item_names(&r.id);
                Ok(ReturnClaim {
                    amount: Money::new(r.amount_value, currency),
                    id: r.id,
                    claim_type: r.claim_type,
                    order_id: r.order_id,
                    order_number,
                    item,
                    reason: r.claim_reason,
                    status: r.claim_status,
                    requested_at: r.start_time,
                })
            })
            .collect()
    })
}
