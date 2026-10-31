//! `get_promotion` (docs/api/promotion.md#promotion).

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{applies_to, product_counts, summary};
use crate::repo;
use crate::{check_id, not_found, percent, pick};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetPromotionInput {
    /// The promotion.
    #[schemars(regex(pattern = r"^promo_[a-z0-9_]+$"), example = &"promo_autumn_knit")]
    pub promotion_id: String,
    /// Only these fields of the promotion (`id` always comes back).
    #[schemars(example = ["status", "applies_to", "coupon_ids"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetPromotion;

impl Operation for GetPromotion {
    const META: OpMeta = OpMeta {
        address: "promotion.promotion.get",
        tool: "get_promotion",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/promotions/{promotion_id}",
        summary: "One promotion with what it covers, how many member or sale prices it holds, its coupons, and the change set that created it.",
        consumes: &[Slot::one("promotion_id")],
        produces: &[Slot::many("coupon_id")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetPromotionInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetPromotion {
    async fn read(&self, cx: &Cx, input: GetPromotionInput) -> Result<Value, ToolError> {
        check_id("promotion_id", "promo", &input.promotion_id, false)?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let r = repo::promotion(&mut tx, &input.promotion_id)
            .await?
            .ok_or_else(|| not_found("promotion", &input.promotion_id, "search_promotions"))?;
        let counts = product_counts(&mut tx, std::slice::from_ref(&r)).await?;
        let prices = repo::price_count(&mut tx, &r.id).await?;
        let coupons = repo::coupon_ids_of(&mut tx, &r.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut out = summary(cx, &r, now);
        out["description"] = json!(r.description);
        out["percent_off"] = r.percent_off.map_or(Value::Null, percent);
        out["amount_off"] = match (r.amount_off_value, &r.amount_off_currency) {
            (Some(v), Some(c)) => crate::money(v, c),
            _ => Value::Null,
        };
        out["applies_to"] = applies_to(&r, counts.get(&r.id).copied().flatten());
        out["price_specification_count"] = json!(prices);
        out["coupon_ids"] = json!(coupons);
        out["change_set_id"] = json!(r.change_set_id);
        out["version"] = json!(r.version);
        Ok(pick(out, input.fields.as_deref()))
    }
}
