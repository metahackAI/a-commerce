//! What `partner` reads from sales for a seller's dashboard, grade and settlement: the order
//! lines of the seller's variants, and how many orders with such lines are still open. Partner
//! knows which variants a seller sells; sales knows the orders.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// One order line of a seller's variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SellerLine {
    pub order_id: String,
    pub order_number: String,
    pub order_date: DateTime<Utc>,
    /// The order's `order_status` word.
    pub order_status: String,
    /// When the shopper confirmed the purchase (or it was confirmed automatically); `None` before.
    pub confirmed_at: Option<DateTime<Utc>>,
    pub order_item_id: String,
    /// The variant (`var_…`).
    pub variant_id: String,
    pub order_quantity: i32,
    /// The line's `order_item_status` word.
    pub order_item_status: String,
    /// Unit price.
    pub price: Decimal,
    pub price_currency: String,
    /// Discount on the whole line, in the same currency.
    pub discount_value: Decimal,
}

/// Lines of these variants in orders placed in `[from, until)`, oldest order first.
pub async fn lines(
    tx: &mut TxCx,
    variant_ids: &[String],
    from: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Result<Vec<SellerLine>, ToolError> {
    if variant_ids.is_empty() {
        return Ok(Vec::new());
    }
    type Row = (
        String,
        String,
        DateTime<Utc>,
        String,
        Option<DateTime<Utc>>,
        String,
        String,
        i32,
        String,
        Decimal,
        String,
        Decimal,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT o.id, o.order_number, o.order_date, o.order_status, o.confirmed_at,
                i.id, i.product_id, i.order_quantity, i.order_item_status, i.price,
                i.price_currency, i.discount_value
         FROM sales.order_item i
         JOIN sales.sales_order o ON o.tenant_id = i.tenant_id AND o.id = i.order_id
         WHERE i.tenant_id = $1 AND i.product_id = ANY($2)
           AND o.order_date >= $3 AND o.order_date < $4
         ORDER BY o.order_date, o.id, i.position",
    )
    .bind(tx.tenant.as_str())
    .bind(variant_ids)
    .bind(from)
    .bind(until)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(rows
        .into_iter()
        .map(
            |(
                order_id,
                order_number,
                order_date,
                order_status,
                confirmed_at,
                order_item_id,
                variant_id,
                order_quantity,
                order_item_status,
                price,
                price_currency,
                discount_value,
            )| SellerLine {
                order_id,
                order_number,
                order_date,
                order_status,
                confirmed_at,
                order_item_id,
                variant_id,
                order_quantity,
                order_item_status,
                price,
                price_currency,
                discount_value,
            },
        )
        .collect())
}

/// Orders with at least one line of these variants that is not finished yet (waiting for
/// payment, being prepared, on its way, or held with a problem).
pub async fn open_order_count(tx: &mut TxCx, variant_ids: &[String]) -> Result<i64, ToolError> {
    if variant_ids.is_empty() {
        return Ok(0);
    }
    sqlx::query_scalar(
        "SELECT count(DISTINCT i.order_id) FROM sales.order_item i
         WHERE i.tenant_id = $1 AND i.product_id = ANY($2)
           AND i.order_item_status IN ('payment_due', 'processing', 'in_transit', 'problem')",
    )
    .bind(tx.tenant.as_str())
    .bind(variant_ids)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}
