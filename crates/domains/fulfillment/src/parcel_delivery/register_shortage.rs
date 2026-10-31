//! `register_shortage`: some lines of a parcel are short of stock, so the parcel waits
//! (`delivery.shortage_state`) until stock comes in; meanwhile the shopper may cancel them.

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
use crate::errors::{DELIVERY_NOT_FOUND, ORDER_ITEM_NOT_FOUND};
use crate::repo::{self, ClaimEvent};
use crate::rules;

/// Parcel states that can still go short: not yet labelled.
const SHORT_FROM: &[&str] = &["ship_instructed", "packed"];

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegisterShortageInput {
    /// The parcel, from `schedule_shipment` or `get_delivery`.
    #[schemars(regex(pattern = r"^dlv_[a-z0-9_]+$"), example = &"dlv_20455")]
    pub delivery_id: String,
    /// The lines in the parcel that are short of stock.
    #[schemars(length(min = 1), inner(regex(pattern = r"^oln_[a-z0-9_]+$")), example = ["oln_2"])]
    pub order_item_ids: Vec<String>,
    /// Why, in a sentence (for example the supplier's delay).
    #[schemars(length(max = 500), example = &"Supplier delivery slipped to Nov 12.")]
    pub reason: Option<String>,
}

/// The shortage, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShortagePlan {
    pub delivery_id: String,
    pub from_state: String,
    pub to_state: String,
    /// The short lines and what they were; empty for a reshipment.
    pub lines: Vec<(String, sales::ItemProgress)>,
    pub reason: Option<String>,
}

pub struct RegisterShortage;

