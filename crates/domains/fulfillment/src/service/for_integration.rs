//! What `integration` reads and changes in fulfillment: the parcel a carrier's tracking number
//! names and the carrier's progress applied to it, a claim moved along its life cycle as an
//! outside channel reports it, and a refund confirmed or failed by the payment provider
//! (`refund.settlement: payment_event_confirmed`). Integration never writes `fulfillment.*`
//! itself; the moves each record may make are decided here.

use acom_contract::{ErrorCode, ToolError};
use acom_core::{Currency, Money};
use acom_sales::service::for_fulfillment as sales;
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::json;

use crate::parcel_delivery::parcel_lines;
use crate::repo::{self, ClaimEvent, db};

/// What an outside event does to a record in some state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    /// The record moves to this state.
    To(&'static str),
    /// The record is there already: nothing changes.
    Already,
    /// Not from this state; these are the states the event applies from.
    NotFrom(&'static [&'static str]),
}

fn not_from(entity: &str, id: &str, from: &str, event: &str, allowed: &[&str]) -> ToolError {
    ToolError::new(
        ErrorCode::INVALID_STATE,
        format!(
            "{} {id} is {}; {} does not apply now.",
            entity.replace('_', " "),
            from.replace('_', " "),
            event.replace('_', " ")
        ),
    )
    .detail(json!({"entity": entity, "from": from, "allowed_from": allowed}))
}

/// A parcel as integration needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryFacts {
    pub id: String,
    pub order_id: String,
    /// `acom:deliveryState` word.
    pub delivery_state: String,
    pub tracking_number: Option<String>,
    pub version: i32,
}

/// The newest parcel with this tracking number, or `None` when no label carries it yet.
pub async fn delivery_by_tracking(
    tx: &mut TxCx,
    tracking_number: &str,
) -> Result<Option<DeliveryFacts>, ToolError> {
    Ok(repo::delivery_by_tracking(tx, tracking_number)
        .await?
        .map(|d| DeliveryFacts {
            id: d.id,
            order_id: d.order_id,
            delivery_state: d.delivery_state,
            tracking_number: d.tracking_number,
            version: d.version,
        }))
}

/// A carrier's progress on a parcel (`acom:carrierEventType`): the label, the pickup, the way,
/// the delivery or a failed delivery. Each moves the parcel forward only; a delivered or
/// cancelled parcel takes no more events.
pub fn carrier_move(event: &str, from: &str) -> Move {
    let allowed: &'static [&'static str] = match event {
        "label_created" => &["awaiting_stock", "ship_instructed", "packed"],
        "picked_up" => &["packed", "label_created"],
        "in_transit" => &["label_created", "picked_up", "delivery_failed"],
        "delivered" => &[
            "label_created",
            "picked_up",
            "in_transit",
            "delivery_failed",
        ],
        "delivery_failed" => &["picked_up", "in_transit"],
        _ => return Move::NotFrom(&[]),
    };
    let to: &'static str = match event {
        "label_created" => "label_created",
        "picked_up" => "picked_up",
        "in_transit" => "in_transit",
        "delivered" => "delivered",
        _ => "delivery_failed",
    };
    if from == to {
        Move::Already
    } else if allowed.contains(&from) {
        Move::To(to)
    } else {
        Move::NotFrom(allowed)
    }
}

