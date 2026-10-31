//! `confirm_shipment`: the parcel is labelled with the carrier's tracking number
//! (`label_created`); its lines are shipped, and the order is in transit once every line is.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_sales::service::for_fulfillment as sales;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{delivery_not_found, parcel_lines, parcel_state_error};
use crate::errors::{DELIVERY_NOT_FOUND, DUPLICATE_TRACKING_NUMBER, TRACKING_REQUIRED};
use crate::model::delivery::tracking_format;
use crate::repo::{self, ClaimEvent};
use crate::rules;

/// What a carrier is in this system: not connected. Said in the label's record.
const CARRIER_NOTE: &str = "No carrier is connected in this demo: the label is recorded, and carrier events move the parcel from here.";

/// Parcel states from which a label can be made: packed, or still being picked (the label
/// then also records the packing).
const LABEL_FROM: &[&str] = &["packed", "ship_instructed"];

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfirmShipmentInput {
    /// The parcel, from `schedule_shipment` or `get_delivery`.
    #[schemars(regex(pattern = r"^dlv_[a-z0-9_]+$"), example = &"dlv_20455")]
    pub delivery_id: String,
    /// The carrier's tracking number on the label.
    #[schemars(length(max = 64), example = &"1ZNW204550001")]
    pub tracking_number: String,
    /// The carrier, for example UPS, USPS or FedEx.
    #[schemars(length(min = 1, max = 40), example = &"UPS")]
    pub carrier: String,
}

/// The label, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfirmPlan {
    pub delivery_id: String,
    pub order_id: String,
    pub from_state: String,
    pub carrier: String,
    pub tracking_number: String,
    /// The label the parcel had before (carrier, tracking number).
    pub label_before: (Option<String>, Option<String>),
    /// The order's own lines and what they were; empty for a reshipment.
    pub lines: Vec<(String, sales::ItemProgress)>,
    /// The order's status before, when every line is now shipped.
    pub order_status_before: Option<String>,
}

pub struct ConfirmShipment;