impl Operation for RegisterShortage {
    const META: OpMeta = OpMeta {
        address: "fulfillment.parcel_delivery.register_shortage",
        tool: "register_shortage",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/fulfillment/deliveries/{delivery_id}/shortage",
        summary: "Register that lines of a parcel are short of stock: the parcel waits (awaiting_stock) until stock comes in, and the shopper may cancel those lines meanwhile.",
        consumes: &[Slot::one("delivery_id")],
        produces: &[Slot::one("delivery_id")],
        checks: &["delivery.shortage_state", "cancel.self_service_states"],
        errors: &[
            DELIVERY_NOT_FOUND,
            ORDER_ITEM_NOT_FOUND,
            ErrorCode::INVALID_STATE,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = RegisterShortageInput;
    type Output = Value;
}

impl WriteOp<Cx> for RegisterShortage {
    type Plan = ShortagePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: RegisterShortageInput,
    ) -> Result<Planned<ShortagePlan>, ToolError> {
        let rules = rules(&cx.policy)?;
        let to_state = rules
            .lookup("delivery.shortage_state")
            .and_then(Value::as_str)
            .unwrap_or("awaiting_stock")
            .to_owned();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let d = repo::delivery(&mut tx, &input.delivery_id)
            .await?
            .ok_or_else(|| delivery_not_found(&input.delivery_id))?;
        if !SHORT_FROM.contains(&d.delivery_state.as_str()) {
            return Err(parcel_state_error(&d, &[to_state.as_str()], "marked short"));
        }
        let items = sales::order_items(&mut tx, &d.order_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let in_parcel = parcel_lines(&d, &items);
        let mut changes = vec![Change {
            op: ChangeOp::Update,
            entity: "parcel_delivery".into(),
            id: d.id.clone(),
            from: json!(d.delivery_state),
            to: json!(to_state),
            note: None,
        }];
        let mut basis = vec![Basis {
            entity: "fulfillment.parcel_delivery".into(),
            id: d.id.clone(),
            version: i64::from(d.version),
        }];
        let mut lines = Vec::new();
        let mut names = Vec::new();
        for id in &input.order_item_ids {
            if !in_parcel.iter().any(|(l, _)| l == id) {
                return Err(ToolError::new(
                    ORDER_ITEM_NOT_FOUND,
                    format!("Parcel {} does not carry line {id}.", d.id),
                )
                .detail(json!({"entity": "order_item", "id": id}))
                .next("get_delivery"));
            }
            let Some(i) = items.iter().find(|i| &i.id == id) else {
                continue;
            };
            names.push(i.name.clone());
            if d.claim_id.is_some() || lines.iter().any(|(l, _)| l == id) {
                continue;
            }
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "order_item".into(),
                id: i.id.clone(),
                from: json!({"fulfillment_stage": i.fulfillment_stage}),
                to: json!({"fulfillment_stage": to_state}),
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
        let self_cancel = rules
            .list("cancel.self_service_states")
            .iter()
            .any(|s| s == &to_state);
        let count = if names.len() == 1 {
            "1 item short".to_owned()
        } else {
            format!("{} items short", names.len())
        };
        let checks = vec![
            Check {
                rule: "delivery.shortage_state".into(),
                passed: true,
                detail: Some(format!(
                    "{count} ({}); the parcel waits as {}",
                    names.join(", "),
                    to_state.replace('_', " ")
                )),
                fix_tool: None,
            },
            Check {
                rule: "cancel.self_service_states".into(),
                passed: true,
                detail: Some(if self_cancel {
                    "The customer can cancel it themselves while waiting".into()
                } else {
                    "Cancelling while it waits needs the store".into()
                }),
                fix_tool: (!self_cancel).then(|| "cancel_order_lines".into()),
            },
        ];
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            plan: ShortagePlan {
                delivery_id: d.id.clone(),
                from_state: d.delivery_state.clone(),
                to_state,
                lines,
                reason: input
                    .reason
                    .map(|r| r.trim().to_owned())
                    .filter(|r| !r.is_empty()),
            },
            changes,
            checks,
            basis,
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval
                        .reason
                        .unwrap_or_else(|| "Registers a stock shortage".into()),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ShortagePlan) -> Result<Committed, ToolError> {
        let done = Committed {
            id: plan.delivery_id.clone(),
            produced: Map::from_iter([("delivery_id".to_owned(), json!(plan.delivery_id))]),
        };
        let d = repo::delivery_for_update(tx, &plan.delivery_id)
            .await?
            .ok_or_else(|| delivery_not_found(&plan.delivery_id))?;
        if d.delivery_state == plan.to_state {
            return Ok(done);
        }
        if d.delivery_state != plan.from_state {
            return Err(parcel_state_error(
                &d,
                &[plan.to_state.as_str()],
                "marked short",
            ));
        }
        repo::set_delivery_state(tx, &d.id, &plan.to_state).await?;
        for (line, before) in &plan.lines {
            sales::set_item_progress(
                tx,
                line,
                Some(&before.order_item_status),
                Some(&plan.to_state),
            )
            .await?;
        }
        repo::append_delivery_event(
            tx,
            &d.id,
            &ClaimEvent {
                action_type: "UpdateAction",
                action_status: "active",
                instrument: Self::META.tool,
                from_state: Some(&plan.from_state),
                to_state: Some(&plan.to_state),
                result: Some(
                    json!({"order_item_ids": plan.lines.iter().map(|(l, _)| l).collect::<Vec<_>>(),
                                    "reason": plan.reason}),
                ),
            },
        )
        .await?;
        Ok(done)
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ShortagePlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let d = repo::delivery(&mut tx, &plan.delivery_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match d {
            None => vec![format!("parcel {} is missing", plan.delivery_id)],
            Some(d) if d.delivery_state == plan.to_state => Vec::new(),
            Some(d) => vec![format!("parcel {} is {}", d.id, d.delivery_state)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &ShortagePlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        let d = repo::delivery_for_update(tx, &plan.delivery_id)
            .await?
            .ok_or_else(|| delivery_not_found(&plan.delivery_id))?;
        if d.delivery_state != plan.to_state {
            return Err(parcel_state_error(
                &d,
                &[plan.from_state.as_str()],
                "taken back",
            ));
        }
        repo::set_delivery_state(tx, &d.id, &plan.from_state).await?;
        for (line, before) in &plan.lines {
            sales::set_item_progress(
                tx,
                line,
                Some(&before.order_item_status),
                before.fulfillment_stage.as_deref(),
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
                from_state: Some(&plan.to_state),
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

    #[test]
    fn names_at_least_one_line() {
        assert!(
            serde_json::from_value::<RegisterShortageInput>(
                json!({"delivery_id": "dlv_20455", "order_item_ids": ["oln_2"], "reason": "late"})
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<RegisterShortageInput>(json!({"delivery_id": "dlv_20455"}))
                .is_err()
        );
    }
}
