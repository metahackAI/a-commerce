//! `track_shipment`: the carrier's tracking events for a parcel.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{events, visible_delivery};
use crate::errors::TRACKING_NOT_FOUND;
use crate::repo;

/// How to find the parcel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrackBy {
    /// By the parcel id (`delivery_id`).
    Delivery,
    /// By the carrier's tracking number (`tracking_number`).
    TrackingNumber,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrackShipmentInput {
    /// delivery: find it by delivery_id; tracking_number: by the carrier's number.
    #[schemars(example = &"delivery")]
    pub mode: TrackBy,
    /// The parcel, when mode is delivery.
    #[schemars(regex(pattern = r"^dlv_[a-z0-9_]+$"), example = &"dlv_20402")]
    pub delivery_id: Option<String>,
    /// The carrier's tracking number, when mode is tracking_number.
    #[schemars(length(min = 1, max = 64), example = &"1ZNW204020001")]
    pub tracking_number: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct TrackShipment;

impl Operation for TrackShipment {
    const META: OpMeta = OpMeta {
        address: "fulfillment.parcel_delivery.track",
        tool: "track_shipment",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/deliveries/{delivery_id}/tracking",
        summary: "The carrier's tracking events for a parcel, found by parcel id or tracking number.",
        consumes: &[Slot::one("delivery_id")],
        produces: &[],
        checks: &[],
        errors: &[TRACKING_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = TrackShipmentInput;
    type Output = Value;
}

impl ReadOp<Cx> for TrackShipment {
    async fn read(&self, cx: &Cx, input: TrackShipmentInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = match input.mode {
            TrackBy::Delivery => match &input.delivery_id {
                Some(id) => repo::delivery(&mut tx, id).await?,
                None => {
                    return Err(ToolError::invalid_input("mode delivery needs delivery_id.")
                        .detail(json!({"field": "delivery_id", "reason": "missing"})));
                }
            },
            TrackBy::TrackingNumber => match &input.tracking_number {
                Some(n) => repo::delivery_by_tracking(&mut tx, n).await?,
                None => {
                    return Err(ToolError::invalid_input(
                        "mode tracking_number needs tracking_number.",
                    )
                    .detail(json!({"field": "tracking_number", "reason": "missing"})));
                }
            },
        };
        let d = visible_delivery(&mut tx, row).await?.ok_or_else(|| {
            ToolError::new(
                TRACKING_NOT_FOUND,
                "No parcel matches that tracking request.",
            )
            .detail(json!({"entity": "parcel_delivery"}))
            .next("get_delivery")
        })?;
        tx.commit().await.map_err(ToolError::from)?;
        let all = events(cx, &d);
        let total = all.len() as i64;
        let items: Vec<Value> = all
            .into_iter()
            .skip(usize::try_from(page.offset).unwrap_or(0))
            .take(usize::try_from(page.limit).unwrap_or(0))
            .collect();
        let mut out = json!({"items": items, "total": total, "delivery_id": d.id,
                             "carrier": d.carrier, "tracking_number": d.tracking_number});
        if let Some(next) = page.next_cursor(items.len(), total) {
            out["next_cursor"] = json!(next);
        }
        Ok(out)
    }
}
