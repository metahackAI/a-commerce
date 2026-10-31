//! The `coupon` resource (docs/api/promotion.md#coupon). A coupon records its promotion's
//! benefit on the order; it never adds a second discount (`coupon.records_benefit_only`). What
//! its operations share: the error codes, the coupon as reads show it, and policy reads.

pub(crate) mod create_coupon;
pub(crate) mod get_coupon;
pub(crate) mod issue_coupon;
pub(crate) mod list_member_coupons;
pub(crate) mod redeem_coupon;
pub(crate) mod restore_coupon;
pub(crate) mod revoke_coupon;
pub(crate) mod search_coupons;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::repo::db;

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
pub const COUPON_EXPIRED: ErrorCode = ErrorCode::new("coupon_expired", ErrorClass::Domain);
pub const MIN_ORDER_NOT_MET: ErrorCode = ErrorCode::new("min_order_not_met", ErrorClass::Domain);
pub const ALREADY_USED: ErrorCode = ErrorCode::new("already_used", ErrorClass::InvalidState);

/// One member coupon (`promotion.member_coupon`) as the writes on it read it.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct MemberCouponState {
    pub id: String,
    pub coupon_id: String,
    pub customer_id: String,
    /// The stored `coupon_status` word.
    pub coupon_status: String,
    pub order_id: Option<String>,
    pub owned_through: Option<DateTime<Utc>>,
    pub version: i32,
}

const MEMBER_COUPON_SELECT: &str = "SELECT id, coupon_id, customer_id, coupon_status, order_id,
        owned_through, version FROM promotion.member_coupon WHERE tenant_id = $1";

/// One member coupon of a coupon, by id.
pub(crate) async fn member_coupon(
    tx: &mut TxCx,
    coupon_id: &str,
    id: &str,
) -> Result<Option<MemberCouponState>, ToolError> {
    sqlx::query_as::<_, MemberCouponState>(sqlx::AssertSqlSafe(format!(
        "{MEMBER_COUPON_SELECT} AND coupon_id = $2 AND id = $3"
    )))
    .bind(tx.tenant.as_str())
    .bind(coupon_id)
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The member coupons of a coupon recorded on an order.
pub(crate) async fn on_order(
    tx: &mut TxCx,
    coupon_id: &str,
    order_id: &str,
) -> Result<Vec<MemberCouponState>, ToolError> {
    sqlx::query_as::<_, MemberCouponState>(sqlx::AssertSqlSafe(format!(
        "{MEMBER_COUPON_SELECT} AND coupon_id = $2 AND order_id = $3 ORDER BY id"
    )))
    .bind(tx.tenant.as_str())
    .bind(coupon_id)
    .bind(order_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The member coupons of a coupon in any of `statuses`.
pub(crate) async fn in_statuses(
    tx: &mut TxCx,
    coupon_id: &str,
    statuses: &[&str],
) -> Result<Vec<MemberCouponState>, ToolError> {
    sqlx::query_as::<_, MemberCouponState>(sqlx::AssertSqlSafe(format!(
        "{MEMBER_COUPON_SELECT} AND coupon_id = $2 AND coupon_status = ANY($3) ORDER BY id"
    )))
    .bind(tx.tenant.as_str())
    .bind(coupon_id)
    .bind(statuses)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Every member coupon of a customer used on an order, any coupon.
pub(crate) async fn used_by(
    tx: &mut TxCx,
    customer_id: &str,
) -> Result<Vec<MemberCouponState>, ToolError> {
    sqlx::query_as::<_, MemberCouponState>(sqlx::AssertSqlSafe(format!(
        "{MEMBER_COUPON_SELECT} AND customer_id = $2 AND coupon_status = 'used' ORDER BY id"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Moves a member coupon from one status to another and sets when it expires; `false` when it
/// was not in `from`.
pub(crate) async fn move_member_coupon(
    tx: &mut TxCx,
    id: &str,
    from: &str,
    to: &str,
    owned_through: Option<DateTime<Utc>>,
) -> Result<bool, ToolError> {
    Ok(sqlx::query(
        "UPDATE promotion.member_coupon SET coupon_status = $4, owned_through = $5,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND coupon_status = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(from)
    .bind(to)
    .bind(owned_through)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected()
        == 1)
}

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
