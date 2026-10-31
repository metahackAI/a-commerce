//! SQL for the promotion tables the whole domain shares (`migrations/0005_promotion.sql`,
//! `0022_promotion_engage.sql`); the tables of one resource (ledger, events, segments, gifts) have
//! their SQL in that resource's `mod.rs`. Every query filters `tenant_id`; values are bind
//! parameters and the few dynamic parts are fixed strings.

use acom_contract::ToolError;
use acom_store::{Page, StoreError, TxCx};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

pub(crate) fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// A promotion row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct PromotionRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub promotion_status: String,
    pub discount_layer: String,
    pub valid_for_member_tier: Option<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: Option<DateTime<Utc>>,
    pub percent_off: Option<Decimal>,
    pub amount_off_value: Option<Decimal>,
    pub amount_off_currency: Option<String>,
    pub applies_to: Value,
    pub change_set_id: Option<String>,
    pub version: i32,
}

const PROMOTION_COLUMNS: &str = "id, name, description, promotion_status, discount_layer,
    valid_for_member_tier, valid_from, valid_through, percent_off, amount_off_value,
    amount_off_currency, applies_to, change_set_id, version";

/// The status now, in SQL (same rule as `model::promotion_status`); `now` names the parameter.
pub(crate) fn status_sql(now: &str) -> String {
    format!(
        "(CASE WHEN promotion_status IN ('draft', 'paused', 'ended') THEN promotion_status
               WHEN {now} < valid_from THEN 'scheduled'
               WHEN valid_through IS NOT NULL AND {now} >= valid_through + interval '1 minute'
                 THEN 'ended'
               ELSE 'running' END)"
    )
}

pub(crate) async fn promotion(tx: &mut TxCx, id: &str) -> Result<Option<PromotionRow>, ToolError> {
    sqlx::query_as::<_, PromotionRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {PROMOTION_COLUMNS} FROM promotion.promotion WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Which promotions a list wants. Empty or `None` means no filter.
#[derive(Debug, Clone, Default)]
pub(crate) struct PromotionFilter {
    /// Status words, compared with the status now.
    pub statuses: Vec<String>,
    /// A tier id or `everyone`.
    pub tier: Option<String>,
    /// With `tier`: also the promotions for everyone (what reaches that tier).
    pub tier_reach: bool,
    pub layer: Option<String>,
    /// The promotion's period overlaps `from`–`through`.
    pub from: Option<DateTime<Utc>>,
    pub through: Option<DateTime<Utc>>,
    /// Words in the name.
    pub query: Option<String>,
}

/// The matching promotions (newest start first) and how many match in all. Without `page`,
/// every match.
pub(crate) async fn promotions(
    tx: &mut TxCx,
    f: &PromotionFilter,
    now: DateTime<Utc>,
    page: Option<Page>,
) -> Result<(i64, Vec<PromotionRow>), ToolError> {
    let filter = format!(
        "tenant_id = $1
         AND (cardinality($3::text[]) = 0 OR {} = ANY($3))
         AND ($4::text IS NULL
              OR ($4 = 'everyone' AND valid_for_member_tier IS NULL)
              OR valid_for_member_tier = $4
              OR ($5 AND valid_for_member_tier IS NULL))
         AND ($6::text IS NULL OR discount_layer = $6)
         AND ($7::timestamptz IS NULL OR valid_through IS NULL OR valid_through >= $7)
         AND ($8::timestamptz IS NULL OR valid_from <= $8)
         AND ($9::text IS NULL OR name ILIKE '%' || $9 || '%')",
        status_sql("$2")
    );
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM promotion.promotion WHERE {filter}"
    )))
    .bind(tx.tenant.as_str())
    .bind(now)
    .bind(&f.statuses)
    .bind(f.tier.as_deref())
    .bind(f.tier_reach)
    .bind(f.layer.as_deref())
    .bind(f.from)
    .bind(f.through)
    .bind(f.query.as_deref())
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let (limit, offset) = page.map_or((None, 0), |p| (Some(p.limit), p.offset));
    let rows = sqlx::query_as::<_, PromotionRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {PROMOTION_COLUMNS} FROM promotion.promotion WHERE {filter}
         ORDER BY valid_from DESC, created_at, id LIMIT $10 OFFSET $11"
    )))
    .bind(tx.tenant.as_str())
    .bind(now)
    .bind(&f.statuses)
    .bind(f.tier.as_deref())
    .bind(f.tier_reach)
    .bind(f.layer.as_deref())
    .bind(f.from)
    .bind(f.through)
    .bind(f.query.as_deref())
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

