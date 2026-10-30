//! SQL for the merchandising tables (`migrations/0006_merchandising.sql`). Every query filters
//! `tenant_id`; values are bind parameters and the dynamic parts are fixed strings.

use acom_contract::ToolError;
use acom_store::{Page, StoreError, TxCx};
use chrono::{DateTime, Utc};
use serde_json::Value;

pub(crate) fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// A category with its place in the tree.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct CategoryRow {
    pub id: String,
    pub name: String,
    pub term_code: String,
    pub term_set: String,
    pub parent_id: Option<String>,
    pub display_order: i32,
    pub description: Option<String>,
    /// Active itself and every category above it active.
    pub active: bool,
    /// Ids from the top category down to the parent.
    pub ancestors: Vec<String>,
}

/// The whole category tree, walked from the top, with inherited activity and a sort path.
/// Cycles are cut off.
const TREE: &str = "WITH RECURSIVE tree AS (
       SELECT id, name, term_code, in_defined_term_set, broader_id, position, description,
              is_active AS active, ARRAY[]::text[] AS ancestors,
              ARRAY[lpad(position::text, 6, '0') || id] AS sort_path
       FROM merchandising.defined_term WHERE tenant_id = $1 AND broader_id IS NULL
       UNION ALL
       SELECT c.id, c.name, c.term_code, c.in_defined_term_set, c.broader_id, c.position,
              c.description, t.active AND c.is_active, t.ancestors || t.id,
              t.sort_path || (lpad(c.position::text, 6, '0') || c.id)
       FROM merchandising.defined_term c JOIN tree t ON c.broader_id = t.id
       WHERE c.tenant_id = $1 AND NOT c.id = ANY(t.ancestors || t.id))";

const CATEGORY_COLUMNS: &str = "id, name, term_code, in_defined_term_set AS term_set,
    broader_id AS parent_id, position AS display_order, description, active, ancestors";