/// Applies a carrier's event to a parcel (see [`carrier_move`]). A parcel on its way ships its
/// lines and puts the order in transit, as `confirm_shipment` does; a delivered parcel delivers
/// its lines, and the order once every line that is not cancelled is delivered. Returns
/// `(from, to)`, or `None` when the parcel was there already.
pub async fn apply_carrier_event(
    tx: &mut TxCx,
    delivery_id: &str,
    event: &str,
    happened_at: DateTime<Utc>,
    instrument: &str,
) -> Result<Option<(String, String)>, ToolError> {
    let d = repo::delivery_for_update(tx, delivery_id)
        .await?
        .ok_or_else(|| {
            ToolError::new(
                ErrorCode::NOT_FOUND,
                format!("There is no parcel {delivery_id}."),
            )
            .detail(json!({"entity": "parcel_delivery", "id": delivery_id}))
        })?;
    let to = match carrier_move(event, &d.delivery_state) {
        Move::Already => return Ok(None),
        Move::NotFrom(allowed) => {
            return Err(not_from(
                "parcel_delivery",
                &d.id,
                &d.delivery_state,
                event,
                allowed,
            ));
        }
        Move::To(to) => to,
    };
    repo::set_delivery_state(tx, &d.id, to).await?;
    sqlx::query(
        "UPDATE fulfillment.parcel_delivery SET
           shipped_at = CASE WHEN $3 IN ('picked_up', 'in_transit', 'delivered')
                             THEN coalesce(shipped_at, $4) ELSE shipped_at END,
           delivered_at = CASE WHEN $3 = 'delivered' THEN $4 ELSE delivered_at END
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(&d.id)
    .bind(to)
    .bind(happened_at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    // A reshipped exchange parcel carries no lines of its own order.
    if d.claim_id.is_none() && matches!(to, "picked_up" | "in_transit" | "delivered") {
        let items = sales::order_items(tx, &d.order_id).await?;
        let line_status = if to == "delivered" {
            "delivered"
        } else {
            "in_transit"
        };
        for (line, _) in parcel_lines(&d, &items) {
            let Some(item) = items.iter().find(|i| i.id == line) else {
                continue;
            };
            let behind = match line_status {
                "delivered" => !matches!(
                    item.order_item_status.as_str(),
                    "delivered" | "returned" | "cancelled"
                ),
                _ => matches!(
                    item.order_item_status.as_str(),
                    "payment_due" | "processing" | "problem"
                ),
            };
            if behind {
                sales::set_item_progress(tx, &line, Some(line_status), Some("shipped")).await?;
            }
        }
        let after = sales::order_items(tx, &d.order_id).await?;
        let open: Vec<&sales::OrderItem> = after
            .iter()
            .filter(|i| i.order_item_status != "cancelled")
            .collect();
        let order = sales::order(tx, &d.order_id).await?;
        if let Some(o) = order {
            let all = |s: &[&str]| {
                !open.is_empty()
                    && open
                        .iter()
                        .all(|i| s.contains(&i.order_item_status.as_str()))
            };
            let next = if all(&["delivered", "returned"]) && o.order_status != "returned" {
                Some("delivered")
            } else if all(&["in_transit", "delivered", "returned"])
                && matches!(o.order_status.as_str(), "processing" | "payment_due")
            {
                Some("in_transit")
            } else {
                None
            };
            if let Some(next) = next
                && next != o.order_status
            {
                sales::set_order_status(tx, &o.id, next, "UpdateAction", instrument).await?;
            }
        }
    }
    repo::append_delivery_event(
        tx,
        &d.id,
        &ClaimEvent {
            action_type: "UpdateAction",
            action_status: "completed",
            instrument,
            from_state: Some(&d.delivery_state),
            to_state: Some(to),
            result: Some(json!({"carrier_event": event, "happened_at": happened_at,
                                "source": "carrier"})),
        },
    )
    .await?;
    Ok(Some((d.delivery_state, to.to_owned())))
}

/// A claim as integration needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimFacts {
    pub id: String,
    pub order_id: String,
    /// `cancel`, `return` or `exchange`.
    pub claim_type: String,
    pub claim_status: String,
    pub version: i32,
}

/// One claim, or `None` when the store has no such claim.
pub async fn claim(tx: &mut TxCx, claim_id: &str) -> Result<Option<ClaimFacts>, ToolError> {
    Ok(repo::claim(tx, claim_id).await?.map(|c| ClaimFacts {
        id: c.id,
        order_id: c.order_id,
        claim_type: c.claim_type,
        claim_status: c.claim_status,
        version: c.version,
    }))
}

/// The statuses a claim may move to from `from` when an outside channel reports a change
/// (`status_mapping.transitions_from_core`): received, asked for photos, approved or declined,
/// picked up and inspected, reshipped for an exchange, completed, or withdrawn while it has not
/// been picked up (`return.withdraw`). Closed claims stay closed; converting an exchange to a
/// return is `convert_exchange_to_return`, never a channel's status.
pub fn claim_moves(claim_type: &str, from: &str) -> &'static [&'static str] {
    match (claim_type, from) {
        (_, "charge_due") => &["requested", "withdrawn"],
        (_, "requested") => &["photos_requested", "approved", "declined", "withdrawn"],
        (_, "photos_requested") => &["requested", "approved", "declined", "withdrawn"],
        ("cancel", "approved") => &["completed"],
        (_, "approved") => &["pickup_instructed", "picked_up", "withdrawn"],
        (_, "pickup_instructed") => &["picked_up"],
        (_, "picked_up") => &["inspected"],
        ("exchange", "inspected") => &["reship_requested", "declined"],
        (_, "inspected") => &["completed", "declined"],
        ("exchange", "reship_requested") => &["completed"],
        _ => &[],
    }
}