/// How many member or sale prices a promotion holds.
pub(crate) async fn price_count(tx: &mut TxCx, promotion_id: &str) -> Result<i64, ToolError> {
    sqlx::query_scalar(
        "SELECT count(*) FROM promotion.unit_price_specification
         WHERE tenant_id = $1 AND promotion_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(promotion_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A promotion as `set_sale_prices` creates it.
#[derive(Debug)]
pub(crate) struct NewPromotion<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub description: &'a str,
    pub status: &'a str,
    pub tier: Option<&'a str>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: DateTime<Utc>,
    pub percent_off: Decimal,
    pub applies_to: &'a Value,
}

/// Inserts an item-price promotion; nothing happens when it is already there.
pub(crate) async fn insert_promotion(tx: &mut TxCx, p: &NewPromotion<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO promotion.promotion (tenant_id, id, name, description, promotion_status,
           discount_layer, valid_for_member_tier, valid_from, valid_through, percent_off, applies_to)
         VALUES ($1, $2, $3, $4, $5, 'item_price', $6, $7, $8, $9, $10)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(p.id)
    .bind(p.name)
    .bind(p.description)
    .bind(p.status)
    .bind(p.tier)
    .bind(p.valid_from)
    .bind(p.valid_through)
    .bind(p.percent_off)
    .bind(p.applies_to)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Moves a promotion from one status to another; returns how many rows changed (0 or 1).
pub(crate) async fn set_status(
    tx: &mut TxCx,
    id: &str,
    from: &str,
    to: &str,
) -> Result<u64, ToolError> {
    Ok(sqlx::query(
        "UPDATE promotion.promotion SET promotion_status = $4, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND promotion_status = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(from)
    .bind(to)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected())
}

/// One sale price to insert.
#[derive(Debug, Clone)]
pub(crate) struct NewPrice {
    pub id: String,
    pub offer_id: String,
    pub price: Decimal,
    pub price_currency: String,
    pub percent_off: Decimal,
}

/// Inserts sale prices for one promotion; ids already there are skipped.
pub(crate) async fn insert_prices(
    tx: &mut TxCx,
    promotion_id: &str,
    tier: Option<&str>,
    valid_from: DateTime<Utc>,
    valid_through: DateTime<Utc>,
    prices: &[NewPrice],
) -> Result<(), ToolError> {
    let ids: Vec<&str> = prices.iter().map(|p| p.id.as_str()).collect();
    let offers: Vec<&str> = prices.iter().map(|p| p.offer_id.as_str()).collect();
    let amounts: Vec<Decimal> = prices.iter().map(|p| p.price).collect();
    let currencies: Vec<&str> = prices.iter().map(|p| p.price_currency.as_str()).collect();
    let percents: Vec<Decimal> = prices.iter().map(|p| p.percent_off).collect();
    sqlx::query(
        "INSERT INTO promotion.unit_price_specification (tenant_id, id, offer_id, promotion_id,
           price, price_currency, price_type, valid_for_member_tier, valid_from, valid_through,
           percent_off)
         SELECT $1, u.id, u.offer_id, $2, u.price, u.currency, 'sale_price', $3, $4, $5, u.percent
         FROM unnest($6::text[], $7::text[], $8::numeric[], $9::text[], $10::numeric[])
           AS u(id, offer_id, price, currency, percent)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(promotion_id)
    .bind(tier)
    .bind(valid_from)
    .bind(valid_through)
    .bind(&ids)
    .bind(&offers)
    .bind(&amounts)
    .bind(&currencies)
    .bind(&percents)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// A stored sale price, as read back.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct PriceRow {
    pub id: String,
    pub offer_id: String,
    pub price: Decimal,
    pub price_currency: String,
    pub percent_off: Option<Decimal>,
    pub valid_for_member_tier: Option<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: Option<DateTime<Utc>>,
}

pub(crate) async fn prices(tx: &mut TxCx, promotion_id: &str) -> Result<Vec<PriceRow>, ToolError> {
    sqlx::query_as::<_, PriceRow>(
        "SELECT id, offer_id, price, price_currency, percent_off, valid_for_member_tier,
                valid_from, valid_through
         FROM promotion.unit_price_specification
         WHERE tenant_id = $1 AND promotion_id = $2 ORDER BY id",
    )
    .bind(tx.tenant.as_str())
    .bind(promotion_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Deletes the given sale prices of a promotion.
pub(crate) async fn delete_prices(
    tx: &mut TxCx,
    promotion_id: &str,
    ids: &[String],
) -> Result<u64, ToolError> {
    Ok(sqlx::query(
        "DELETE FROM promotion.unit_price_specification
         WHERE tenant_id = $1 AND promotion_id = $2 AND id = ANY($3)",
    )
    .bind(tx.tenant.as_str())
    .bind(promotion_id)
    .bind(ids)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected())
}

/// A coupon with the period it follows (its own, else its promotion's).
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct CouponRow {
    pub id: String,
    pub name: Option<String>,
    pub discount_code: String,
    pub promotion_id: String,
    pub promotion_name: String,
    pub valid_for_member_tier: Option<String>,
    pub application: String,
    pub stacking: String,
    pub is_benefit_record_only: bool,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_through: Option<DateTime<Utc>>,
    pub version: i32,
}

const COUPON_SELECT: &str = "SELECT c.id, c.name, c.discount_code, c.promotion_id,
        p.name AS promotion_name, c.valid_for_member_tier, c.application, c.stacking,
        c.is_benefit_record_only, coalesce(c.valid_from, p.valid_from) AS valid_from,
        CASE WHEN c.valid_from IS NULL AND c.valid_through IS NULL THEN p.valid_through
             ELSE c.valid_through END AS valid_through,
        c.version
     FROM promotion.coupon c
     JOIN promotion.promotion p ON p.tenant_id = c.tenant_id AND p.id = c.promotion_id
     WHERE c.tenant_id = $1";

pub(crate) async fn coupon(tx: &mut TxCx, id: &str) -> Result<Option<CouponRow>, ToolError> {
    sqlx::query_as::<_, CouponRow>(sqlx::AssertSqlSafe(format!(
        "{COUPON_SELECT} AND c.id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The id of the coupon that already uses `code`, if any.
pub(crate) async fn coupon_with_code(
    tx: &mut TxCx,
    code: &str,
) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM promotion.coupon WHERE tenant_id = $1 AND discount_code = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(code)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Which coupons a list wants. `None` means no filter.
#[derive(Debug, Clone, Default)]
pub(crate) struct CouponFilter {
    pub code: Option<String>,
    pub promotion_id: Option<String>,
    pub application: Option<String>,
    /// A tier id, or `everyone` for coupons without a tier.
    pub tier: Option<String>,
}

pub(crate) async fn coupons(
    tx: &mut TxCx,
    f: &CouponFilter,
    page: Option<Page>,
) -> Result<(i64, Vec<CouponRow>), ToolError> {
    let filter = "AND ($2::text IS NULL OR c.discount_code = upper($2))
         AND ($3::text IS NULL OR c.promotion_id = $3)
         AND ($4::text IS NULL OR c.application = $4)
         AND ($5::text IS NULL OR c.valid_for_member_tier = $5
              OR ($5 = 'everyone' AND c.valid_for_member_tier IS NULL))";
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM promotion.coupon c WHERE c.tenant_id = $1 {filter}"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.code.as_deref())
    .bind(f.promotion_id.as_deref())
    .bind(f.application.as_deref())
    .bind(f.tier.as_deref())
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let (limit, offset) = page.map_or((None, 0), |p| (Some(p.limit), p.offset));
    let rows = sqlx::query_as::<_, CouponRow>(sqlx::AssertSqlSafe(format!(
        "{COUPON_SELECT} {filter} ORDER BY c.created_at DESC, c.id LIMIT $6 OFFSET $7"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.code.as_deref())
    .bind(f.promotion_id.as_deref())
    .bind(f.application.as_deref())
    .bind(f.tier.as_deref())
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

/// Ids of a promotion's coupons.
pub(crate) async fn coupon_ids_of(
    tx: &mut TxCx,
    promotion_id: &str,
) -> Result<Vec<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM promotion.coupon WHERE tenant_id = $1 AND promotion_id = $2 ORDER BY id",
    )
    .bind(tx.tenant.as_str())
    .bind(promotion_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A coupon as `create_coupon` creates it. Its period follows the promotion (NULL columns).
#[derive(Debug)]
pub(crate) struct NewCoupon<'a> {
    pub id: &'a str,
    pub code: &'a str,
    pub promotion_id: &'a str,
    pub tier: Option<&'a str>,
    pub application: &'a str,
    pub stacking: &'a str,
    pub records_benefit_only: bool,
}

pub(crate) async fn insert_coupon(tx: &mut TxCx, c: &NewCoupon<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO promotion.coupon (tenant_id, id, discount_code, promotion_id,
           valid_for_member_tier, application, stacking, is_benefit_record_only)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(c.id)
    .bind(c.code)
    .bind(c.promotion_id)
    .bind(c.tier)
    .bind(c.application)
    .bind(c.stacking)
    .bind(c.records_benefit_only)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Deletes a coupon nobody holds or used; returns whether it was deleted.
pub(crate) async fn delete_coupon_if_unheld(tx: &mut TxCx, id: &str) -> Result<bool, ToolError> {
    Ok(sqlx::query(
        "DELETE FROM promotion.coupon c WHERE c.tenant_id = $1 AND c.id = $2
           AND NOT EXISTS (SELECT 1 FROM promotion.member_coupon m
                           WHERE m.tenant_id = c.tenant_id AND m.coupon_id = c.id)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected()
        > 0)
}

/// Member coupons of a coupon, by status word.
pub(crate) async fn coupon_counts(
    tx: &mut TxCx,
    coupon_id: &str,
) -> Result<Vec<(String, i64)>, ToolError> {
    sqlx::query_as(
        "SELECT coupon_status, count(*) FROM promotion.member_coupon
         WHERE tenant_id = $1 AND coupon_id = $2 GROUP BY coupon_status",
    )
    .bind(tx.tenant.as_str())
    .bind(coupon_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A coupon a member holds, with its status now.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct MemberCouponRow {
    pub id: String,
    pub coupon_id: String,
    pub code: String,
    pub name: String,
    pub status: String,
    pub order_id: Option<String>,
    pub owned_from: DateTime<Utc>,
    pub owned_through: Option<DateTime<Utc>>,
    pub used_at: Option<DateTime<Utc>>,
}

/// A member coupon's status now: unused ones past `owned_through` read as expired.
const MEMBER_STATUS: &str = "(CASE WHEN m.coupon_status IN ('issued', 'available')
          AND m.owned_through IS NOT NULL AND $3 >= m.owned_through + interval '1 minute'
        THEN 'expired' ELSE m.coupon_status END)";

pub(crate) async fn member_coupons(
    tx: &mut TxCx,
    customer_id: &str,
    now: DateTime<Utc>,
    statuses: &[String],
    page: Page,
) -> Result<(i64, Vec<MemberCouponRow>), ToolError> {
    let filter = format!(
        "m.tenant_id = $1 AND m.customer_id = $2
         AND (cardinality($4::text[]) = 0 OR {MEMBER_STATUS} = ANY($4))"
    );
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM promotion.member_coupon m WHERE {filter}"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(now)
    .bind(statuses)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, MemberCouponRow>(sqlx::AssertSqlSafe(format!(
        "SELECT m.id, m.coupon_id, c.discount_code AS code,
                coalesce(c.name, p.name) AS name, {MEMBER_STATUS} AS status, m.order_id,
                m.owned_from, m.owned_through, m.used_at
         FROM promotion.member_coupon m
         JOIN promotion.coupon c ON c.tenant_id = m.tenant_id AND c.id = m.coupon_id
         JOIN promotion.promotion p ON p.tenant_id = c.tenant_id AND p.id = c.promotion_id
         WHERE {filter}
         ORDER BY m.owned_from DESC, m.id LIMIT $5 OFFSET $6"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(now)
    .bind(statuses)
    .bind(page.limit)
    .bind(page.offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

/// For each of `customer_ids`, how many unused (issued or available) and how many not revoked
/// copies of the coupon they hold.
pub(crate) async fn holdings(
    tx: &mut TxCx,
    coupon_id: &str,
    customer_ids: &[String],
) -> Result<Vec<(String, i64, i64)>, ToolError> {
    sqlx::query_as(
        "SELECT customer_id,
                count(*) FILTER (WHERE coupon_status IN ('issued', 'available')),
                count(*) FILTER (WHERE coupon_status <> 'revoked')
         FROM promotion.member_coupon
         WHERE tenant_id = $1 AND coupon_id = $2 AND customer_id = ANY($3)
         GROUP BY customer_id",
    )
    .bind(tx.tenant.as_str())
    .bind(coupon_id)
    .bind(customer_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Copies of the coupon given out and not revoked (the pool used so far).
pub(crate) async fn issued_count(tx: &mut TxCx, coupon_id: &str) -> Result<i64, ToolError> {
    sqlx::query_scalar(
        "SELECT count(*) FROM promotion.member_coupon
         WHERE tenant_id = $1 AND coupon_id = $2 AND coupon_status <> 'revoked'",
    )
    .bind(tx.tenant.as_str())
    .bind(coupon_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Gives the coupon to members: `(member coupon id, customer id)` pairs. Ids already there
/// are skipped.
pub(crate) async fn insert_member_coupons(
    tx: &mut TxCx,
    coupon_id: &str,
    holders: &[(String, String)],
    owned_from: DateTime<Utc>,
    owned_through: Option<DateTime<Utc>>,
) -> Result<(), ToolError> {
    let ids: Vec<&str> = holders.iter().map(|(id, _)| id.as_str()).collect();
    let customers: Vec<&str> = holders.iter().map(|(_, c)| c.as_str()).collect();
    sqlx::query(
        "INSERT INTO promotion.member_coupon (tenant_id, id, coupon_id, customer_id,
           coupon_status, owned_from, owned_through)
         SELECT $1, u.id, $2, u.customer_id, 'available', $3, $4
         FROM unnest($5::text[], $6::text[]) AS u(id, customer_id)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(coupon_id)
    .bind(owned_from)
    .bind(owned_through)
    .bind(&ids)
    .bind(&customers)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Status of each of the given member coupons.
pub(crate) async fn member_coupon_states(
    tx: &mut TxCx,
    ids: &[String],
) -> Result<Vec<(String, String)>, ToolError> {
    sqlx::query_as(
        "SELECT id, coupon_status FROM promotion.member_coupon
         WHERE tenant_id = $1 AND id = ANY($2)",
    )
    .bind(tx.tenant.as_str())
    .bind(ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Revokes the unused ones among `ids` (or every unused copy of the coupon when `ids` is
/// `None`); used coupons stay as they are.
pub(crate) async fn revoke_unused(
    tx: &mut TxCx,
    coupon_id: &str,
    ids: Option<&[String]>,
) -> Result<u64, ToolError> {
    Ok(sqlx::query(
        "UPDATE promotion.member_coupon SET coupon_status = 'revoked', updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND coupon_id = $2 AND coupon_status IN ('issued', 'available')
           AND ($3::text[] IS NULL OR id = ANY($3))",
    )
    .bind(tx.tenant.as_str())
    .bind(coupon_id)
    .bind(ids)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected())
}

/// What a promotion holds beyond [`PromotionRow`]: its type and terms (`create_promotion`), the
/// minimum subtotal, and the channels.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct PromotionExtras {
    pub promotion_type: Option<String>,
    pub promotion_terms: Value,
    pub sales_channels: Option<Vec<String>>,
    pub eligible_transaction_volume_value: Option<Decimal>,
    pub eligible_transaction_volume_currency: Option<String>,
}

pub(crate) async fn promotion_extras(
    tx: &mut TxCx,
    id: &str,
) -> Result<Option<PromotionExtras>, ToolError> {
    sqlx::query_as::<_, PromotionExtras>(
        "SELECT promotion_type, promotion_terms, sales_channels,
                eligible_transaction_volume_value,
                eligible_transaction_volume_currency::text AS eligible_transaction_volume_currency
         FROM promotion.promotion WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A promotion as `create_promotion` creates it.
#[derive(Debug)]
pub(crate) struct NewTypedPromotion<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub description: &'a str,
    pub status: &'a str,
    pub promotion_type: &'a str,
    pub discount_layer: &'a str,
    pub tier: Option<&'a str>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: DateTime<Utc>,
    pub amount_off: Option<(Decimal, &'a str)>,
    pub min_subtotal: Option<(Decimal, &'a str)>,
    pub applies_to: &'a Value,
    pub terms: &'a Value,
    pub channels: Option<&'a [String]>,
    pub change_set_id: Option<&'a str>,
}

/// Inserts a promotion of a type; nothing happens when it is already there.
pub(crate) async fn insert_typed_promotion(
    tx: &mut TxCx,
    p: &NewTypedPromotion<'_>,
) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO promotion.promotion (tenant_id, id, name, description, promotion_status,
           discount_layer, valid_for_member_tier, valid_from, valid_through, amount_off_value,
           amount_off_currency, eligible_transaction_volume_value,
           eligible_transaction_volume_currency, applies_to, change_set_id, promotion_type,
           promotion_terms, sales_channels)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(p.id)
    .bind(p.name)
    .bind(p.description)
    .bind(p.status)
    .bind(p.discount_layer)
    .bind(p.tier)
    .bind(p.valid_from)
    .bind(p.valid_through)
    .bind(p.amount_off.map(|a| a.0))
    .bind(p.amount_off.map(|a| a.1))
    .bind(p.min_subtotal.map(|a| a.0))
    .bind(p.min_subtotal.map(|a| a.1))
    .bind(p.applies_to)
    .bind(p.change_set_id)
    .bind(p.promotion_type)
    .bind(p.terms)
    .bind(p.channels)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// The fields `update_promotion` changes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct PromotionFields {
    pub name: String,
    pub valid_from: DateTime<Utc>,
    pub valid_through: Option<DateTime<Utc>>,
    pub applies_to: Value,
}

/// Sets name, period and scope; the sale prices of the promotion follow its period. Returns
/// whether the promotion was there.
pub(crate) async fn set_promotion_fields(
    tx: &mut TxCx,
    id: &str,
    f: &PromotionFields,
) -> Result<bool, ToolError> {
    let done = sqlx::query(
        "UPDATE promotion.promotion SET name = $3, valid_from = $4, valid_through = $5,
           applies_to = $6, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(&f.name)
    .bind(f.valid_from)
    .bind(f.valid_through)
    .bind(&f.applies_to)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected();
    sqlx::query(
        "UPDATE promotion.unit_price_specification SET valid_from = $3, valid_through = $4,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND promotion_id = $2
           AND (valid_from <> $3 OR valid_through IS DISTINCT FROM $4)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(f.valid_from)
    .bind(f.valid_through)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(done == 1)
}

/// The store's next id with `prefix` (`lyt`, `evt`…): a lower-case ULID suffix.
pub(crate) fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", acom_core::id::new_suffix())
}
