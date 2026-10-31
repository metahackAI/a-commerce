//! What `integration` reads and changes in sales: an order found by the reference an outside
//! system uses (our order id or the number people see), the moves of the order life cycle an
//! outside channel may report, and a payment provider's event applied to a payment the way sales
//! confirms a deposit. Integration never writes `sales.*` itself; the rules stay here.

use acom_contract::{ActionType, ErrorCode, ToolError};
use acom_core::enums::{OrderStatus, PaymentStatus};
use acom_core::{Currency, Money};
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::json;

use crate::repo::{self, OrderRow, db};

fn money(value: Decimal, currency: &str) -> Result<Money, ToolError> {
    let currency = Currency::parse(currency.trim())
        .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?;
    Ok(Money::new(value, currency))
}

/// An order as integration needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderFacts {
    pub id: String,
    /// The number people see (`20402`).
    pub order_number: String,
    pub order_status: String,
    pub total_payment_due: Money,
    pub version: i32,
}

fn facts(o: OrderRow) -> Result<OrderFacts, ToolError> {
    Ok(OrderFacts {
        total_payment_due: money(o.total_payment_due_value, &o.total_payment_due_currency)?,
        id: o.id,
        order_number: o.order_number,
        order_status: o.order_status,
        version: o.version,
    })
}

/// One order, or `None` when the store has no such order.
pub async fn order(tx: &mut TxCx, order_id: &str) -> Result<Option<OrderFacts>, ToolError> {
    repo::order(tx, order_id).await?.map(facts).transpose()
}

/// The order an outside system names: our id (`ord_20402`) or the number people see (`20402`,
/// `#20402`). `None` when no order matches.
pub async fn order_by_reference(
    tx: &mut TxCx,
    reference: &str,
) -> Result<Option<OrderFacts>, ToolError> {
    let r = reference.trim();
    if r.starts_with("ord_") {
        return order(tx, r).await;
    }
    let number = r.trim_start_matches('#');
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(None);
    }
    repo::order_by_number(tx, number)
        .await?
        .map(facts)
        .transpose()
}

/// The statuses an order may move to from `from` when an outside channel reports a change
/// (`status_mapping.transitions_from_core`): forward along payment, shipping and delivery, to
/// `problem` from any open status and back out of it; `cancelled` and `returned` are final.
pub fn order_moves(from: &str) -> &'static [&'static str] {
    match from {
        "payment_due" => &["processing", "cancelled", "problem"],
        "processing" => &["in_transit", "cancelled", "problem"],
        "in_transit" => &["delivered", "problem"],
        "delivered" => &["returned", "problem"],
        "problem" => &[
            "processing",
            "in_transit",
            "delivered",
            "cancelled",
            "returned",
        ],
        _ => &[],
    }
}

fn not_now(id: &str, from: &str, to: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INVALID_STATE,
        format!(
            "Order {id} is {}; it cannot become {}.",
            from.replace('_', " "),
            to.replace('_', " ")
        ),
    )
    .detail(json!({"entity": "order", "from": from, "allowed_to": order_moves(from)}))
}

/// Moves an order and its lines from `from` to `to` (one of [`order_moves`]) and records the
/// order event; `instrument` is the tool that caused it. Refused when the order is no longer in
/// `from`.
pub async fn move_order(
    tx: &mut TxCx,
    order_id: &str,
    from: &str,
    to: &str,
    instrument: &str,
) -> Result<(), ToolError> {
    let o = repo::order_for_update(tx, order_id).await?.ok_or_else(|| {
        ToolError::new(
            ErrorCode::NOT_FOUND,
            format!("There is no order {order_id}."),
        )
        .detail(json!({"entity": "order", "id": order_id}))
    })?;
    if o.order_status != from || !order_moves(from).contains(&to) {
        return Err(not_now(&o.id, &o.order_status, to));
    }
    to.parse::<OrderStatus>()
        .map_err(|e| ToolError::invalid_input(e.to_string()))?;
    repo::set_order_and_items_status(tx, &o.id, to).await?;
    repo::append_event(
        tx,
        &o.id,
        ActionType::UPDATE.0,
        instrument,
        Some(from),
        Some(to),
        &json!({"order_status": to, "source": "outside_channel"}),
    )
    .await
}

/// One payment of an order, with the payment provider's reference.
#[derive(Debug, Clone, PartialEq)]
pub struct PaymentFacts {
    pub id: String,
    pub order_id: String,
    pub payment_method: String,
    pub payment_status: String,
    pub amount: Money,
    /// The provider's reference, once it told us one.
    pub external_id: Option<String>,
    pub version: i32,
}

type PaymentRow = (
    String,
    String,
    String,
    String,
    Decimal,
    String,
    Option<String>,
    i32,
);

fn payment_from(r: PaymentRow) -> Result<PaymentFacts, ToolError> {
    Ok(PaymentFacts {
        amount: money(r.4, &r.5)?,
        id: r.0,
        order_id: r.1,
        payment_method: r.2,
        payment_status: r.3,
        external_id: r.6,
        version: r.7,
    })
}

const PAYMENT_COLUMNS: &str = "id, order_id, payment_method, payment_status, amount_value,
    amount_currency::text, external_id, version";

