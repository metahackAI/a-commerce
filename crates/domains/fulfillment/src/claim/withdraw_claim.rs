//! Person-only `POST /v1/fulfillment/claims/{claim_id}/withdraw`: the shopper takes back a
//! return or exchange request. Not a tool; it needs `human:claim`.

use acom_catalog::service::for_fulfillment as catalog;
use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, PersonOp, Slot,
    ToolError,
};
use acom_core::Money;
use acom_core::ids::RefundId;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{not_yours, refund_paid_charge};
use crate::errors::{CLAIM_NOT_FOUND, NOT_REVOCABLE, NOT_YOUR_ORDER};
use crate::repo::{self, ClaimEvent};
use crate::{rules, shopper};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WithdrawClaimInput {
    /// The shopper's own return or exchange request.
    #[schemars(regex(pattern = r"^clm_[a-z0-9_]+$"), example = &"clm_3301")]
    pub claim_id: String,
    /// Why, in the shopper's words.
    #[schemars(length(max = 500), example = &"Found my size after all.")]
    pub reason_text: Option<String>,
}

/// `POST /v1/fulfillment/claims/{claim_id}/withdraw`: the shopper takes back a return or
/// exchange request before its pickup label exists.
pub struct WithdrawClaim;

impl Operation for WithdrawClaim {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.withdraw",
        tool: "withdraw_claim",
        group: Group::Transact,
        kind: Kind::Person,
        method: Method::Post,
        path: "/v1/fulfillment/claims/{claim_id}/withdraw",
        summary: "The shopper withdraws their own return or exchange request, whole, before the pickup label is made. Cancels cannot be withdrawn.",
        consumes: &[Slot::one("claim_id")],
        produces: &[Slot::one("claim_id")],
        checks: &[],
        errors: &[
            ErrorCode::INVALID_STATE,
            NOT_REVOCABLE,
            NOT_YOUR_ORDER,
            CLAIM_NOT_FOUND,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::UPDATE,
        person_scope: "human:claim",
    };
    type Input = WithdrawClaimInput;
    type Output = Value;
}

impl PersonOp<Cx> for WithdrawClaim {
    async fn run(&self, tx: &mut TxCx, input: WithdrawClaimInput) -> Result<Value, ToolError> {
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
        let zero = Money::zero(c.amount()?.currency);
        if c.claim_status == "withdrawn" {
            return Ok(json!({"ok": true, "id": c.id, "status": "withdrawn",
                             "charge_refunded": zero, "undo": null}));
        }
        let allowed = match c.claim_type.as_str() {
            "cancel" => {
                return Err(ToolError::new(
                    NOT_REVOCABLE,
                    "A cancel cannot be withdrawn; place a new order instead.",
                )
                .detail(json!({"rule": "cancel.revocable"})));
            }
            "exchange" => &rules.exchange_withdraw_states,
            _ => &rules.return_withdraw_states,
        };
        if !allowed.contains(&c.claim_status) {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "This request is {} and can no longer be withdrawn.",
                    c.claim_status.replace('_', " ")
                ),
            )
            .detail(json!({"entity": "claim", "from": c.claim_status, "allowed_to": []})));
        }
        if c.claim_type == "return" && repo::refund_of_claim(tx, &c.id).await?.is_some() {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                "The refund for this return has already been sent.",
            )
            .detail(json!({"entity": "claim", "from": c.claim_status})));
        }
        repo::move_claim(tx, &c.id, &c.claim_status, "withdrawn").await?;
        // Release a replacement the exchange was holding (`exchange.withdraw.on_withdraw`).
        if c.claim_type == "exchange" {
            for i in repo::claim_items(tx, std::slice::from_ref(&c.id)).await? {
                if let Some(v) = &i.replacement_product_id {
                    catalog::move_stock(tx, v, i.order_quantity).await?;
                }
            }
        }
        // A charge the shopper paid comes back (`exchange.withdraw.on_withdraw:
        // refund_paid_charge`).
        let instrument = "POST /v1/fulfillment/claims/{claim_id}/withdraw";
        let refunded = refund_paid_charge(tx, &c, &RefundId::generate().to_string(), instrument)
            .await?
            .unwrap_or(zero);
        repo::append_event(
            tx,
            &c.id,
            &ClaimEvent {
                action_type: "UpdateAction",
                action_status: "completed",
                instrument,
                from_state: Some(&c.claim_status),
                to_state: Some("withdrawn"),
                result: Some(json!({"reason_text": input.reason_text,
                                    "charge_refunded": refunded})),
            },
        )
        .await?;
        Ok(json!({"ok": true, "id": c.id, "status": "withdrawn",
                  "charge_refunded": refunded, "undo": null}))
    }
}
