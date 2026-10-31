//! SQL on the `partner` schema. Every function runs in the caller's store-scoped transaction;
//! other domains' data comes through their services (`activity`), never from here.

use acom_contract::ToolError;
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

use crate::db;

// ── sellers ──────────────────────────────────────────────────────────────────────────────

/// One `partner.seller` row.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct SellerRow {
    pub id: String,
    pub legal_name: String,
    pub tax_id: String,
    pub seller_status: String,
    pub seller_type: String,
    pub address: Value,
    pub contact_point: Value,
    pub business_category: Vec<String>,
    pub documents: Value,
    pub organization_id: Option<String>,
    pub applied_at: DateTime<Utc>,
    pub version: i32,
}

const SELLER_COLUMNS: &str = "s.id, s.legal_name, s.tax_id, s.seller_status, s.seller_type,
    s.address, s.contact_point, s.business_category, s.documents, s.organization_id,
    s.applied_at, s.version";

pub async fn seller(tx: &mut TxCx, id: &str) -> Result<Option<SellerRow>, ToolError> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {SELLER_COLUMNS} FROM partner.seller s WHERE s.tenant_id = $1 AND s.id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The seller whose vendor organization this is.
pub async fn seller_of_organization(
    tx: &mut TxCx,
    organization_id: &str,
) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM partner.seller WHERE tenant_id = $1 AND organization_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(organization_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Sellers whose tax id has these letters and digits (`model::normalized_tax_id`).
pub async fn sellers_with_tax_id(
    tx: &mut TxCx,
    normalized: &str,
) -> Result<Vec<(String, String)>, ToolError> {
    sqlx::query_as(
        "SELECT id, legal_name FROM partner.seller
         WHERE tenant_id = $1 AND upper(regexp_replace(tax_id, '[^0-9A-Za-z]', '', 'g')) = $2
         ORDER BY id",
    )
    .bind(tx.tenant.as_str())
    .bind(normalized)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Whether a seller id is taken.
pub async fn seller_exists(tx: &mut TxCx, id: &str) -> Result<bool, ToolError> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM partner.seller WHERE tenant_id = $1 AND id = $2)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// How `search_sellers` orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SellerOrder {
    Newest,
    Name,
    Grade,
}

/// What `search_sellers` filters on.
#[derive(Debug, Clone, Default)]
pub struct SellerFilter {
    /// Part of the legal name, any case.
    pub name_part: Option<String>,
    /// Part of the tax id's letters and digits.
    pub tax_id_part: Option<String>,
    pub status: Option<String>,
    pub seller_type: Option<String>,
    pub grade_level: Option<String>,
    /// Only this seller (a seller's own staff see only their seller).
    pub only: Option<String>,
}

/// A found seller with its grade now.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct FoundSeller {
    #[sqlx(flatten)]
    pub seller: SellerRow,
    pub grade_level: Option<String>,
}

/// The grade in force at `$3`: the latest grade row whose period covers it.
const GRADE_NOW: &str = "(SELECT g.grade_level FROM partner.seller_grade g
    WHERE g.tenant_id = s.tenant_id AND g.seller_id = s.id
      AND g.valid_from <= $3 AND g.valid_through >= $3
    ORDER BY g.position DESC LIMIT 1)";

pub async fn search_sellers(
    tx: &mut TxCx,
    f: &SellerFilter,
    order: SellerOrder,
    now: DateTime<Utc>,
    limit: i64,
    offset: i64,
) -> Result<(i64, Vec<FoundSeller>), ToolError> {
    let filter = format!(
        "s.tenant_id = $1
         AND ($2::text IS NULL OR s.id = $2)
         AND ($4::text IS NULL AND $5::text IS NULL
              OR s.legal_name ILIKE '%' || $4 || '%'
              OR ($5::text IS NOT NULL
                  AND upper(regexp_replace(s.tax_id, '[^0-9A-Za-z]', '', 'g')) LIKE '%' || $5 || '%'))
         AND ($6::text IS NULL OR s.seller_status = $6)
         AND ($7::text IS NULL OR s.seller_type = $7)
         AND ($8::text IS NULL OR {GRADE_NOW} = $8)"
    );
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM partner.seller s WHERE {filter}"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.only.as_deref())
    .bind(now)
    .bind(f.name_part.as_deref())
    .bind(f.tax_id_part.as_deref())
    .bind(f.status.as_deref())
    .bind(f.seller_type.as_deref())
    .bind(f.grade_level.as_deref())
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let order_by = match order {
        SellerOrder::Newest => "x.applied_at DESC, x.id",
        SellerOrder::Name => "lower(x.legal_name), x.id",
        SellerOrder::Grade => {
            "array_position(ARRAY['platinum','gold','silver','bronze','watch'], x.grade_level) \
             NULLS LAST, lower(x.legal_name), x.id"
        }
    };
    let rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT * FROM (SELECT {SELLER_COLUMNS}, {GRADE_NOW} AS grade_level
                        FROM partner.seller s WHERE {filter}) x
         ORDER BY {order_by} LIMIT $9 OFFSET $10"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.only.as_deref())
    .bind(now)
    .bind(f.name_part.as_deref())
    .bind(f.tax_id_part.as_deref())
    .bind(f.status.as_deref())
    .bind(f.seller_type.as_deref())
    .bind(f.grade_level.as_deref())
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

/// A new application.
#[derive(Debug, Clone)]
pub struct NewSeller<'a> {
    pub id: &'a str,
    pub legal_name: &'a str,
    pub tax_id: &'a str,
    pub seller_status: &'a str,
    pub seller_type: &'a str,
    pub address: &'a Value,
    pub contact_point: &'a Value,
    pub business_category: &'a [String],
    pub documents: &'a Value,
    pub applied_at: DateTime<Utc>,
}

