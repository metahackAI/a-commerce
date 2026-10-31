//! The `shipping_rate` resource (docs/api/fulfillment.md `## shipping_rate`): delivery zones and
//! what shipping costs in each (schema.org `ShippingRateSettings`). Until the store saves a
//! zone, every address uses `delivery.default_zone` from the policy. This module holds what the
//! operations share: the zones as the model sees them and how a zone reads on the wire.

pub(crate) mod list_shipping_zones;
pub(crate) mod manage_shipping_zone;
pub(crate) mod set_shipping_rate;

pub use list_shipping_zones::ListShippingZones;
pub use manage_shipping_zone::ManageShippingZone;
pub use set_shipping_rate::SetShippingRate;

use acom_contract::ToolError;
use acom_store::TxCx;
use serde_json::{Value, json};

use crate::errors::ZONE_NOT_FOUND;
use crate::model::Rules;
use crate::model::zone::{DEFAULT_ZONE_ID, Zone, default_zone, regions};
use crate::repo::{self, ZoneRow};

/// A stored zone as the model sees it.
pub(crate) fn zone_of(row: &ZoneRow) -> Result<Zone, ToolError> {
    let v = row.values()?;
    Ok(Zone {
        id: row.id.clone(),
        name: v.name,
        zone_type: v.zone_type,
        shipping_destination: regions(&v.shipping_destination),
        fee_type: v.fee_type,
        shipping_rate: v.shipping_rate,
        free_over: v.free_over,
        surcharge: v.surcharge,
        transit_business_days: i64::from(v.transit_business_days),
    })
}

/// The store's zones: the saved ones, or the policy's default zone while none is saved.
pub(crate) async fn zones(tx: &mut TxCx, rules: &Rules) -> Result<Vec<Zone>, ToolError> {
    let rows = repo::zones(tx).await?;
    if rows.is_empty() {
        return Ok(vec![default_zone(rules)]);
    }
    rows.iter().map(zone_of).collect()
}

/// A zone as tools return it.
pub(crate) fn zone_json(z: &Zone) -> Value {
    let mut v = json!({
        "id": z.id,
        "name": z.name,
        "zone_type": z.zone_type,
        "shipping_destination": z.shipping_destination,
        "fee_type": z.fee_type,
        "shipping_rate": z.shipping_rate,
        "free_over": z.free_over,
        "surcharge": z.surcharge,
        "transit_business_days": z.transit_business_days,
    });
    if z.id == DEFAULT_ZONE_ID {
        v["note"] = json!(
            "From the policy (delivery.default_zone) until the store saves a zone with manage_shipping_zone"
        );
    }
    v
}

pub(crate) fn zone_not_found(zone_id: &str, next: &'static str) -> ToolError {
    let message = if zone_id == DEFAULT_ZONE_ID {
        "The default zone comes from the policy and is not saved; save a zone with manage_shipping_zone first.".to_owned()
    } else {
        format!("There is no shipping zone {zone_id}.")
    };
    ToolError::new(ZONE_NOT_FOUND, message)
        .detail(json!({"entity": "shipping_zone", "id": zone_id}))
        .next(next)
}