/// Moves a claim from `from` to `to` (one of [`claim_moves`]) and records the claim event. A
/// return completes only once its refund was sent: refunds go through `refund_return`, never
/// through a channel's status.
pub async fn move_claim(
    tx: &mut TxCx,
    claim_id: &str,
    from: &str,
    to: &str,
    instrument: &str,
) -> Result<(), ToolError> {
    let c = repo::claim_for_update(tx, claim_id).await?.ok_or_else(|| {
        ToolError::new(
            ErrorCode::NOT_FOUND,
            format!("There is no claim {claim_id}."),
        )
        .detail(json!({"entity": "claim", "id": claim_id}))
    })?;
    let allowed = claim_moves(&c.claim_type, &c.claim_status);
    if c.claim_status != from || !allowed.contains(&to) {
        return Err(ToolError::new(
            ErrorCode::INVALID_STATE,
            format!(
                "Claim {} is {}; it cannot become {}.",
                c.id,
                c.claim_status.replace('_', " "),
                to.replace('_', " ")
            ),
        )
        .detail(json!({"entity": "claim", "from": c.claim_status, "allowed_to": allowed})));
    }
    if c.claim_type == "return" && to == "completed" {
        let refunded = repo::refund_of_claim(tx, &c.id)
            .await?
            .is_some_and(|r| r.refund_status != "failed");
        if !refunded {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "Return {} has no refund yet; refund it with refund_return before it completes.",
                    c.id
                ),
            )
            .detail(json!({"entity": "claim", "from": c.claim_status, "reason": "not_refunded"}))
            .next("refund_return"));
        }
    }
    if !repo::move_claim(tx, &c.id, from, to).await? {
        return Err(ToolError::new(
            ErrorCode::INVALID_STATE,
            format!("Claim {} changed while it was being moved.", c.id),
        ));
    }
    repo::append_event(
        tx,
        &c.id,
        &ClaimEvent {
            action_type: "UpdateAction",
            action_status: "completed",
            instrument,
            from_state: Some(from),
            to_state: Some(to),
            result: Some(json!({"claim_status": to, "source": "outside_channel"})),
        },
    )
    .await
}

/// A refund as integration needs it, with the payment provider's reference.
#[derive(Debug, Clone, PartialEq)]
pub struct RefundFacts {
    pub id: String,
    pub order_id: String,
    pub claim_id: String,
    /// `requested`, `completed` or `failed`.
    pub refund_status: String,
    pub amount: Money,
    pub external_id: Option<String>,
}

type RefundRow = (
    String,
    String,
    String,
    String,
    Decimal,
    String,
    Option<String>,
);

fn refund_from(r: RefundRow) -> Result<RefundFacts, ToolError> {
    let currency = Currency::parse(r.5.trim())
        .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?;
    Ok(RefundFacts {
        id: r.0,
        order_id: r.1,
        claim_id: r.2,
        refund_status: r.3,
        amount: Money::new(r.4, currency),
        external_id: r.6,
    })
}

const REFUND_COLUMNS: &str = "id, order_id, claim_id, refund_status, amount_value,
    amount_currency::text, external_id";

/// The refunds of an order, oldest first.
pub async fn refunds_of(tx: &mut TxCx, order_id: &str) -> Result<Vec<RefundFacts>, ToolError> {
    let rows: Vec<RefundRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {REFUND_COLUMNS} FROM fulfillment.refund WHERE tenant_id = $1 AND order_id = $2
         ORDER BY start_time, id"
    )))
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    rows.into_iter().map(refund_from).collect()
}