/// Categories of the tree: in one set, under one parent, down to a depth (from the top, or
/// below the parent).
pub(crate) async fn categories(
    tx: &mut TxCx,
    term_set: Option<&str>,
    parent_id: Option<&str>,
    depth: Option<i32>,
    page: Page,
) -> Result<(i64, Vec<CategoryRow>), ToolError> {
    let filter = "($2::text IS NULL OR in_defined_term_set = $2)
         AND ($3::text IS NULL OR $3 = ANY(ancestors))
         AND ($4::int IS NULL OR cardinality(ancestors) + 1
               - coalesce(array_position(ancestors, $3::text), 0) <= $4)";
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "{TREE} SELECT count(*) FROM tree WHERE {filter}"
    )))
    .bind(tx.tenant.as_str())
    .bind(term_set)
    .bind(parent_id)
    .bind(depth)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, CategoryRow>(sqlx::AssertSqlSafe(format!(
        "{TREE} SELECT {CATEGORY_COLUMNS} FROM tree WHERE {filter}
         ORDER BY in_defined_term_set DESC, sort_path LIMIT $5 OFFSET $6"
    )))
    .bind(tx.tenant.as_str())
    .bind(term_set)
    .bind(parent_id)
    .bind(depth)
    .bind(page.limit)
    .bind(page.offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

pub(crate) async fn category(tx: &mut TxCx, id: &str) -> Result<Option<CategoryRow>, ToolError> {
    sqlx::query_as::<_, CategoryRow>(sqlx::AssertSqlSafe(format!(
        "{TREE} SELECT {CATEGORY_COLUMNS} FROM tree WHERE id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Products placed in each of the categories or anywhere under them: `(category, product)`
/// pairs in placement order, each product once per category.
pub(crate) async fn placed_products(
    tx: &mut TxCx,
    category_ids: &[String],
) -> Result<Vec<(String, String)>, ToolError> {
    sqlx::query_as(
        "WITH RECURSIVE under AS (
           SELECT id AS root, id FROM merchandising.defined_term
           WHERE tenant_id = $1 AND id = ANY($2)
           UNION
           SELECT u.root, c.id FROM merchandising.defined_term c
           JOIN under u ON c.broader_id = u.id WHERE c.tenant_id = $1)
         SELECT u.root, pc.product_group_id
         FROM under u
         JOIN merchandising.product_group_category pc
           ON pc.tenant_id = $1 AND pc.category_id = u.id
         GROUP BY u.root, pc.product_group_id
         ORDER BY u.root, min(pc.position) NULLS LAST, pc.product_group_id",
    )
    .bind(tx.tenant.as_str())
    .bind(category_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// `(id, name)` of the categories among the ids that exist.
pub(crate) async fn category_names(
    tx: &mut TxCx,
    ids: &[String],
) -> Result<Vec<(String, String)>, ToolError> {
    sqlx::query_as(
        "SELECT id, name FROM merchandising.defined_term WHERE tenant_id = $1 AND id = ANY($2)",
    )
    .bind(tx.tenant.as_str())
    .bind(ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A display slot.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct SlotRow {
    pub id: String,
    pub name: String,
    pub page: String,
    pub slot_type: String,
    pub position: i32,
    pub item_list_element: Value,
    pub version: i32,
}

const SLOT_COLUMNS: &str = "id, name, page, slot_type, position, item_list_element, version";

pub(crate) async fn slots(
    tx: &mut TxCx,
    page_word: &str,
    slot_type: Option<&str>,
    page: Page,
) -> Result<(i64, Vec<SlotRow>), ToolError> {
    let filter = "tenant_id = $1 AND page = $2 AND ($3::text IS NULL OR slot_type = $3)";
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM merchandising.display_slot WHERE {filter}"
    )))
    .bind(tx.tenant.as_str())
    .bind(page_word)
    .bind(slot_type)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, SlotRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {SLOT_COLUMNS} FROM merchandising.display_slot WHERE {filter}
         ORDER BY position, id LIMIT $4 OFFSET $5"
    )))
    .bind(tx.tenant.as_str())
    .bind(page_word)
    .bind(slot_type)
    .bind(page.limit)
    .bind(page.offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

pub(crate) async fn slot(tx: &mut TxCx, id: &str) -> Result<Option<SlotRow>, ToolError> {
    sqlx::query_as::<_, SlotRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {SLOT_COLUMNS} FROM merchandising.display_slot WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A booking with the banner it shows.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct BookingRow {
    pub id: String,
    pub display_slot_id: String,
    pub banner_id: Option<String>,
    pub valid_for_member_tier: Option<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: Option<DateTime<Utc>>,
    pub display_status: String,
    pub promotion_id: Option<String>,
    pub change_set_id: Option<String>,
    pub version: i32,
    pub eyebrow: Option<String>,
    pub headline: Option<String>,
    pub alternative_headline: Option<String>,
    pub cta_label: Option<String>,
    pub url: Option<String>,
    pub image: Option<String>,
    pub image_product_group_id: Option<String>,
    pub image_color: Option<String>,
}

const BOOKING_SELECT: &str = "SELECT d.id, d.display_slot_id, d.banner_id,
        d.valid_for_member_tier, d.valid_from, d.valid_through, d.display_status, d.promotion_id,
        d.change_set_id, d.version, b.eyebrow, b.headline, b.alternative_headline, b.cta_label,
        b.url, b.image, b.image_product_group_id, b.image_color
     FROM merchandising.display_booking d
     LEFT JOIN merchandising.banner b ON b.tenant_id = d.tenant_id AND b.id = d.banner_id
     WHERE d.tenant_id = $1";

/// Bookings of the slots that share a moment with `from`–`through` (drafts left out).
pub(crate) async fn bookings_between(
    tx: &mut TxCx,
    slot_ids: &[String],
    from: DateTime<Utc>,
    through: DateTime<Utc>,
) -> Result<Vec<BookingRow>, ToolError> {
    sqlx::query_as::<_, BookingRow>(sqlx::AssertSqlSafe(format!(
        "{BOOKING_SELECT} AND d.display_slot_id = ANY($2) AND d.display_status <> 'draft'
           AND d.valid_from <= $4 AND (d.valid_through IS NULL OR d.valid_through >= $3)
         ORDER BY d.valid_from, d.id"
    )))
    .bind(tx.tenant.as_str())
    .bind(slot_ids)
    .bind(from)
    .bind(through)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Bookings of a slot that are live or still to come at `at`.
pub(crate) async fn bookings_from(
    tx: &mut TxCx,
    slot_id: &str,
    at: DateTime<Utc>,
) -> Result<Vec<BookingRow>, ToolError> {
    sqlx::query_as::<_, BookingRow>(sqlx::AssertSqlSafe(format!(
        "{BOOKING_SELECT} AND d.display_slot_id = $2 AND d.display_status <> 'draft'
           AND (d.valid_through IS NULL OR d.valid_through + interval '1 minute' > $3)
         ORDER BY d.valid_from, d.id"
    )))
    .bind(tx.tenant.as_str())
    .bind(slot_id)
    .bind(at)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

pub(crate) async fn booking(tx: &mut TxCx, id: &str) -> Result<Option<BookingRow>, ToolError> {
    sqlx::query_as::<_, BookingRow>(sqlx::AssertSqlSafe(format!(
        "{BOOKING_SELECT} AND d.id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Bookings that show any of the banners, oldest first.
pub(crate) async fn bookings_of_banners(
    tx: &mut TxCx,
    banner_ids: &[String],
) -> Result<Vec<BookingRow>, ToolError> {
    sqlx::query_as::<_, BookingRow>(sqlx::AssertSqlSafe(format!(
        "{BOOKING_SELECT} AND d.banner_id = ANY($2) ORDER BY d.valid_from, d.id"
    )))
    .bind(tx.tenant.as_str())
    .bind(banner_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Banner content.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct BannerRow {
    pub id: String,
    pub name: String,
    pub eyebrow: Option<String>,
    pub headline: String,
    pub alternative_headline: Option<String>,
    pub cta_label: Option<String>,
    pub url: String,
    pub image: Option<String>,
    pub image_product_group_id: Option<String>,
    pub image_color: Option<String>,
    pub version: i32,
}

const BANNER_COLUMNS: &str = "id, name, eyebrow, headline, alternative_headline, cta_label, url,
    image, image_product_group_id, image_color, version";

pub(crate) async fn banner(tx: &mut TxCx, id: &str) -> Result<Option<BannerRow>, ToolError> {
    sqlx::query_as::<_, BannerRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {BANNER_COLUMNS} FROM merchandising.banner WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Banners booked on a page, live at a moment, or all; newest first.
pub(crate) async fn banners(
    tx: &mut TxCx,
    page_word: Option<&str>,
    live_at: Option<DateTime<Utc>>,
    page: Page,
) -> Result<(i64, Vec<BannerRow>), ToolError> {
    let filter = "b.tenant_id = $1
         AND ($2::text IS NULL OR EXISTS (
               SELECT 1 FROM merchandising.display_booking d
               JOIN merchandising.display_slot s
                 ON s.tenant_id = d.tenant_id AND s.id = d.display_slot_id
               WHERE d.tenant_id = b.tenant_id AND d.banner_id = b.id AND s.page = $2))
         AND ($3::timestamptz IS NULL OR EXISTS (
               SELECT 1 FROM merchandising.display_booking d
               WHERE d.tenant_id = b.tenant_id AND d.banner_id = b.id
                 AND d.display_status NOT IN ('draft', 'ended') AND d.valid_from <= $3
                 AND (d.valid_through IS NULL OR d.valid_through + interval '1 minute' > $3)))";
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM merchandising.banner b WHERE {filter}"
    )))
    .bind(tx.tenant.as_str())
    .bind(page_word)
    .bind(live_at)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, BannerRow>(sqlx::AssertSqlSafe(format!(
        "SELECT b.id, b.name, b.eyebrow, b.headline, b.alternative_headline, b.cta_label, b.url,
                b.image, b.image_product_group_id, b.image_color, b.version
         FROM merchandising.banner b WHERE {filter}
         ORDER BY b.created_at DESC, b.id LIMIT $4 OFFSET $5"
    )))
    .bind(tx.tenant.as_str())
    .bind(page_word)
    .bind(live_at)
    .bind(page.limit)
    .bind(page.offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

/// Inserts banner content; nothing happens when the id is already there.
pub(crate) async fn insert_banner(tx: &mut TxCx, b: &BannerRow) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO merchandising.banner (tenant_id, id, name, eyebrow, headline,
           alternative_headline, cta_label, url, image, image_product_group_id, image_color)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(&b.id)
    .bind(&b.name)
    .bind(&b.eyebrow)
    .bind(&b.headline)
    .bind(&b.alternative_headline)
    .bind(&b.cta_label)
    .bind(&b.url)
    .bind(&b.image)
    .bind(&b.image_product_group_id)
    .bind(&b.image_color)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Sets a banner's image and link; only touches the row (and its version) when they differ.
pub(crate) async fn update_banner_image(
    tx: &mut TxCx,
    id: &str,
    image: Option<&str>,
    image_product_group_id: Option<&str>,
    image_color: Option<&str>,
    url: &str,
) -> Result<u64, ToolError> {
    Ok(sqlx::query(
        "UPDATE merchandising.banner SET image = $3, image_product_group_id = $4,
           image_color = $5, url = $6, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2
           AND (image IS DISTINCT FROM $3 OR image_product_group_id IS DISTINCT FROM $4
                OR image_color IS DISTINCT FROM $5 OR url IS DISTINCT FROM $6)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(image)
    .bind(image_product_group_id)
    .bind(image_color)
    .bind(url)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected())
}

/// Deletes a banner no booking shows; returns whether it was deleted.
pub(crate) async fn delete_banner_if_unbooked(tx: &mut TxCx, id: &str) -> Result<bool, ToolError> {
    Ok(sqlx::query(
        "DELETE FROM merchandising.banner b WHERE b.tenant_id = $1 AND b.id = $2
           AND NOT EXISTS (SELECT 1 FROM merchandising.display_booking d
                           WHERE d.tenant_id = b.tenant_id AND d.banner_id = b.id)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected()
        > 0)
}

/// A booking as `schedule_banner` creates it.
#[derive(Debug)]
pub(crate) struct NewBooking<'a> {
    pub id: &'a str,
    pub slot_id: &'a str,
    pub banner_id: &'a str,
    pub tier: Option<&'a str>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: DateTime<Utc>,
    pub status: &'a str,
    pub promotion_id: Option<&'a str>,
}

pub(crate) async fn insert_booking(tx: &mut TxCx, b: &NewBooking<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO merchandising.display_booking (tenant_id, id, display_slot_id, banner_id,
           valid_for_member_tier, valid_from, valid_through, display_status, promotion_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(b.id)
    .bind(b.slot_id)
    .bind(b.banner_id)
    .bind(b.tier)
    .bind(b.valid_from)
    .bind(b.valid_through)
    .bind(b.status)
    .bind(b.promotion_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Removes a booking that never showed.
pub(crate) async fn delete_booking(tx: &mut TxCx, id: &str) -> Result<u64, ToolError> {
    Ok(
        sqlx::query("DELETE FROM merchandising.display_booking WHERE tenant_id = $1 AND id = $2")
            .bind(tx.tenant.as_str())
            .bind(id)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?
            .rows_affected(),
    )
}

/// Ends a booking that already showed, at `at`.
pub(crate) async fn end_booking(
    tx: &mut TxCx,
    id: &str,
    at: DateTime<Utc>,
) -> Result<u64, ToolError> {
    Ok(sqlx::query(
        "UPDATE merchandising.display_booking SET display_status = 'ended',
           valid_through = CASE WHEN $3 > valid_from AND (valid_through IS NULL OR $3 < valid_through)
                                THEN $3 ELSE valid_through END,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND display_status <> 'ended'",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected())
}

/// Products placed in a category or under it.
pub(crate) async fn product_count(tx: &mut TxCx, category_id: &str) -> Result<i64, ToolError> {
    Ok(placed_products(tx, &[category_id.to_owned()]).await?.len() as i64)
}
