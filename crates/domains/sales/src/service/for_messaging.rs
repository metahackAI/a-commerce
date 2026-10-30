//! What `messaging` reads from sales: who an order belongs to and the number people see, so a
//! customer message is only ever about that customer's own order.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};

/// The order a message is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderOwner {
    pub id: String,
    pub order_number: String,
    /// `None` for a guest order.
    pub customer_id: Option<String>,
}

/// One order's number and owner, or `None` when the store has no such order.
pub async fn order_owner(tx: &mut TxCx, order_id: &str) -> Result<Option<OrderOwner>, ToolError> {
    let row: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, order_number, customer_id FROM sales.sales_order
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(|e| ToolError::from(StoreError::from(e)))?;
    Ok(row.map(|(id, order_number, customer_id)| OrderOwner {
        id,
        order_number,
        customer_id,
    }))
}
