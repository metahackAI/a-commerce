//! Catalog SQL. Every query is scoped to the caller's store (`tenant_id = $1`) on top of
//! row-level security. Dynamic parts are fixed strings chosen in code; values are always bind
//! parameters.

use acom_contract::ToolError;
use acom_store::TxCx;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

use crate::db;
use crate::model::{ReviewFacts, Variant};

/// A product as stored (`catalog.product_group`).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub brand_id: Option<String>,
    pub approval_status: String,
    pub varies_by: Vec<String>,
    pub material: Option<String>,
    pub keywords: Vec<String>,
    pub additional_property: Value,
    pub image: Option<String>,
    pub has_merchant_return_policy_id: Option<String>,
    pub release_date: Option<NaiveDate>,
    pub version: i32,
    pub updated_at: DateTime<Utc>,
}

const GROUP_COLUMNS: &str = "g.id, g.name, g.description, g.brand_id, g.approval_status,
    g.varies_by, g.material, g.keywords, g.additional_property, g.image,
    g.has_merchant_return_policy_id, g.release_date, g.version, g.updated_at";

pub async fn group(tx: &mut TxCx, id: &str) -> Result<Option<Group>, ToolError> {
    sqlx::query_as::<_, Group>(sqlx::AssertSqlSafe(format!(
        "SELECT {GROUP_COLUMNS} FROM catalog.product_group g WHERE g.tenant_id = $1 AND g.id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Products by id, in no particular order.
pub async fn groups(tx: &mut TxCx, ids: &[String]) -> Result<Vec<Group>, ToolError> {
    sqlx::query_as::<_, Group>(sqlx::AssertSqlSafe(format!(
        "SELECT {GROUP_COLUMNS} FROM catalog.product_group g
         WHERE g.tenant_id = $1 AND g.id = ANY($2::text[])"
    )))
    .bind(tx.tenant.as_str())
    .bind(ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

const VARIANT_COLUMNS: &str = "p.id, p.product_group_id, p.sku, p.name, p.color, p.size,
    p.sale_type, p.is_managed, o.id AS offer_id, o.price, o.price_currency::text AS price_currency,
    o.availability, o.inventory_level, o.safety_stock, o.cost_value,
    o.cost_currency::text AS cost_currency";

/// The variants of these products with their offers (none for a draft variant without one).
pub async fn variants(tx: &mut TxCx, group_ids: &[String]) -> Result<Vec<Variant>, ToolError> {
    sqlx::query_as::<_, Variant>(sqlx::AssertSqlSafe(format!(
        "SELECT {VARIANT_COLUMNS}
         FROM catalog.product p
         LEFT JOIN catalog.offer o ON o.tenant_id = p.tenant_id AND o.product_id = p.id
         WHERE p.tenant_id = $1 AND p.product_group_id = ANY($2::text[])"
    )))
    .bind(tx.tenant.as_str())
    .bind(group_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

pub async fn variant(tx: &mut TxCx, variant_id: &str) -> Result<Option<Variant>, ToolError> {
    sqlx::query_as::<_, Variant>(sqlx::AssertSqlSafe(format!(
        "SELECT {VARIANT_COLUMNS}
         FROM catalog.product p
         LEFT JOIN catalog.offer o ON o.tenant_id = p.tenant_id AND o.product_id = p.id
         WHERE p.tenant_id = $1 AND p.id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(variant_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// One variant's offer as a write sees it: values a change replaces, and the row version.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct OfferState {
    pub variant_id: String,
    pub product_group_id: String,
    pub is_managed: bool,
    pub variant_version: i32,
    pub offer_id: String,
    pub price: Decimal,
    pub price_currency: String,
    pub availability: String,
    pub inventory_level: i32,
    pub cost_value: Option<Decimal>,
    pub cost_currency: Option<String>,
    pub sold_out_at: Option<DateTime<Utc>>,
    pub availability_starts: Option<DateTime<Utc>>,
    pub version: i32,
}

pub async fn offer_state(tx: &mut TxCx, variant_id: &str) -> Result<Option<OfferState>, ToolError> {
    sqlx::query_as::<_, OfferState>(
        "SELECT p.id AS variant_id, p.product_group_id, p.is_managed, p.version AS variant_version,
                o.id AS offer_id, o.price, o.price_currency::text AS price_currency,
                o.availability, o.inventory_level, o.cost_value,
                o.cost_currency::text AS cost_currency, o.sold_out_at, o.availability_starts,
                o.version
         FROM catalog.product p
         JOIN catalog.offer o ON o.tenant_id = p.tenant_id AND o.product_id = p.id
         WHERE p.tenant_id = $1 AND p.id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(variant_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Sets an offer's list price, and its cost when `cost` is `Some`.
pub async fn set_offer_price(
    tx: &mut TxCx,
    offer_id: &str,
    price: Decimal,
    currency: &str,
    cost: Option<(Option<Decimal>, Option<String>)>,
) -> Result<u64, ToolError> {
    let (set_cost, cost_value, cost_currency) = match cost {
        Some((v, c)) => (true, v, c),
        None => (false, None, None),
    };
    sqlx::query(
        "UPDATE catalog.offer SET price = $3, price_currency = $4,
           cost_value = CASE WHEN $5 THEN $6 ELSE cost_value END,
           cost_currency = CASE WHEN $5 THEN $7 ELSE cost_currency END,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(offer_id)
    .bind(price)
    .bind(currency)
    .bind(set_cost)
    .bind(cost_value)
    .bind(cost_currency)
    .execute(&mut *tx.tx)
    .await
    .map(|r| r.rows_affected())
    .map_err(db)
}

/// Stock and the availability fields that move with it.
#[derive(Debug, Clone, PartialEq)]
pub struct StockWrite<'a> {
    pub offer_id: &'a str,
    pub inventory_level: i32,
    pub availability: &'a str,
    pub sold_out_at: Option<DateTime<Utc>>,
    pub availability_starts: Option<DateTime<Utc>>,
}

pub async fn set_stock(tx: &mut TxCx, w: &StockWrite<'_>) -> Result<u64, ToolError> {
    sqlx::query(
        "UPDATE catalog.offer SET inventory_level = $3, availability = $4, sold_out_at = $5,
           availability_starts = $6, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(w.offer_id)
    .bind(w.inventory_level)
    .bind(w.availability)
    .bind(w.sold_out_at)
    .bind(w.availability_starts)
    .execute(&mut *tx.tx)
    .await
    .map(|r| r.rows_affected())
    .map_err(db)
}

pub async fn set_managed(tx: &mut TxCx, variant_id: &str, managed: bool) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE catalog.product SET is_managed = $3, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND is_managed <> $3",
    )
    .bind(tx.tenant.as_str())
    .bind(variant_id)
    .bind(managed)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

pub async fn discontinue_offer(tx: &mut TxCx, offer_id: &str) -> Result<u64, ToolError> {
    sqlx::query(
        "UPDATE catalog.offer SET availability = 'discontinued', updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND availability <> 'discontinued'",
    )
    .bind(tx.tenant.as_str())
    .bind(offer_id)
    .execute(&mut *tx.tx)
    .await
    .map(|r| r.rows_affected())
    .map_err(db)
}

/// Per product: total stock, lowest list price still sold, and availability derived from the
/// variants. Mirrors `model::summarize` and `model::product_availability` so filters and sorts
/// run inside the query.
const STOCK: &str = "stock AS (
    SELECT p.product_group_id AS id,
           sum(o.inventory_level)::bigint AS inventory_level,
           coalesce(min(o.price) FILTER (WHERE o.availability <> 'discontinued'), min(o.price))
             AS price,
           min(o.price_currency::text) AS price_currency,
           CASE WHEN bool_or(o.availability = 'in_stock') THEN 'in_stock'
                WHEN bool_or(o.availability = 'pre_order') THEN 'pre_order'
                WHEN bool_or(o.availability = 'back_order') THEN 'back_order'
                WHEN bool_and(o.availability = 'discontinued') THEN 'discontinued'
                ELSE 'out_of_stock' END AS availability,
           count(*)::bigint AS variant_count
    FROM catalog.product p
    JOIN catalog.offer o ON o.tenant_id = p.tenant_id AND o.product_id = p.id
    WHERE p.tenant_id = $1
    GROUP BY p.product_group_id)";

/// How a product list is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Relevance,
    Price,
    PriceDesc,
    Newest,
    Inventory,
    InventoryDesc,
}

impl Order {
    fn sql(self) -> &'static str {
        match self {
            Order::Relevance => "coalesce(r.rank, 0) DESC, g.release_date DESC NULLS LAST, g.id",
            Order::Price => "s.price ASC NULLS LAST, g.id",
            Order::PriceDesc => "s.price DESC NULLS LAST, g.id",
            Order::Newest => "g.release_date DESC NULLS LAST, g.id",
            Order::Inventory => "s.inventory_level ASC NULLS LAST, g.id",
            Order::InventoryDesc => "s.inventory_level DESC NULLS LAST, g.id",
        }
    }
}

/// A product search: what to match and keep, how to order, which page.
#[derive(Debug, Clone, PartialEq)]
pub struct Find<'a> {
    /// Products a text query matched, with their relevance. `None`: no text query.
    pub ranked: Option<&'a [(String, f64)]>,
    /// Only these products (a category). `None`: no such filter.
    pub only: Option<&'a [String]>,
    /// Products tagged with all of these.
    pub tags: &'a [String],
    /// Product availability words; empty is any.
    pub availability: &'a [String],
    pub min_inventory: Option<i64>,
    pub max_inventory: Option<i64>,
    pub min_price: Option<Decimal>,
    pub max_price: Option<Decimal>,
    pub price_currency: Option<&'a str>,
    /// Shoppers see active products only.
    pub active_only: bool,
    /// Products that have no variant with an offer yet are left out unless this is set.
    pub with_drafts: bool,
    pub brand_id: Option<&'a str>,
    pub approval_status: Option<&'a str>,
    pub order: Order,
    pub limit: i64,
    pub offset: i64,
}

const FIND_FROM: &str = "FROM catalog.product_group g
    LEFT JOIN stock s ON s.id = g.id
    LEFT JOIN ranked r ON r.id = g.id
    WHERE g.tenant_id = $1
      AND (NOT $4::boolean OR r.id IS NOT NULL)
      AND (NOT $5::boolean OR g.id = ANY($6::text[]))
      AND g.keywords @> $7::text[]
      AND (cardinality($8::text[]) = 0 OR s.availability = ANY($8::text[]))
      AND ($9::bigint IS NULL OR s.inventory_level >= $9)
      AND ($10::bigint IS NULL OR s.inventory_level <= $10)
      AND ($11::numeric IS NULL OR (s.price >= $11 AND s.price_currency = $13::text))
      AND ($12::numeric IS NULL OR (s.price <= $12 AND s.price_currency = $13::text))
      AND (NOT $14::boolean OR g.approval_status = 'active')
      AND ($15::boolean OR s.id IS NOT NULL)
      AND ($16::text IS NULL OR g.brand_id = $16)
      AND ($17::text IS NULL OR g.approval_status = $17)";

/// Product ids in order for one page, and how many products match in all.
pub async fn find(tx: &mut TxCx, f: &Find<'_>) -> Result<(Vec<String>, i64), ToolError> {
    let (ids, ranks): (Vec<String>, Vec<f64>) =
        f.ranked.unwrap_or_default().iter().cloned().unzip();
    let with = format!(
        "WITH {STOCK}, ranked AS (SELECT * FROM unnest($2::text[], $3::float8[]) AS r(id, rank))"
    );
    macro_rules! binds {
        ($q:expr) => {
            $q.bind(tx.tenant.as_str())
                .bind(&ids)
                .bind(&ranks)
                .bind(f.ranked.is_some())
                .bind(f.only.is_some())
                .bind(f.only.unwrap_or_default())
                .bind(f.tags)
                .bind(f.availability)
                .bind(f.min_inventory)
                .bind(f.max_inventory)
                .bind(f.min_price)
                .bind(f.max_price)
                .bind(f.price_currency)
                .bind(f.active_only)
                .bind(f.with_drafts)
                .bind(f.brand_id)
                .bind(f.approval_status)
        };
    }
    let total: i64 = binds!(sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "{with} SELECT count(*) {FIND_FROM}"
    ))))
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let page: Vec<String> = binds!(sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "{with} SELECT g.id {FIND_FROM}
         ORDER BY {} LIMIT $18 OFFSET $19",
        f.order.sql()
    ))))
    .bind(f.limit)
    .bind(f.offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((page, total))
}

/// A product's words for the catalog's own text match (operators looking up products).
const GROUP_TEXT: &str = "to_tsvector('english', g.name || ' ' || coalesce(g.material, '') || ' '
    || coalesce(g.description, '') || ' ' || array_to_string(g.keywords, ' '))";

/// Products whose name, material, description or tags match the words, or whose name or a
/// variant's SKU contains the text, with a relevance (a name hit counts most).
pub async fn match_text(tx: &mut TxCx, text: &str) -> Result<Vec<(String, f64)>, ToolError> {
    sqlx::query_as::<_, (String, f64)>(sqlx::AssertSqlSafe(format!(
        "SELECT g.id,
                (ts_rank({GROUP_TEXT}, plainto_tsquery('english', $2))
                 + CASE WHEN g.name ILIKE $3 THEN 1 ELSE 0 END)::float8 AS rank
         FROM catalog.product_group g
         WHERE g.tenant_id = $1
           AND ({GROUP_TEXT} @@ plainto_tsquery('english', $2)
                OR g.name ILIKE $3
                OR EXISTS (SELECT 1 FROM catalog.product p
                           WHERE p.tenant_id = g.tenant_id AND p.product_group_id = g.id
                             AND p.sku ILIKE $3))"
    )))
    .bind(tx.tenant.as_str())
    .bind(text)
    .bind(crate::model::like_pattern(text))
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The text of a product that search ranks: name, brand, tags, material, description.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ProductText {
    pub id: String,
    pub name: String,
    pub brand_name: Option<String>,
    pub keywords: Vec<String>,
    pub material: Option<String>,
    pub description: Option<String>,
    pub release_date: Option<NaiveDate>,
}

pub async fn product_texts(
    tx: &mut TxCx,
    active_only: bool,
) -> Result<Vec<ProductText>, ToolError> {
    sqlx::query_as::<_, ProductText>(
        "SELECT g.id, g.name, b.name AS brand_name, g.keywords, g.material, g.description,
                g.release_date
         FROM catalog.product_group g
         LEFT JOIN catalog.brand b ON b.tenant_id = g.tenant_id AND b.id = g.brand_id
         WHERE g.tenant_id = $1 AND (NOT $2 OR g.approval_status = 'active')",
    )
    .bind(tx.tenant.as_str())
    .bind(active_only)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// One garment measurement (`catalog.measurement`).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Measurement {
    pub size: String,
    pub name: String,
    pub value: Decimal,
    pub unit_text: String,
    pub measurement_technique: Option<String>,
}

pub async fn measurements(tx: &mut TxCx, group_id: &str) -> Result<Vec<Measurement>, ToolError> {
    sqlx::query_as::<_, Measurement>(
        "SELECT size, name, value, unit_text, measurement_technique
         FROM catalog.measurement WHERE tenant_id = $1 AND product_group_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(group_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

#[derive(Debug, sqlx::FromRow)]
struct FactsRow {
    id: String,
    rating_value: i16,
    review_body: String,
    fit_vote: Option<String>,
    author_height_value: Option<Decimal>,
    size: Option<String>,
    keywords: Vec<String>,
    date_published: NaiveDate,
}

/// The reviews a summary counts: not hidden, within the reviewer height range and size bought.
pub async fn review_facts(
    tx: &mut TxCx,
    group_id: &str,
    height: Option<(Decimal, Decimal)>,
    size: Option<&str>,
) -> Result<Vec<ReviewFacts>, ToolError> {
    let rows = sqlx::query_as::<_, FactsRow>(
        "SELECT r.id, r.rating_value, r.review_body, r.fit_vote, r.author_height_value, p.size,
                r.keywords, r.date_published
         FROM catalog.review r
         LEFT JOIN catalog.product p ON p.tenant_id = r.tenant_id AND p.id = r.product_id
         WHERE r.tenant_id = $1 AND r.product_group_id = $2
           AND r.creative_work_status <> 'hidden'
           AND ($3::numeric IS NULL OR r.author_height_value >= $3)
           AND ($4::numeric IS NULL OR r.author_height_value <= $4)
           AND ($5::text IS NULL OR p.size = $5)",
    )
    .bind(tx.tenant.as_str())
    .bind(group_id)
    .bind(height.map(|h| h.0))
    .bind(height.map(|h| h.1))
    .bind(size)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(rows
        .into_iter()
        .map(|r| ReviewFacts {
            id: r.id,
            rating: r.rating_value,
            body: r.review_body,
            fit: r.fit_vote,
            height_cm: r.author_height_value,
            size: r.size,
            keywords: r.keywords,
            date_published: r.date_published,
        })
        .collect())
}

/// A review as lists show it (`catalog.review` + the size bought + the helpful votes).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Review {
    pub id: String,
    pub product_group_id: String,
    pub product_id: Option<String>,
    pub author_name: String,
    pub rating_value: i16,
    pub review_body: String,
    pub fit_vote: Option<String>,
    pub author_height_value: Option<Decimal>,
    pub size: Option<String>,
    pub image: Vec<String>,
    pub keywords: Vec<String>,
    pub creative_work_status: String,
    pub date_published: NaiveDate,
    pub helpful_count: i64,
}

/// Filters of [`reviews`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReviewFilter<'a> {
    pub product_id: Option<&'a str>,
    pub rating: Option<i16>,
    pub status: Option<&'a str>,
    /// Shoppers never see hidden reviews.
    pub hide_hidden: bool,
    /// Highest rating first instead of newest first.
    pub by_rating: bool,
    pub limit: i64,
    pub offset: i64,
}

const REVIEW_WHERE: &str = "WHERE r.tenant_id = $1
      AND ($2::text IS NULL OR r.product_group_id = $2)
      AND ($3::smallint IS NULL OR r.rating_value = $3)
      AND ($4::text IS NULL OR r.creative_work_status = $4)
      AND (NOT $5::boolean OR r.creative_work_status <> 'hidden')";

pub async fn reviews(tx: &mut TxCx, f: &ReviewFilter<'_>) -> Result<(Vec<Review>, i64), ToolError> {
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM catalog.review r {REVIEW_WHERE}"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.product_id)
    .bind(f.rating)
    .bind(f.status)
    .bind(f.hide_hidden)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let order = if f.by_rating {
        "r.rating_value DESC, r.date_published DESC, r.id DESC"
    } else {
        "r.date_published DESC, r.id DESC"
    };
    let rows = sqlx::query_as::<_, Review>(sqlx::AssertSqlSafe(format!(
        "SELECT r.id, r.product_group_id, r.product_id, r.author_name, r.rating_value,
                r.review_body, r.fit_vote, r.author_height_value, p.size, r.image, r.keywords,
                r.creative_work_status, r.date_published,
                (SELECT count(*) FROM catalog.review_vote v
                  WHERE v.tenant_id = r.tenant_id AND v.review_id = r.id AND v.is_helpful)
                  AS helpful_count
         FROM catalog.review r
         LEFT JOIN catalog.product p ON p.tenant_id = r.tenant_id AND p.id = r.product_id
         {REVIEW_WHERE}
         ORDER BY {order} LIMIT $6 OFFSET $7"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.product_id)
    .bind(f.rating)
    .bind(f.status)
    .bind(f.hide_hidden)
    .bind(f.limit)
    .bind(f.offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((rows, total))
}

/// A review as a vote sees it.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ReviewHead {
    pub id: String,
    pub product_group_id: String,
    pub creative_work_status: String,
    pub version: i32,
    pub helpful_count: i64,
}

pub async fn review_head(tx: &mut TxCx, id: &str) -> Result<Option<ReviewHead>, ToolError> {
    sqlx::query_as::<_, ReviewHead>(
        "SELECT r.id, r.product_group_id, r.creative_work_status, r.version,
                (SELECT count(*) FROM catalog.review_vote v
                  WHERE v.tenant_id = r.tenant_id AND v.review_id = r.id AND v.is_helpful)
                  AS helpful_count
         FROM catalog.review r WHERE r.tenant_id = $1 AND r.id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A shopper's current vote on a review, if any.
pub async fn vote(
    tx: &mut TxCx,
    review_id: &str,
    customer_id: &str,
) -> Result<Option<bool>, ToolError> {
    sqlx::query_scalar(
        "SELECT is_helpful FROM catalog.review_vote
         WHERE tenant_id = $1 AND review_id = $2 AND customer_id = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(review_id)
    .bind(customer_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

pub async fn set_vote(
    tx: &mut TxCx,
    review_id: &str,
    customer_id: &str,
    helpful: bool,
) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO catalog.review_vote (tenant_id, review_id, customer_id, is_helpful)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (tenant_id, review_id, customer_id) DO UPDATE
           SET is_helpful = EXCLUDED.is_helpful, updated_at = now(),
               version = catalog.review_vote.version + 1
           WHERE catalog.review_vote.is_helpful <> EXCLUDED.is_helpful",
    )
    .bind(tx.tenant.as_str())
    .bind(review_id)
    .bind(customer_id)
    .bind(helpful)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// The review a shopper already wrote for this product and order, if any.
pub async fn review_by(
    tx: &mut TxCx,
    author_id: &str,
    order_id: &str,
    group_id: &str,
) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM catalog.review
         WHERE tenant_id = $1 AND author_id = $2 AND order_id = $3 AND product_group_id = $4
         ORDER BY created_at LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(author_id)
    .bind(order_id)
    .bind(group_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The mention phrases reviews of this product already use.
pub async fn known_mentions(tx: &mut TxCx, group_id: &str) -> Result<Vec<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT DISTINCT k FROM catalog.review r, unnest(r.keywords) AS k
         WHERE r.tenant_id = $1 AND r.product_group_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(group_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A new review (`create_review`).
#[derive(Debug, Clone, PartialEq)]
pub struct NewReview<'a> {
    pub id: &'a str,
    pub group_id: &'a str,
    pub variant_id: Option<&'a str>,
    pub author_id: &'a str,
    pub author_name: &'a str,
    pub order_id: &'a str,
    pub rating: i16,
    pub body: &'a str,
    pub fit: Option<&'a str>,
    pub height_cm: Option<Decimal>,
    pub keywords: &'a [String],
    pub image: &'a [String],
    pub status: &'a str,
    pub date_published: NaiveDate,
}

/// Inserts the review unless it is already there (the same plan run twice).
pub async fn insert_review(tx: &mut TxCx, r: &NewReview<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO catalog.review (tenant_id, id, product_group_id, product_id, author_id,
           author_name, order_id, rating_value, review_body, fit_vote, author_height_value,
           author_height_unit_text, keywords, image, creative_work_status, date_published)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                 CASE WHEN $11::numeric IS NULL THEN NULL ELSE 'cm' END, $12, $13, $14, $15)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(r.id)
    .bind(r.group_id)
    .bind(r.variant_id)
    .bind(r.author_id)
    .bind(r.author_name)
    .bind(r.order_id)
    .bind(r.rating)
    .bind(r.body)
    .bind(r.fit)
    .bind(r.height_cm)
    .bind(r.keywords)
    .bind(r.image)
    .bind(r.status)
    .bind(r.date_published)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// What a written review reads back as.
pub async fn review_written(
    tx: &mut TxCx,
    id: &str,
) -> Result<Option<(i16, String, String)>, ToolError> {
    sqlx::query_as(
        "SELECT rating_value, review_body, product_group_id FROM catalog.review
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A brand (`catalog.brand`).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Brand {
    pub id: String,
    pub name: String,
    pub logo: Option<String>,
    pub description: Option<String>,
}

pub async fn brands(
    tx: &mut TxCx,
    name: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<Brand>, i64), ToolError> {
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM catalog.brand
         WHERE tenant_id = $1 AND ($2::text IS NULL OR name ILIKE $2)",
    )
    .bind(tx.tenant.as_str())
    .bind(name)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, Brand>(
        "SELECT id, name, logo, description FROM catalog.brand
         WHERE tenant_id = $1 AND ($2::text IS NULL OR name ILIKE $2)
         ORDER BY name, id LIMIT $3 OFFSET $4",
    )
    .bind(tx.tenant.as_str())
    .bind(name)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((rows, total))
}

pub async fn brand(tx: &mut TxCx, id: &str) -> Result<Option<Brand>, ToolError> {
    sqlx::query_as::<_, Brand>(
        "SELECT id, name, logo, description FROM catalog.brand WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Return terms (`catalog.merchant_return_policy`).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ReturnPolicy {
    pub id: String,
    pub name: String,
    pub merchant_return_days: i32,
    pub return_fees: String,
    pub return_method: Vec<String>,
    pub refund_type: String,
    pub size_exchange_days: Option<i32>,
}

pub async fn return_policy(tx: &mut TxCx, id: &str) -> Result<Option<ReturnPolicy>, ToolError> {
    sqlx::query_as::<_, ReturnPolicy>(
        "SELECT id, name, merchant_return_days, return_fees, return_method, refund_type,
                size_exchange_days
         FROM catalog.merchant_return_policy WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// One change in a product's history: a committed preview's change row and the action that ran
/// it. Previews and the action log are the change-set engine's store (`acom-changeset`), shared
/// by every domain's writes; catalog reads only the changes to its own records from it.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct HistoryRow {
    pub id: String,
    pub actor_kind: String,
    pub agent_id: Option<String>,
    pub agent_name: Option<String>,
    pub instrument: String,
    pub end_time: DateTime<Utc>,
    pub change: Value,
}

const HISTORY_FROM: &str = "FROM platform.action_log a
    JOIN platform.preview p ON p.tenant_id = a.tenant_id AND p.id = a.preview_id
    CROSS JOIN LATERAL jsonb_array_elements(p.changes) AS c(value)
    WHERE a.tenant_id = $1 AND a.action_status = 'completed' AND p.preview_state = 'committed'
      AND c.value->>'id' = ANY($2::text[])";

/// Changes that ran on any of these records (a product, its variants, their offers), newest
/// first.
pub async fn history(
    tx: &mut TxCx,
    record_ids: &[String],
    limit: i64,
    offset: i64,
) -> Result<(Vec<HistoryRow>, i64), ToolError> {
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) {HISTORY_FROM}"
    )))
    .bind(tx.tenant.as_str())
    .bind(record_ids)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, HistoryRow>(sqlx::AssertSqlSafe(format!(
        "SELECT a.id, a.actor_kind, a.agent_id, a.agent_name, a.instrument,
                coalesce(a.end_time, a.start_time) AS end_time, c.value AS change
         {HISTORY_FROM}
         ORDER BY coalesce(a.end_time, a.start_time) DESC, a.id DESC LIMIT $3 OFFSET $4"
    )))
    .bind(tx.tenant.as_str())
    .bind(record_ids)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((rows, total))
}

/// A change to one of these records waiting in a change set (same store as [`history`]).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct PendingRow {
    pub id: String,
    pub instrument: String,
    pub change_set_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub change: Value,
}

pub async fn pending(tx: &mut TxCx, record_ids: &[String]) -> Result<Vec<PendingRow>, ToolError> {
    sqlx::query_as::<_, PendingRow>(
        "SELECT p.id, p.instrument, p.change_set_id, p.created_at, c.value AS change
         FROM platform.preview p
         CROSS JOIN LATERAL jsonb_array_elements(p.changes) AS c(value)
         WHERE p.tenant_id = $1 AND p.preview_state = 'open' AND p.change_set_id IS NOT NULL
           AND c.value->>'id' = ANY($2::text[])
         ORDER BY p.created_at, p.id",
    )
    .bind(tx.tenant.as_str())
    .bind(record_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}
