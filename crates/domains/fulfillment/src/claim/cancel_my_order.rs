//! Person-only `POST /v1/fulfillment/orders/{order_id}/cancel`: the shopper cancels before
//! shipment. Not a tool; it needs `human:cancel`.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, Slot,
    ToolError,
};
use acom_core::Money;
use acom_core::enums::CancelReason;
use acom_core::ids::{ClaimId, RefundId};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    ApproveStep, CancelItemInput, PlannedRefund, RefundDestination, RefundToInput, apply_effects,
    asked_items, cancel_block_error, cancel_effects, cancel_view, changed, not_yours,
};
use crate::errors::*;
use crate::model::cancel::{CancelAsk, cancel_outcome};
use crate::repo::{self, ClaimEvent, NewClaim};
use crate::{order_for, rules, shopper};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancelMyOrderInput {
    /// The shopper's own order.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20455")]
    pub order_id: String,
    /// Lines and units; the whole order when left out.
    #[schemars(example = json!([{"order_item_id": "oln_1", "quantity": 1}]))]
    pub items: Option<Vec<CancelItemInput>>,
    /// A reason from get_cancel_options.reasons.
    #[schemars(example = &"changed_mind")]
    pub reason: CancelReason,
    /// The shopper's own words.
    #[schemars(length(max = 500), example = &"Ordered the wrong color")]
    pub reason_text: Option<String>,
    /// The refund the shopper confirmed (`estimate.refund_total`).
    #[schemars(example = json!({"value": 65.20, "currency": "USD"}))]
    pub expected_refund: Money,
    /// The extra charge the shopper confirmed (`estimate.charge_due`, default 0.00).
    #[schemars(example = json!({"value": 0.00, "currency": "USD"}))]
    pub expected_charge: Option<Money>,
    /// Only when `needs_refund_destination` was true.
    pub refund_to: Option<RefundToInput>,
}

/// `POST /v1/fulfillment/orders/{order_id}/cancel`: the shopper cancels before shipment.
pub struct CancelMyOrder;

impl Operation for CancelMyOrder {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.cancel_my_order",
        tool: "cancel_my_order",
        group: Group::Transact,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/fulfillment/orders/{order_id}/cancel",
        summary: "The shopper cancels all or part of their order before it ships, as prepared with get_cancel_options.",
        consumes: &[Slot::one("order_id")],
        produces: &[Slot::one("claim_id")],
        checks: &[],
        errors: &[
            ALREADY_SHIPPED,
            NOT_CANCELLABLE,
            QUANTITY_OVER_CANCELLABLE,
            WHOLE_QUANTITY_ONLY,
            REASON_NOT_SELECTABLE,
            ErrorCode::ESTIMATE_CHANGED,
            REFUND_DESTINATION_REQUIRED,
            ALREADY_CANCELLED,
            CLAIM_IN_PROGRESS,
            NOT_YOUR_ORDER,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::CANCEL,
        person_scope: "human:cancel",
    };
    type Input = CancelMyOrderInput;
    type Output = Value;
}

