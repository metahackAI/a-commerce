//! `get_product` (docs/api/catalog.md#product).

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::viewer::Viewer;
use crate::{TIER_NOT_FOUND, cards, not_found, project};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetProductInput {
    /// The product, as `search_products` returns it.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_windbreaker")]
    pub product_id: String,
    /// Operators only. The member tier whose price to show (`list_tiers`), or `everyone`.
    /// Default everyone.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"vip")]
    pub valid_for_member_tier: Option<String>,
    /// Operators only. The moment to price at, in store time. Default now.
    #[schemars(example = &"2026-11-07T00:00:00-05:00")]
    pub as_of: Option<String>,
    /// Only these top-level fields.
    #[schemars(example = ["id", "price", "has_variant"])]
    pub fields: Option<Vec<String>>,
}

/// One product for selling: list price, the caller's member price, stock per variant, and the
/// attributes used for comparing.
#[derive(Debug)]
pub struct GetProduct;

impl Operation for GetProduct {
    const META: OpMeta = OpMeta {
        address: "catalog.product.get",
        tool: "get_product",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/products/{product_id}",
        summary: "One product as it sells: list price, the member price that applies to the caller, total and per-variant stock, the variant ids to add to a cart, and the attributes used to compare. Operators also see cost.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::many("variant_id")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND, TIER_NOT_FOUND, ErrorCode::FORBIDDEN],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetProductInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetProduct {
    async fn read(&self, cx: &Cx, input: GetProductInput) -> Result<Value, ToolError> {
        let tier = input.valid_for_member_tier.as_deref();
        let as_of = input.as_of.as_deref();
        Viewer::check_inputs(cx, tier, as_of)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let viewer = Viewer::resolve(cx, &mut tx, tier, as_of).await?;
        let card = cards::load(&mut tx, std::slice::from_ref(&input.product_id), &viewer)
            .await?
            .into_iter()
            .next();
        tx.commit().await.map_err(ToolError::from)?;
        let card =
            card.ok_or_else(|| not_found("product", &input.product_id, "search_products"))?;
        Ok(project(
            card.product_json(&viewer, &cx.time),
            &input.fields.unwrap_or_default(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;
    use serde_json::json;

    #[tokio::test]
    async fn shoppers_cannot_ask_for_another_tier() {
        let e = GetProduct
            .read(
                &testing::shopper(),
                GetProductInput {
                    product_id: "prd_windbreaker".into(),
                    valid_for_member_tier: Some("vip".into()),
                    as_of: None,
                    fields: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        assert_eq!(e.detail.map(|d| d["reason"].clone()), Some(json!("scope")));
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn windbreaker_sells_at_list_price_on_friday() {
        let cx = testing::operator_db().await;
        let out = GetProduct
            .read(
                &cx,
                GetProductInput {
                    product_id: "prd_windbreaker".into(),
                    valid_for_member_tier: Some("vip".into()),
                    as_of: None,
                    fields: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out["price"].to_string(), "89.00");
        assert_eq!(out["cost"].to_string(), "36.00");
        assert_eq!(out["inventory_level"], json!(214));
        assert_eq!(out["member_price"], Value::Null);
    }
}
