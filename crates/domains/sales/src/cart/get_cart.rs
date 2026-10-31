//! `get_cart` (`sales.cart.get`): the caller's own cart, priced by `calculate_benefits`.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::owner;
use crate::pricing::{self, DeliveryChoice};
use crate::rules;

/// Top-level fields of a cart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CartField {
    /// The cart id.
    Id,
    /// The lines.
    Lines,
    /// Sum of list prices.
    Subtotal,
    /// One discount line per promotion.
    Discounts,
    /// Shipping charged.
    Shipping,
    /// What the shopper pays.
    Total,
    /// Where it goes and when it arrives.
    Delivery,
    /// How it is paid.
    Payment,
    /// Coupons the order will record.
    CouponsRecorded,
}

impl CartField {
    fn word(self) -> &'static str {
        match self {
            CartField::Id => "id",
            CartField::Lines => "lines",
            CartField::Subtotal => "subtotal",
            CartField::Discounts => "discounts",
            CartField::Shipping => "shipping",
            CartField::Total => "total",
            CartField::Delivery => "delivery",
            CartField::Payment => "payment",
            CartField::CouponsRecorded => "coupons_recorded",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCartInput {
    /// Only these top-level fields. All when left out.
    #[schemars(example = ["lines", "total"])]
    pub fields: Option<Vec<CartField>>,
}

pub(crate) struct GetCart;

impl Operation for GetCart {
    const META: OpMeta = OpMeta {
        address: "sales.cart.get",
        tool: "get_cart",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/sales/carts/mine",
        summary: "The caller's own cart: lines, subtotal, one discount line per promotion, shipping, total, delivery estimate, payment method and the coupons the order will record, all from calculate_benefits. An empty cart when there is none.",
        consumes: &[],
        produces: &[Slot::one("cart_id"), Slot::many("cart_line_id")],
        checks: &[
            "discount_layers",
            "member_benefits_per_order",
            "free_shipping_min_subtotal",
        ],
        errors: &[],
        audience: Audience::Shopper,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetCartInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetCart {
    async fn read(&self, cx: &Cx, input: GetCartInput) -> Result<Value, ToolError> {
        let owners = owner::owners(&cx.principal)?;
        let limits = rules::cart_limits(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let carts = pricing::load_carts(&mut tx, &owners, limits).await?;
        let view = pricing::view(&mut tx, &carts, &DeliveryChoice::CartDefault, None).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let view = view.map_err(|id| crate::address_not_found(&id))?;
        let fields: Vec<String> = input
            .fields
            .unwrap_or_default()
            .iter()
            .map(|f| f.word().to_owned())
            .collect();
        Ok(pricing::pick(pricing::cart_json(&view), &fields))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<GetCartInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
    }
}