impl PersonOp<Cx> for CancelMyOrder {
    async fn run(&self, tx: &mut TxCx, input: CancelMyOrderInput) -> Result<Value, ToolError> {
        let rules = rules(&tx.policy)?;
        let me = shopper(&tx.principal).ok_or_else(not_yours)?;
        let now = tx.clock.now();
        let order = order_for(tx, &input.order_id, Some(&me), "get_my_orders").await?;
        let view = cancel_view(tx, &order, None).await?;
        let asked = asked_items(&input.items);
        let outcome = cancel_outcome(
            &rules,
            &view,
            CancelAsk {
                items: asked.as_deref(),
                reason: Some(input.reason.as_str()),
                ..CancelAsk::default()
            },
            now,
        )
        .map_err(cancel_block_error)?;
        let e = &outcome.estimate;
        let seen_charge = input
            .expected_charge
            .map_or(rust_decimal::Decimal::ZERO, |m| m.value);
        if e.refund_total.value != input.expected_refund.value || e.charge_due.value != seen_charge
        {
            return Err(changed("estimate", json!(e)));
        }
        if e.needs_refund_destination && input.refund_to.is_none() {
            return Err(ToolError::new(
                REFUND_DESTINATION_REQUIRED,
                "The original payment can no longer be refunded; choose where the money goes.",
            )
            .detail(json!({"field": "refund_to", "allowed": rules.list("refund.refund_destination_types")})));
        }

        let claim_id = ClaimId::generate().to_string();
        let status = outcome.status.as_str();
        repo::insert_claim(
            tx,
            &NewClaim {
                id: &claim_id,
                order_id: &order.id,
                customer_id: &me,
                claim_type: "cancel",
                claim_status: status,
                claim_path: if status == "requested" {
                    "store_review"
                } else {
                    "self_service"
                },
                claim_reason: None,
                cancel_reason: Some(&outcome.reason),
                reason_text: input
                    .reason_text
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty()),
                liability: &outcome.liability,
                return_fees: None,
                send_back_method: None,
                pickup_window: None,
                evidence: json!([]),
                amount: e.cancelled_amount,
                refund_estimate: Some(e.refund_total),
                items: outcome
                    .chosen
                    .iter()
                    .map(|(id, q)| (id.clone(), *q, None))
                    .collect(),
            },
        )
        .await?;
        let goes_through = matches!(status, "completed" | "approved");
        let mut refund_id = None;
        if goes_through {
            let fx = cancel_effects(tx, &order.id, &view, &outcome.chosen).await?;
            let part = e
                .refund_breakdown
                .iter()
                .find(|p| !p.amount.value.is_zero())
                .or(e.refund_breakdown.first());
            let method = match (&input.refund_to, e.needs_refund_destination) {
                (Some(to), true) => match to.destination {
                    RefundDestination::BankAccount => "bank_account",
                    RefundDestination::StoreCredit => "store_credit",
                },
                _ => rules.refund_to.as_str(),
            };
            let refund =
                (status == "approved" && !e.refund_total.value.is_zero()).then(|| PlannedRefund {
                    refund_id: RefundId::generate().to_string(),
                    payment_id: (method == "original_payment")
                        .then(|| part.map(|p| p.payment_id.clone()))
                        .flatten(),
                    amount: e.refund_total,
                    refund_method: method.to_owned(),
                    refund_type: rules.defect_refund_type.clone(),
                    to: part.map(|p| p.to.clone()).unwrap_or_default(),
                });
            refund_id = refund.as_ref().map(|r| r.refund_id.clone());
            let step = ApproveStep {
                claim_id: claim_id.clone(),
                claim_type: "cancel".into(),
                order_id: order.id.clone(),
                from_status: status.into(),
                to_status: status.into(),
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
            apply_effects(tx, &step, "POST /v1/fulfillment/orders/{order_id}/cancel").await?;
        }
        repo::append_event(
            tx,
            &claim_id,
            &ClaimEvent {
                action_type: "CancelAction",
                action_status: if goes_through { "completed" } else { "potential" },
                instrument: "POST /v1/fulfillment/orders/{order_id}/cancel",
                from_state: None,
                to_state: Some(status),
                result: Some(json!({"refund_id": refund_id, "refund_total": e.refund_total,
                                    "refund_to": input.refund_to.as_ref().map(RefundToInput::wire)})),
            },
        )
        .await?;

        let mut out = json!({
            "ok": true,
            "id": claim_id,
            "claim_type": "cancel",
            "status": status,
            "order_id": order.id,
            "items": outcome.chosen.iter()
                .map(|(id, q)| json!({"order_item_id": id, "quantity": q}))
                .collect::<Vec<_>>(),
            "liability": outcome.liability,
            "refund": {
                "total": e.refund_total,
                "breakdown": e.refund_breakdown.iter()
                    .filter(|p| !p.amount.value.is_zero())
                    .map(|p| json!({"to": p.to, "amount": p.amount}))
                    .collect::<Vec<_>>(),
                "money_back_business_days": e.money_back_business_days,
            },
            "extra_charge": e.extra_charge.iter()
                .map(|x| json!({"kind": x.kind, "amount": x.amount,
                                "collected": if e.charge_due.value.is_zero() { "deducted_from_refund" } else { "charge_due" }}))
                .collect::<Vec<_>>(),
            "coupons_restored": e.coupons_restored,
            "points_pending_cancelled": e.points_pending_cancelled,
            "confirmed_by": e.confirmed_by,
            "undo": null,
        });
        if let Some(id) = refund_id {
            out["refund_id"] = json!(id);
        }
        if status == "charge_due" {
            out["pay_charge"] = json!({
                "amount": e.charge_due,
                "endpoint": format!("POST /v1/fulfillment/claims/{claim_id}/pay-charge"),
            });
        }
        Ok(out)
    }
}
