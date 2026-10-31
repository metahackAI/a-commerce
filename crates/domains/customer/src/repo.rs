//! SQL for the customer domain. Every query filters `tenant_id`; values are bind parameters.
//! Tables: `migrations/0003_customer.sql`, `0013_customer_wishlist.sql`, `0021_customer_members_and_service.sql`.

use acom_contract::ToolError;
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

use crate::access::db;

// ── people ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct PersonRow {
    pub id: String,
    pub name: String,
    pub member_state: String,
    pub member_type: String,
    pub sizes: Value,
    pub card_last4: Option<String>,
    pub version: i32,
}

const PERSON_COLUMNS: &str = "id, name, member_state, member_type, sizes, card_last4, version";

pub(crate) async fn person(tx: &mut TxCx, id: &str) -> Result<Option<PersonRow>, ToolError> {
    sqlx::query_as::<_, PersonRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {PERSON_COLUMNS} FROM customer.person WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Filters of `search_members`; `None` means any.
#[derive(Debug, Default)]
pub(crate) struct MemberFilter {
    pub query: Option<String>,
    pub member_type: Option<String>,
    pub member_state: Option<String>,
    pub tier_id: Option<String>,
    pub joined_from: Option<DateTime<Utc>>,
    pub joined_through: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct MemberRow {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub member_state: String,
    pub tier_id: Option<String>,
}

/// Members matching the filter, newest first, with the total.
pub(crate) async fn search_members(
    tx: &mut TxCx,
    f: &MemberFilter,
    limit: i64,
    offset: i64,
) -> Result<(i64, Vec<MemberRow>), ToolError> {
    // The tier is the first program's, like `service::for_sales::member_tier`.
    let from = "customer.person p
        LEFT JOIN LATERAL (SELECT m.tier_id FROM customer.program_membership m
                           WHERE m.tenant_id = p.tenant_id AND m.customer_id = p.id
                           ORDER BY m.member_program_id LIMIT 1) t ON true";
    let filter = "p.tenant_id = $1
        AND ($2::text IS NULL OR p.name ILIKE '%' || $2 || '%' OR p.email ILIKE '%' || $2 || '%'
             OR p.telephone ILIKE '%' || $2 || '%' OR p.id = $2)
        AND ($3::text IS NULL OR p.member_type = $3)
        AND ($4::text IS NULL OR p.member_state = $4)
        AND ($5::text IS NULL OR t.tier_id = $5)
        AND ($6::timestamptz IS NULL OR p.registered_at >= $6)
        AND ($7::timestamptz IS NULL OR p.registered_at <= $7)";
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {from} WHERE {filter}"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.query.as_deref())
    .bind(f.member_type.as_deref())
    .bind(f.member_state.as_deref())
    .bind(f.tier_id.as_deref())
    .bind(f.joined_from)
    .bind(f.joined_through)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, MemberRow>(sqlx::AssertSqlSafe(format!(
        "SELECT p.id, p.name, p.email, p.member_state, t.tier_id FROM {from} WHERE {filter}
         ORDER BY p.registered_at DESC, p.id LIMIT $8 OFFSET $9"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.query.as_deref())
    .bind(f.member_type.as_deref())
    .bind(f.member_state.as_deref())
    .bind(f.tier_id.as_deref())
    .bind(f.joined_from)
    .bind(f.joined_through)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

// ── programs, tiers, memberships ────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ProgramRow {
    pub id: String,
    pub name: String,
    pub evaluation: Value,
    pub movement: Value,
    pub version: i32,
}

pub(crate) async fn program(tx: &mut TxCx, id: &str) -> Result<Option<ProgramRow>, ToolError> {
    sqlx::query_as::<_, ProgramRow>(
        "SELECT id, name, evaluation, movement, version FROM customer.member_program
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct TierRow {
    pub id: String,
    pub member_program_id: String,
    pub name: String,
    pub rank: i32,
    pub has_tier_requirement: Value,
    pub has_tier_benefit: Value,
    pub membership_points_earned_value: Option<Decimal>,
    pub membership_points_earned_unit_text: Option<String>,
}

const TIER_COLUMNS: &str = "id, member_program_id, name, rank, has_tier_requirement,
    has_tier_benefit, membership_points_earned_value, membership_points_earned_unit_text";

/// Tiers of one program (or every program), highest rank first.
pub(crate) async fn tiers(
    tx: &mut TxCx,
    program_id: Option<&str>,
) -> Result<Vec<TierRow>, ToolError> {
    sqlx::query_as::<_, TierRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {TIER_COLUMNS} FROM customer.member_program_tier
         WHERE tenant_id = $1 AND ($2::text IS NULL OR member_program_id = $2)
         ORDER BY rank DESC, member_program_id, id"
    )))
    .bind(tx.tenant.as_str())
    .bind(program_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

pub(crate) async fn tier(tx: &mut TxCx, id: &str) -> Result<Option<TierRow>, ToolError> {
    sqlx::query_as::<_, TierRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {TIER_COLUMNS} FROM customer.member_program_tier WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Members per tier, leaving out blocked and withdrawn people.
pub(crate) async fn member_counts(
    tx: &mut TxCx,
    program_id: Option<&str>,
) -> Result<Vec<(String, i64)>, ToolError> {
    sqlx::query_as(
        "SELECT m.tier_id, count(*) FROM customer.program_membership m
         JOIN customer.person p ON p.tenant_id = m.tenant_id AND p.id = m.customer_id
         WHERE m.tenant_id = $1 AND ($2::text IS NULL OR m.member_program_id = $2)
           AND p.member_state NOT IN ('blocked', 'withdrawn')
         GROUP BY m.tier_id",
    )
    .bind(tx.tenant.as_str())
    .bind(program_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct MembershipRow {
    pub member_program_id: String,
    pub tier_id: String,
    pub membership_number: String,
    pub tier_since: DateTime<Utc>,
    pub tier_override: Option<Value>,
}

/// A person's memberships, first program first.
pub(crate) async fn memberships(
    tx: &mut TxCx,
    customer_id: &str,
) -> Result<Vec<MembershipRow>, ToolError> {
    sqlx::query_as::<_, MembershipRow>(
        "SELECT member_program_id, tier_id, membership_number, tier_since, tier_override
         FROM customer.program_membership WHERE tenant_id = $1 AND customer_id = $2
         ORDER BY member_program_id",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

// ── addresses ───────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct AddressRow {
    pub id: String,
    pub customer_id: String,
    pub label: Option<String>,
    pub name: Option<String>,
    pub street_address: String,
    pub address_locality: String,
    pub address_region: Option<String>,
    pub postal_code: String,
    pub address_country: String,
    pub telephone: Option<String>,
    pub address_usage: Vec<String>,
    pub is_primary: bool,
    pub version: i32,
}

const ADDRESS_COLUMNS: &str = "id, customer_id, label, name, street_address, address_locality,
    address_region, postal_code, trim(address_country) AS address_country, telephone,
    address_usage, is_primary, version";

/// The person's addresses, primary first, then oldest first; with the total.
pub(crate) async fn addresses(
    tx: &mut TxCx,
    customer_id: &str,
    limit: i64,
    offset: i64,
) -> Result<(i64, Vec<AddressRow>), ToolError> {
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM customer.postal_address WHERE tenant_id = $1 AND customer_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, AddressRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ADDRESS_COLUMNS} FROM customer.postal_address
         WHERE tenant_id = $1 AND customer_id = $2
         ORDER BY is_primary DESC, created_at, id LIMIT $3 OFFSET $4"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

pub(crate) async fn address(
    tx: &mut TxCx,
    customer_id: &str,
    id: &str,
) -> Result<Option<AddressRow>, ToolError> {
    sqlx::query_as::<_, AddressRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ADDRESS_COLUMNS} FROM customer.postal_address
         WHERE tenant_id = $1 AND customer_id = $2 AND id = $3"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The person's primary address, if one is marked.
pub(crate) async fn primary_address(
    tx: &mut TxCx,
    customer_id: &str,
) -> Result<Option<AddressRow>, ToolError> {
    sqlx::query_as::<_, AddressRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ADDRESS_COLUMNS} FROM customer.postal_address
         WHERE tenant_id = $1 AND customer_id = $2 AND is_primary
         ORDER BY created_at, id LIMIT 1"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The fields an address has, without its id and owner.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct AddressFields {
    pub label: Option<String>,
    pub name: Option<String>,
    pub street_address: String,
    pub address_locality: String,
    pub address_region: Option<String>,
    pub postal_code: String,
    pub address_country: String,
    pub telephone: Option<String>,
    pub usage: Vec<String>,
}

impl From<&AddressRow> for AddressFields {
    fn from(r: &AddressRow) -> Self {
        Self {
            label: r.label.clone(),
            name: r.name.clone(),
            street_address: r.street_address.clone(),
            address_locality: r.address_locality.clone(),
            address_region: r.address_region.clone(),
            postal_code: r.postal_code.clone(),
            address_country: r.address_country.clone(),
            telephone: r.telephone.clone(),
            usage: r.address_usage.clone(),
        }
    }
}

/// Adds an address; nothing happens when the id already exists (a repeated run).
pub(crate) async fn insert_address(
    tx: &mut TxCx,
    id: &str,
    customer_id: &str,
    a: &AddressFields,
    is_primary: bool,
) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO customer.postal_address (tenant_id, id, customer_id, label, name,
           street_address, address_locality, address_region, postal_code, address_country,
           telephone, address_usage, is_primary)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(customer_id)
    .bind(a.label.as_deref())
    .bind(a.name.as_deref())
    .bind(&a.street_address)
    .bind(&a.address_locality)
    .bind(a.address_region.as_deref())
    .bind(&a.postal_code)
    .bind(&a.address_country)
    .bind(a.telephone.as_deref())
    .bind(&a.usage)
    .bind(is_primary)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Writes every field of an address (only when something differs, so a repeated run is a no-op).
pub(crate) async fn write_address(
    tx: &mut TxCx,
    customer_id: &str,
    id: &str,
    a: &AddressFields,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE customer.postal_address SET label = $4, name = $5, street_address = $6,
           address_locality = $7, address_region = $8, postal_code = $9, address_country = $10,
           telephone = $11, address_usage = $12, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND customer_id = $2 AND id = $3
           AND (label, name, street_address, address_locality, address_region, postal_code,
                trim(address_country), telephone, address_usage)
               IS DISTINCT FROM ($4, $5, $6, $7, $8, $9, $10, $11, $12)",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(id)
    .bind(a.label.as_deref())
    .bind(a.name.as_deref())
    .bind(&a.street_address)
    .bind(&a.address_locality)
    .bind(a.address_region.as_deref())
    .bind(&a.postal_code)
    .bind(&a.address_country)
    .bind(a.telephone.as_deref())
    .bind(&a.usage)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Marks or unmarks one address as primary (no-op when it already has that value).
pub(crate) async fn set_primary(
    tx: &mut TxCx,
    customer_id: &str,
    id: &str,
    is_primary: bool,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE customer.postal_address SET is_primary = $4, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND customer_id = $2 AND id = $3 AND is_primary <> $4",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(id)
    .bind(is_primary)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

pub(crate) async fn delete_address(
    tx: &mut TxCx,
    customer_id: &str,
    id: &str,
) -> Result<(), ToolError> {
    sqlx::query(
        "DELETE FROM customer.postal_address WHERE tenant_id = $1 AND customer_id = $2 AND id = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

// ── preferences (memories) ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct PreferenceRow {
    pub id: String,
    pub customer_id: String,
    pub preference_key: String,
    pub value: String,
    pub description: String,
    pub source: String,
    pub conversation_id: Option<String>,
    pub learned_at: DateTime<Utc>,
    pub version: i32,
}

const PREFERENCE_COLUMNS: &str = "id, customer_id, preference_key, value, description, source,
    conversation_id, learned_at, version";

/// A person's memories, newest first, optionally of one key; with the total.
pub(crate) async fn preferences(
    tx: &mut TxCx,
    customer_id: &str,
    key: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(i64, Vec<PreferenceRow>), ToolError> {
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM customer.preference
         WHERE tenant_id = $1 AND customer_id = $2 AND ($3::text IS NULL OR preference_key = $3)",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(key)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, PreferenceRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {PREFERENCE_COLUMNS} FROM customer.preference
         WHERE tenant_id = $1 AND customer_id = $2 AND ($3::text IS NULL OR preference_key = $3)
         ORDER BY learned_at DESC, id LIMIT $4 OFFSET $5"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(key)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

/// Every memory of a person, oldest first (for the profile).
pub(crate) async fn all_preferences(
    tx: &mut TxCx,
    customer_id: &str,
) -> Result<Vec<PreferenceRow>, ToolError> {
    sqlx::query_as::<_, PreferenceRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {PREFERENCE_COLUMNS} FROM customer.preference
         WHERE tenant_id = $1 AND customer_id = $2 ORDER BY learned_at, id"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// One memory by id, whoever it belongs to.
pub(crate) async fn preference(
    tx: &mut TxCx,
    id: &str,
) -> Result<Option<PreferenceRow>, ToolError> {
    sqlx::query_as::<_, PreferenceRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {PREFERENCE_COLUMNS} FROM customer.preference WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Remembers a preference. The same key and value twice stay one memory: the existing id is
/// returned.
pub(crate) async fn insert_preference(
    tx: &mut TxCx,
    p: &PreferenceRow,
) -> Result<String, ToolError> {
    sqlx::query(
        "INSERT INTO customer.preference (tenant_id, id, customer_id, preference_key, value,
           description, source, conversation_id, learned_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)
         ON CONFLICT DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(&p.id)
    .bind(&p.customer_id)
    .bind(&p.preference_key)
    .bind(&p.value)
    .bind(&p.description)
    .bind(&p.source)
    .bind(p.conversation_id.as_deref())
    .bind(p.learned_at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    sqlx::query_scalar(
        "SELECT id FROM customer.preference
         WHERE tenant_id = $1 AND customer_id = $2 AND preference_key = $3 AND value = $4",
    )
    .bind(tx.tenant.as_str())
    .bind(&p.customer_id)
    .bind(&p.preference_key)
    .bind(&p.value)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Forgets one of a person's memories. `false` when it was not there.
pub(crate) async fn delete_preference(
    tx: &mut TxCx,
    customer_id: &str,
    id: &str,
) -> Result<bool, ToolError> {
    let done = sqlx::query(
        "DELETE FROM customer.preference WHERE tenant_id = $1 AND customer_id = $2 AND id = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(done.rows_affected() > 0)
}

// ── wishlist ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct WishlistRow {
    pub product_group_id: String,
    pub created_at: DateTime<Utc>,
}

/// A person's wishlist, newest first, with the total.
pub(crate) async fn wishlist(
    tx: &mut TxCx,
    customer_id: &str,
    limit: i64,
    offset: i64,
) -> Result<(i64, Vec<WishlistRow>), ToolError> {
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM customer.wishlist_item WHERE tenant_id = $1 AND customer_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, WishlistRow>(
        "SELECT product_group_id, created_at FROM customer.wishlist_item
         WHERE tenant_id = $1 AND customer_id = $2
         ORDER BY created_at DESC, product_group_id LIMIT $3 OFFSET $4",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

pub(crate) async fn wishlist_item(
    tx: &mut TxCx,
    customer_id: &str,
    product_id: &str,
) -> Result<Option<WishlistRow>, ToolError> {
    sqlx::query_as::<_, WishlistRow>(
        "SELECT product_group_id, created_at FROM customer.wishlist_item
         WHERE tenant_id = $1 AND customer_id = $2 AND product_group_id = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(product_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Keeps a product; already kept stays as it is.
pub(crate) async fn insert_wishlist_item(
    tx: &mut TxCx,
    customer_id: &str,
    product_id: &str,
    added_at: DateTime<Utc>,
) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO customer.wishlist_item (tenant_id, customer_id, product_group_id, created_at)
         VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(product_id)
    .bind(added_at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

pub(crate) async fn delete_wishlist_item(
    tx: &mut TxCx,
    customer_id: &str,
    product_id: &str,
) -> Result<(), ToolError> {
    sqlx::query(
        "DELETE FROM customer.wishlist_item
         WHERE tenant_id = $1 AND customer_id = $2 AND product_group_id = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(product_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

// ── accounts (sign-in and account facts of a person, 0021) ─────────────────────────────

/// Everything about a person's account the person writes need. Password hashes stay inside
/// the crate and never go to an output or a log.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct AccountRow {
    pub id: String,
    pub name: String,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub email: Option<String>,
    pub telephone: Option<String>,
    pub login_id: Option<String>,
    pub member_state: String,
    pub member_type: String,
    pub legal_name: Option<String>,
    pub tax_id: Option<String>,
    pub business_verification: String,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub telephone_verified_at: Option<DateTime<Utc>>,
    pub password_hash: Option<String>,
    pub password_history: Vec<String>,
    pub failed_password_attempts: i32,
    pub first_failed_password_at: Option<DateTime<Utc>>,
    pub locked_until: Option<DateTime<Utc>>,
    pub block_reason: Option<String>,
    pub blocked_until: Option<DateTime<Utc>>,
    pub withdrawn_at: Option<DateTime<Utc>>,
    pub card_last4: Option<String>,
    pub registered_at: DateTime<Utc>,
    pub version: i32,
}

const ACCOUNT_COLUMNS: &str = "id, name, given_name, family_name, email, telephone, login_id,
    member_state, member_type, legal_name, tax_id, business_verification, email_verified_at,
    telephone_verified_at, password_hash, password_history, failed_password_attempts,
    first_failed_password_at, locked_until, block_reason, blocked_until, withdrawn_at,
    card_last4, registered_at, version";

pub(crate) async fn account(tx: &mut TxCx, id: &str) -> Result<Option<AccountRow>, ToolError> {
    sqlx::query_as::<_, AccountRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ACCOUNT_COLUMNS} FROM customer.person WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The account (not withdrawn) whose email, sign-in id or phone is `normalized`
/// (`secret::normalize`).
pub(crate) async fn account_by_identifier(
    tx: &mut TxCx,
    normalized: &str,
) -> Result<Option<AccountRow>, ToolError> {
    sqlx::query_as::<_, AccountRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ACCOUNT_COLUMNS} FROM customer.person
         WHERE tenant_id = $1 AND member_state <> 'withdrawn'
           AND (lower(email) = $2 OR lower(login_id) = $2
                OR regexp_replace(telephone, '[^0-9+]', '', 'g') = $2)
         ORDER BY registered_at, id LIMIT 1"
    )))
    .bind(tx.tenant.as_str())
    .bind(normalized)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The contact fields a member can change.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ContactFields {
    pub name: String,
    pub email: Option<String>,
    pub telephone: Option<String>,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub telephone_verified_at: Option<DateTime<Utc>>,
}

impl From<&AccountRow> for ContactFields {
    fn from(a: &AccountRow) -> Self {
        Self {
            name: a.name.clone(),
            email: a.email.clone(),
            telephone: a.telephone.clone(),
            email_verified_at: a.email_verified_at,
            telephone_verified_at: a.telephone_verified_at,
        }
    }
}

pub(crate) async fn write_contact(
    tx: &mut TxCx,
    id: &str,
    c: &ContactFields,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE customer.person SET name = $3, email = $4, telephone = $5,
           email_verified_at = $6, telephone_verified_at = $7, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2
           AND (name, email, telephone, email_verified_at, telephone_verified_at)
               IS DISTINCT FROM ($3, $4, $5, $6, $7)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(&c.name)
    .bind(c.email.as_deref())
    .bind(c.telephone.as_deref())
    .bind(c.email_verified_at)
    .bind(c.telephone_verified_at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// A member's state and block fields.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct StateFields {
    pub member_state: String,
    pub block_reason: Option<String>,
    pub blocked_until: Option<DateTime<Utc>>,
}

impl From<&AccountRow> for StateFields {
    fn from(a: &AccountRow) -> Self {
        Self {
            member_state: a.member_state.clone(),
            block_reason: a.block_reason.clone(),
            blocked_until: a.blocked_until,
        }
    }
}

pub(crate) async fn write_state(tx: &mut TxCx, id: &str, s: &StateFields) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE customer.person SET member_state = $3, block_reason = $4, blocked_until = $5,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2
           AND (member_state, block_reason, blocked_until) IS DISTINCT FROM ($3, $4, $5)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(&s.member_state)
    .bind(s.block_reason.as_deref())
    .bind(s.blocked_until)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// One tier change to add to the history (append only).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct TierChange {
    pub customer_id: String,
    pub program_id: String,
    pub from_tier_id: Option<String>,
    pub to_tier_id: Option<String>,
    /// A word of `tier_change_cause`.
    pub cause: String,
    pub evaluation_id: Option<String>,
    pub snapshot: Option<Value>,
    pub agent_id: String,
    pub reason: Option<String>,
    pub expires: Option<DateTime<Utc>>,
}

/// Adds one tier change at the end of the member's history in that program.
pub(crate) async fn add_history(tx: &mut TxCx, c: &TierChange) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO customer.program_membership_event (tenant_id, customer_id, member_program_id,
           position, from_tier_id, to_tier_id, cause, evaluation_id, snapshot, agent_id, reason,
           expires, changed_at)
         SELECT $1, $2, $3, COALESCE(max(position), 0) + 1, $4, $5, $6, $7, $8, $9, $10, $11, $12
         FROM customer.program_membership_event
         WHERE tenant_id = $1 AND customer_id = $2 AND member_program_id = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(&c.customer_id)
    .bind(&c.program_id)
    .bind(c.from_tier_id.as_deref())
    .bind(c.to_tier_id.as_deref())
    .bind(&c.cause)
    .bind(c.evaluation_id.as_deref())
    .bind(c.snapshot.as_ref())
    .bind(&c.agent_id)
    .bind(c.reason.as_deref())
    .bind(c.expires)
    .bind(tx.clock.now())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Sets a member's tier in a program (`tier_override` as given; NULL clears it).
pub(crate) async fn set_membership_tier(
    tx: &mut TxCx,
    customer_id: &str,
    program_id: &str,
    tier_id: &str,
    since: DateTime<Utc>,
    tier_override: Option<&Value>,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE customer.program_membership SET tier_id = $4, tier_since = $5, tier_override = $6,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND customer_id = $2 AND member_program_id = $3
           AND (tier_id, tier_since, tier_override) IS DISTINCT FROM ($4, $5, $6)",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(program_id)
    .bind(tier_id)
    .bind(since)
    .bind(tier_override)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}
