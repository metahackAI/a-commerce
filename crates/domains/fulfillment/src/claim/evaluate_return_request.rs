//! `evaluate_return_request`: the policy's suggestion for one return request.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_sales::service::for_fulfillment as sales;
use acom_store::Cx;
use chrono::Duration;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{claim_lines, visible_claim};
use crate::errors::*;
use crate::model::delivery::days_since;
use crate::model::returns::{EvalFacts, evaluate, photo_count, refund_amount};
use crate::repo::{self};
use crate::{delivered_at, rules};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvaluateReturnRequestInput {
    /// The return request to evaluate, from `search_return_requests`.
    #[schemars(regex(pattern = r"^clm_[a-z0-9_]+$"), example = &"clm_20402")]
    pub claim_id: String,
}

pub struct EvaluateReturnRequest;

impl Operation for EvaluateReturnRequest {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.evaluate_return_request",
        tool: "evaluate_return_request",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/return-requests/{claim_id}/evaluation",
        summary: "Apply the return policy to one return request and suggest refund, ask_for_photos or decline, with the policy rule behind it, the refund amount, whether a person must approve, and flags such as frequent returns. Changes nothing.",
        consumes: &[Slot::one("claim_id")],
        produces: &[Slot::one("claim_id")],
        checks: &[],
        errors: &[CLAIM_NOT_FOUND, ErrorCode::INVALID_STATE, NOT_A_RETURN],
        audience: Audience::Operator,
        action_type: ActionType::ASSESS,
        person_scope: "",
    };
    type Input = EvaluateReturnRequestInput;
    type Output = Value;
}

impl ReadOp<Cx> for EvaluateReturnRequest {
    async fn read(&self, cx: &Cx, input: EvaluateReturnRequestInput) -> Result<Value, ToolError> {
        let rules = rules(&cx.policy)?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let claim = visible_claim(&mut tx, &input.claim_id, "search_return_requests").await?;
        if claim.claim_type != "return" {
            return Err(ToolError::new(
                NOT_A_RETURN,
                format!(
                    "Claim {} is a {}, not a return.",
                    claim.id, claim.claim_type
                ),
            )
            .detail(json!({"entity": "claim", "claim_type": claim.claim_type}))
            .next("get_claim"));
        }
        if !matches!(
            claim.claim_status.as_str(),
            "requested" | "photos_requested"
        ) {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "Return {} is already {}.",
                    claim.id,
                    claim.claim_status.replace('_', " ")
                ),
            )
            .detail(json!({"entity": "claim", "from": claim.claim_status}))
            .next("get_claim"));
        }
        let lines = claim_lines(&mut tx, std::slice::from_ref(&claim.id)).await?;
        let order = sales::order(&mut tx, &claim.order_id).await?;
        let deliveries =
            repo::deliveries_of_orders(&mut tx, std::slice::from_ref(&claim.order_id)).await?;
        let recent = repo::recent_returns(
            &mut tx,
            &claim.customer_id,
            now - Duration::days(rules.flag_days),
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;

        let lines = lines.get(&claim.id).cloned().unwrap_or_default();
        let amount = refund_amount(&lines, rules.coupon_deducted).unwrap_or(claim.amount()?);
        let facts = EvalFacts {
            reason: claim.claim_reason.clone().unwrap_or_default(),
            liability: claim.liability.clone(),
            photos: photo_count(&claim.evidence),
            days_since_delivery: delivered_at(&deliveries, &claim.order_id)
                .map(|d| days_since(d, now, cx.time.zone)),
            refund_amount: amount,
            recent_returns: recent,
        };
        let s = evaluate(&rules, &cx.policy, &facts).ok_or_else(|| {
            ToolError::new(
                ErrorCode::INVALID_STATE,
                "No return rule applies yet: the order has not been delivered.",
            )
            .detail(json!({"entity": "claim", "from": claim.claim_status}))
            .next("get_claim")
        })?;
        let item = json!({
            "claim_id": claim.id,
            "order_number": order.map(|o| o.order_number),
            "suggested": s.suggested,
            "rule": s.rule,
            "rule_text": s.rule_text,
            "liability": s.liability,
            "refund_amount": s.refund_amount,
            "return_fees": s.return_fees,
            "needs_person": s.needs_person,
            "flags": s.flags,
        });
        Ok(json!({"items": [item], "total": 1}))
    }
}
