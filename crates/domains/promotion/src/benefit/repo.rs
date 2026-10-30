//! SQL for benefits: sale prices, order and add-on promotions, coupons and the coupons a member
//! holds, redeeming coupons, and what refers to a tier.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

pub(crate) fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// A sale price (`promotion.unit_price_specification`) with its promotion's state.
#[derive(Debug, sqlx::FromRow)]
pub(crate) struct SalePriceRow {
    pub offer_id: String,
    pub promotion_id: Option<String>,
    pub promotion_name: Option<String>,
    pub price: Decimal,
    pub price_currency: String,
    pub valid_for_member_tier: Option<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: Option<DateTime<Utc>>,
    pub promotion_status: Option<String>,
    pub promotion_from: Option<DateTime<Utc>>,
    pub promotion_through: Option<DateTime<Utc>>,
}

/// Every sale price of these offers; the caller keeps the ones that apply now.
pub(crate) async fn sale_prices(
    tx: &mut TxCx,
    offer_ids: &[String],
) -> Result<Vec<SalePriceRow>, ToolError> {
    if offer_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, SalePriceRow>(
        "SELECT u.offer_id, u.promotion_id, p.name AS promotion_name, u.price,
                u.price_currency::text AS price_currency, u.valid_for_member_tier, u.valid_from,
                u.valid_through, p.promotion_status, p.valid_from AS promotion_from,
                p.valid_through AS promotion_through
         FROM promotion.unit_price_specification u
         LEFT JOIN promotion.promotion p ON p.tenant_id = u.tenant_id AND p.id = u.promotion_id
         WHERE u.tenant_id = $1 AND u.offer_id = ANY($2) AND u.price_type = 'sale_price'",
    )
    .bind(tx.tenant.as_str())
    .bind(offer_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A promotion (`promotion.promotion`).
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct PromotionRow {
    pub id: String,
    pub name: String,
    pub promotion_status: String,
    pub discount_layer: String,
    pub valid_for_member_tier: Option<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: Option<DateTime<Utc>>,
    pub percent_off: Option<Decimal>,
    pub amount_off_value: Option<Decimal>,
    pub amount_off_currency: Option<String>,
    pub eligible_transaction_volume_value: Option<Decimal>,
    pub eligible_transaction_volume_currency: Option<String>,
    pub applies_to: Value,
}

const PROMOTION_COLUMNS: &str = "id, name, promotion_status, discount_layer, valid_for_member_tier,
    valid_from, valid_through, percent_off, amount_off_value,
    amount_off_currency::text AS amount_off_currency, eligible_transaction_volume_value,
    eligible_transaction_volume_currency::text AS eligible_transaction_volume_currency, applies_to";

/// Scheduled and running order-discount and add-on promotions.
pub(crate) async fn order_promotions(tx: &mut TxCx) -> Result<Vec<PromotionRow>, ToolError> {
    sqlx::query_as::<_, PromotionRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {PROMOTION_COLUMNS} FROM promotion.promotion
         WHERE tenant_id = $1 AND discount_layer IN ('order_discount', 'add_on')
           AND promotion_status IN ('scheduled', 'running')
         ORDER BY id"
    )))
    .bind(tx.tenant.as_str())
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// These promotions, whatever their state.
pub(crate) async fn promotions(
    tx: &mut TxCx,
    ids: &[String],
) -> Result<Vec<PromotionRow>, ToolError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, PromotionRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {PROMOTION_COLUMNS} FROM promotion.promotion
         WHERE tenant_id = $1 AND id = ANY($2) ORDER BY id"
    )))
    .bind(tx.tenant.as_str())
    .bind(ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A coupon definition (`promotion.coupon`).
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct CouponRow {
    pub id: String,
    pub discount_code: String,
    pub promotion_id: String,
    pub valid_for_member_tier: Option<String>,
    pub application: String,
    pub stacking: String,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_through: Option<DateTime<Utc>>,
}

/// The coupons tied to these promotions, plus the coupons named by id.
pub(crate) async fn coupons(
    tx: &mut TxCx,
    promotion_ids: &[String],
    coupon_ids: &[String],
) -> Result<Vec<CouponRow>, ToolError> {
    sqlx::query_as::<_, CouponRow>(
        "SELECT id, discount_code, promotion_id, valid_for_member_tier, application, stacking,
                valid_from, valid_through
         FROM promotion.coupon
         WHERE tenant_id = $1 AND (promotion_id = ANY($2) OR id = ANY($3))
         ORDER BY id",
    )
    .bind(tx.tenant.as_str())
    .bind(promotion_ids)
    .bind(coupon_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A coupon issued to the member and not used yet (`promotion.member_coupon`).
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct HeldRow {
    pub id: String,
    pub coupon_id: String,
    pub owned_from: DateTime<Utc>,
    pub owned_through: Option<DateTime<Utc>>,
}

pub(crate) async fn held_coupons(
    tx: &mut TxCx,
    customer_id: &str,
) -> Result<Vec<HeldRow>, ToolError> {
    sqlx::query_as::<_, HeldRow>(
        "SELECT id, coupon_id, owned_from, owned_through FROM promotion.member_coupon
         WHERE tenant_id = $1 AND customer_id = $2
           AND coupon_status IN ('issued', 'available', 'restored')
         ORDER BY owned_from, id",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The member coupon already used on this order for this coupon, if any.
pub(crate) async fn used_on_order(
    tx: &mut TxCx,
    coupon_id: &str,
    order_id: &str,
) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM promotion.member_coupon
         WHERE tenant_id = $1 AND coupon_id = $2 AND order_id = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(coupon_id)
    .bind(order_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Marks a held member coupon used on an order. `false` when it was no longer usable.
pub(crate) async fn use_held(
    tx: &mut TxCx,
    member_coupon_id: &str,
    order_id: &str,
    benefit: (Decimal, &str),
) -> Result<bool, ToolError> {
    let now = tx.clock.now();
    let done = sqlx::query(
        "UPDATE promotion.member_coupon
         SET coupon_status = 'used', order_id = $3, used_at = $4, benefit_recorded_value = $5,
             benefit_recorded_currency = $6, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2
           AND coupon_status IN ('issued', 'available', 'restored')",
    )
    .bind(tx.tenant.as_str())
    .bind(member_coupon_id)
    .bind(order_id)
    .bind(now)
    .bind(benefit.0)
    .bind(benefit.1)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(done.rows_affected() == 1)
}

/// Records a coupon that is applied without being issued first (automatic or code entry) as
/// used on an order: such coupons get their member coupon row when they are used.
pub(crate) async fn insert_used(
    tx: &mut TxCx,
    id: &str,
    coupon_id: &str,
    customer_id: &str,
    order_id: &str,
    benefit: (Decimal, &str),
) -> Result<(), ToolError> {
    let now = tx.clock.now();
    sqlx::query(
        "INSERT INTO promotion.member_coupon (tenant_id, id, coupon_id, customer_id,
           coupon_status, owned_from, order_id, used_at, benefit_recorded_value,
           benefit_recorded_currency)
         VALUES ($1, $2, $3, $4, 'used', $5, $6, $5, $7, $8)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(coupon_id)
    .bind(customer_id)
    .bind(now)
    .bind(order_id)
    .bind(benefit.0)
    .bind(benefit.1)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// How many scheduled or running promotions, and coupons of such promotions, name the tier.
pub(crate) async fn tier_references(tx: &mut TxCx, tier_id: &str) -> Result<(i64, i64), ToolError> {
    sqlx::query_as(
        "SELECT
           (SELECT count(*) FROM promotion.promotion
             WHERE tenant_id = $1 AND valid_for_member_tier = $2
               AND promotion_status IN ('scheduled', 'running')),
           (SELECT count(*) FROM promotion.coupon c
             JOIN promotion.promotion p ON p.tenant_id = c.tenant_id AND p.id = c.promotion_id
             WHERE c.tenant_id = $1 AND c.valid_for_member_tier = $2
               AND p.promotion_status IN ('scheduled', 'running'))",
    )
    .bind(tx.tenant.as_str())
    .bind(tier_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Names of these promotions.
pub(crate) async fn promotion_names(
    tx: &mut TxCx,
    ids: &[String],
) -> Result<Vec<(String, String)>, ToolError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as("SELECT id, name FROM promotion.promotion WHERE tenant_id = $1 AND id = ANY($2)")
        .bind(tx.tenant.as_str())
        .bind(ids)
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)
}

/// The code and promotion of these coupons.
pub(crate) async fn coupon_facts(
    tx: &mut TxCx,
    ids: &[String],
) -> Result<Vec<(String, String, String)>, ToolError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as(
        "SELECT id, discount_code, promotion_id FROM promotion.coupon
         WHERE tenant_id = $1 AND id = ANY($2)",
    )
    .bind(tx.tenant.as_str())
    .bind(ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}
