//! What `partner` reads from fulfillment for a seller's dashboard, grade and settlement: when the
//! seller's order lines shipped and arrived, and the claims (cancel, return, exchange) on them.
//! Partner knows which lines are the seller's; fulfillment knows parcels and claims.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};
use serde_json::Value;

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// One outgoing parcel of an order (exchange reships are left out).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shipment {
    pub order_id: String,
    /// The order lines in the parcel; `None` when the parcel carries the whole order.
    pub order_item_ids: Option<Vec<String>>,
    /// `delivery_state` word.
    pub delivery_state: String,
    pub shipped_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
}

/// The outgoing parcels of these orders.
pub async fn shipments(tx: &mut TxCx, order_ids: &[String]) -> Result<Vec<Shipment>, ToolError> {
    if order_ids.is_empty() {
        return Ok(Vec::new());
    }
    /// order_id, item_shipped, delivery_state, shipped_at, delivered_at
    type Row = (
        String,
        Option<Value>,
        String,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT order_id, item_shipped, delivery_state, shipped_at, delivered_at
         FROM fulfillment.parcel_delivery
         WHERE tenant_id = $1 AND order_id = ANY($2) AND claim_id IS NULL
         ORDER BY order_id, id",
    )
    .bind(tx.tenant.as_str())
    .bind(order_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(rows
        .into_iter()
        .map(
            |(order_id, items, delivery_state, shipped_at, delivered_at)| Shipment {
                order_id,
                order_item_ids: items.and_then(|v| {
                    v.as_array().map(|list| {
                        list.iter()
                            .filter_map(|i| i["order_item_id"].as_str().map(str::to_owned))
                            .collect()
                    })
                }),
                delivery_state,
                shipped_at,
                delivered_at,
            },
        )
        .collect())
}

/// One order line in a claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimLine {
    pub claim_id: String,
    /// `cancel`, `return` or `exchange`.
    pub claim_type: String,
    /// `claim_status` word.
    pub claim_status: String,
    /// `liability` word: who was at fault.
    pub liability: String,
    pub order_item_id: String,
    pub order_quantity: i32,
    /// When the claim was sent.
    pub start_time: DateTime<Utc>,
}

/// The claims on these order lines, oldest first.
pub async fn claims(tx: &mut TxCx, order_item_ids: &[String]) -> Result<Vec<ClaimLine>, ToolError> {
    if order_item_ids.is_empty() {
        return Ok(Vec::new());
    }
    /// id, claim_type, claim_status, liability, order_item_id, order_quantity, start_time
    type Row = (String, String, String, String, String, i32, DateTime<Utc>);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT c.id, c.claim_type, c.claim_status, c.liability, ci.order_item_id,
                ci.order_quantity, c.start_time
         FROM fulfillment.claim_item ci
         JOIN fulfillment.claim c ON c.tenant_id = ci.tenant_id AND c.id = ci.claim_id
         WHERE ci.tenant_id = $1 AND ci.order_item_id = ANY($2)
         ORDER BY c.start_time, c.id, ci.order_item_id",
    )
    .bind(tx.tenant.as_str())
    .bind(order_item_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(rows
        .into_iter()
        .map(
            |(
                claim_id,
                claim_type,
                claim_status,
                liability,
                order_item_id,
                order_quantity,
                start_time,
            )| ClaimLine {
                claim_id,
                claim_type,
                claim_status,
                liability,
                order_item_id,
                order_quantity,
                start_time,
            },
        )
        .collect())
}
