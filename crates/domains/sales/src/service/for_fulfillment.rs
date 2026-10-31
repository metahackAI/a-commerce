//! What `fulfillment` reads and changes in sales: an order, its items and payments, and the
//! status and warehouse step of items and orders as a cancel or a shipment goes through, and the
//! order priced again when the store cancels lines. Claims, refunds and deliveries stay in
//! fulfillment; this file only touches `sales.*`.

use acom_contract::{ErrorCode, ToolError};
use acom_core::{Currency, Money};
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::json;

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

fn money(value: Decimal, currency: &str) -> Result<Money, ToolError> {
    let currency = Currency::parse(currency.trim())
        .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?;
    Ok(Money::new(value, currency))
}

/// An order as fulfillment needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub id: String,
    pub order_number: String,
    /// `None` for a guest order.
    pub customer_id: Option<String>,
    pub order_status: String,
    pub order_date: DateTime<Utc>,
    pub coupon_id: Option<String>,
    pub discount_code: Option<String>,
    /// List prices × quantities, before discounts.
    pub subtotal: Money,
    pub discount: Money,
    /// Shipping charged; zero when it was free.
    pub shipping: Money,
    pub total_payment_due: Money,
    pub version: i32,
}

/// One line of an order.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderItem {
    pub id: String,
    pub order_id: String,
    pub position: i32,
    /// The variant bought (`var_…`).
    pub variant_id: String,
    pub name: String,
    pub order_quantity: i32,
    pub order_item_status: String,
    pub fulfillment_stage: Option<String>,
    /// List price per unit when ordered.
    pub price: Money,
    /// Discount on the whole line, including its share of order discounts.
    pub discount: Money,
    pub promotion_id: Option<String>,
    pub version: i32,
}

/// One payment of an order.
#[derive(Debug, Clone, PartialEq)]
pub struct Payment {
    pub id: String,
    pub payment_method: String,
    pub payment_status: String,
    pub amount: Money,
    pub card_last4: Option<String>,
    pub paid_at: Option<DateTime<Utc>>,
}

type OrderRow = (
    String,
    String,
    Option<String>,
    String,
    DateTime<Utc>,
    Option<String>,
    Option<String>,
    Decimal,
    String,
    Decimal,
    String,
    Decimal,
    String,
    Decimal,
    String,
    i32,
);

const ORDER_COLUMNS: &str = "id, order_number, customer_id, order_status, order_date, coupon_id,
    discount_code, subtotal_value, subtotal_currency::text, discount_value, discount_currency::text,
    shipping_value, shipping_currency::text, total_payment_due_value,
    total_payment_due_currency::text, version";

fn order_from(r: OrderRow) -> Result<Order, ToolError> {
    Ok(Order {
        id: r.0,
        order_number: r.1,
        customer_id: r.2,
        order_status: r.3,
        order_date: r.4,
        coupon_id: r.5,
        discount_code: r.6,
        subtotal: money(r.7, &r.8)?,
        discount: money(r.9, &r.10)?,
        shipping: money(r.11, &r.12)?,
        total_payment_due: money(r.13, &r.14)?,
        version: r.15,
    })
}

/// One order, or `None` when the store has no such order.
pub async fn order(tx: &mut TxCx, order_id: &str) -> Result<Option<Order>, ToolError> {
    let row: Option<OrderRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ORDER_COLUMNS} FROM sales.sales_order WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    row.map(order_from).transpose()
}

/// Several orders at once (for list reads); missing ids are left out.
pub async fn orders(tx: &mut TxCx, order_ids: &[String]) -> Result<Vec<Order>, ToolError> {
    let rows: Vec<OrderRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ORDER_COLUMNS} FROM sales.sales_order WHERE tenant_id = $1 AND id = ANY($2)"
    )))
    .bind(tx.tenant.as_str())
    .bind(order_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    rows.into_iter().map(order_from).collect()
}

type ItemRow = (
    String,
    String,
    i32,
    String,
    String,
    i32,
    String,
    Option<String>,
    Decimal,
    String,
    Decimal,
    String,
    Option<String>,
    i32,
);

const ITEM_COLUMNS: &str = "id, order_id, position, product_id, name, order_quantity,
    order_item_status, fulfillment_stage, price, price_currency::text, discount_value,
    discount_currency::text, promotion_id, version";

fn item_from(r: ItemRow) -> Result<OrderItem, ToolError> {
    Ok(OrderItem {
        id: r.0,
        order_id: r.1,
        position: r.2,
        variant_id: r.3,
        name: r.4,
        order_quantity: r.5,
        order_item_status: r.6,
        fulfillment_stage: r.7,
        price: money(r.8, &r.9)?,
        discount: money(r.10, &r.11)?,
        promotion_id: r.12,
        version: r.13,
    })
}

/// Every line of an order, in line order.
pub async fn order_items(tx: &mut TxCx, order_id: &str) -> Result<Vec<OrderItem>, ToolError> {
    let rows: Vec<ItemRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ITEM_COLUMNS} FROM sales.order_item WHERE tenant_id = $1 AND order_id = $2
         ORDER BY position"
    )))
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    rows.into_iter().map(item_from).collect()
}

