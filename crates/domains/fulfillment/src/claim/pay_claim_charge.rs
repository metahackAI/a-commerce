//! Person-only `POST /v1/fulfillment/claims/{claim_id}/pay-charge`: the shopper pays the extra
//! cost a claim waits for (`charge_due`): cancel shipping the refund does not cover, an
//! exchange's shipping, return shipping over the refund. Then the claim goes ahead. Not a tool;
//! it needs `human:claim`.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, Slot,
    ToolError,
};
use acom_core::Money;
use acom_core::ids::RefundId;
use acom_sales::service::for_fulfillment as sales;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    ApproveStep, PlannedRefund, apply_effects, cancel_effects, cancel_view, changed, not_yours,
};
use crate::errors::{CLAIM_NOT_FOUND, NOT_YOUR_ORDER, PAYMENT_DECLINED};
use crate::model::cancel::{CancelAsk, cancel_outcome};
use crate::repo::{self, ClaimEvent, ClaimRow};
use crate::{broken, rules, shopper};

/// What the payment provider is in this system: a stub. Said in the answer and the record.
const PROVIDER_NOTE: &str = "No payment provider is connected in this demo: the charge is recorded as paid with the saved payment method; nothing is collected.";

/// The stub provider declines saved payment methods whose reference starts with this, so the
/// refusal path can be tried.
const DECLINED_PREFIX: &str = "declined";

const INSTRUMENT: &str = "POST /v1/fulfillment/claims/{claim_id}/pay-charge";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PayClaimChargeInput {
    /// The shopper's claim waiting for the charge (status charge_due).
    #[schemars(regex(pattern = r"^clm_[a-z0-9_]+$"), example = &"clm_3302")]
    pub claim_id: String,
    /// The amount the shopper confirmed paying.
    #[schemars(example = json!({"value": 12.00, "currency": "USD"}))]
    pub amount: Money,
    /// The saved payment method to charge (a reference, never card details).
    #[schemars(length(min = 1, max = 80), example = &"pm_card_4821")]
    pub payment_method_ref: String,
}

/// `POST /v1/fulfillment/claims/{claim_id}/pay-charge`.
pub struct PayClaimCharge;

impl Operation for PayClaimCharge {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.pay_charge",
        tool: "pay_claim_charge",
        group: Group::Transact,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/fulfillment/claims/{claim_id}/pay-charge",
        summary: "The shopper pays the extra cost a cancel, return or exchange waits for; then the claim goes ahead (a cancel is approved, a return or exchange requested).",
        consumes: &[Slot::one("claim_id")],
        produces: &[Slot::one("claim_id")],
        checks: &[],
        errors: &[
            CLAIM_NOT_FOUND,
            ErrorCode::ESTIMATE_CHANGED,
            ErrorCode::INVALID_STATE,
            PAYMENT_DECLINED,
            NOT_YOUR_ORDER,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::PAY,
        person_scope: "human:claim",
    };
    type Input = PayClaimChargeInput;
    type Output = Value;
}

/// The charge a claim waits for: recorded when the claim was sent, else (for a cancel sent
/// before charges were recorded) computed again.
async fn charge_of(
    tx: &mut TxCx,
    rules: &crate::model::Rules,
    c: &ClaimRow,
) -> Result<Money, ToolError> {
    if let Some(m) = c.charge()? {
        return Ok(m);
    }
    if c.claim_type != "cancel" {
        return Err(broken(format!("claim {} has no recorded charge", c.id)));
    }
    let outcome = cancel_again(tx, rules, c).await?.0;
    Ok(outcome.estimate.charge_due)
}

/// The cancel of a claim's lines as it stands now, the claim itself left out.
async fn cancel_again(
    tx: &mut TxCx,
    rules: &crate::model::Rules,
    c: &ClaimRow,
) -> Result<
    (
        crate::model::cancel::CancelOutcome,
        crate::model::cancel::CancelOrder,
        sales::Order,
    ),
    ToolError,
> {
    let order = sales::order(tx, &c.order_id)
        .await?
        .ok_or_else(|| broken(format!("claim {} has no order", c.id)))?;
    let view = cancel_view(tx, &order, Some(&c.id)).await?;
    let asked: Vec<(String, i32)> = repo::claim_items(tx, std::slice::from_ref(&c.id))
        .await?
        .into_iter()
        .map(|i| (i.order_item_id, i.order_quantity))
        .collect();
    let now = tx.clock.now();
    let outcome = cancel_outcome(
        rules,
        &view,
        CancelAsk {
            items: Some(&asked),
            reason: c.cancel_reason.as_deref(),
            liability: Some(&c.liability),
            operator: true,
        },
        now,
    )
    .map_err(super::cancel_block_error)?;
    Ok((outcome, view, order))
}

