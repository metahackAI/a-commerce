//! Where an order goes. Orders have no address columns: the address is recorded in the order's
//! events (`sales.order_event.result.delivery`): the first one when the order is placed, a later
//! one when the shopper changes the address before it ships (`change_shipping_address`) or a
//! gift's recipient gives it when accepting. Fulfillment reads it here to schedule the shipment.

use acom_contract::ToolError;
use acom_store::TxCx;
use serde_json::Value;

use crate::repo;

/// `{address_id, delivery_address: {name, street_address, address_locality, address_region,
/// postal_code, address_country}, address_label, arrives}` as the latest event recorded it;
/// `None` for orders placed before this was recorded, and for a gift still waiting for its
/// recipient's address.
pub async fn delivery(tx: &mut TxCx, order_id: &str) -> Result<Option<Value>, ToolError> {
    repo::latest_delivery(tx, order_id).await
}
