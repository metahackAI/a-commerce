//! The `coupon` resource (docs/api/promotion.md#coupon). A coupon records its promotion's
//! benefit on the order; it never adds a second discount (`coupon.records_benefit_only`). What
//! its operations share: the error codes, the coupon as reads show it, and policy reads.

pub(crate) mod create_coupon;
pub(crate) mod get_coupon;
pub(crate) mod issue_coupon;
pub(crate) mod list_member_coupons;
pub(crate) mod search_coupons;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_store::Cx;
use serde_json::{Value, json};

use crate::model::decimal;
use crate::repo::CouponRow;
use crate::{audience, time_or_null};

pub const CODE_TAKEN: ErrorCode = ErrorCode::new("code_taken", ErrorClass::AlreadyExists);
pub const COUPON_TIER_MISMATCH: ErrorCode =
    ErrorCode::new("coupon_tier_mismatch", ErrorClass::Domain);
pub const RECIPIENT_TIER_MISMATCH: ErrorCode =
    ErrorCode::new("recipient_tier_mismatch", ErrorClass::Domain);
pub const STACKING_CONFLICT: ErrorCode =
    ErrorCode::new("stacking_conflict", ErrorClass::PolicyViolation);
pub const POOL_EXHAUSTED: ErrorCode = ErrorCode::new("pool_exhausted", ErrorClass::LimitExceeded);

/// A coupon as the reads show it.
fn coupon_json(cx: &Cx, c: &CouponRow) -> Value {
    json!({
        "id": c.id,
        "code": c.discount_code,
        "name": c.name.clone().unwrap_or_else(|| c.promotion_name.clone()),
        "promotion_id": c.promotion_id,
        "valid_for_member_tier": audience(c.valid_for_member_tier.as_deref()),
        "application": c.application,
        "stacking": c.stacking,
        "valid_from": time_or_null(&cx.time, c.valid_from),
        "valid_through": time_or_null(&cx.time, c.valid_through),
    })
}

/// A string list from a policy key.
fn words(cx: &Cx, key: &str) -> Vec<String> {
    cx.policy
        .lookup(key)
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(|w| w.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn policy_number(cx: &Cx, key: &str) -> Result<i64, ToolError> {
    cx.policy
        .lookup(key)
        .and_then(decimal)
        .and_then(|d| i64::try_from(d.trunc()).ok())
        .ok_or_else(|| {
            ToolError::new(
                ErrorCode::INTERNAL,
                format!("The policy value `{key}` is missing or not readable."),
            )
        })
}
