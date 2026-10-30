//! SQL for carts, orders, order items, payments and order events. Every query filters by store.

use acom_contract::{ErrorCode, ToolError};
use acom_core::{Currency, Money};
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

pub(crate) fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

pub(crate) fn money(value: Decimal, currency: &str) -> Result<Money, ToolError> {
    let c = Currency::parse(currency.trim())
        .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?;
    Ok(acom_promotion::service::for_sales::money(value, c))
}

// ---------------------------------------------------------------- carts

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct CartRow {
    pub id: String,
    pub customer_id: Option<String>,
    pub session_id: Option<String>,
    pub delivery_address_id: Option<String>,
    pub payment_method: Option<String>,
    pub version: i32,
}

const CART_COLUMNS: &str =
    "id, customer_id, session_id, delivery_address_id, payment_method, version";

/// The shopper's cart (the most recently changed one, should there be several).
pub(crate) async fn customer_cart(
    tx: &mut TxCx,
    customer_id: &str,
) -> Result<Option<CartRow>, ToolError> {
    sqlx::query_as::<_, CartRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {CART_COLUMNS} FROM sales.cart WHERE tenant_id = $1 AND customer_id = $2
         ORDER BY updated_at DESC, id LIMIT 1"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The guest cart of a browser session (one nobody has signed in to yet).
pub(crate) async fn session_cart(
    tx: &mut TxCx,
    session_id: &str,
) -> Result<Option<CartRow>, ToolError> {
    sqlx::query_as::<_, CartRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {CART_COLUMNS} FROM sales.cart
         WHERE tenant_id = $1 AND session_id = $2 AND customer_id IS NULL
         ORDER BY updated_at DESC, id LIMIT 1"
    )))
    .bind(tx.tenant.as_str())
    .bind(session_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

pub(crate) async fn cart_by_id(tx: &mut TxCx, id: &str) -> Result<Option<CartRow>, ToolError> {
    sqlx::query_as::<_, CartRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {CART_COLUMNS} FROM sales.cart WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct LineRow {
    pub id: String,
    pub cart_id: String,
    /// The variant (`var_…`).
    pub product_id: String,
    pub order_quantity: i32,
    pub position: i32,
    pub description: Option<String>,
    pub version: i32,
}

/// The lines of a cart, in cart order.
pub(crate) async fn lines(tx: &mut TxCx, cart_id: &str) -> Result<Vec<LineRow>, ToolError> {
    sqlx::query_as::<_, LineRow>(
        "SELECT id, cart_id, product_id, order_quantity, position, description, version
         FROM sales.cart_line WHERE tenant_id = $1 AND cart_id = $2 ORDER BY position, id",
    )
    .bind(tx.tenant.as_str())
    .bind(cart_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Creates a cart unless it exists.
pub(crate) async fn insert_cart(
    tx: &mut TxCx,
    id: &str,
    customer_id: Option<&str>,
    session_id: Option<&str>,
    sales_channel: &str,
) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO sales.cart (tenant_id, id, customer_id, session_id, sales_channel)
         VALUES ($1, $2, $3, $4, $5) ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(customer_id)
    .bind(session_id)
    .bind(sales_channel)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Hands a guest cart to the shopper who just signed in.
pub(crate) async fn adopt_cart(
    tx: &mut TxCx,
    id: &str,
    customer_id: &str,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE sales.cart SET customer_id = $3, session_id = NULL, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(customer_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Marks a cart changed.
pub(crate) async fn touch_cart(tx: &mut TxCx, id: &str) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE sales.cart SET updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Puts a variant in a cart, or sets its quantity when the cart already has it. Returns the id
/// of the line that holds it.
pub(crate) async fn upsert_line(
    tx: &mut TxCx,
    id: &str,
    cart_id: &str,
    variant_id: &str,
    quantity: u32,
    position: u32,
) -> Result<String, ToolError> {
    sqlx::query_scalar(
        "INSERT INTO sales.cart_line (tenant_id, id, cart_id, product_id, order_quantity, position)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (tenant_id, cart_id, product_id) DO UPDATE
           SET order_quantity = EXCLUDED.order_quantity, updated_at = now(),
               version = sales.cart_line.version + 1
         RETURNING id",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(cart_id)
    .bind(variant_id)
    .bind(i32::try_from(quantity).unwrap_or(i32::MAX))
    .bind(i32::try_from(position).unwrap_or(i32::MAX))
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Sets a line's variant and quantity.
pub(crate) async fn update_line(
    tx: &mut TxCx,
    id: &str,
    variant_id: &str,
    quantity: u32,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE sales.cart_line SET product_id = $3, order_quantity = $4, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(variant_id)
    .bind(i32::try_from(quantity).unwrap_or(i32::MAX))
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Moves a guest line into the shopper's cart.
pub(crate) async fn move_line(
    tx: &mut TxCx,
    id: &str,
    cart_id: &str,
    position: u32,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE sales.cart_line SET cart_id = $3, position = $4, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(cart_id)
    .bind(i32::try_from(position).unwrap_or(i32::MAX))
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

pub(crate) async fn delete_lines(tx: &mut TxCx, ids: &[String]) -> Result<u64, ToolError> {
    if ids.is_empty() {
        return Ok(0);
    }
    let done = sqlx::query("DELETE FROM sales.cart_line WHERE tenant_id = $1 AND id = ANY($2)")
        .bind(tx.tenant.as_str())
        .bind(ids)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    Ok(done.rows_affected())
}

/// Puts a removed line back, as it was.
pub(crate) async fn restore_line(tx: &mut TxCx, l: &LineSnapshot) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO sales.cart_line (tenant_id, id, cart_id, product_id, order_quantity,
           position, description)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(&l.id)
    .bind(&l.cart_id)
    .bind(&l.variant_id)
    .bind(i32::try_from(l.quantity).unwrap_or(i32::MAX))
    .bind(i32::try_from(l.position).unwrap_or(i32::MAX))
    .bind(l.note.as_deref())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// A cart line as a plan keeps it, so a removal can be undone.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct LineSnapshot {
    pub id: String,
    pub cart_id: String,
    pub variant_id: String,
    pub quantity: u32,
    pub position: u32,
    pub note: Option<String>,
}

impl From<&LineRow> for LineSnapshot {
    fn from(l: &LineRow) -> Self {
        Self {
            id: l.id.clone(),
            cart_id: l.cart_id.clone(),
            variant_id: l.product_id.clone(),
            quantity: u32::try_from(l.order_quantity).unwrap_or(0),
            position: u32::try_from(l.position).unwrap_or(0),
            note: l.description.clone(),
        }
    }
}

/// The quantity a line has now, if it exists.
pub(crate) async fn line_quantity(
    tx: &mut TxCx,
    id: &str,
) -> Result<Option<(String, String, i32)>, ToolError> {
    sqlx::query_as(
        "SELECT cart_id, product_id, order_quantity FROM sales.cart_line
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

// ---------------------------------------------------------------- orders

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct OrderRow {
    pub id: String,
    pub order_number: String,
    pub order_date: DateTime<Utc>,
    pub order_status: String,
    pub customer_id: Option<String>,
    pub sales_channel: String,
    pub discount_code: Option<String>,
    pub coupon_id: Option<String>,
    pub subtotal_value: Decimal,
    pub subtotal_currency: String,
    pub shipping_value: Decimal,
    pub shipping_currency: String,
    pub total_payment_due_value: Decimal,
    pub total_payment_due_currency: String,
    /// The wrap fee of a wrapped gift.
    pub gift_wrap_value: Option<Decimal>,
    pub gift_wrap_currency: Option<String>,
    /// `{card_message, gift_receipt, …, recipient, recipient_mode}` of a gift; `None` otherwise.
    pub gift_options: Option<Value>,
}

const ORDER_COLUMNS: &str = "id, order_number, order_date, order_status, customer_id,
    sales_channel, discount_code, coupon_id, subtotal_value,
    subtotal_currency::text AS subtotal_currency, shipping_value,
    shipping_currency::text AS shipping_currency, total_payment_due_value,
    total_payment_due_currency::text AS total_payment_due_currency, gift_wrap_value,
    gift_wrap_currency::text AS gift_wrap_currency, gift_options";

pub(crate) async fn order(tx: &mut TxCx, id: &str) -> Result<Option<OrderRow>, ToolError> {
    sqlx::query_as::<_, OrderRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ORDER_COLUMNS} FROM sales.sales_order WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Filters for order lists. `None` means no filter.
#[derive(Debug, Clone, Default)]
pub(crate) struct OrderFilter {
    pub customer_id: Option<String>,
    /// The number people see (`20402`).
    pub order_number: Option<String>,
    /// Orders of any of these customers.
    pub customer_ids: Option<Vec<String>>,
    pub statuses: Vec<String>,
    pub from: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    /// A fixed `ORDER BY` clause chosen from a word list.
    pub order_by: &'static str,
    pub limit: i64,
    pub offset: i64,
}

const ORDER_FILTER: &str = "tenant_id = $1
    AND ($2::text IS NULL OR customer_id = $2)
    AND (cardinality($3::text[]) = 0 OR order_status = ANY($3))
    AND ($4::timestamptz IS NULL OR order_date >= $4)
    AND ($5::timestamptz IS NULL OR order_date < $5)
    AND ($6::text IS NULL OR order_number = $6)
    AND ($7::text[] IS NULL OR customer_id = ANY($7))";

/// One page of orders and the total that match.
pub(crate) async fn orders(
    tx: &mut TxCx,
    f: &OrderFilter,
) -> Result<(Vec<OrderRow>, i64), ToolError> {
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM sales.sales_order WHERE {ORDER_FILTER}"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.customer_id.as_deref())
    .bind(&f.statuses)
    .bind(f.from)
    .bind(f.until)
    .bind(f.order_number.as_deref())
    .bind(f.customer_ids.as_deref())
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, OrderRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ORDER_COLUMNS} FROM sales.sales_order WHERE {ORDER_FILTER}
         ORDER BY {} LIMIT $8 OFFSET $9",
        f.order_by
    )))
    .bind(tx.tenant.as_str())
    .bind(f.customer_id.as_deref())
    .bind(&f.statuses)
    .bind(f.from)
    .bind(f.until)
    .bind(f.order_number.as_deref())
    .bind(f.customer_ids.as_deref())
    .bind(f.limit)
    .bind(f.offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((rows, total))
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ItemRow {
    pub id: String,
    pub order_id: String,
    /// The variant (`var_…`).
    pub product_id: String,
    pub name: String,
    pub order_quantity: i32,
    pub order_item_status: String,
    pub price: Decimal,
    pub price_currency: String,
    pub discount_value: Decimal,
    pub discount_currency: String,
    pub promotion_id: Option<String>,
}

/// The items of these orders, in order and line order.
pub(crate) async fn items(tx: &mut TxCx, order_ids: &[String]) -> Result<Vec<ItemRow>, ToolError> {
    if order_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, ItemRow>(
        "SELECT id, order_id, product_id, name, order_quantity, order_item_status,
                price, price_currency::text AS price_currency, discount_value,
                discount_currency::text AS discount_currency, promotion_id
         FROM sales.order_item WHERE tenant_id = $1 AND order_id = ANY($2)
         ORDER BY order_id, position",
    )
    .bind(tx.tenant.as_str())
    .bind(order_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// When each of these orders was last delivered (the latest event into `delivered`).
pub(crate) async fn delivered_at(
    tx: &mut TxCx,
    order_ids: &[String],
) -> Result<Vec<(String, DateTime<Utc>)>, ToolError> {
    if order_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as(
        "SELECT order_id, max(start_time) FROM sales.order_event
         WHERE tenant_id = $1 AND order_id = ANY($2) AND to_state = 'delivered'
         GROUP BY order_id",
    )
    .bind(tx.tenant.as_str())
    .bind(order_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// What the order recorded when it was placed (the first event's result), if anything.
pub(crate) async fn placed_record(
    tx: &mut TxCx,
    order_id: &str,
) -> Result<Option<Value>, ToolError> {
    let v: Option<Option<Value>> = sqlx::query_scalar(
        "SELECT result FROM sales.order_event
         WHERE tenant_id = $1 AND order_id = $2 ORDER BY position LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(v.flatten())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct PaymentRow {
    pub id: String,
    pub order_id: String,
    pub payment_method: String,
    pub payment_status: String,
    pub amount_value: Decimal,
    pub amount_currency: String,
    pub card_last4: Option<String>,
    pub installment_months: i32,
}

/// The order's payment (the first one, should there be several).
pub(crate) async fn payment(
    tx: &mut TxCx,
    order_id: &str,
) -> Result<Option<PaymentRow>, ToolError> {
    sqlx::query_as::<_, PaymentRow>(
        "SELECT id, order_id, payment_method, payment_status, amount_value,
                amount_currency::text AS amount_currency, card_last4, installment_months
         FROM sales.payment WHERE tenant_id = $1 AND order_id = $2
         ORDER BY created_at, id LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The next order number of the store. Holds a transaction lock so two orders placed at the
/// same moment get different numbers.
pub(crate) async fn next_order_number(tx: &mut TxCx) -> Result<String, ToolError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("{}:sales.order_number", tx.tenant.as_str()))
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    let next: i64 = sqlx::query_scalar(
        "SELECT coalesce(max(order_number::bigint), 0) + 1 FROM sales.sales_order
         WHERE tenant_id = $1",
    )
    .bind(tx.tenant.as_str())
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(next.to_string())
}

/// A new order row.
#[derive(Debug, Clone)]
pub(crate) struct NewOrder<'a> {
    pub id: &'a str,
    pub order_number: &'a str,
    pub order_date: DateTime<Utc>,
    pub order_status: &'a str,
    pub customer_id: &'a str,
    pub sales_channel: &'a str,
    pub discount_code: Option<&'a str>,
    pub coupon_id: Option<&'a str>,
    pub subtotal: Money,
    pub discount: Money,
    pub shipping: Money,
    pub total: Money,
    pub preview_id: &'a str,
    pub is_gift: bool,
    /// `{card_message, gift_receipt, receipt_hides_prices, exchange_until, recipient}` of a gift.
    pub gift_options: Option<&'a Value>,
    pub gift_wrap: Option<Money>,
}

pub(crate) async fn insert_order(tx: &mut TxCx, o: &NewOrder<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO sales.sales_order (tenant_id, id, order_number, order_date, order_status,
           customer_id, sales_channel, discount_code, coupon_id, subtotal_value, subtotal_currency,
           discount_value, discount_currency, shipping_value, shipping_currency,
           total_payment_due_value, total_payment_due_currency, preview_id, is_gift, gift_options,
           gift_wrap_value, gift_wrap_currency)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22)",
    )
    .bind(tx.tenant.as_str())
    .bind(o.id)
    .bind(o.order_number)
    .bind(o.order_date)
    .bind(o.order_status)
    .bind(o.customer_id)
    .bind(o.sales_channel)
    .bind(o.discount_code)
    .bind(o.coupon_id)
    .bind(o.subtotal.value)
    .bind(o.subtotal.currency.as_str())
    .bind(o.discount.value)
    .bind(o.discount.currency.as_str())
    .bind(o.shipping.value)
    .bind(o.shipping.currency.as_str())
    .bind(o.total.value)
    .bind(o.total.currency.as_str())
    .bind(o.preview_id)
    .bind(o.is_gift)
    .bind(o.gift_options)
    .bind(o.gift_wrap.map(|m| m.value))
    .bind(o.gift_wrap.map(|m| m.currency.as_str().to_owned()))
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// A new order item row.
#[derive(Debug, Clone)]
pub(crate) struct NewItem<'a> {
    pub id: &'a str,
    pub order_id: &'a str,
    pub position: i32,
    pub variant_id: &'a str,
    pub name: &'a str,
    pub quantity: i32,
    pub status: &'a str,
    pub price: Money,
    pub discount: Money,
    pub promotion_id: Option<&'a str>,
}

pub(crate) async fn insert_item(tx: &mut TxCx, i: &NewItem<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO sales.order_item (tenant_id, id, order_id, position, product_id, name,
           order_quantity, order_item_status, price, price_currency, discount_value,
           discount_currency, promotion_id)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
    )
    .bind(tx.tenant.as_str())
    .bind(i.id)
    .bind(i.order_id)
    .bind(i.position)
    .bind(i.variant_id)
    .bind(i.name)
    .bind(i.quantity)
    .bind(i.status)
    .bind(i.price.value)
    .bind(i.price.currency.as_str())
    .bind(i.discount.value)
    .bind(i.discount.currency.as_str())
    .bind(i.promotion_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// A new payment row.
#[derive(Debug, Clone)]
pub(crate) struct NewPayment<'a> {
    pub id: &'a str,
    pub order_id: &'a str,
    pub method: &'a str,
    pub status: &'a str,
    pub amount: Money,
    pub card_last4: Option<&'a str>,
    pub paid_at: Option<DateTime<Utc>>,
}

pub(crate) async fn insert_payment(tx: &mut TxCx, p: &NewPayment<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO sales.payment (tenant_id, id, order_id, payment_method, payment_status,
           amount_value, amount_currency, card_last4, paid_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(tx.tenant.as_str())
    .bind(p.id)
    .bind(p.order_id)
    .bind(p.method)
    .bind(p.status)
    .bind(p.amount.value)
    .bind(p.amount.currency.as_str())
    .bind(p.card_last4)
    .bind(p.paid_at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Appends an order event (`schema:Action`); its position follows the order's last event.
pub(crate) async fn append_event(
    tx: &mut TxCx,
    order_id: &str,
    action_type: &str,
    instrument: &str,
    from_state: Option<&str>,
    to_state: Option<&str>,
    result: &Value,
) -> Result<(), ToolError> {
    let now = tx.clock.now();
    let agent_id = tx.principal.subject.clone();
    let agent_name = tx.principal.name.clone();
    sqlx::query(
        "INSERT INTO sales.order_event (tenant_id, order_id, position, action_type, action_status,
           agent_id, agent_name, instrument, from_state, to_state, result, start_time, end_time,
           request_id)
         VALUES ($1, $2,
           (SELECT coalesce(max(position), 0) + 1 FROM sales.order_event
             WHERE tenant_id = $1 AND order_id = $2),
           $3, 'completed', $4, $5, $6, $7, $8, $9, $10, $10, $11)",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .bind(action_type)
    .bind(agent_id)
    .bind(agent_name)
    .bind(instrument)
    .bind(from_state)
    .bind(to_state)
    .bind(result)
    .bind(now)
    .bind(tx.request_id.clone())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Sets a line's quantity.
pub(crate) async fn set_quantity(tx: &mut TxCx, id: &str, quantity: u32) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE sales.cart_line SET order_quantity = $3, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(i32::try_from(quantity).unwrap_or(i32::MAX))
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

// ---------------------------------------------------------------- gifts the recipient accepts

/// A gift waiting for (or done with) its recipient (`sales.gift_order`).
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct GiftOrderRow {
    pub id: String,
    pub order_id: String,
    pub recipient_name: String,
    pub action_status: String,
    pub expires: DateTime<Utc>,
}

pub(crate) async fn insert_gift_order(
    tx: &mut TxCx,
    id: &str,
    order_id: &str,
    recipient_name: &str,
    recipient_contact: &str,
    expires: DateTime<Utc>,
) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO sales.gift_order (tenant_id, id, order_id, recipient_name, recipient_contact,
           expires)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(order_id)
    .bind(recipient_name)
    .bind(recipient_contact)
    .bind(expires)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// The gift, locked for its acceptance.
pub(crate) async fn gift_order_for_update(
    tx: &mut TxCx,
    id: &str,
) -> Result<Option<GiftOrderRow>, ToolError> {
    sqlx::query_as::<_, GiftOrderRow>(
        "SELECT id, order_id, recipient_name, action_status, expires FROM sales.gift_order
         WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The gift as its recipient sees it, without locking it.
pub(crate) async fn gift_order(tx: &mut TxCx, id: &str) -> Result<Option<GiftOrderRow>, ToolError> {
    sqlx::query_as::<_, GiftOrderRow>(
        "SELECT id, order_id, recipient_name, action_status, expires FROM sales.gift_order
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The acceptance of a gift order, if it waits (or waited) for its recipient.
pub(crate) async fn gift_order_of(
    tx: &mut TxCx,
    order_id: &str,
) -> Result<Option<GiftOrderRow>, ToolError> {
    sqlx::query_as::<_, GiftOrderRow>(
        "SELECT id, order_id, recipient_name, action_status, expires FROM sales.gift_order
         WHERE tenant_id = $1 AND order_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Ends the acceptance: `completed` when the recipient accepted, `failed` when it lapsed.
pub(crate) async fn finish_gift_order(
    tx: &mut TxCx,
    id: &str,
    status: &str,
    accepted_at: Option<DateTime<Utc>>,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE sales.gift_order SET action_status = $3, accepted_at = $4, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(status)
    .bind(accepted_at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Moves an order and all its items to `status` (a held gift order once it is accepted).
pub(crate) async fn set_order_and_items_status(
    tx: &mut TxCx,
    order_id: &str,
    status: &str,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE sales.sales_order SET order_status = $3, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .bind(status)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    sqlx::query(
        "UPDATE sales.order_item SET order_item_status = $3, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND order_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .bind(status)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Swaps an item to another variant of the same product (the recipient chose a size or color).
pub(crate) async fn set_item_variant(
    tx: &mut TxCx,
    item_id: &str,
    variant_id: &str,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE sales.order_item SET product_id = $3, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(item_id)
    .bind(variant_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Records that a held payment went through.
pub(crate) async fn complete_payment(
    tx: &mut TxCx,
    payment_id: &str,
    paid_at: DateTime<Utc>,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE sales.payment SET payment_status = 'payment_complete', paid_at = $3,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(payment_id)
    .bind(paid_at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}
