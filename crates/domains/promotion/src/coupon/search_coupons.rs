//! `search_coupons` (docs/api/promotion.md#coupon).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::CouponApplication;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::coupon_json;
use crate::repo::{self, CouponFilter};
use crate::{INVALID_FILTER, check_id};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchCouponsInput {
    /// The exact code (any letter case).
    #[schemars(length(min = 4, max = 20), example = &"VIPWEEKEND20")]
    pub code: Option<String>,
    /// Only the coupons of this promotion.
    #[schemars(regex(pattern = r"^promo_[a-z0-9_]+$"), example = &"promo_birthday")]
    pub promotion_id: Option<String>,
    /// Only coupons that reach an order this way. automatic_at_checkout, code_entry or download.
    #[schemars(example = &"download")]
    pub application: Option<CouponApplication>,
    /// Only coupons for this tier id, or `everyone` for those without a tier.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"gold")]
    pub valid_for_member_tier: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct SearchCoupons;

impl Operation for SearchCoupons {
    const META: OpMeta = OpMeta {
        address: "promotion.coupon.search",
        tool: "search_coupons",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/coupons",
        summary: "Coupon definitions by code, promotion, application or tier, with the period each follows.",
        consumes: &[],
        produces: &[Slot::many("coupon_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchCouponsInput;
    type Output = Value;
}

impl ReadOp<Cx> for SearchCoupons {
    async fn read(&self, cx: &Cx, input: SearchCouponsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        if let Some(p) = &input.promotion_id {
            check_id("promotion_id", "promo", p, false)
                .map_err(|e| ToolError::new(INVALID_FILTER, e.message).next("list_enums"))?;
        }
        let filter = CouponFilter {
            code: input.code.map(|c| c.trim().to_uppercase()),
            promotion_id: input.promotion_id,
            application: input.application.map(|a| a.as_str().to_owned()),
            tier: input.valid_for_member_tier,
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (total, rows) = repo::coupons(&mut tx, &filter, Some(page)).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows.iter().map(|c| coupon_json(cx, c)).collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}
