//! `list_member_coupons` (docs/api/promotion.md#coupon).

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_core::enums::CouponStatus;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::repo;
use crate::{known_member, time_or_null, whose};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListMemberCouponsInput {
    /// The member, or `me` for the signed-in shopper.
    #[schemars(regex(pattern = r"^(cus_[a-z0-9_]+|me)$"), example = &"me")]
    pub customer_id: String,
    /// Only these statuses (unused copies past their end read as expired).
    #[schemars(example = ["available"])]
    pub status: Option<Vec<CouponStatus>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListMemberCoupons;

impl Operation for ListMemberCoupons {
    const META: OpMeta = OpMeta {
        address: "promotion.coupon.list_member_coupons",
        tool: "list_member_coupons",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/customers/{customer_id}/coupons",
        summary: "The coupons one member holds and their status. A shopper uses customer_id `me` for their own.",
        consumes: &[Slot::one("customer_id")],
        produces: &[Slot::many("member_coupon_id")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListMemberCouponsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListMemberCoupons {
    async fn read(&self, cx: &Cx, input: ListMemberCouponsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let customer_id = whose(cx, &input.customer_id)?;
        let statuses: Vec<String> = input
            .status
            .unwrap_or_default()
            .iter()
            .map(|s| s.as_str().to_owned())
            .collect();
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        known_member(&mut tx, &customer_id).await?;
        let (total, rows) =
            repo::member_coupons(&mut tx, &customer_id, now, &statuses, page).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|m| {
                json!({"id": m.id, "coupon_id": m.coupon_id, "code": m.code, "name": m.name,
                       "status": m.status, "order_id": m.order_id,
                       "valid_from": cx.time.format(m.owned_from),
                       "valid_through": time_or_null(&cx.time, m.owned_through),
                       "used_at": time_or_null(&cx.time, m.used_at)})
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}
