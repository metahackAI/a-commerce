//! `get_refund`: one refund and its status as the payment provider confirms it.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_sales::service::for_fulfillment as sales;
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::errors::*;
use crate::repo::{self};
use crate::{payment_label, shopper};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetRefundInput {
    /// The refund.
    #[schemars(regex(pattern = r"^rfd_[a-z0-9_]+$"), example = &"rfd_20402")]
    pub refund_id: String,
    /// Top-level fields to return, for example only the status.
    #[schemars(example = ["refund_status", "events"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetRefund;

impl Operation for GetRefund {
    const META: OpMeta = OpMeta {
        address: "fulfillment.refund.get",
        tool: "get_refund",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/refunds/{refund_id}",
        summary: "One refund: amount, where the money goes, and its status as the payment provider confirms it (requested, completed, failed).",
        consumes: &[Slot::one("refund_id")],
        produces: &[Slot::one("claim_id")],
        checks: &[],
        errors: &[REFUND_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetRefundInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetRefund {
    async fn read(&self, cx: &Cx, input: GetRefundInput) -> Result<Value, ToolError> {
        let not_found = || {
            ToolError::new(
                REFUND_NOT_FOUND,
                format!("There is no refund {}.", input.refund_id),
            )
            .detail(json!({"entity": "refund", "id": input.refund_id}))
            .next("search_claims")
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = repo::refund(&mut tx, &input.refund_id)
            .await?
            .ok_or_else(not_found)?;
        if let Some(me) = shopper(&cx.principal) {
            let owner = repo::claim(&mut tx, &r.claim_id)
                .await?
                .map(|c| c.customer_id);
            if owner.as_deref() != Some(me.as_str()) {
                return Err(not_found());
            }
        }
        let payments = sales::payments(&mut tx, &r.order_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let to = match r.refund_method.as_str() {
            "original_payment" => r
                .payment_id
                .as_ref()
                .and_then(|id| payments.iter().find(|p| &p.id == id))
                .map(payment_label)
                .unwrap_or_else(|| "the original payment".to_owned()),
            other => other.replace('_', " "),
        };
        let mut events =
            vec![json!({"refund_status": "requested", "at": cx.time.format(r.start_time)})];
        if let Some(end) = r.end_time
            && r.refund_status != "requested"
        {
            events.push(json!({"refund_status": r.refund_status, "at": cx.time.format(end)}));
        }
        let out = json!({
            "id": r.id,
            "claim_id": r.claim_id,
            "order_id": r.order_id,
            "amount": r.amount()?,
            "refund_method": r.refund_method,
            "refund_type": r.refund_type,
            "to": to,
            "refund_status": r.refund_status,
            "events": events,
        });
        Ok(crate::select(out, &input.fields.unwrap_or_default()))
    }
}
