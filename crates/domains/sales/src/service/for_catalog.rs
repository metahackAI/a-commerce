//! What catalog needs from sales: the lines of a shopper's own order, so a review can be
//! accepted only for something they received (`catalog.yml review.verified_purchase_only`).

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};

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
