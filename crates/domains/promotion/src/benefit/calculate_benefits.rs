//! `calculate_benefits` (docs/api/promotion.md#benefit): the tool over [`super::calculate`].

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_core::Money;
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CART_NOT_FOUND, CART_READER, Request, breakdown_json, calculate};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalculateBenefitsInput {
    /// The cart whose lines and unit prices to use (`get_cart`).
    #[schemars(regex(pattern = r"^cart_[a-z0-9_]+$"), example = &"cart_jamie")]
    pub cart_id: String,
    /// The shopper; the server looks up their tier and coupons.
    #[schemars(regex(pattern = r"^cus_[a-z0-9_]+$"), example = &"cus_jamie")]
    pub customer_id: String,
    /// Code-entry or downloaded coupons to apply (`cpn_…`). Automatic coupons are found by the
    /// server.
    #[schemars(example = ["cpn_birthday"])]
    pub coupon_ids: Option<Vec<String>>,
    /// Loyalty points to spend, as money.
    #[schemars(example = json!({"value": 0.00, "currency": "USD"}))]
    pub spend_loyalty: Option<Money>,
}

#[derive(Debug)]
pub struct CalculateBenefits;

impl Operation for CalculateBenefits {
    const META: OpMeta = OpMeta {
        address: "promotion.benefit.calculate",
        tool: "calculate_benefits",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Post,
        path: "/v1/promotion/benefits/calculate",
        summary: "Every discount layer of a cart with its reason: member and sale prices, one order discount, coupons recorded, loyalty, free shipping, and the totals. The single source of cart and checkout numbers.",
        consumes: &[Slot::one("cart_id"), Slot::one("customer_id")],
        produces: &[],
        checks: &[
            "discount_layers",
            "member_benefits_per_order",
            "max_discount",
            "loyalty.spend",
            "price_floor",
            "free_shipping_min_subtotal",
        ],
        errors: &[CART_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::ASSESS,
        person_scope: "",
    };
    type Input = CalculateBenefitsInput;
    type Output = Value;
}

impl ReadOp<Cx> for CalculateBenefits {
    async fn read(&self, cx: &Cx, input: CalculateBenefitsInput) -> Result<Value, ToolError> {
        let shopper = cx.principal.customer_id().map(str::to_owned);
        let staff = shopper.is_none() && !cx.principal.subject.starts_with("ses_");
        if !staff && shopper.as_deref() != Some(input.customer_id.as_str()) {
            return Err(
                ToolError::new(ErrorCode::FORBIDDEN, "You can only price your own cart.")
                    .detail(json!({"reason": "owner"})),
            );
        }
        let coupon_ids = input.coupon_ids.unwrap_or_default();
        if let Some(bad) = coupon_ids.iter().find(|c| !c.starts_with("cpn_")) {
            return Err(
                ToolError::invalid_input(format!("`{bad}` is not a coupon id."))
                    .detail(json!({"field": "coupon_ids", "reason": "format"})),
            );
        }
        let reader = CART_READER.get().copied().ok_or_else(|| {
            ToolError::new(
                ErrorCode::UPSTREAM_UNAVAILABLE,
                "Carts are not connected to the benefit calculator.",
            )
        })?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let cart = reader(&mut tx, &input.cart_id, &input.customer_id)
            .await?
            .ok_or_else(|| cart_not_found(&input.cart_id))?;
        let session = cx.principal.session_id.as_deref();
        let owned = cart.customer_id.as_deref() == Some(input.customer_id.as_str())
            || (cart.session_id.is_some() && cart.session_id.as_deref() == session);
        if !staff && !owned {
            return Err(cart_not_found(&input.cart_id));
        }
        let b = calculate(
            &mut tx,
            &Request {
                customer_id: Some(input.customer_id.clone()),
                tier_id: cart.tier_id.clone(),
                lines: cart.lines.clone(),
                coupon_ids,
                spend_loyalty: input.spend_loyalty,
            },
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(breakdown_json(&cart.cart_id, &b))
    }
}

fn cart_not_found(id: &str) -> ToolError {
    ToolError::new(CART_NOT_FOUND, format!("There is no cart {id}."))
        .detail(json!({"entity": "cart", "id": id}))
        .next("get_cart")
}
