//! `cancel_order_lines`: the store cancels lines before they ship (out of stock, gift gone,
//! seller or store decision). The store is at fault, so no shipping is charged and coupons come
//! back; sales prices the order again for what stays, and the difference is refunded to the
//! original payment. Always approved by a person; the refund cannot be undone.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_core::enums::CancelReason;
use acom_core::ids::{ClaimId, RefundId};
use acom_sales::service::for_fulfillment as sales;
use acom_store::{Cx, TxCx};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    ApproveStep, CancelItemInput, PlannedRefund, apply_effects, cancel_block_error, cancel_effects,
    cancel_view,
};
use crate::errors::{
    ALREADY_SHIPPED, CLAIM_IN_PROGRESS, NOT_REVOCABLE, ORDER_ITEM_NOT_FOUND, ORDER_NOT_FOUND,
    QUANTITY_OVER_CANCELLABLE, REASON_NOT_STORE_FAULT, WHOLE_QUANTITY_ONLY,
};
use crate::model::cancel::{
    CancelAsk, CancelBlock, cancel_liability, cancel_outcome, split_refund,
};
use crate::model::money_text;
use crate::repo::{self, ClaimEvent, NewClaim};
use crate::{broken, rules};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancelOrderLinesInput {
    /// The order, from `search_orders` or `get_order`.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20461")]
    pub order_id: String,
    /// The lines and units the store cancels; they must not have shipped.
    #[schemars(length(min = 1, max = 50), example = json!([{"order_item_id": "oln_1", "quantity": 1}]))]
    pub items: Vec<CancelItemInput>,
    /// Why the store cancels: out_of_stock, free_gift_unavailable, seller_cancelled,
    /// store_cancelled or shipping_delay (store-fault reasons only).
    #[schemars(example = &"out_of_stock")]
    pub reason: CancelReason,
    /// A note for the record, for example the supplier's answer.
    #[schemars(length(max = 500), example = &"Supplier cannot restock the navy M this season.")]
    pub note: Option<String>,
}

/// The cancel, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CancelLinesPlan {
    pub claim_id: String,
    pub order_id: String,
    pub reason: String,
    pub note: Option<String>,
    pub items: Vec<(String, i32)>,
    /// What the cancelled units cost the shopper.
    pub amount: Money,
    pub effects: ApproveStep,
    /// The order's money as stored before, and as sales prices it for what stays.
    pub price_before: sales::Priced,
    pub price_after: sales::Priced,
}

pub struct CancelOrderLines;

impl Operation for CancelOrderLines {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.cancel_order_lines",
        tool: "cancel_order_lines",
        group: Group::Transact,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/fulfillment/orders/{order_id}/cancellations",
        summary: "The store cancels order lines before they ship (out of stock, free gift gone, seller or store decision): stock and coupons come back, the order is priced again for what stays, and the difference is refunded. A person approves; the refund cannot be undone.",
        consumes: &[Slot::one("order_id")],
        produces: &[Slot::one("claim_id")],
        checks: &[
            "cancel.not_cancellable_states",
            "cancel.restore",
            "refund.to",
            "change_set.irreversible",
        ],
        errors: &[
            ORDER_NOT_FOUND,
            ALREADY_SHIPPED,
            ORDER_ITEM_NOT_FOUND,
            REASON_NOT_STORE_FAULT,
            WHOLE_QUANTITY_ONLY,
            QUANTITY_OVER_CANCELLABLE,
            CLAIM_IN_PROGRESS,
        ],
        audience: Audience::Operator,
        action_type: ActionType::CANCEL,
        person_scope: "",
    };
    type Input = CancelOrderLinesInput;
    type Output = Value;
}

fn block(b: CancelBlock) -> ToolError {
    match b {
        CancelBlock::AlreadyShipped(id) => ToolError::new(
            ALREADY_SHIPPED,
            "This item is already with the carrier; it can no longer be cancelled.",
        )
        .detail(json!({"order_item_id": id, "rule": "cancel.not_cancellable_states"}))
        .next("get_delivery"),
        CancelBlock::QuantityOverCancellable {
            order_item_id,
            cancellable,
        } => ToolError::new(
            QUANTITY_OVER_CANCELLABLE,
            format!("Only {cancellable} of this item can be cancelled."),
        )
        .detail(json!({"order_item_id": order_item_id, "limit": cancellable}))
        .next("cancel_order_lines"),
        other => cancel_block_error(other),
    }
}