/// Lines by id, from any order (for list reads); missing ids are left out.
pub async fn items(tx: &mut TxCx, order_item_ids: &[String]) -> Result<Vec<OrderItem>, ToolError> {
    let rows: Vec<ItemRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {ITEM_COLUMNS} FROM sales.order_item WHERE tenant_id = $1 AND id = ANY($2)
         ORDER BY order_id, position"
    )))
    .bind(tx.tenant.as_str())
    .bind(order_item_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    rows.into_iter().map(item_from).collect()
}

/// The payments of an order, oldest first (the first is the primary payment).
pub async fn payments(tx: &mut TxCx, order_id: &str) -> Result<Vec<Payment>, ToolError> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        String,
        String,
        String,
        Decimal,
        String,
        Option<String>,
        Option<DateTime<Utc>>,
    )> = sqlx::query_as(
        "SELECT id, payment_method, payment_status, amount_value, amount_currency::text,
                card_last4, paid_at
         FROM sales.payment WHERE tenant_id = $1 AND order_id = $2
         ORDER BY created_at, id",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    rows.into_iter()
        .map(|r| {
            Ok(Payment {
                id: r.0,
                payment_method: r.1,
                payment_status: r.2,
                amount: money(r.3, &r.4)?,
                card_last4: r.5,
                paid_at: r.6,
            })
        })
        .collect()
}