impl PersonOp<Cx> for PayClaimCharge {
    async fn run(&self, tx: &mut TxCx, input: PayClaimChargeInput) -> Result<Value, ToolError> {
        let rules = rules(&tx.policy)?;
        let me = shopper(&tx.principal).ok_or_else(not_yours)?;
        let c = repo::claim_for_update(tx, &input.claim_id)
            .await?
            .ok_or_else(|| {
                ToolError::new(
                    CLAIM_NOT_FOUND,
                    format!("There is no request {}.", input.claim_id),
                )
                .detail(json!({"entity": "claim", "id": input.claim_id}))
            })?;
        if c.customer_id != me {
            return Err(not_yours());
        }
        if c.claim_status != "charge_due" {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "This request is {}; there is nothing to pay.",
                    c.claim_status.replace('_', " ")
                ),
            )
            .detail(json!({"entity": "claim", "from": c.claim_status, "allowed_to": []})));
        }
        let charge = charge_of(tx, &rules, &c).await?;
        if input.amount.value != charge.value || input.amount.currency != charge.currency {
            return Err(changed("charge", json!({"amount": charge})));
        }
        let reference = input.payment_method_ref.trim();
        if reference.is_empty() {
            return Err(ToolError::invalid_input("Choose a saved payment method.")
                .detail(json!({"field": "payment_method_ref", "reason": "missing"})));
        }
        if reference.starts_with(DECLINED_PREFIX) {
            return Err(ToolError::new(
                PAYMENT_DECLINED,
                "The payment method was declined; try another one.",
            )
            .detail(json!({"payment_provider": PROVIDER_NOTE})));
        }
        let now = tx.clock.now();
        repo::set_charge_paid(tx, &c.id, now).await?;

        let automatic = rules
            .lookup("exchange.pickup.instruction")
            .and_then(Value::as_str)
            == Some("automatic");
        let next = match c.claim_type.as_str() {
            "cancel" => "approved",
            _ => "requested",
        };
        repo::move_claim(tx, &c.id, "charge_due", next).await?;
        let mut refund_id = None;
        if c.claim_type == "cancel" {
            let (outcome, view, order) = cancel_again(tx, &rules, &c).await?;
            let fx = cancel_effects(tx, &c.order_id, &view, &outcome.chosen).await?;
            let e = &outcome.estimate;
            let part = e
                .refund_breakdown
                .iter()
                .find(|p| !p.amount.value.is_zero())
                .or(e.refund_breakdown.first());
            let refund = (!e.refund_total.value.is_zero()).then(|| PlannedRefund {
                refund_id: RefundId::generate().to_string(),
                payment_id: part.map(|p| p.payment_id.clone()),
                amount: e.refund_total,
                refund_method: rules.refund_to.clone(),
                refund_type: rules.defect_refund_type.clone(),
                to: part.map(|p| p.to.clone()).unwrap_or_default(),
            });
            refund_id = refund.as_ref().map(|r| r.refund_id.clone());
            let step = ApproveStep {
                claim_id: c.id.clone(),
                claim_type: c.claim_type.clone(),
                order_id: c.order_id.clone(),
                from_status: "charge_due".into(),
                to_status: next.into(),
                liability_change: None,
                lines_cancelled: fx.lines_cancelled,
                restock: fx.restock,
                deliveries_cancelled: fx.deliveries_cancelled,
                coupons_restored: e
                    .coupons_restored
                    .iter()
                    .map(|x| x.member_coupon_id.clone())
                    .collect(),
                order_cancelled_from: (e.scope == "full").then(|| order.order_status.clone()),
                refund,
            };
            apply_effects(tx, &step, INSTRUMENT).await?;
        }
        repo::append_event(
            tx,
            &c.id,
            &ClaimEvent {
                action_type: "PayAction",
                action_status: "completed",
                instrument: INSTRUMENT,
                from_state: Some("charge_due"),
                to_state: Some(next),
                result: Some(json!({"charged": charge, "payment_method_ref": reference,
                                    "refund_id": refund_id, "payment_provider": PROVIDER_NOTE})),
            },
        )
        .await?;
        let mut status = next;
        if c.claim_type == "exchange" && c.claim_path == "self_service" && automatic {
            repo::move_claim(tx, &c.id, "requested", "pickup_instructed").await?;
            repo::append_event(
                tx,
                &c.id,
                &ClaimEvent {
                    action_type: "AuthorizeAction",
                    action_status: "active",
                    instrument: INSTRUMENT,
                    from_state: Some("requested"),
                    to_state: Some("pickup_instructed"),
                    result: Some(json!({"rule": "exchange.pickup.instruction"})),
                },
            )
            .await?;
            status = "pickup_instructed";
        }
        Ok(json!({
            "ok": true,
            "id": c.id,
            "status": status,
            "charged": charge,
            "payment": PROVIDER_NOTE,
            "undo": null,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_is_an_amount_and_a_saved_method() {
        assert!(
            serde_json::from_value::<PayClaimChargeInput>(json!({
                "claim_id": "clm_3302", "amount": {"value": 12.00, "currency": "USD"},
                "payment_method_ref": "pm_card_4821"
            }))
            .is_ok()
        );
        assert!(
            serde_json::from_value::<PayClaimChargeInput>(json!({
                "claim_id": "clm_3302", "amount": {"value": 12.00, "currency": "USD"},
                "card_number": "4111111111111111"
            }))
            .is_err()
        );
    }
}
