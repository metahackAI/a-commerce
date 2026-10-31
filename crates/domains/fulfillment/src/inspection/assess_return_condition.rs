//! `assess_return_condition`: record the inspection of items that came back. Only items found
//! as new go back to stock (`return.inventory_restore_when`); an exchange whose item is as new
//! moves on to its reshipment (`exchange.reship_after: inspected`).

use acom_catalog::service::for_fulfillment as catalog;
use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::enums::{InspectionCondition, Liability};
use acom_sales::service::for_fulfillment as sales;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::claim::{claim_not_found, pays_sentence, visible_claim};
use crate::errors::{CLAIM_NOT_FOUND, NOT_PICKED_UP};
use crate::model::holds;
use crate::repo::{self, ClaimEvent, ClaimRow, NewInspection};
use crate::rules;

/// What the inspector recommends next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InspectionNext {
    /// Go ahead: refund the return, or reship the exchange.
    Approve,
    /// Keep the claim at inspected while someone looks again.
    Hold,
    /// Recommend declining (then `reject_claim` with `inspection_failed`).
    Decline,
}

impl InspectionNext {
    fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Hold => "hold",
            Self::Decline => "decline",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssessReturnConditionInput {
    /// The return or exchange whose items came back (status picked_up).
    #[schemars(regex(pattern = r"^clm_[a-z0-9_]+$"), example = &"clm_20418")]
    pub claim_id: String,
    /// What was found: as_new (goes back to stock), damaged, or missing_parts.
    #[schemars(example = &"as_new")]
    pub condition: InspectionCondition,
    /// Who is at fault, when the inspection shows something other than the request said.
    #[schemars(example = &"customer_fault")]
    pub liability: Option<Liability>,
    /// approve (default when as new), hold, or decline (then reject_claim).
    #[schemars(example = &"approve")]
    pub next: Option<InspectionNext>,
    /// The inspector's note.
    #[schemars(length(max = 500), example = &"Tags on, no wear.")]
    pub note: Option<String>,
}

/// The inspection, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InspectionPlan {
    pub inspection_id: String,
    pub claim_id: String,
    pub from_status: String,
    pub to_status: String,
    pub condition: String,
    /// The claim's liability before and after, when the inspection changes it.
    pub liability_change: Option<(String, String)>,
    pub liability: Option<String>,
    /// `(variant_id, units)` put back in stock.
    pub restock: Vec<(String, i32)>,
    pub next: String,
    pub note: Option<String>,
}

pub struct AssessReturnCondition;

impl Operation for AssessReturnCondition {
    const META: OpMeta = OpMeta {
        address: "fulfillment.inspection.assess",
        tool: "assess_return_condition",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/fulfillment/claims/{claim_id}/inspection",
        summary: "Record the inspection of items that came back for a return or exchange: as new (back to stock; an exchange's replacement ships), damaged or missing parts. Sets the claim to inspected.",
        consumes: &[Slot::one("claim_id")],
        produces: &[Slot::one("inspection_id")],
        checks: &[
            "return.inventory_restore_when",
            "liability.pays",
            "exchange.reship_after",
        ],
        errors: &[CLAIM_NOT_FOUND, NOT_PICKED_UP, ErrorCode::INVALID_STATE],
        audience: Audience::Operator,
        action_type: ActionType("CheckAction"),
        person_scope: "",
    };
    type Input = AssessReturnConditionInput;
    type Output = Value;
}

fn state_error(c: &ClaimRow) -> ToolError {
    let before_pickup = [
        "charge_due",
        "requested",
        "photos_requested",
        "approved",
        "pickup_instructed",
    ];
    if before_pickup.contains(&c.claim_status.as_str()) {
        ToolError::new(
            NOT_PICKED_UP,
            format!(
                "Claim {} is {}: the items have not come back yet.",
                c.id,
                c.claim_status.replace('_', " ")
            ),
        )
        .detail(json!({"entity": "claim", "from": c.claim_status, "allowed_to": ["inspected"]}))
        .next("get_claim")
    } else {
        ToolError::new(
            ErrorCode::INVALID_STATE,
            format!(
                "Claim {} is {}; there is nothing to inspect.",
                c.id,
                c.claim_status.replace('_', " ")
            ),
        )
        .detail(json!({"entity": "claim", "from": c.claim_status, "allowed_to": []}))
        .next("get_claim")
    }
}