pub async fn insert_seller(tx: &mut TxCx, s: &NewSeller<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO partner.seller (tenant_id, id, legal_name, tax_id, seller_status, seller_type,
           address, contact_point, business_category, documents, applied_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(tx.tenant.as_str())
    .bind(s.id)
    .bind(s.legal_name)
    .bind(s.tax_id)
    .bind(s.seller_status)
    .bind(s.seller_type)
    .bind(s.address)
    .bind(s.contact_point)
    .bind(s.business_category)
    .bind(s.documents)
    .bind(s.applied_at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Removes an application that never went further (undo of `onboard_seller`): its events and
/// the seller. Returns whether it was removed; a seller with a contract, a grade or products
/// stays.
pub async fn delete_application(tx: &mut TxCx, id: &str) -> Result<bool, ToolError> {
    let busy: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM partner.seller_contract WHERE tenant_id = $1 AND seller_id = $2)
             OR EXISTS (SELECT 1 FROM partner.seller_grade WHERE tenant_id = $1 AND seller_id = $2)
             OR EXISTS (SELECT 1 FROM partner.seller_product_group WHERE tenant_id = $1 AND seller_id = $2)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    if busy {
        return Ok(false);
    }
    sqlx::query("DELETE FROM partner.seller_event WHERE tenant_id = $1 AND seller_id = $2")
        .bind(tx.tenant.as_str())
        .bind(id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    let n = sqlx::query("DELETE FROM partner.seller WHERE tenant_id = $1 AND id = $2")
        .bind(tx.tenant.as_str())
        .bind(id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?
        .rows_affected();
    Ok(n > 0)
}

/// The profile fields `update_seller` changes; `None` keeps a field.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Profile {
    pub contact_point: Option<Value>,
    pub address: Option<Value>,
    pub business_category: Option<Vec<String>>,
}

pub async fn update_profile(tx: &mut TxCx, id: &str, p: &Profile) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE partner.seller SET
           contact_point = COALESCE($3, contact_point),
           address = COALESCE($4, address),
           business_category = COALESCE($5, business_category),
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(p.contact_point.as_ref())
    .bind(p.address.as_ref())
    .bind(p.business_category.as_deref())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

/// Moves a seller from one status to another; `false` when it is no longer in `from`.
pub async fn set_status(tx: &mut TxCx, id: &str, from: &str, to: &str) -> Result<bool, ToolError> {
    let r = sqlx::query(
        "UPDATE partner.seller SET seller_status = $4, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND seller_status = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(from)
    .bind(to)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected() > 0)
}

// ── status events ────────────────────────────────────────────────────────────────────────

/// One `partner.seller_event` row as written.
#[derive(Debug, Clone)]
pub struct NewEvent<'a> {
    pub seller_id: &'a str,
    pub action_type: &'a str,
    pub action_status: &'a str,
    pub instrument: &'a str,
    pub from_state: Option<&'a str>,
    pub to_state: &'a str,
    pub result: Value,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
}

/// Appends an event and returns its position.
pub async fn insert_event(tx: &mut TxCx, e: &NewEvent<'_>) -> Result<i32, ToolError> {
    let agent = tx.principal.subject.clone();
    let agent = (agent.starts_with("adm_") || agent.starts_with("sys_")).then_some(agent);
    sqlx::query_scalar(
        "INSERT INTO partner.seller_event (tenant_id, seller_id, position, action_type,
           action_status, agent_id, instrument, from_state, to_state, result, start_time,
           end_time, request_id)
         SELECT $1, $2, COALESCE(max(position), 0) + 1, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12
         FROM partner.seller_event WHERE tenant_id = $1 AND seller_id = $2
         RETURNING position",
    )
    .bind(tx.tenant.as_str())
    .bind(e.seller_id)
    .bind(e.action_type)
    .bind(e.action_status)
    .bind(agent)
    .bind(e.instrument)
    .bind(e.from_state)
    .bind(e.to_state)
    .bind(&e.result)
    .bind(e.start_time)
    .bind(e.end_time)
    .bind(&tx.request_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// One event, as the booked-change job reads it.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct EventRow {
    pub position: i32,
    pub action_status: String,
    pub from_state: Option<String>,
    pub to_state: Option<String>,
    pub result: Option<Value>,
    pub start_time: DateTime<Utc>,
}

pub async fn event(
    tx: &mut TxCx,
    seller_id: &str,
    position: i32,
) -> Result<Option<EventRow>, ToolError> {
    sqlx::query_as(
        "SELECT position, action_status, from_state, to_state, result, start_time
         FROM partner.seller_event WHERE tenant_id = $1 AND seller_id = $2 AND position = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(seller_id)
    .bind(position)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Booked status changes of a seller still waiting for their time.
pub async fn waiting_events(tx: &mut TxCx, seller_id: &str) -> Result<Vec<EventRow>, ToolError> {
    sqlx::query_as(
        "SELECT position, action_status, from_state, to_state, result, start_time
         FROM partner.seller_event
         WHERE tenant_id = $1 AND seller_id = $2 AND action_status = 'potential'
         ORDER BY start_time, position",
    )
    .bind(tx.tenant.as_str())
    .bind(seller_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Settles a booked change: `completed` (with what it did) or `failed` (with why).
pub async fn finish_event(
    tx: &mut TxCx,
    seller_id: &str,
    position: i32,
    status: &str,
    result: Option<&Value>,
    error: Option<&Value>,
    at: DateTime<Utc>,
) -> Result<bool, ToolError> {
    let r = sqlx::query(
        "UPDATE partner.seller_event SET action_status = $4, result = COALESCE($5, result),
           error = $6, end_time = $7, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND seller_id = $2 AND position = $3 AND action_status = 'potential'",
    )
    .bind(tx.tenant.as_str())
    .bind(seller_id)
    .bind(position)
    .bind(status)
    .bind(result)
    .bind(error)
    .bind(at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected() > 0)
}

// ── products ─────────────────────────────────────────────────────────────────────────────

/// The products a seller sells, with when its suspension hid each one.
pub async fn products(
    tx: &mut TxCx,
    seller_id: &str,
) -> Result<Vec<(String, Option<DateTime<Utc>>)>, ToolError> {
    sqlx::query_as(
        "SELECT product_group_id, hidden_at FROM partner.seller_product_group
         WHERE tenant_id = $1 AND seller_id = $2 ORDER BY product_group_id",
    )
    .bind(tx.tenant.as_str())
    .bind(seller_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Records that these products were hidden for the seller (`Some`) or are back (`None`).
pub async fn mark_hidden(
    tx: &mut TxCx,
    product_ids: &[String],
    at: Option<DateTime<Utc>>,
) -> Result<(), ToolError> {
    if product_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE partner.seller_product_group SET hidden_at = $3, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND product_group_id = ANY($2)",
    )
    .bind(tx.tenant.as_str())
    .bind(product_ids)
    .bind(at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

// ── contracts ────────────────────────────────────────────────────────────────────────────

/// The contract of a seller: its id and row version.
pub async fn contract_of(
    tx: &mut TxCx,
    seller_id: &str,
) -> Result<Option<(String, i32)>, ToolError> {
    sqlx::query_as(
        "SELECT id, version FROM partner.seller_contract WHERE tenant_id = $1 AND seller_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(seller_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Whether a contract id is taken.
pub async fn contract_exists(tx: &mut TxCx, id: &str) -> Result<bool, ToolError> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM partner.seller_contract WHERE tenant_id = $1 AND id = $2)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// One version of a contract's terms.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct VersionRow {
    pub version_number: i32,
    pub default_commission_rate: Decimal,
    pub commission_rate: Value,
    pub settlement_cycle: String,
    pub settlement_method: String,
    pub payout_account_ref: String,
    pub valid_from: DateTime<Utc>,
    pub valid_through: DateTime<Utc>,
}

/// Every version of a contract, oldest first.
pub async fn versions(tx: &mut TxCx, contract_id: &str) -> Result<Vec<VersionRow>, ToolError> {
    sqlx::query_as(
        "SELECT version_number, default_commission_rate, commission_rate, settlement_cycle,
                settlement_method, payout_account_ref, valid_from, valid_through
         FROM partner.seller_contract_version
         WHERE tenant_id = $1 AND seller_contract_id = $2 ORDER BY version_number",
    )
    .bind(tx.tenant.as_str())
    .bind(contract_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

pub async fn insert_contract(tx: &mut TxCx, id: &str, seller_id: &str) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO partner.seller_contract (tenant_id, id, seller_id) VALUES ($1, $2, $3)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(seller_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Marks the contract as changed (a version was added or removed).
pub async fn touch_contract(tx: &mut TxCx, id: &str) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE partner.seller_contract SET updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

pub async fn insert_version(
    tx: &mut TxCx,
    contract_id: &str,
    v: &VersionRow,
) -> Result<(), ToolError> {
    let agent = tx.principal.subject.clone();
    sqlx::query(
        "INSERT INTO partner.seller_contract_version (tenant_id, seller_contract_id,
           version_number, default_commission_rate, commission_rate, settlement_cycle,
           settlement_method, payout_account_ref, valid_from, valid_through, agent_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(tx.tenant.as_str())
    .bind(contract_id)
    .bind(v.version_number)
    .bind(v.default_commission_rate)
    .bind(&v.commission_rate)
    .bind(&v.settlement_cycle)
    .bind(&v.settlement_method)
    .bind(&v.payout_account_ref)
    .bind(v.valid_from)
    .bind(v.valid_through)
    .bind(agent)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

pub async fn delete_version(
    tx: &mut TxCx,
    contract_id: &str,
    version_number: i32,
) -> Result<(), ToolError> {
    sqlx::query(
        "DELETE FROM partner.seller_contract_version
         WHERE tenant_id = $1 AND seller_contract_id = $2 AND version_number = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(contract_id)
    .bind(version_number)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

pub async fn delete_contract(tx: &mut TxCx, id: &str) -> Result<(), ToolError> {
    sqlx::query("DELETE FROM partner.seller_contract WHERE tenant_id = $1 AND id = $2")
        .bind(tx.tenant.as_str())
        .bind(id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    Ok(())
}

// ── grades ───────────────────────────────────────────────────────────────────────────────

/// One confirmed grade.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct GradeRow {
    pub position: i32,
    pub grade_level: String,
    pub computed_grade_level: Option<String>,
    pub adjustment_reason: Option<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: DateTime<Utc>,
}

/// The grade in force at `at`: the latest confirmed grade whose period covers it.
pub async fn grade_at(
    tx: &mut TxCx,
    seller_id: &str,
    at: DateTime<Utc>,
) -> Result<Option<GradeRow>, ToolError> {
    sqlx::query_as(
        "SELECT position, grade_level, computed_grade_level, adjustment_reason, valid_from,
                valid_through
         FROM partner.seller_grade
         WHERE tenant_id = $1 AND seller_id = $2 AND valid_from <= $3 AND valid_through >= $3
         ORDER BY position DESC LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(seller_id)
    .bind(at)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A grade to confirm.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NewGrade {
    pub grade_level: String,
    pub computed_grade_level: Option<String>,
    pub adjustment_reason: Option<String>,
    pub metrics: Value,
    pub valid_from: DateTime<Utc>,
    pub valid_through: DateTime<Utc>,
}

/// Adds a confirmed grade and returns its position.
pub async fn insert_grade(tx: &mut TxCx, seller_id: &str, g: &NewGrade) -> Result<i32, ToolError> {
    let agent = tx.principal.subject.clone();
    sqlx::query_scalar(
        "INSERT INTO partner.seller_grade (tenant_id, seller_id, position, grade_level,
           computed_grade_level, adjustment_reason, metrics, valid_from, valid_through, agent_id)
         SELECT $1, $2, COALESCE(max(position), 0) + 1, $3, $4, $5, $6, $7, $8, $9
         FROM partner.seller_grade WHERE tenant_id = $1 AND seller_id = $2
         RETURNING position",
    )
    .bind(tx.tenant.as_str())
    .bind(seller_id)
    .bind(&g.grade_level)
    .bind(g.computed_grade_level.as_deref())
    .bind(g.adjustment_reason.as_deref())
    .bind(&g.metrics)
    .bind(g.valid_from)
    .bind(g.valid_through)
    .bind(agent)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

pub async fn delete_grade(tx: &mut TxCx, seller_id: &str, position: i32) -> Result<(), ToolError> {
    sqlx::query(
        "DELETE FROM partner.seller_grade WHERE tenant_id = $1 AND seller_id = $2 AND position = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(seller_id)
    .bind(position)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// The latest grade row with exactly this level and period, by position.
pub async fn find_grade(
    tx: &mut TxCx,
    seller_id: &str,
    g: &NewGrade,
) -> Result<Option<i32>, ToolError> {
    sqlx::query_scalar(
        "SELECT max(position) FROM partner.seller_grade
         WHERE tenant_id = $1 AND seller_id = $2 AND grade_level = $3
           AND valid_from = $4 AND valid_through = $5",
    )
    .bind(tx.tenant.as_str())
    .bind(seller_id)
    .bind(&g.grade_level)
    .bind(g.valid_from)
    .bind(g.valid_through)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}
