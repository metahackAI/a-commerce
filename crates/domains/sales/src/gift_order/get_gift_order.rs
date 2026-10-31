//! `get_gift_order` (`sales.gift_order.get`): the gift as its recipient opens it from the
//! link. No prices.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::gift_view;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetGiftOrderInput {
    /// The gift, from the accept link.
    #[schemars(regex(pattern = r"^gift_[a-z0-9_]+$"), example = &"gift_01jc9x2m")]
    pub gift_order_id: String,
}

/// What the recipient sees through the link. The link is the permission, so nothing about the
/// sender's contact, address or payment, or the recipient's contact, is shown, and no price
/// (`gift.gift_receipt_hides_prices`).
pub(crate) struct GetGiftOrder;

impl Operation for GetGiftOrder {
    const META: OpMeta = OpMeta {
        address: "sales.gift_order.get",
        tool: "get_gift_order",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/sales/gift-orders/{gift_order_id}",
        summary: "A gift as its recipient opens it from the link: what it is, who sent it and their card, the sizes and colors they may switch to at the same price, and until when they can accept. No prices.",
        consumes: &[Slot::one("gift_order_id")],
        produces: &[Slot::one("gift_order_id")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Shopper,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetGiftOrderInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetGiftOrder {
    async fn read(&self, cx: &Cx, input: GetGiftOrderInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let view = gift_view(&mut tx, &input.gift_order_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<GetGiftOrderInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
    }
}