impl Operation for ConfirmShipment {
    const META: OpMeta = OpMeta {
        address: "fulfillment.parcel_delivery.confirm",
        tool: "confirm_shipment",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/fulfillment/deliveries/{delivery_id}/confirm",
        summary: "Confirm a parcel's shipment with the carrier's tracking number: the parcel becomes label_created, its lines shipped, and the order in_transit once every line has shipped.",
        consumes: &[Slot::one("delivery_id")],
        produces: &[Slot::one("delivery_id")],
        checks: &["delivery.tracking"],
        errors: &[
            DELIVERY_NOT_FOUND,
            TRACKING_REQUIRED,
            DUPLICATE_TRACKING_NUMBER,
            ErrorCode::INVALID_STATE,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = ConfirmShipmentInput;
    type Output = Value;
}

impl WriteOp<Cx> for ConfirmShipment {
    type Plan = ConfirmPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ConfirmShipmentInput,
    ) -> Result<Planned<ConfirmPlan>, ToolError> {
        let number = input.tracking_number.trim().to_owned();
        if number.is_empty() {
            return Err(ToolError::new(
                TRACKING_REQUIRED,
                "Give the tracking number printed on the label.",
            )
            .detail(json!({"field": "tracking_number", "reason": "missing"}))
            .next("confirm_shipment"));
        }
        let carrier = input.carrier.trim().to_owned();
        if carrier.is_empty() {
            return Err(ToolError::invalid_input("Name the carrier.")
                .detail(json!({"field": "carrier", "reason": "missing"})));
        }
        let rules = rules(&cx.policy)?;
        let format = tracking_format(&rules, &carrier, &number);
        if let Some((false, why)) = &format {
            return Err(ToolError::invalid_input(format!("{why}; {number} is not one.")).detail(
                json!({"field": "tracking_number", "reason": "format", "rule": "delivery.tracking"}),
            ));
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let d = repo::delivery(&mut tx, &input.delivery_id)
            .await?
            .ok_or_else(|| delivery_not_found(&input.delivery_id))?;
        if !LABEL_FROM.contains(&d.delivery_state.as_str()) {
            return Err(parcel_state_error(&d, &["label_created"], "labelled"));
        }
        if let Some(other) = repo::tracking_taken(&mut tx, &carrier, &number, &d.id).await? {
            return Err(ToolError::new(
                DUPLICATE_TRACKING_NUMBER,
                format!("{carrier} {number} is already on parcel {other}."),
            )
            .detail(json!({"entity": "parcel_delivery", "field": "tracking_number", "existing_id": other}))
            .next("get_delivery"));
        }
        let items = sales::order_items(&mut tx, &d.order_id).await?;
        let order = sales::order(&mut tx, &d.order_id).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let mut changes = vec![Change {
            op: ChangeOp::Update,
            entity: "parcel_delivery".into(),
            id: d.id.clone(),
            from: json!(d.delivery_state),
            to: json!("label_created"),
            note: Some(CARRIER_NOTE.into()),
        }];
        let mut basis = vec![Basis {
            entity: "fulfillment.parcel_delivery".into(),
            id: d.id.clone(),
            version: i64::from(d.version),
        }];
        let mut lines = Vec::new();
        if d.claim_id.is_none() {
            for (id, _) in parcel_lines(&d, &items) {
                let Some(i) = items.iter().find(|i| i.id == id) else {
                    continue;
                };
                if i.order_item_status == "cancelled" {
                    continue;
                }
                changes.push(Change {
                    op: ChangeOp::Update,
                    entity: "order_item".into(),
                    id: i.id.clone(),
                    from: json!({"order_item_status": i.order_item_status,
                                 "fulfillment_stage": i.fulfillment_stage}),
                    to: json!({"order_item_status": "in_transit", "fulfillment_stage": "shipped"}),
                    note: None,
                });
                basis.push(Basis {
                    entity: "sales.order_item".into(),
                    id: i.id.clone(),
                    version: i64::from(i.version),
                });
                lines.push((
                    i.id.clone(),
                    sales::ItemProgress {
                        order_item_status: i.order_item_status.clone(),
                        fulfillment_stage: i.fulfillment_stage.clone(),
                    },
                ));
            }
        }
        // The order is in transit once no line waits any more.
        let all_shipped = d.claim_id.is_none()
            && items.iter().all(|i| {
                i.order_item_status == "cancelled"
                    || lines.iter().any(|(id, _)| id == &i.id)
                    || i.fulfillment_stage.as_deref() == Some("shipped")
                    || matches!(
                        i.order_item_status.as_str(),
                        "in_transit" | "delivered" | "returned"
                    )
            });
        let order_status_before = order
            .filter(|o| all_shipped && o.order_status == "processing")
            .map(|o| {
                changes.push(Change {
                    op: ChangeOp::Update,
                    entity: "order".into(),
                    id: o.id.clone(),
                    from: json!(o.order_status),
                    to: json!("in_transit"),
                    note: None,
                });
                o.order_status
            });
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            plan: ConfirmPlan {
                delivery_id: d.id.clone(),
                order_id: d.order_id.clone(),
                from_state: d.delivery_state.clone(),
                carrier: carrier.clone(),
                tracking_number: number,
                label_before: (d.carrier.clone(), d.tracking_number.clone()),
                lines,
                order_status_before,
            },
            changes,
            checks: vec![Check {
                rule: "delivery.tracking".into(),
                passed: true,
                detail: Some(match format {
                    Some((_, ok)) => ok,
                    None => format!("No number format on file for {carrier}; not checked"),
                }),
                fix_tool: None,
            }],
            basis,
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval
                        .reason
                        .unwrap_or_else(|| "Confirms a shipment".into()),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ConfirmPlan) -> Result<Committed, ToolError> {
        let done = Committed {
            id: plan.delivery_id.clone(),
            produced: Map::from_iter([("delivery_id".to_owned(), json!(plan.delivery_id))]),
        };
        let d = repo::delivery_for_update(tx, &plan.delivery_id)
            .await?
            .ok_or_else(|| delivery_not_found(&plan.delivery_id))?;
        if d.delivery_state == "label_created"
            && d.tracking_number.as_deref() == Some(plan.tracking_number.as_str())
        {
            return Ok(done);
        }
        if d.delivery_state != plan.from_state {
            return Err(parcel_state_error(&d, &["label_created"], "labelled"));
        }
        repo::set_delivery_state(tx, &d.id, "label_created").await?;
        repo::set_delivery_label(tx, &d.id, Some(&plan.carrier), Some(&plan.tracking_number))
            .await?;
        for (line, _) in &plan.lines {
            sales::set_item_progress(tx, line, Some("in_transit"), Some("shipped")).await?;
        }
        if plan.order_status_before.is_some() {
            sales::set_order_status(
                tx,
                &plan.order_id,
                "in_transit",
                "UpdateAction",
                Self::META.tool,
            )
            .await?;
        }
        repo::append_delivery_event(
            tx,
            &d.id,
            &ClaimEvent {
                action_type: "UpdateAction",
                action_status: "completed",
                instrument: Self::META.tool,
                from_state: Some(&plan.from_state),
                to_state: Some("label_created"),
                result: Some(
                    json!({"carrier": plan.carrier, "tracking_number": plan.tracking_number,
                                    "carrier_note": CARRIER_NOTE}),
                ),
            },
        )
        .await?;
        Ok(done)
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ConfirmPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let d = repo::delivery(&mut tx, &plan.delivery_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match d {
            None => vec![format!("parcel {} is missing", plan.delivery_id)],
            Some(d) if d.tracking_number.as_deref() == Some(plan.tracking_number.as_str()) => {
                Vec::new()
            }
            Some(d) => vec![format!(
                "parcel {} has no label {}",
                d.id, plan.tracking_number
            )],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &ConfirmPlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        let d = repo::delivery_for_update(tx, &plan.delivery_id)
            .await?
            .ok_or_else(|| delivery_not_found(&plan.delivery_id))?;
        if d.delivery_state != "label_created" {
            return Err(parcel_state_error(
                &d,
                &[plan.from_state.as_str()],
                "taken back",
            ));
        }
        repo::set_delivery_state(tx, &d.id, &plan.from_state).await?;
        repo::set_delivery_label(
            tx,
            &d.id,
            plan.label_before.0.as_deref(),
            plan.label_before.1.as_deref(),
        )
        .await?;
        for (line, before) in &plan.lines {
            sales::set_item_progress(
                tx,
                line,
                Some(&before.order_item_status),
                before.fulfillment_stage.as_deref(),
            )
            .await?;
        }
        if let Some(before) = &plan.order_status_before {
            sales::set_order_status(
                tx,
                &plan.order_id,
                before,
                "UpdateAction",
                "undo_change_set",
            )
            .await?;
        }
        repo::append_delivery_event(
            tx,
            &d.id,
            &ClaimEvent {
                action_type: "UpdateAction",
                action_status: "completed",
                instrument: "undo_change_set",
                from_state: Some("label_created"),
                to_state: Some(&plan.from_state),
                result: None,
            },
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{input, offline};

    #[tokio::test]
    async fn needs_a_tracking_number_in_the_carrier_format() {
        let e = ConfirmShipment
            .preview(
                &offline("adm_sj"),
                input(
                    json!({"delivery_id": "dlv_20455", "tracking_number": " ", "carrier": "UPS"}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "tracking_required");
        assert_eq!(e.next_tool.as_deref(), Some("confirm_shipment"));
        let e = ConfirmShipment
            .preview(
                &offline("adm_sj"),
                input(json!({"delivery_id": "dlv_20455", "tracking_number": "9400111", "carrier": "UPS"})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
        assert_eq!(e.detail.as_ref().unwrap()["rule"], "delivery.tracking");
    }
}