/// The payments of an order, oldest first (the first is the primary payment).
pub async fn payments_of(tx: &mut TxCx, order_id: &str) -> Result<Vec<PaymentFacts>, ToolError> {
    let rows: Vec<PaymentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {PAYMENT_COLUMNS} FROM sales.payment WHERE tenant_id = $1 AND order_id = $2
         ORDER BY created_at, id"
    )))
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    rows.into_iter().map(payment_from).collect()
}

/// What a payment provider's event does to a payment in `from`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaymentMove {
    /// The payment moves to this status.
    To(&'static str),
    /// The payment is there already: nothing changes.
    Already,
    /// Not from this status; these are the statuses it applies from.
    NotFrom(&'static [&'static str]),
}

/// The payment life cycle a provider's event follows: an authorization or a confirmed deposit
/// completes a payment that is due, a cancellation declines it. Refund events are not payments
/// (fulfillment's refunds take them).
pub fn payment_move(event: &str, from: &str) -> PaymentMove {
    const OPEN: &[&str] = &["payment_due", "payment_past_due"];
    let (to, already) = match event {
        "authorized" | "deposit_confirmed" => ("payment_complete", "payment_complete"),
        "cancelled" => ("payment_declined", "payment_declined"),
        _ => return PaymentMove::NotFrom(&[]),
    };
    if from == already {
        PaymentMove::Already
    } else if OPEN.contains(&from) {
        PaymentMove::To(to)
    } else {
        PaymentMove::NotFrom(OPEN)
    }
}

/// Applies a provider's event to a payment (see [`payment_move`]) and records its reference. A
/// completed payment releases an order waiting for its money to `processing`, as a confirmed
/// deposit does, unless a gift still waits for its recipient. Returns `(from, to)`, or `None`
/// when the payment was there already.
pub async fn apply_payment_event(
    tx: &mut TxCx,
    payment_id: &str,
    event: &str,
    payment_ref: &str,
    happened_at: DateTime<Utc>,
    instrument: &str,
) -> Result<Option<(String, String)>, ToolError> {
    let row: Option<PaymentRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {PAYMENT_COLUMNS} FROM sales.payment WHERE tenant_id = $1 AND id = $2 FOR UPDATE"
    )))
    .bind(tx.tenant.as_str())
    .bind(payment_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    let p = row.map(payment_from).transpose()?.ok_or_else(|| {
        ToolError::new(
            ErrorCode::NOT_FOUND,
            format!("There is no payment {payment_id}."),
        )
        .detail(json!({"entity": "payment", "id": payment_id}))
    })?;
    let to = match payment_move(event, &p.payment_status) {
        PaymentMove::Already => return Ok(None),
        PaymentMove::NotFrom(allowed) => {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "Payment {} is {}; a {} event does not apply.",
                    p.id,
                    p.payment_status.replace('_', " "),
                    event.replace('_', " ")
                ),
            )
            .detail(
                json!({"entity": "payment", "from": p.payment_status, "allowed_from": allowed}),
            ));
        }
        PaymentMove::To(to) => to,
    };
    sqlx::query(
        "UPDATE sales.payment SET payment_status = $3,
           paid_at = CASE WHEN $3 = 'payment_complete' THEN $4 ELSE paid_at END,
           external_id = coalesce(external_id, $5), updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(&p.id)
    .bind(to)
    .bind(happened_at)
    .bind(payment_ref)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    let o = repo::order_for_update(tx, &p.order_id).await?;
    let mut order_moved = None;
    if to == PaymentStatus::PaymentComplete.as_str()
        && let Some(o) = &o
        && o.order_status == OrderStatus::PaymentDue.as_str()
    {
        let waits_for_recipient = repo::gift_order_of(tx, &o.id)
            .await?
            .is_some_and(|g| g.action_status == "potential");
        if !waits_for_recipient {
            repo::set_order_and_items_status(tx, &o.id, OrderStatus::Processing.as_str()).await?;
            order_moved = Some((o.order_status.clone(), OrderStatus::Processing.as_str()));
        }
    }
    repo::append_event(
        tx,
        &p.order_id,
        ActionType::PAY.0,
        instrument,
        order_moved.as_ref().map(|(f, _)| f.as_str()),
        order_moved.as_ref().map(|(_, t)| *t),
        &json!({"payment_id": p.id, "payment_status": to, "payment_ref": payment_ref,
                "source": "payment_provider"}),
    )
    .await?;
    Ok(Some((p.payment_status, to.to_owned())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_move_forward_and_stop_when_final() {
        assert!(order_moves("processing").contains(&"in_transit"));
        assert!(!order_moves("in_transit").contains(&"processing"));
        assert!(order_moves("cancelled").is_empty());
        assert!(order_moves("returned").is_empty());
        for from in OrderStatus::WORDS {
            for to in order_moves(from) {
                assert!(to.parse::<OrderStatus>().is_ok(), "{from} → {to}");
            }
        }
    }

    #[test]
    fn provider_events_complete_or_decline_a_due_payment_once() {
        assert_eq!(
            payment_move("deposit_confirmed", "payment_due"),
            PaymentMove::To("payment_complete")
        );
        assert_eq!(
            payment_move("authorized", "payment_complete"),
            PaymentMove::Already
        );
        assert_eq!(
            payment_move("cancelled", "payment_due"),
            PaymentMove::To("payment_declined")
        );
        assert!(matches!(
            payment_move("cancelled", "payment_complete"),
            PaymentMove::NotFrom(_)
        ));
    }
}
