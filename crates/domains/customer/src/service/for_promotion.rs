//! What the promotion domain reads from customers: the store's tiers with how many members each
//! has (who a sale or coupon reaches), and members with their tier (who may hold a coupon).
//! Plain reads over the caller's transaction.

/// A shopper's own tier (the promotions that reach them).
pub use super::for_catalog::tier_of;

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, NaiveDate, Utc};

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// A tier of the store's member program.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Tier {
    /// The store's word for the tier (`vip`), used as `valid_for_member_tier` everywhere.
    pub id: String,
    /// Name people read (`VIP`).
    pub name: String,
    /// Higher is better.
    pub rank: i32,
    /// Members in the tier now, without withdrawn or blocked ones (`list_tiers`).
    pub member_count: i64,
}

const TIER_QUERY: &str = "SELECT t.id, t.name, t.rank,
        (SELECT count(*) FROM customer.program_membership m
         JOIN customer.person p ON p.tenant_id = m.tenant_id AND p.id = m.customer_id
         WHERE m.tenant_id = t.tenant_id AND m.tier_id = t.id
           AND p.member_state NOT IN ('withdrawn', 'blocked')) AS member_count
     FROM customer.member_program_tier t
     WHERE t.tenant_id = $1";

/// Every tier, highest rank first.
pub async fn tiers(tx: &mut TxCx) -> Result<Vec<Tier>, ToolError> {
    sqlx::query_as::<_, Tier>(sqlx::AssertSqlSafe(format!(
        "{TIER_QUERY} ORDER BY t.rank DESC, t.id"
    )))
    .bind(tx.tenant.as_str())
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// One tier, or `None` when the store has no tier with that id.
pub async fn tier(tx: &mut TxCx, tier_id: &str) -> Result<Option<Tier>, ToolError> {
    sqlx::query_as::<_, Tier>(sqlx::AssertSqlSafe(format!("{TIER_QUERY} AND t.id = $2")))
        .bind(tx.tenant.as_str())
        .bind(tier_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)
}

/// A member and the tier they are in.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Member {
    /// `cus_…`.
    pub customer_id: String,
    pub name: String,
    /// `member_state` word.
    pub member_state: String,
    /// `None` when the person is in no member program.
    pub tier_id: Option<String>,
    /// Since when they are in that tier.
    pub tier_since: Option<DateTime<Utc>>,
    pub birth_date: Option<NaiveDate>,
}

const MEMBER_QUERY: &str = "SELECT p.id AS customer_id, p.name, p.member_state,
        m.tier_id, m.tier_since, p.birth_date
     FROM customer.person p
     LEFT JOIN LATERAL (
       SELECT tier_id, tier_since FROM customer.program_membership
       WHERE tenant_id = p.tenant_id AND customer_id = p.id
       ORDER BY member_program_id LIMIT 1) m ON true
     WHERE p.tenant_id = $1";

/// The members among `customer_ids` that exist, in no fixed order.
pub async fn members(tx: &mut TxCx, customer_ids: &[String]) -> Result<Vec<Member>, ToolError> {
    sqlx::query_as::<_, Member>(sqlx::AssertSqlSafe(format!(
        "{MEMBER_QUERY} AND p.id = ANY($2)"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Active members (in `tier_id`, or in any tier when `None`) whose birthday falls in `month`
/// (1–12): who a birthday coupon goes to this month.
pub async fn members_with_birthday_in(
    tx: &mut TxCx,
    tier_id: Option<&str>,
    month: u32,
) -> Result<Vec<Member>, ToolError> {
    sqlx::query_as::<_, Member>(sqlx::AssertSqlSafe(format!(
        "{MEMBER_QUERY} AND p.member_state = 'active' AND m.tier_id IS NOT NULL
           AND ($2::text IS NULL OR m.tier_id = $2)
           AND extract(month FROM p.birth_date) = $3
         ORDER BY p.id"
    )))
    .bind(tx.tenant.as_str())
    .bind(tier_id)
    .bind(i32::try_from(month).unwrap_or(0))
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Active members who entered `tier_id` (or any tier when `None`) at or after `since`: who a
/// tier-entry coupon goes to.
pub async fn members_entered_tier_since(
    tx: &mut TxCx,
    tier_id: Option<&str>,
    since: DateTime<Utc>,
) -> Result<Vec<Member>, ToolError> {
    sqlx::query_as::<_, Member>(sqlx::AssertSqlSafe(format!(
        "{MEMBER_QUERY} AND p.member_state = 'active' AND m.tier_id IS NOT NULL
           AND ($2::text IS NULL OR m.tier_id = $2) AND m.tier_since >= $3
         ORDER BY p.id"
    )))
    .bind(tx.tenant.as_str())
    .bind(tier_id)
    .bind(since)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Active members in any of `tier_ids`, or every active person when `tier_ids` is empty, by id:
/// who a segment can hold.
pub async fn active_members(tx: &mut TxCx, tier_ids: &[String]) -> Result<Vec<Member>, ToolError> {
    sqlx::query_as::<_, Member>(sqlx::AssertSqlSafe(format!(
        "{MEMBER_QUERY} AND p.member_state = 'active'
           AND (cardinality($2::text[]) = 0 OR m.tier_id = ANY($2))
         ORDER BY p.id"
    )))
    .bind(tx.tenant.as_str())
    .bind(tier_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Where each of `customer_ids` that has one gets parcels by default (`(customer, address id)`):
/// the primary shipping address, else the first shipping address saved. Who can be sent a
/// product reward.
pub async fn shipping_addresses(
    tx: &mut TxCx,
    customer_ids: &[String],
) -> Result<Vec<(String, String)>, ToolError> {
    sqlx::query_as(
        "SELECT DISTINCT ON (customer_id) customer_id, id FROM customer.postal_address
         WHERE tenant_id = $1 AND customer_id = ANY($2) AND 'shipping' = ANY(address_usage)
         ORDER BY customer_id, is_primary DESC, created_at, id",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The member type (`individual`, `business`) of each of `customer_ids` that exists: who an
/// event open to some member types lets in.
pub async fn member_types(
    tx: &mut TxCx,
    customer_ids: &[String],
) -> Result<Vec<(String, String)>, ToolError> {
    sqlx::query_as(
        "SELECT id, member_type FROM customer.person WHERE tenant_id = $1 AND id = ANY($2)",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}
