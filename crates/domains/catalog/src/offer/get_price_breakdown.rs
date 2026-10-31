//! `get_price_breakdown` (docs/api/catalog.md#offer).

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_core::Money;
use acom_store::Cx;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::{self, num};
use crate::repo::{self};
use crate::rules::PriceFloorRule;
use crate::viewer::Viewer;
use crate::{PRODUCT_NOT_FOUND, TIER_NOT_FOUND, neighbors, product_not_found};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetPriceBreakdownInput {
    /// The variant.
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_windbreaker_m_slate")]
    pub variant_id: String,
    /// Operators only. The member tier to price for (`list_tiers`), or `everyone`.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"vip")]
    pub valid_for_member_tier: Option<String>,
    /// Operators only. The moment to price at, in store time. Default now.
    #[schemars(example = &"2026-11-07T00:00:00-05:00")]
    pub as_of: Option<String>,
    /// Units, to see the line total too.
    #[schemars(range(min = 1), example = 2)]
    pub order_quantity: Option<u32>,
}

/// How a variant's price is made up for the caller.
#[derive(Debug)]
pub struct GetPriceBreakdown;

impl Operation for GetPriceBreakdown {
    const META: OpMeta = OpMeta {
        address: "catalog.offer.get_breakdown",
        tool: "get_price_breakdown",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/variants/{variant_id}/price-breakdown",
        summary: "The price of one variant taken apart: list price, the member or sale price in effect and the promotion it comes from. Operators also see cost and the price floor.",
        consumes: &[Slot::one("variant_id")],
        produces: &[],
        checks: &[],
        errors: &[PRODUCT_NOT_FOUND, TIER_NOT_FOUND, ErrorCode::FORBIDDEN],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetPriceBreakdownInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetPriceBreakdown {
    async fn read(&self, cx: &Cx, input: GetPriceBreakdownInput) -> Result<Value, ToolError> {
        let tier = input.valid_for_member_tier.as_deref();
        let as_of = input.as_of.as_deref();
        Viewer::check_inputs(cx, tier, as_of)?;
        if input.order_quantity == Some(0) {
            return Err(
                ToolError::invalid_input("`order_quantity` must be at least 1.")
                    .detail(json!({"field": "order_quantity", "reason": "range"})),
            );
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let viewer = Viewer::resolve(cx, &mut tx, tier, as_of).await?;
        let missing = || product_not_found("variant", &input.variant_id, "search_products");
        let mut v = repo::variant(&mut tx, &input.variant_id)
            .await?
            .filter(|v| v.offer_id.is_some())
            .ok_or_else(missing)?;
        crate::write::apply_scheduled_prices(&mut tx, std::slice::from_mut(&mut v), viewer.at)
            .await?;
        let group = crate::visible_group(cx, &mut tx, &v.product_group_id)
            .await?
            .ok_or_else(missing)?;
        let offer = v.offer_id.clone().unwrap_or_default();
        let prices = neighbors::sale_prices(
            &mut tx,
            std::slice::from_ref(&offer),
            viewer.tier.as_deref(),
            viewer.at,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let list = v.list_price().ok_or_else(missing)?;
        let best = model::best_sale_price(list.value, &prices);
        let mut out = json!({
            "variant_id": v.id, "product_id": group.id,
            "list_price": num(list.value),
            "member_price": best.map_or(Value::Null, |p| num(p.price)),
            "price_currency": list.currency.to_string(),
            "applied": best.map(|p| vec![json!({
                "promotion_id": p.promotion_id, "name": p.name,
                "valid_for_member_tier": p.valid_for_member_tier.as_deref().unwrap_or("everyone"),
                "valid_through": p.valid_through.map(|t| cx.time.format(t))})])
                .unwrap_or_default(),
        });
        if viewer.operator {
            let rule = PriceFloorRule::from_policy(&cx.policy)?;
            out["cost"] = v.cost().map_or(Value::Null, |c| num(c.value));
            out["price_floor"] = v.cost().map_or(Value::Null, |c| num(rule.floor(c).value));
        }
        if let Some(q) = input.order_quantity {
            let unit = best.map_or(list.value, |p| p.price);
            let total = Money::new(unit, list.currency)
                .times(Decimal::from(q))
                .rounded();
            out["order_quantity"] = json!(q);
            out["total"] = json!(total);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn windbreaker_breakdown_shows_cost_and_floor_to_operators() {
        let cx = testing::operator_db().await;
        let out = GetPriceBreakdown
            .read(
                &cx,
                GetPriceBreakdownInput {
                    variant_id: "var_windbreaker_m_slate".into(),
                    valid_for_member_tier: None,
                    as_of: None,
                    order_quantity: Some(2),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out["list_price"].to_string(), "89.00");
        assert_eq!(out["price_floor"].to_string(), "41.40");
        assert_eq!(out["total"]["value"].to_string(), "178.00");
    }
}
