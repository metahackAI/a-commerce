//! `search_promotions` (docs/api/promotion.md#promotion).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::{DiscountLayer, PromotionStatus};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{period, summary};
use crate::INVALID_FILTER;
use crate::repo::{self, PromotionFilter};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchPromotionsInput {
    /// Only these statuses (the status now). All when left out.
    #[schemars(example = ["ended"])]
    pub status: Option<Vec<PromotionStatus>>,
    /// Only promotions for this tier id, or `everyone` for those without a tier.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"everyone")]
    pub valid_for_member_tier: Option<String>,
    /// Only this discount layer. item_price: member and sale prices; order_discount: discounts
    /// on the order; add_on: store points and shipping.
    #[schemars(example = &"item_price")]
    pub discount_layer: Option<DiscountLayer>,
    /// Only promotions whose period reaches this moment or later (store time without an offset).
    #[schemars(example = &"2026-10-01T00:00")]
    pub valid_from: Option<String>,
    /// Only promotions that start by this moment (store time without an offset).
    #[schemars(example = &"2026-10-31T23:59")]
    pub valid_through: Option<String>,
    /// Words in the promotion's name.
    #[schemars(length(min = 1, max = 100), example = &"linen")]
    pub query: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct SearchPromotions;

impl Operation for SearchPromotions {
    const META: OpMeta = OpMeta {
        address: "promotion.promotion.search",
        tool: "search_promotions",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/promotions",
        summary: "Find promotions by status, period, tier, discount layer or name, whether or not they overlap anything.",
        consumes: &[],
        produces: &[Slot::many("promotion_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchPromotionsInput;
    type Output = Value;
}

impl ReadOp<Cx> for SearchPromotions {
    async fn read(&self, cx: &Cx, input: SearchPromotionsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let (from, through) = period(
            cx,
            input.valid_from.as_deref(),
            input.valid_through.as_deref(),
            INVALID_FILTER,
            "list_enums",
        )?;
        let filter = PromotionFilter {
            statuses: input
                .status
                .unwrap_or_default()
                .iter()
                .map(|s| s.as_str().to_owned())
                .collect(),
            tier: input.valid_for_member_tier,
            tier_reach: false,
            layer: input.discount_layer.map(|l| l.as_str().to_owned()),
            from,
            through,
            query: input.query.filter(|q| !q.trim().is_empty()),
        };
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (total, rows) = repo::promotions(&mut tx, &filter, now, Some(page)).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows.iter().map(|r| summary(cx, r, now)).collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}
