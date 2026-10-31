//! The `parcel_delivery` resource (docs/api/fulfillment.md `## parcel_delivery`): parcels of
//! order items, their warehouse steps and carrier progress. This module holds what the
//! operations share: which parcels a caller may see, a parcel's history as events, the lines a
//! parcel carries, and the errors for a parcel that is missing or in the wrong state.

pub(crate) mod confirm_shipment;
pub(crate) mod get_delivery;
pub(crate) mod get_delivery_dates;
pub(crate) mod register_shortage;
pub(crate) mod schedule_shipment;
pub(crate) mod track_shipment;

pub use confirm_shipment::ConfirmShipment;
pub use get_delivery::GetDelivery;
pub use get_delivery_dates::GetDeliveryDates;
pub use register_shortage::RegisterShortage;
pub use schedule_shipment::ScheduleShipment;
pub use track_shipment::TrackShipment;

use acom_contract::{ErrorCode, ToolError};
use acom_sales::service::for_fulfillment as sales;
use acom_store::{Cx, TxCx};
use serde_json::{Value, json};

use crate::errors::DELIVERY_NOT_FOUND;
use crate::repo::DeliveryRow;
use crate::shopper;

/// Parcel states a parcel no longer leaves.
pub(crate) const CLOSED_PARCEL_STATES: &[&str] = &["cancelled", "delivered"];

pub(crate) fn delivery_not_found(delivery_id: &str) -> ToolError {
    ToolError::new(
        DELIVERY_NOT_FOUND,
        format!("There is no parcel {delivery_id}."),
    )
    .detail(json!({"entity": "parcel_delivery", "id": delivery_id}))
    .next("get_order")
}

pub(crate) fn parcel_state_error(d: &DeliveryRow, allowed: &[&str], doing: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INVALID_STATE,
        format!(
            "Parcel {} is {}; it cannot be {doing} now.",
            d.id,
            d.delivery_state.replace('_', " ")
        ),
    )
    .detail(json!({"entity": "parcel_delivery", "from": d.delivery_state, "allowed_to": allowed}))
    .next("get_delivery")
}

/// The order lines a parcel carries: its recorded lines, or every line of the order that is
/// not cancelled when the parcel recorded none.
pub(crate) fn parcel_lines(d: &DeliveryRow, items: &[sales::OrderItem]) -> Vec<(String, i32)> {
    d.lines().unwrap_or_else(|| {
        items
            .iter()
            .filter(|i| i.order_item_status != "cancelled")
            .map(|i| (i.id.clone(), i.order_quantity))
            .collect()
    })
}

/// A delivery the caller may see: shoppers only see parcels of their own orders.
pub(crate) async fn visible_delivery(
    tx: &mut TxCx,
    d: Option<DeliveryRow>,
) -> Result<Option<DeliveryRow>, ToolError> {
    let Some(d) = d else {
        return Ok(None);
    };
    if let Some(me) = shopper(&tx.principal) {
        let owner = sales::order(tx, &d.order_id)
            .await?
            .and_then(|o| o.customer_id);
        if owner.as_deref() != Some(me.as_str()) {
            return Ok(None);
        }
    }
    Ok(Some(d))
}

/// The parcel's history as carrier events: picked up when shipped, delivered when delivered,
/// and the current state when no event explains it.
pub(crate) fn events(cx: &Cx, d: &DeliveryRow) -> Vec<Value> {
    let place = {
        let a = &d.delivery_address;
        match (
            a.get("address_locality").and_then(Value::as_str),
            a.get("address_region").and_then(Value::as_str),
        ) {
            (Some(city), Some(region)) => Some(format!("{city}, {region}")),
            (Some(city), None) => Some(city.to_owned()),
            _ => None,
        }
    };
    let mut out = Vec::new();
    if let Some(t) = d.shipped_at {
        out.push(json!({"at": cx.time.format(t), "delivery_state": "picked_up", "location": null}));
    }
    if let Some(t) = d.delivered_at {
        out.push(
            json!({"at": cx.time.format(t), "delivery_state": "delivered", "location": place}),
        );
    }
    // A parcel in transit has no scan of its own beyond the pickup: the pickup is its last event.
    // Another state shows when the row last changed, but only when that time makes sense (after
    // the pickup, not after now); a row's write time is not a carrier scan.
    let covered = (d.delivery_state == "delivered" && d.delivered_at.is_some())
        || (matches!(d.delivery_state.as_str(), "picked_up" | "in_transit")
            && d.shipped_at.is_some());
    let plausible =
        d.shipped_at.is_none_or(|s| d.updated_at >= s) && d.updated_at <= cx.clock.now();
    if !covered && plausible {
        out.push(
            json!({"at": cx.time.format(d.updated_at), "delivery_state": d.delivery_state,
                        "location": null}),
        );
    }
    out
}
