//! `get_coupon` (docs/api/promotion.md#coupon).

use std::collections::BTreeMap;

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError, fields,
};
use acom_core::enums::CouponStatus;
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::coupon_json;
use crate::repo;
use crate::{check_id, not_found};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCouponInput {
    /// The coupon.
    #[schemars(regex(pattern = r"^cpn_[a-z0-9_]+$"), example = &"cpn_birthday")]
    pub coupon_id: String,
    /// Only these fields of the coupon (`id` always comes back).
    #[schemars(example = ["code", "counts"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetCoupon;

impl Operation for GetCoupon {
    const META: OpMeta = OpMeta {
        address: "promotion.coupon.get",
        tool: "get_coupon",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/coupons/{coupon_id}",
        summary: "One coupon definition with how many member copies are in each status (issued, available, used, restored, expired, revoked).",
        consumes: &[Slot::one("coupon_id")],
        produces: &[Slot::one("promotion_id")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetCouponInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetCoupon {
    async fn read(&self, cx: &Cx, input: GetCouponInput) -> Result<Value, ToolError> {
        check_id("coupon_id", "cpn", &input.coupon_id, false)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let c = repo::coupon(&mut tx, &input.coupon_id)
            .await?
            .ok_or_else(|| not_found("coupon", &input.coupon_id, "search_coupons"))?;
        let counted: BTreeMap<String, i64> = repo::coupon_counts(&mut tx, &c.id)
            .await?
            .into_iter()
            .collect();
        tx.commit().await.map_err(ToolError::from)?;
        let counts: serde_json::Map<String, Value> = CouponStatus::ALL
            .iter()
            .map(|s| {
                (
                    s.as_str().to_owned(),
                    json!(counted.get(s.as_str()).copied().unwrap_or(0)),
                )
            })
            .collect();
        let mut out = coupon_json(cx, &c);
        out["records_benefit_only"] = json!(c.is_benefit_record_only);
        out["counts"] = Value::Object(counts);
        fields::keep(out, &input.fields.unwrap_or_default())
    }
}