/// What a payment provider's refund event does to a refund in `from`: a refund sent
/// (`requested`) is confirmed or fails. A failed refund is sent again with `retry_refund`.
pub fn refund_move(event: &str, from: &str) -> Move {
    let to = match event {
        "refund_completed" => "completed",
        "refund_failed" => "failed",
        _ => return Move::NotFrom(&[]),
    };
    if from == to {
        Move::Already
    } else if from == "requested" {
        Move::To(to)
    } else {
        Move::NotFrom(&["requested"])
    }
}

/// Applies a provider's refund event (see [`refund_move`]), records its reference and the refund
/// event. Returns `(from, to)`, or `None` when the refund was there already.
pub async fn apply_refund_event(
    tx: &mut TxCx,
    refund_id: &str,
    event: &str,
    payment_ref: &str,
    instrument: &str,
) -> Result<Option<(String, String)>, ToolError> {
    let r = repo::refund_for_update(tx, refund_id)
        .await?
        .ok_or_else(|| {
            ToolError::new(
                ErrorCode::NOT_FOUND,
                format!("There is no refund {refund_id}."),
            )
            .detail(json!({"entity": "refund", "id": refund_id}))
        })?;
    let to = match refund_move(event, &r.refund_status) {
        Move::Already => return Ok(None),
        Move::NotFrom(allowed) => {
            return Err(not_from("refund", &r.id, &r.refund_status, event, allowed));
        }
        Move::To(to) => to,
    };
    if !repo::move_refund(tx, &r.id, &r.refund_status, to).await? {
        return Err(ToolError::new(
            ErrorCode::INVALID_STATE,
            format!("Refund {} changed while it was being moved.", r.id),
        ));
    }
    sqlx::query(
        "UPDATE fulfillment.refund SET external_id = coalesce(external_id, $3)
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(&r.id)
    .bind(payment_ref)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    let mut result = json!({"payment_ref": payment_ref, "source": "payment_provider"});
    if to == "failed" {
        result["next_tool"] = json!("retry_refund");
    }
    repo::append_refund_event(
        tx,
        &r.id,
        &ClaimEvent {
            action_type: "RefundAction",
            action_status: "completed",
            instrument,
            from_state: Some(&r.refund_status),
            to_state: Some(to),
            result: Some(result),
        },
    )
    .await?;
    Ok(Some((r.refund_status, to.to_owned())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_core::enums::{ClaimStatus, DeliveryState};

    #[test]
    fn carrier_events_move_parcels_forward_once() {
        assert_eq!(
            carrier_move("delivered", "in_transit"),
            Move::To("delivered")
        );
        assert_eq!(carrier_move("delivered", "delivered"), Move::Already);
        assert!(matches!(
            carrier_move("picked_up", "delivered"),
            Move::NotFrom(_)
        ));
        assert!(matches!(
            carrier_move("in_transit", "cancelled"),
            Move::NotFrom(_)
        ));
        for (event, from) in [("picked_up", "packed"), ("delivery_failed", "in_transit")] {
            let Move::To(to) = carrier_move(event, from) else {
                panic!("{event} from {from}");
            };
            assert!(to.parse::<DeliveryState>().is_ok());
        }
    }

    #[test]
    fn claims_follow_their_life_cycle_and_close_for_good() {
        assert!(claim_moves("return", "requested").contains(&"approved"));
        assert!(claim_moves("cancel", "approved").contains(&"completed"));
        assert!(!claim_moves("return", "approved").contains(&"completed"));
        assert!(claim_moves("exchange", "inspected").contains(&"reship_requested"));
        for closed in ["completed", "declined", "withdrawn", "converted"] {
            assert!(claim_moves("return", closed).is_empty());
        }
        for t in ["cancel", "return", "exchange"] {
            for from in ClaimStatus::WORDS {
                for to in claim_moves(t, from) {
                    assert!(to.parse::<ClaimStatus>().is_ok(), "{t}: {from} → {to}");
                }
            }
        }
    }

    #[test]
    fn refunds_are_confirmed_or_fail_once() {
        assert_eq!(
            refund_move("refund_completed", "requested"),
            Move::To("completed")
        );
        assert_eq!(refund_move("refund_failed", "failed"), Move::Already);
        assert!(matches!(
            refund_move("refund_failed", "completed"),
            Move::NotFrom(_)
        ));
    }
}
