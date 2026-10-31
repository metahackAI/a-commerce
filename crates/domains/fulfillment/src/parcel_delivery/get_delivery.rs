//! `get_delivery`: one parcel with its state, tracking and history.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{events, visible_delivery};
use crate::errors::DELIVERY_NOT_FOUND;
use crate::model::delivery::local_date;
use crate::repo;
use crate::select;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetDeliveryInput {
    /// The parcel.
    #[schemars(regex(pattern = r"^dlv_[a-z0-9_]+$"), example = &"dlv_20402")]
    pub delivery_id: String,
    /// Top-level fields to return, for example only the state.
    #[schemars(example = ["delivery_state", "expected_arrival_until"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetDelivery;

impl Operation for GetDelivery {
    const META: OpMeta = OpMeta {
        address: "fulfillment.parcel_delivery.get",
        tool: "get_delivery",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/deliveries/{delivery_id}",
        summary: "One parcel: where it goes, carrier and tracking number, its state and history, and when it is expected.",
        consumes: &[Slot::one("delivery_id")],
        produces: &[Slot::one("order_id")],
        checks: &[],
        errors: &[DELIVERY_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetDeliveryInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetDelivery {
    async fn read(&self, cx: &Cx, input: GetDeliveryInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::delivery(&mut tx, &input.delivery_id).await?;
        let d = visible_delivery(&mut tx, row).await?.ok_or_else(|| {
            ToolError::new(
                DELIVERY_NOT_FOUND,
                format!("There is no parcel {}.", input.delivery_id),
            )
            .detail(json!({"entity": "parcel_delivery", "id": input.delivery_id}))
            .next("get_order")
        })?;
        let recorded = repo::delivery_events(&mut tx, &d.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut history: Vec<Value> = recorded
            .iter()
            .map(|e| {
                json!({"at": cx.time.format(e.start_time), "delivery_state": e.to_state,
                       "by": e.instrument, "location": null})
            })
            .collect();
        if history.is_empty() {
            history = events(cx, &d);
        } else {
            // Carrier scans are not recorded here; add them from the parcel's own times.
            history.extend(events(cx, &d).into_iter().filter(|e| {
                matches!(
                    e["delivery_state"].as_str(),
                    Some("picked_up" | "delivered")
                )
            }));
        }
        let zone = cx.time.zone;
        let out = json!({
            "id": d.id,
            "order_id": d.order_id,
            "delivery_address": d.delivery_address.get("street_address"),
            "tracking_number": d.tracking_number,
            "tracking_url": d.tracking_url,
            "carrier": d.carrier,
            "has_delivery_method": d.has_delivery_method,
            "delivery_state": d.delivery_state,
            "expected_arrival_until": d.expected_arrival_until.map(|t| local_date(t, zone).to_string()),
            "events": history,
        });
        Ok(select(out, &input.fields.unwrap_or_default()))
    }
}
