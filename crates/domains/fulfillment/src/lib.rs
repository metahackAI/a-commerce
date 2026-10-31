//! The `fulfillment` domain: claims (cancel, return, exchange), inspection, refunds, deliveries, shipping rates.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/fulfillment.md`.
//! Layout follows the API hierarchy `fulfillment.{resource}.{operation}`: one folder per
//! resource (`claim`, `inspection`, `refund`, `parcel_delivery`, `shipping_rate`) with one file
//! per operation, named by its tool (person-only endpoints included), and the resource's shared
//! code in its `mod.rs`. Domain-wide: `model` (pure business rules, unit-tested against scenes B2
//! and F5), `repo` (SQL). Other domains are reached only through their `service` modules, never
//! through their tables.

pub mod errors;
pub mod model;
pub mod service;

mod claim;
mod parcel_delivery;
mod refund;
mod repo;
#[cfg(test)]
mod tests;
mod watch;

use acom_contract::{ErrorCode, RegistryBuilder, ToolError};
use acom_sales::service::for_fulfillment as sales;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::errors::{NOT_YOUR_ORDER, ORDER_NOT_FOUND};
use crate::model::Rules;

/// Adds every operation of this domain to the registry.
pub fn register(b: &mut RegistryBuilder<Cx>) {
    // Readers that sales (get_order's parcel) and customer (get_return_history) call into.
    service::for_sales::install();
    service::for_customer::install();
    b.read(claim::SearchReturnRequests)
        .read(claim::EvaluateReturnRequest)
        .read(claim::SearchClaims)
        .read(claim::GetClaim)
        .read(claim::GetReturnOptions)
        .read(claim::GetCancelOptions)
        .write(claim::ApproveClaim)
        .write(claim::RejectClaim)
        .write(refund::RefundReturn)
        .read(refund::GetRefund)
        .read(parcel_delivery::GetDeliveryDates)
        .read(parcel_delivery::GetDelivery)
        .read(parcel_delivery::TrackShipment)
        .person(claim::SendReturnRequest)
        .person(claim::CancelMyOrder)
        .person(claim::WithdrawClaim);
    // Scheduled work (platform.job): the watcher over waiting return requests.
    b.job(watch::ReturnsOverRefundThreshold);
}

/// The store's fulfillment rules for this request.
pub(crate) fn rules(policy: &acom_policy::PolicySet) -> Result<Rules, ToolError> {
    Rules::load(policy)
}

/// The signed-in shopper, when the caller is one. Shoppers only see their own orders and
/// claims; operators and the system see the store's.
pub(crate) fn shopper(p: &acom_auth::Principal) -> Option<String> {
    p.customer_id().map(str::to_owned)
}

/// A time input in store time.
pub(crate) fn parse_time(
    time: &acom_core::StoreTime,
    field: &str,
    s: Option<&str>,
) -> Result<Option<DateTime<Utc>>, ToolError> {
    s.map(|s| {
        time.parse(s).map_err(|e| {
            ToolError::new(crate::errors::INVALID_FILTER, e.to_string())
                .detail(json!({"field": field, "reason": "format"}))
        })
    })
    .transpose()
}

/// An order the caller may act on: it exists, and a shopper may only use their own.
pub(crate) async fn order_for(
    tx: &mut TxCx,
    order_id: &str,
    shopper: Option<&str>,
    next: &'static str,
) -> Result<sales::Order, ToolError> {
    let order = sales::order(tx, order_id).await?.ok_or_else(|| {
        ToolError::new(ORDER_NOT_FOUND, format!("There is no order {order_id}."))
            .detail(json!({"entity": "order", "id": order_id}))
            .next(next)
    })?;
    if let Some(me) = shopper
        && order.customer_id.as_deref() != Some(me)
    {
        return Err(
            ToolError::new(NOT_YOUR_ORDER, "That order belongs to someone else.")
                .detail(json!({"reason": "owner"}))
                .next(next),
        );
    }
    Ok(order)
}

/// Where refunded money goes for a payment: `card ending 0937`, `loyalty points`, …
pub(crate) fn payment_label(p: &sales::Payment) -> String {
    match (p.payment_method.as_str(), &p.card_last4) {
        ("credit_card", Some(last4)) => format!("card ending {last4}"),
        ("credit_card", None) => "card".to_owned(),
        (other, _) => other.replace('_', " "),
    }
}

/// The payment a refund goes back to (`refund.to: original_payment`): the first cash-like
/// payment that went through, else the first payment.
pub(crate) fn original_payment(payments: &[sales::Payment]) -> Option<&sales::Payment> {
    payments
        .iter()
        .find(|p| p.payment_method != "loyalty_points" && p.payment_status == "payment_complete")
        .or_else(|| payments.first())
}

/// When the order's parcels were delivered (the latest delivery), if they were.
pub(crate) fn delivered_at(
    deliveries: &[repo::DeliveryRow],
    order_id: &str,
) -> Option<DateTime<Utc>> {
    deliveries
        .iter()
        .filter(|d| d.order_id == order_id && d.delivery_state == "delivered")
        .filter_map(|d| d.delivered_at)
        .max()
}

/// Keeps only the named top-level fields (all when `fields` is empty); `id` always stays.
pub(crate) fn select(v: Value, fields: &[String]) -> Value {
    if fields.is_empty() {
        return v;
    }
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter()
                .filter(|(k, _)| k == "id" || fields.iter().any(|f| f == k))
                .collect(),
        ),
        other => other,
    }
}

/// An internal error for a state the data should never be in.
pub(crate) fn broken(what: impl Into<String>) -> ToolError {
    ToolError::new(ErrorCode::INTERNAL, what)
}
