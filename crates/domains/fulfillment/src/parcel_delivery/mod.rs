//! The `parcel_delivery` resource (docs/api/fulfillment.md `## parcel_delivery`): parcels of
//! order items, their warehouse steps and carrier progress. This module holds what the
//! operations share: which parcels a caller may see and a parcel's history as events.

pub(crate) mod get_delivery;
pub(crate) mod get_delivery_dates;
pub(crate) mod track_shipment;

pub use get_delivery::GetDelivery;
pub use get_delivery_dates::GetDeliveryDates;
pub use track_shipment::TrackShipment;

use acom_contract::ToolError;
use acom_sales::service::for_fulfillment as sales;
use acom_store::{Cx, TxCx};
use serde_json::{Value, json};

use crate::repo::DeliveryRow;
use crate::shopper;

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
