//! The `reorder` resource (docs/api/catalog.md#reorder): stock the store orders from its supplier
//! (`create_reorder`, sized by `suggest_reorder`), receives (`receive_reorder`), and what
//! `get_inventory` shows of it until it arrives.
//!
//! A reorder is an order the store places as the buyer. Each item is a variant and a quantity at
//! the variant's cost; the expected arrival is the store's date plus `reorder.lead_time_days`.
//! Ordered stock is not sellable, so nothing here touches `catalog.offer.inventory_level`.

pub mod create_reorder;
pub mod receive_reorder;
pub mod suggest_reorder;

use std::collections::BTreeMap;

use acom_contract::ToolError;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::db;

/// Reorders not received yet: their units are on order.
const OPEN: [&str; 2] = ["processing", "in_transit"];

/// Reorders of a product that have not arrived, soonest first, and the units each variant has
/// on order in them.
pub(crate) struct OnOrder {
    pub reorders: Vec<Value>,
    pub by_variant: BTreeMap<String, i64>,
}

pub(crate) async fn on_order(
    cx: &Cx,
    tx: &mut TxCx,
    product_id: &str,
) -> Result<OnOrder, ToolError> {
    let open: Vec<String> = OPEN.iter().map(|s| (*s).to_owned()).collect();
    let rows: Vec<(String, String, DateTime<Utc>, String, i32)> = sqlx::query_as(
        "SELECT r.id, r.order_status, r.expected_arrival_until, i.product_id, i.order_quantity
         FROM catalog.reorder r
         JOIN catalog.reorder_item i ON i.tenant_id = r.tenant_id AND i.reorder_id = r.id
         WHERE r.tenant_id = $1 AND r.product_group_id = $2 AND r.order_status = ANY($3)
         ORDER BY r.expected_arrival_until, r.id, i.position",
    )
    .bind(tx.tenant.as_str())
    .bind(product_id)
    .bind(&open)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    let mut reorders: Vec<Value> = Vec::new();
    let mut by_variant: BTreeMap<String, i64> = BTreeMap::new();
    for (id, status, arrives, variant, quantity) in rows {
        *by_variant.entry(variant).or_default() += i64::from(quantity);
        match reorders.last_mut() {
            Some(r) if r["id"] == id.as_str() => {
                let units = r["total_units"].as_i64().unwrap_or(0) + i64::from(quantity);
                r["total_units"] = json!(units);
            }
            _ => reorders.push(json!({"id": id, "order_status": status,
                                     "total_units": quantity,
                                     "expected_arrival_until": cx.time.format(arrives)})),
        }
    }
    Ok(OnOrder {
        reorders,
        by_variant,
    })
}
