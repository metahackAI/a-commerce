//! `get_cancel_options`: prepares a cancel of the shopper's own order. Changes nothing;
//! `POST /v1/fulfillment/orders/{order_id}/cancel` computes the same again.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_core::enums::CancelReason;
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CancelItemInput, asked_items, cancel_block_error, cancel_view};
use crate::errors::{
    ALREADY_CANCELLED, ALREADY_SHIPPED, CLAIM_IN_PROGRESS, NOT_YOUR_ORDER, ORDER_ITEM_NOT_FOUND,
    ORDER_NOT_FOUND, QUANTITY_OVER_CANCELLABLE, REASON_NOT_SELECTABLE, WHOLE_QUANTITY_ONLY,
};
use crate::model::cancel::{CancelAsk, cancel_outcome};
use crate::{order_for, rules, shopper};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCancelOptionsInput {
    /// The shopper's own order.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20455")]
    pub order_id: String,
    /// Lines and units to try cancelling. The whole order when left out.
    #[schemars(example = json!([{"order_item_id": "oln_1", "quantity": 1}]))]
    pub items: Option<Vec<CancelItemInput>>,
    /// Why; decides who is at fault and so shipping and restores. changed_mind (the most
    /// careful estimate) when left out.
    #[schemars(example = &"changed_mind")]
    pub reason: Option<CancelReason>,
}

pub struct GetCancelOptions;

impl Operation for GetCancelOptions {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.get_cancel_options",
        tool: "get_cancel_options",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/orders/{order_id}/cancel-options",
        summary: "Prepare a cancel of the shopper's own order: which lines and units can be cancelled, the reasons they may pick, the refund estimate by payment method, restored coupons, any extra shipping charge, and who confirms. Only the shopper's button sends it.",
        consumes: &[Slot::one("order_id")],
        produces: &[],
        checks: &[],
        errors: &[
            ORDER_NOT_FOUND,
            NOT_YOUR_ORDER,
            ORDER_ITEM_NOT_FOUND,
            QUANTITY_OVER_CANCELLABLE,
            WHOLE_QUANTITY_ONLY,
            ALREADY_SHIPPED,
            ALREADY_CANCELLED,
            CLAIM_IN_PROGRESS,
            REASON_NOT_SELECTABLE,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::CANCEL,
        person_scope: "",
    };
    type Input = GetCancelOptionsInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetCancelOptions {
    async fn read(&self, cx: &Cx, input: GetCancelOptionsInput) -> Result<Value, ToolError> {
        let rules = rules(&cx.policy)?;
        let me = shopper(&cx.principal);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let order = order_for(&mut tx, &input.order_id, me.as_deref(), "get_my_orders").await?;
        let view = cancel_view(&mut tx, &order, None).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let asked = asked_items(&input.items);
        let outcome = cancel_outcome(
            &rules,
            &view,
            CancelAsk {
                items: asked.as_deref(),
                reason: input.reason.map(CancelReason::as_str),
                ..CancelAsk::default()
            },
            cx.clock.now(),
        )
        .map_err(cancel_block_error)?;
        let total = outcome.items.len();
        let mut out = serde_json::to_value(&outcome)
            .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?;
        out["total"] = json!(total);
        Ok(out)
    }
}
