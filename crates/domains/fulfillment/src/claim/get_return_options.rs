//! `get_return_options`: the shopper's choices for returning one order line. Changes nothing;
//! `POST /v1/fulfillment/returns` computes the same again when the shopper presses the button.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::ClaimReason;
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{options_for, return_view};
use crate::errors::{
    CLAIM_IN_PROGRESS, NOT_RETURNABLE, NOT_SHIPPED_YET, NOT_YOUR_ORDER, ORDER_ITEM_NOT_FOUND,
    ORDER_NOT_FOUND, OUTSIDE_RETURN_WINDOW,
};
use crate::rules;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetReturnOptionsInput {
    /// The shopper's own order, from `get_my_orders`.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20402")]
    pub order_id: String,
    /// The order line to return.
    #[schemars(regex(pattern = r"^oln_[a-z0-9_]+$"), example = &"oln_20402_1")]
    pub order_item_id: String,
    /// Why. Store fault: defect, damaged_in_transit, not_as_described, wrong_item_sent,
    /// wrong_option_sent, missing_parts. Shopper's choice: wrong_size,
    /// different_option_wanted, changed_mind.
    #[schemars(example = &"defect")]
    pub reason: ClaimReason,
    /// Units to return, 1 or more. Every unit of the line when left out.
    #[schemars(range(min = 1), example = 1)]
    pub quantity: Option<u32>,
}

pub struct GetReturnOptions;

impl Operation for GetReturnOptions {
    const META: OpMeta = OpMeta {
        address: "fulfillment.claim.get_return_options",
        tool: "get_return_options",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/orders/{order_id}/items/{order_item_id}/return-options",
        summary: "The shopper's choices for returning one of their order lines: a replacement or a refund, each with when it happens, what they pay, who confirms it, and how the item goes back. Only the shopper's button sends the request.",
        consumes: &[Slot::one("order_id"), Slot::one("order_item_id")],
        produces: &[],
        checks: &[],
        errors: &[
            ORDER_NOT_FOUND,
            ORDER_ITEM_NOT_FOUND,
            NOT_YOUR_ORDER,
            OUTSIDE_RETURN_WINDOW,
            NOT_RETURNABLE,
            NOT_SHIPPED_YET,
            CLAIM_IN_PROGRESS,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetReturnOptionsInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetReturnOptions {
    async fn read(&self, cx: &Cx, input: GetReturnOptionsInput) -> Result<Value, ToolError> {
        let rules = rules(&cx.policy)?;
        let now = cx.clock.now();
        let quantity = input.quantity.map(|q| i32::try_from(q).unwrap_or(i32::MAX));
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let view = return_view(
            &mut tx,
            &input.order_id,
            &input.order_item_id,
            quantity,
            now,
            cx.time.zone,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let (_, options) = options_for(
            &rules,
            &cx.policy,
            &view,
            input.reason.as_str(),
            quantity,
            now,
            &cx.time,
        )?;
        let total = options.len();
        Ok(json!({"items": options, "total": total}))
    }
}