fn items_text(n: i32) -> String {
    if n == 1 {
        "1 item".to_owned()
    } else {
        format!("{n} items")
    }
}

impl WriteOp<Cx> for CancelOrderLines {
    type Plan = CancelLinesPlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CancelOrderLinesInput,
    ) -> Result<Planned<CancelLinesPlan>, ToolError> {
        let rules = rules(&cx.policy)?;
        let reason = input.reason.as_str();
        if cancel_liability(&rules, reason).as_deref() != Some("store_fault") {
            return Err(ToolError::new(
                REASON_NOT_STORE_FAULT,
                format!(
                    "{} is the shopper's reason; the store cancels only for its own reasons.",
                    reason.replace('_', " ")
                ),
            )
            .detail(json!({"field": "reason", "rule": "cancel.reasons",
                           "allowed": rules.list("cancel.reasons.store_fault")}))
            .next("cancel_order_lines"));
        }
        let asked: Vec<(String, i32)> = input
            .items
            .iter()
            .map(|i| {
                (
                    i.order_item_id.clone(),
                    i32::try_from(i.quantity).unwrap_or(i32::MAX),
                )
            })
            .collect();
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let order = sales::order(&mut tx, &input.order_id)
            .await?
            .ok_or_else(|| {
                ToolError::new(
                    ORDER_NOT_FOUND,
                    format!("There is no order {}.", input.order_id),
                )
                .detail(json!({"entity": "order", "id": input.order_id}))
                .next("search_orders")
            })?;
        let view = cancel_view(&mut tx, &order, None).await?;
        let outcome = cancel_outcome(
            &rules,
            &view,
            CancelAsk {
                items: Some(&asked),
                reason: Some(reason),
                liability: Some("store_fault"),
                operator: true,
            },
            now,
        )
        .map_err(block)?;

        // Sales prices the order again for the units that stay; the refund is the difference.
        let units = |id: &str| {
            outcome
                .chosen
                .iter()
                .find(|(l, _)| l == id)
                .map_or(0, |(_, q)| *q)
        };
        let staying_before: Vec<(String, i32)> = view
            .lines
            .iter()
            .map(|l| {
                let left = if l.order_item_status == "cancelled" {
                    0
                } else {
                    (l.ordered - l.already_cancelled).max(0)
                };
                (l.order_item_id.clone(), left)
            })
            .collect();
        let staying_after: Vec<(String, i32)> = staying_before
            .iter()
            .map(|(id, left)| (id.clone(), (left - units(id)).max(0)))
            .collect();
        let before = sales::reprice(&mut tx, &order.id, &staying_before, order.shipping).await?;
        let after = sales::reprice(&mut tx, &order.id, &staying_after, order.shipping).await?;
        let stored = sales::priced(&mut tx, &order.id)
            .await?
            .ok_or_else(|| broken(format!("order {} is missing", order.id)))?;
        let paid = view.payments.iter().any(|p| p.completed);
        let refund_total = if paid {
            Money::new(
                (before.total_payment_due.value - after.total_payment_due.value).max(Decimal::ZERO),
                order.total_payment_due.currency,
            )
            .rounded()
        } else {
            Money::zero(order.total_payment_due.currency)
        };
        let fx = cancel_effects(&mut tx, &order.id, &view, &outcome.chosen).await?;
        let lines = sales::order_items(&mut tx, &order.id).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let parts = split_refund(&view.payments, view.already_refunded, refund_total);
        let part = parts
            .iter()
            .find(|p| !p.amount.value.is_zero())
            .or(parts.first());
        let refund = (!refund_total.value.is_zero()).then(|| PlannedRefund {
            refund_id: RefundId::generate().to_string(),
            payment_id: part.map(|p| p.payment_id.clone()),
            amount: refund_total,
            refund_method: rules.refund_to.clone(),
            refund_type: rules.defect_refund_type.clone(),
            to: part.map(|p| p.to.clone()).unwrap_or_default(),
        });
        let e = &outcome.estimate;
        let claim_id = ClaimId::generate().to_string();
        let mut changes = vec![Change {
            op: ChangeOp::Create,
            entity: "claim".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"claim_type": "cancel", "status": "approved", "order_id": order.id,
                       "reason": reason, "liability": "store_fault",
                       "items": outcome.chosen.iter()
                           .map(|(l, q)| json!({"order_item_id": l, "quantity": q}))
                           .collect::<Vec<_>>()}),
            note: None,
        }];
        changes.extend(fx.parcel_changes.iter().cloned());
        changes.extend(fx.stock_changes.iter().cloned());
        changes.push(Change {
            op: ChangeOp::Update,
            entity: "order".into(),
            id: order.id.clone(),
            from: json!({"subtotal": stored.subtotal, "discount": stored.discount,
                         "shipping": stored.shipping, "total_payment_due": stored.total_payment_due}),
            to: json!({"subtotal": after.subtotal, "discount": after.discount,
                       "shipping": after.shipping, "total_payment_due": after.total_payment_due}),
            note: Some("Priced again by sales for what stays".into()),
        });
        if let Some(r) = &refund {
            changes.push(Change {
                op: ChangeOp::Create,
                entity: "refund".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"amount": r.amount, "refund_method": r.refund_method, "to": r.to}),
                note: None,
            });
        }

        let states: Vec<String> = outcome
            .chosen
            .iter()
            .filter_map(|(id, _)| lines.iter().find(|l| &l.id == id))
            .map(|l| {
                format!(
                    "{} is {}",
                    l.name,
                    l.fulfillment_stage
                        .as_deref()
                        .unwrap_or(&l.order_item_status)
                        .replace('_', " ")
                )
            })
            .collect();
        let coupons = match e.coupons_restored.len() {
            0 => "no coupon on these items".to_owned(),
            1 => "1 coupon restored".to_owned(),
            n => format!("{n} coupons restored"),
        };
        let mut checks = vec![
            Check {
                rule: "cancel.not_cancellable_states".into(),
                passed: true,
                detail: Some(format!("{}, not shipped", states.join("; "))),
                fix_tool: None,
            },
            Check {
                rule: "cancel.restore".into(),
                passed: true,
                detail: Some(format!(
                    "Store fault: {coupons}, no shipping charge; {} pending points cancelled",
                    money_text(e.points_pending_cancelled)
                )),
                fix_tool: None,
            },
        ];
        if let Some(r) = &refund {
            checks.push(Check {
                rule: "refund.to".into(),
                passed: !e.needs_refund_destination,
                detail: Some(if e.needs_refund_destination {
                    "The original payment is too old to reverse; a person must pick where the money goes".to_owned()
                } else {
                    format!("{} back to {}", money_text(r.amount), r.to)
                }),
                fix_tool: None,
            });
            checks.push(Check {
                rule: "change_set.irreversible".into(),
                passed: true,
                detail: Some("The refund can't be undone once it is sent".into()),
                fix_tool: None,
            });
        }
        let mut basis = vec![Basis {
            entity: "sales.sales_order".into(),
            id: order.id.clone(),
            version: i64::from(order.version),
        }];
        basis.extend(
            lines
                .iter()
                .filter(|l| outcome.chosen.iter().any(|(id, _)| id == &l.id))
                .map(|l| Basis {
                    entity: "sales.order_item".into(),
                    id: l.id.clone(),
                    version: i64::from(l.version),
                }),
        );
        basis.extend(fx.basis.iter().cloned());
        let count: i32 = outcome.chosen.iter().map(|(_, q)| *q).sum();
        let approval_reason = match &refund {
            Some(r) => format!("Store-initiated cancel refunds {}", money_text(r.amount)),
            None => format!("Store-initiated cancel of {}", items_text(count)),
        };
        let effects = ApproveStep {
            claim_id: claim_id.clone(),
            claim_type: "cancel".into(),
            order_id: order.id.clone(),
            from_status: "approved".into(),
            to_status: "approved".into(),
            liability_change: None,
            lines_cancelled: fx.lines_cancelled,
            restock: fx.restock,
            deliveries_cancelled: fx.deliveries_cancelled,
            coupons_restored: e
                .coupons_restored
                .iter()
                .map(|c| c.member_coupon_id.clone())
                .collect(),
            order_cancelled_from: (e.scope == "full").then(|| order.order_status.clone()),
            refund,
        };
        Ok(Planned {
            plan: CancelLinesPlan {
                claim_id,
                order_id: order.id.clone(),
                reason: reason.into(),
                note: input
                    .note
                    .map(|n| n.trim().to_owned())
                    .filter(|n| !n.is_empty()),
                items: outcome.chosen.clone(),
                amount: e.cancelled_amount,
                effects,
                price_before: stored,
                price_after: after,
            },
            changes,
            checks,
            basis,
            approval: ApprovalNeed::Needed {
                reason: approval_reason,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &CancelLinesPlan) -> Result<Committed, ToolError> {
        let done = Committed {
            id: plan.claim_id.clone(),
            produced: Map::from_iter([
                ("claim_id".to_owned(), json!(plan.claim_id)),
                (
                    "refund_id".to_owned(),
                    json!(plan.effects.refund.as_ref().map(|r| &r.refund_id)),
                ),
            ]),
        };
        if repo::claim(tx, &plan.claim_id).await?.is_some() {
            return Ok(done);
        }
        let order = sales::order(tx, &plan.order_id)
            .await?
            .ok_or_else(|| broken(format!("order {} is missing", plan.order_id)))?;
        let customer = order
            .customer_id
            .clone()
            .ok_or_else(|| broken(format!("order {} has no customer", order.id)))?;
        let refund_estimate = plan
            .effects
            .refund
            .as_ref()
            .map(|r| r.amount)
            .unwrap_or(Money::zero(plan.amount.currency));
        repo::insert_claim(
            tx,
            &NewClaim {
                id: &plan.claim_id,
                order_id: &plan.order_id,
                customer_id: &customer,
                claim_type: "cancel",
                claim_status: "approved",
                claim_path: "store_review",
                claim_reason: None,
                cancel_reason: Some(&plan.reason),
                reason_text: plan.note.as_deref(),
                liability: "store_fault",
                return_fees: None,
                send_back_method: None,
                pickup_window: None,
                evidence: json!([]),
                amount: plan.amount,
                refund_estimate: Some(refund_estimate),
                charge: None,
                items: plan
                    .items
                    .iter()
                    .map(|(l, q)| (l.clone(), *q, None))
                    .collect(),
            },
        )
        .await?;
        apply_effects(tx, &plan.effects, Self::META.tool).await?;
        sales::apply_price(tx, &plan.order_id, &plan.price_after, Self::META.tool).await?;
        repo::append_event(
            tx,
            &plan.claim_id,
            &ClaimEvent {
                action_type: "CancelAction",
                action_status: "completed",
                instrument: Self::META.tool,
                from_state: None,
                to_state: Some("approved"),
                result: Some(
                    json!({"refund_id": plan.effects.refund.as_ref().map(|r| &r.refund_id),
                                    "refund_total": refund_estimate, "note": plan.note,
                                    "repriced": plan.price_after}),
                ),
            },
        )
        .await?;
        Ok(done)
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &CancelLinesPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = repo::claim(&mut tx, &plan.claim_id).await?;
        let refund = repo::refund_of_claim(&mut tx, &plan.claim_id).await?;
        let price = sales::priced(&mut tx, &plan.order_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        match c {
            Some(c) if c.claim_status == "approved" || !c.is_open() => {}
            Some(c) => mismatches.push(format!("claim {} is {}", c.id, c.claim_status)),
            None => mismatches.push(format!("claim {} is missing", plan.claim_id)),
        }
        if plan.effects.refund.is_some() && refund.is_none() {
            mismatches.push(format!("claim {} has no refund", plan.claim_id));
        }
        if price.map(|p| p.total_payment_due) != Some(plan.price_after.total_payment_due) {
            mismatches.push(format!(
                "order {} is not priced at {}",
                plan.order_id,
                money_text(plan.price_after.total_payment_due)
            ));
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        _: &mut TxCx,
        plan: &CancelLinesPlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            NOT_REVOCABLE,
            format!(
                "Cancel {} has been refunded and cannot be taken back; the shopper can order again.",
                plan.claim_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{input, offline};

    #[tokio::test]
    async fn the_store_cancels_only_for_its_own_reasons() {
        let e = CancelOrderLines
            .preview(
                &offline("adm_sj"),
                input(json!({"order_id": "ord_20461", "reason": "changed_mind",
                             "items": [{"order_item_id": "oln_1", "quantity": 1}]})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "reason_not_store_fault");
        assert_eq!(e.next_tool.as_deref(), Some("cancel_order_lines"));
    }
}
