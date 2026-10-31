//! What sales reads from fulfillment: an order's parcel delivery and its claims, for `get_order`,
//! and the shipping zone of an address, for `change_shipping_address`. Sales cannot
//! depend on fulfillment (fulfillment depends on sales), so fulfillment installs this reader into
//! `acom_sales::service::providers` at start-up: call [`install`] from this crate's `register`.

use acom_contract::ToolError;
use acom_sales::service::providers::{self, AreaFacts, ClaimFacts, DeliveryFacts, Reading};
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};
use serde_json::Value;

/// Lets `get_order` show the parcel and the order's cancel, return and exchange requests.
pub fn install() {
    providers::provide_deliveries(order_delivery);
    providers::provide_claims(order_claims);
    providers::provide_areas(address_area);
}

/// The saved shipping zone an address falls in (a zone naming its state wins over one naming its
/// country); `None` when the store saved no zone for it, so sales uses the policy's default zone.
pub fn address_area<'a>(
    tx: &'a mut TxCx,
    country: &'a str,
    region: Option<&'a str>,
) -> Reading<'a, Option<AreaFacts>> {
    Box::pin(async move {
        let zones = crate::repo::zones(tx)
            .await?
            .iter()
            .map(crate::shipping_rate::zone_of)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(
            crate::model::zone::zone_for(&zones, country, region).map(|z| AreaFacts {
                name: z.name.clone(),
                surcharge: z.surcharge,
            }),
        )
    })
}

/// The claims of an order with the lines each covers, oldest first.
pub fn order_claims<'a>(tx: &'a mut TxCx, order_id: &'a str) -> Reading<'a, Vec<ClaimFacts>> {
    Box::pin(async move {
        #[allow(clippy::type_complexity)]
        let rows: Vec<(String, String, String, Vec<String>, DateTime<Utc>)> = sqlx::query_as(
            "SELECT c.id, c.claim_type, c.claim_status,
                    coalesce(array_agg(i.order_item_id ORDER BY i.order_item_id)
                             FILTER (WHERE i.order_item_id IS NOT NULL), '{}'),
                    c.start_time
             FROM fulfillment.claim c
             LEFT JOIN fulfillment.claim_item i ON i.tenant_id = c.tenant_id AND i.claim_id = c.id
             WHERE c.tenant_id = $1 AND c.order_id = $2
             GROUP BY c.id, c.claim_type, c.claim_status, c.start_time
             ORDER BY c.start_time, c.id",
        )
        .bind(tx.tenant.as_str())
        .bind(order_id)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(|e| ToolError::from(StoreError::from(e)))?;
        Ok(rows
            .into_iter()
            .map(
                |(id, claim_type, claim_status, order_item_ids, requested_at)| ClaimFacts {
                    id,
                    claim_type,
                    claim_status,
                    order_item_ids,
                    requested_at,
                },
            )
            .collect())
    })
}

/// The latest parcel delivery of an order, if there is one.
pub fn order_delivery<'a>(
    tx: &'a mut TxCx,
    order_id: &'a str,
) -> Reading<'a, Option<DeliveryFacts>> {
    Box::pin(async move {
        #[allow(clippy::type_complexity)]
        let row: Option<(
            String,
            Value,
            Option<DateTime<Utc>>,
            String,
            Option<DateTime<Utc>>,
        )> = sqlx::query_as(
            "SELECT id, delivery_address, expected_arrival_until, delivery_state, delivered_at
             FROM fulfillment.parcel_delivery
             WHERE tenant_id = $1 AND order_id = $2
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(tx.tenant.as_str())
        .bind(order_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(|e| ToolError::from(StoreError::from(e)))?;
        Ok(row.map(
            |(id, delivery_address, expected_arrival_until, delivery_state, delivered_at)| {
                DeliveryFacts {
                    id,
                    delivery_address,
                    expected_arrival_until,
                    delivery_state,
                    delivered_at,
                }
            },
        ))
    })
}
