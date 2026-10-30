//! What sales reads from fulfillment: an order's parcel delivery, for `get_order`. Sales cannot
//! depend on fulfillment (fulfillment depends on sales), so fulfillment installs this reader into
//! `acom_sales::service::providers` at start-up: call [`install`] from this crate's `register`.

use acom_contract::ToolError;
use acom_sales::service::providers::{self, DeliveryFacts, Reading};
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};
use serde_json::Value;

/// Lets `get_order` show the parcel.
pub fn install() {
    providers::provide_deliveries(order_delivery);
}

/// The latest parcel delivery of an order, if there is one.
pub fn order_delivery<'a>(
    tx: &'a mut TxCx,
    order_id: &'a str,
) -> Reading<'a, Option<DeliveryFacts>> {
    Box::pin(async move {
        #[allow(clippy::type_complexity)]
        let row: Option<(
            String,
            Value,
            Option<DateTime<Utc>>,
            String,
            Option<DateTime<Utc>>,
        )> = sqlx::query_as(
            "SELECT id, delivery_address, expected_arrival_until, delivery_state, delivered_at
             FROM fulfillment.parcel_delivery
             WHERE tenant_id = $1 AND order_id = $2
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(tx.tenant.as_str())
        .bind(order_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(|e| ToolError::from(StoreError::from(e)))?;
        Ok(row.map(
            |(id, delivery_address, expected_arrival_until, delivery_state, delivered_at)| {
                DeliveryFacts {
                    id,
                    delivery_address,
                    expected_arrival_until,
                    delivery_state,
                    delivered_at,
                }
            },
        ))
    })
}