/// Sets one line's status (a whole line cancelled). Returns the status it had, or `None` when
/// the line does not exist.
pub async fn set_item_status(
    tx: &mut TxCx,
    order_item_id: &str,
    status: &str,
) -> Result<Option<String>, ToolError> {
    let before: Option<String> = sqlx::query_scalar(
        "SELECT order_item_status FROM sales.order_item
         WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(tx.tenant.as_str())
    .bind(order_item_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    if before.as_deref().is_some_and(|b| b != status) {
        sqlx::query(
            "UPDATE sales.order_item SET order_item_status = $3,
               fulfillment_stage = CASE WHEN $3 = 'cancelled' THEN NULL ELSE fulfillment_stage END,
               updated_at = now(), version = version + 1
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(order_item_id)
        .bind(status)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    Ok(before)
}

/// Moves an order to `status` and appends the order event, unless it is already there.
/// `instrument` is the tool or person-only endpoint that caused it.
pub async fn set_order_status(
    tx: &mut TxCx,
    order_id: &str,
    status: &str,
    action_type: &str,
    instrument: &str,
) -> Result<(), ToolError> {
    let before: Option<String> = sqlx::query_scalar(
        "SELECT order_status FROM sales.sales_order WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    let Some(before) = before else {
        return Ok(());
    };
    if before == status {
        return Ok(());
    }
    let now = tx.clock.now();
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
    let agent = tx.principal.subject.clone();
    let agent_name = tx.principal.name.clone();
    sqlx::query(
        "INSERT INTO sales.order_event (tenant_id, order_id, position, action_type, action_status,
           agent_id, agent_name, instrument, from_state, to_state, result, start_time, end_time,
           request_id)
         SELECT $1, $2, coalesce(max(position), 0) + 1, $3, 'completed', $4, $5, $6, $7, $8, $9,
                $10, $10, $11
         FROM sales.order_event WHERE tenant_id = $1 AND order_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .bind(action_type)
    .bind(&agent)
    .bind(agent_name.as_deref())
    .bind(instrument)
    .bind(&before)
    .bind(status)
    .bind(json!({"order_status": status}))
    .bind(now)
    .bind(&tx.request_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// One line's status and warehouse step, as they were before a change.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ItemProgress {
    pub order_item_status: String,
    pub fulfillment_stage: Option<String>,
}

/// Sets one line's warehouse step (`fulfillment_stage`, `None` clears it) and, when given, its
/// status, as a parcel is scheduled, short of stock or handed to the carrier. Returns what the
/// line had, or `None` when the line does not exist.
pub async fn set_item_progress(
    tx: &mut TxCx,
    order_item_id: &str,
    status: Option<&str>,
    stage: Option<&str>,
) -> Result<Option<ItemProgress>, ToolError> {
    let before: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT order_item_status, fulfillment_stage FROM sales.order_item
         WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(tx.tenant.as_str())
    .bind(order_item_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    let Some((was_status, was_stage)) = before else {
        return Ok(None);
    };
    let new_status = status.unwrap_or(&was_status);
    if new_status != was_status || stage != was_stage.as_deref() {
        sqlx::query(
            "UPDATE sales.order_item SET order_item_status = $3, fulfillment_stage = $4,
               updated_at = now(), version = version + 1
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(order_item_id)
        .bind(new_status)
        .bind(stage)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    Ok(Some(ItemProgress {
        order_item_status: was_status,
        fulfillment_stage: was_stage,
    }))
}

/// An order's money: list prices × units, discounts, shipping, gift wrap and the total.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Priced {
    pub subtotal: Money,
    pub discount: Money,
    pub shipping: Money,
    pub gift_wrap: Option<Money>,
    pub total_payment_due: Money,
}

/// Prices the order again for the units that stay, the way checkout priced it: each line's
/// list price × the units that stay, each line's discount in proportion to those units
/// (rounded to the cent per line), the given shipping, gift wrap while anything stays, and
/// the total. `staying` gives the units that stay per line; a line it leaves out keeps all
/// its units unless it is cancelled. Nothing is written; see [`apply_price`].
pub async fn reprice(
    tx: &mut TxCx,
    order_id: &str,
    staying: &[(String, i32)],
    shipping: Money,
) -> Result<Priced, ToolError> {
    let row: Option<(Option<Decimal>, Option<String>)> = sqlx::query_as(
        "SELECT gift_wrap_value, gift_wrap_currency::text FROM sales.sales_order
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    let Some((wrap_value, wrap_currency)) = row else {
        return Err(ToolError::new(
            ErrorCode::INTERNAL,
            format!("order {order_id} is missing"),
        ));
    };
    let currency = shipping.currency;
    let mut subtotal = Decimal::ZERO;
    let mut discount = Decimal::ZERO;
    let mut units_left = 0;
    for line in order_items(tx, order_id).await? {
        let units = staying
            .iter()
            .find(|(id, _)| id == &line.id)
            .map(|(_, q)| *q)
            .unwrap_or(if line.order_item_status == "cancelled" {
                0
            } else {
                line.order_quantity
            })
            .clamp(0, line.order_quantity);
        units_left += units;
        subtotal += line.price.value * Decimal::from(units);
        if line.order_quantity > 0 {
            discount += (line.discount.value * Decimal::from(units)
                / Decimal::from(line.order_quantity))
            .round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
        }
    }
    let gift_wrap = match (wrap_value, wrap_currency) {
        (Some(v), Some(c)) if units_left > 0 => Some(money(v, &c)?),
        (Some(_), Some(c)) => Some(money(Decimal::ZERO, &c)?),
        _ => None,
    };
    let shipping_value = if units_left > 0 {
        shipping.value
    } else {
        Decimal::ZERO
    };
    let total =
        (subtotal - discount + shipping_value + gift_wrap.map_or(Decimal::ZERO, |w| w.value))
            .max(Decimal::ZERO);
    Ok(Priced {
        subtotal: Money::new(subtotal, currency).rounded(),
        discount: Money::new(discount, currency).rounded(),
        shipping: Money::new(shipping_value, currency).rounded(),
        gift_wrap,
        total_payment_due: Money::new(total, currency).rounded(),
    })
}

/// The order's money as stored now.
pub async fn priced(tx: &mut TxCx, order_id: &str) -> Result<Option<Priced>, ToolError> {
    #[allow(clippy::type_complexity)]
    let row: Option<(
        Decimal,
        String,
        Decimal,
        String,
        Decimal,
        String,
        Option<Decimal>,
        Option<String>,
        Decimal,
        String,
    )> = sqlx::query_as(
        "SELECT subtotal_value, subtotal_currency::text, discount_value, discount_currency::text,
                shipping_value, shipping_currency::text, gift_wrap_value,
                gift_wrap_currency::text, total_payment_due_value,
                total_payment_due_currency::text
         FROM sales.sales_order WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    row.map(|r| {
        Ok(Priced {
            subtotal: money(r.0, &r.1)?,
            discount: money(r.2, &r.3)?,
            shipping: money(r.4, &r.5)?,
            gift_wrap: match (r.6, r.7) {
                (Some(v), Some(c)) => Some(money(v, &c)?),
                _ => None,
            },
            total_payment_due: money(r.8, &r.9)?,
        })
    })
    .transpose()
}

/// Writes the order's money (from [`reprice`], or back to what it was on undo) and appends an
/// `UpdateAction` order event; `instrument` is the tool that caused it.
pub async fn apply_price(
    tx: &mut TxCx,
    order_id: &str,
    p: &Priced,
    instrument: &str,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE sales.sales_order SET subtotal_value = $3, discount_value = $4,
           shipping_value = $5, gift_wrap_value = $6, total_payment_due_value = $7,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .bind(p.subtotal.value)
    .bind(p.discount.value)
    .bind(p.shipping.value)
    .bind(p.gift_wrap.map(|w| w.value))
    .bind(p.total_payment_due.value)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    let now = tx.clock.now();
    let agent = tx.principal.subject.clone();
    let agent_name = tx.principal.name.clone();
    sqlx::query(
        "INSERT INTO sales.order_event (tenant_id, order_id, position, action_type, action_status,
           agent_id, agent_name, instrument, result, start_time, end_time, request_id)
         SELECT $1, $2, coalesce(max(position), 0) + 1, 'UpdateAction', 'completed', $3, $4, $5,
                $6, $7, $7, $8
         FROM sales.order_event WHERE tenant_id = $1 AND order_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .bind(&agent)
    .bind(agent_name.as_deref())
    .bind(instrument)
    .bind(json!({"repriced": p}))
    .bind(now)
    .bind(&tx.request_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}