impl WriteOp<Cx> for AssessReturnCondition {
    type Plan = InspectionPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: AssessReturnConditionInput,
    ) -> Result<Planned<InspectionPlan>, ToolError> {
        let rules = rules(&cx.policy)?;
        let condition = input.condition.as_str();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = visible_claim(&mut tx, &input.claim_id, "search_claims").await?;
        if c.claim_type == "cancel" || c.claim_status != "picked_up" {
            return Err(state_error(&c));
        }
        if let Some(i) = repo::inspection(&mut tx, &c.id).await? {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("Claim {} was already inspected ({}).", c.id, i.id),
            )
            .detail(json!({"entity": "inspection", "existing_id": i.id}))
            .next("get_claim"));
        }
        let items = repo::claim_items(&mut tx, std::slice::from_ref(&c.id)).await?;
        let lines = sales::items(
            &mut tx,
            &items
                .iter()
                .map(|i| i.order_item_id.clone())
                .collect::<Vec<_>>(),
        )
        .await?;
        let restores = rules
            .lookup("return.inventory_restore_when")
            .is_some_and(|w| holds(w, &json!({"inspection_condition": condition}), &rules));
        let mut restock = Vec::new();
        let mut changes = Vec::new();
        let mut units_back = 0;
        if restores {
            for i in &items {
                let Some(line) = lines.iter().find(|l| l.id == i.order_item_id) else {
                    continue;
                };
                if let Some(v) = catalog::variant(&mut tx, &line.variant_id).await?
                    && v.is_managed
                    && let Some(s) = &v.stock
                {
                    changes.push(Change {
                        op: ChangeOp::Update,
                        entity: "inventory_level".into(),
                        id: v.id.clone(),
                        from: json!(s.inventory_level),
                        to: json!(s.inventory_level + i.order_quantity),
                        note: None,
                    });
                    restock.push((v.id.clone(), i.order_quantity));
                }
                units_back += i.order_quantity;
            }
        }
        tx.commit().await.map_err(ToolError::from)?;

        let next = input.next.unwrap_or(if condition == "as_new" {
            InspectionNext::Approve
        } else {
            InspectionNext::Hold
        });
        let reship_after = rules
            .lookup("exchange.reship_after")
            .and_then(Value::as_str)
            .unwrap_or("inspected");
        let reships = c.claim_type == "exchange"
            && condition == "as_new"
            && next == InspectionNext::Approve
            && reship_after == "inspected";
        let to_status = if reships {
            "reship_requested"
        } else {
            "inspected"
        };
        let liability = input.liability.map(|l| l.as_str().to_owned());
        let liability_change = liability
            .as_ref()
            .filter(|l| **l != c.liability)
            .map(|l| (c.liability.clone(), l.clone()));
        let inspection_id = format!("ins_{}", acom_core::id::new_suffix());

        changes.insert(
            0,
            Change {
                op: ChangeOp::Update,
                entity: "claim".into(),
                id: c.id.clone(),
                from: json!(c.claim_status),
                to: json!(to_status),
                note: None,
            },
        );
        changes.push(Change {
            op: ChangeOp::Create,
            entity: "inspection".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"claim_id": c.id, "condition": condition,
                       "liability": liability.as_ref().unwrap_or(&c.liability), "next": next.as_str()}),
            note: None,
        });
        let condition_text = condition.replace('_', " ");
        let mut checks = vec![Check {
            rule: "return.inventory_restore_when".into(),
            passed: true,
            detail: Some(if restores {
                let units = if units_back == 1 {
                    "1 unit".to_owned()
                } else {
                    format!("{units_back} units")
                };
                format!("Condition {condition_text}: {units} back to stock")
            } else {
                format!("Condition {condition_text}: nothing goes back to stock")
            }),
            fix_tool: None,
        }];
        if let Some((_, to)) = &liability_change {
            checks.push(Check {
                rule: "liability.pays".into(),
                passed: true,
                detail: Some(pays_sentence(&rules, to)),
                fix_tool: None,
            });
        }
        if c.claim_type == "exchange" {
            checks.push(Check {
                rule: "exchange.reship_after".into(),
                passed: true,
                detail: Some(if reships {
                    "The replacement can ship now: schedule_shipment".to_owned()
                } else {
                    "The replacement waits: the item did not pass as new".to_owned()
                }),
                fix_tool: Some(if reships {
                    "schedule_shipment".into()
                } else {
                    "reject_claim".into()
                }),
            });
        }
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            plan: InspectionPlan {
                inspection_id,
                claim_id: c.id.clone(),
                from_status: c.claim_status.clone(),
                to_status: to_status.into(),
                condition: condition.into(),
                liability_change,
                liability,
                restock,
                next: next.as_str().into(),
                note: input
                    .note
                    .map(|n| n.trim().to_owned())
                    .filter(|n| !n.is_empty()),
            },
            changes,
            checks,
            basis: vec![Basis {
                entity: "fulfillment.claim".into(),
                id: c.id,
                version: i64::from(c.version),
            }],
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval
                        .reason
                        .unwrap_or_else(|| "Records an inspection".into()),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &InspectionPlan) -> Result<Committed, ToolError> {
        let done = Committed {
            id: plan.inspection_id.clone(),
            produced: Map::from_iter([
                ("inspection_id".to_owned(), json!(plan.inspection_id)),
                ("claim_id".to_owned(), json!(plan.claim_id)),
            ]),
        };
        let c = repo::claim_for_update(tx, &plan.claim_id)
            .await?
            .ok_or_else(|| claim_not_found(&plan.claim_id, "search_claims"))?;
        if let Some(i) = repo::inspection(tx, &c.id).await? {
            if i.id == plan.inspection_id {
                return Ok(done);
            }
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("Claim {} was already inspected ({}).", c.id, i.id),
            )
            .next("get_claim"));
        }
        if c.claim_status != plan.from_status {
            return Err(state_error(&c));
        }
        let agent = tx.principal.subject.clone();
        let now = tx.clock.now();
        repo::insert_inspection(
            tx,
            &NewInspection {
                id: &plan.inspection_id,
                claim_id: &plan.claim_id,
                condition: &plan.condition,
                liability: plan.liability.as_deref(),
                description: plan.note.as_deref(),
                agent_id: &agent,
                at: now,
            },
        )
        .await?;
        if let Some((_, to)) = &plan.liability_change {
            repo::set_liability(tx, &plan.claim_id, to).await?;
        }
        repo::move_claim(tx, &plan.claim_id, &plan.from_status, &plan.to_status).await?;
        for (variant, units) in &plan.restock {
            catalog::move_stock(tx, variant, *units).await?;
        }
        repo::append_event(
            tx,
            &plan.claim_id,
            &ClaimEvent {
                action_type: "CheckAction",
                action_status: "completed",
                instrument: Self::META.tool,
                from_state: Some(&plan.from_status),
                to_state: Some(&plan.to_status),
                result: Some(json!({"inspection_id": plan.inspection_id,
                                    "condition": plan.condition, "next": plan.next,
                                    "restocked": plan.restock})),
            },
        )
        .await?;
        Ok(done)
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &InspectionPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let i = repo::inspection(&mut tx, &plan.claim_id).await?;
        let c = repo::claim(&mut tx, &plan.claim_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        match i {
            Some(i) if i.id == plan.inspection_id && i.inspection_condition == plan.condition => {}
            Some(i) => mismatches.push(format!("claim {} has inspection {}", plan.claim_id, i.id)),
            None => mismatches.push(format!("inspection {} is missing", plan.inspection_id)),
        }
        match c {
            Some(c) if c.claim_status == plan.to_status || c.claim_status != plan.from_status => {}
            Some(c) => mismatches.push(format!("claim {} is {}", c.id, c.claim_status)),
            None => mismatches.push(format!("claim {} is missing", plan.claim_id)),
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &InspectionPlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        if !repo::move_claim(tx, &plan.claim_id, &plan.to_status, &plan.from_status).await? {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "Claim {} has moved on since the inspection and cannot be taken back.",
                    plan.claim_id
                ),
            )
            .next("get_claim"));
        }
        repo::delete_inspection(tx, &plan.inspection_id).await?;
        if let Some((from, _)) = &plan.liability_change {
            repo::set_liability(tx, &plan.claim_id, from).await?;
        }
        for (variant, units) in &plan.restock {
            catalog::move_stock(tx, variant, -units).await?;
        }
        repo::append_event(
            tx,
            &plan.claim_id,
            &ClaimEvent {
                action_type: "UpdateAction",
                action_status: "completed",
                instrument: "undo_change_set",
                from_state: Some(&plan.to_status),
                to_state: Some(&plan.from_status),
                result: Some(json!({"inspection_id": plan.inspection_id, "removed": true})),
            },
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::input;

    #[test]
    fn inputs_use_condition_words() {
        assert!(
            serde_json::from_value::<AssessReturnConditionInput>(
                json!({"claim_id": "clm_20418", "condition": "normal"})
            )
            .is_err()
        );
        let i: AssessReturnConditionInput =
            input(json!({"claim_id": "clm_20418", "condition": "as_new", "next": "hold"}));
        assert_eq!(i.next, Some(InspectionNext::Hold));
    }
}
