//! Where an order goes, as the shopper chose it at checkout. Orders have no address columns:
//! the address is recorded in the order's first event (`sales.order_event.result.delivery`),
//! and fulfillment reads it here to schedule the shipment.

use acom_contract::ToolError;
use acom_store::TxCx;
use serde_json::Value;

use crate::repo;

/// `{address_id, delivery_address: {name, street_address, address_locality, address_region,
/// postal_code, address_country}, address_label, arrives}` as recorded when the order was
/// placed; `None` for orders placed before this was recorded.
pub async fn delivery(tx: &mut TxCx, order_id: &str) -> Result<Option<Value>, ToolError> {
    Ok(repo::placed_record(tx, order_id)
        .await?
        .and_then(|r| r.get("delivery").cloned())
        .filter(|d| !d.is_null()))
}
