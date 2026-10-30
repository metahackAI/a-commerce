//! Catalog SQL for writes: products, variants, offers, drafts made from supplier sheets,
//! scheduled list prices, brands and review moderation. Every statement is scoped to the
//! caller's store (`tenant_id = $1`) on top of row-level security, and every change bumps the
//! row's `version` so previews see it.

use std::collections::{BTreeMap, BTreeSet};

use acom_contract::ToolError;
use acom_store::TxCx;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

use crate::db;

/// A new product (`catalog.product_group`), always created as a draft.
#[derive(Debug, Clone)]
pub struct NewGroup<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub brand_id: Option<&'a str>,
    pub varies_by: &'a [String],
    pub material: Option<&'a str>,
    pub image: Option<&'a str>,
}

pub async fn insert_group(tx: &mut TxCx, g: &NewGroup<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO catalog.product_group (tenant_id, id, name, description, brand_id,
           approval_status, varies_by, material, image)
         VALUES ($1, $2, $3, $4, $5, 'draft', $6, $7, $8)",
    )
    .bind(tx.tenant.as_str())
    .bind(g.id)
    .bind(g.name)
    .bind(g.description)
    .bind(g.brand_id)
    .bind(g.varies_by)
    .bind(g.material)
    .bind(g.image)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// The editable fields of a product, as stored. `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GroupFields {
    pub name: Option<String>,
    /// `Some(None)` clears the description.
    pub description: Option<Option<String>>,
    pub brand_id: Option<Option<String>>,
    pub varies_by: Option<Vec<String>>,
    pub material: Option<Option<String>>,
    pub image: Option<Option<String>>,
}

pub async fn update_group(tx: &mut TxCx, id: &str, f: &GroupFields) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE catalog.product_group SET
           name = coalesce($3, name),
           description = CASE WHEN $4 THEN $5 ELSE description END,
           brand_id = CASE WHEN $6 THEN $7 ELSE brand_id END,
           varies_by = coalesce($8, varies_by),
           material = CASE WHEN $9 THEN $10 ELSE material END,
           image = CASE WHEN $11 THEN $12 ELSE image END,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(f.name.as_deref())
    .bind(f.description.is_some())
    .bind(f.description.clone().flatten())
    .bind(f.brand_id.is_some())
    .bind(f.brand_id.clone().flatten())
    .bind(f.varies_by.as_deref())
    .bind(f.material.is_some())
    .bind(f.material.clone().flatten())
    .bind(f.image.is_some())
    .bind(f.image.clone().flatten())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

/// Moves a product to another review state (and sets its release date when given).
pub async fn set_approval_status(
    tx: &mut TxCx,
    id: &str,
    from: &str,
    to: &str,
    release_date: Option<Option<NaiveDate>>,
) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE catalog.product_group SET approval_status = $4,
           release_date = CASE WHEN $5 THEN $6 ELSE release_date END,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND approval_status = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(from)
    .bind(to)
    .bind(release_date.is_some())
    .bind(release_date.flatten())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

/// Removes a draft product and everything made with it: its variants' offers and scheduled
/// prices, its variants, and its sheet row. Refuses (returns `false`) unless it is still a draft
/// or was rejected: a product shoppers may have seen is never removed.
pub async fn delete_draft(tx: &mut TxCx, id: &str) -> Result<bool, ToolError> {
    let status: Option<String> = sqlx::query_scalar(
        "SELECT approval_status FROM catalog.product_group
         WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    match status.as_deref() {
        None => return Ok(true),
        Some("draft" | "rejected") => {}
        Some(_) => return Ok(false),
    }
    for sql in [
        "DELETE FROM catalog.unit_price_specification s USING catalog.offer o, catalog.product p
         WHERE s.tenant_id = $1 AND o.tenant_id = $1 AND p.tenant_id = $1
           AND s.offer_id = o.id AND o.product_id = p.id AND p.product_group_id = $2",
        "DELETE FROM catalog.offer o USING catalog.product p
         WHERE o.tenant_id = $1 AND p.tenant_id = $1 AND o.product_id = p.id
           AND p.product_group_id = $2",
        "DELETE FROM catalog.product WHERE tenant_id = $1 AND product_group_id = $2",
        "DELETE FROM catalog.data_feed_item WHERE tenant_id = $1 AND product_group_id = $2",
    ] {
        sqlx::query(sql)
            .bind(tx.tenant.as_str())
            .bind(id)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
    }
    crate::neighbors::remove_placements(tx, id).await?;
    sqlx::query("DELETE FROM catalog.product_group WHERE tenant_id = $1 AND id = $2")
        .bind(tx.tenant.as_str())
        .bind(id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    Ok(true)
}

/// A new variant (`catalog.product`).
#[derive(Debug, Clone)]
pub struct NewVariant<'a> {
    pub id: &'a str,
    pub product_group_id: &'a str,
    pub sku: &'a str,
    pub name: &'a str,
    pub color: Option<&'a str>,
    pub size: Option<&'a str>,
    pub image: Option<&'a str>,
    pub sale_type: &'a str,
    pub is_managed: bool,
}

pub async fn insert_variant(tx: &mut TxCx, v: &NewVariant<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO catalog.product (tenant_id, id, product_group_id, sku, name, color, size,
           image, sale_type, is_managed)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(tx.tenant.as_str())
    .bind(v.id)
    .bind(v.product_group_id)
    .bind(v.sku)
    .bind(v.name)
    .bind(v.color)
    .bind(v.size)
    .bind(v.image)
    .bind(v.sale_type)
    .bind(v.is_managed)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// The editable fields of a variant, as stored. `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VariantFields {
    pub name: Option<String>,
    pub color: Option<Option<String>>,
    pub size: Option<Option<String>>,
    pub image: Option<Option<String>>,
    pub sale_type: Option<String>,
    pub is_managed: Option<bool>,
}

pub async fn update_variant(tx: &mut TxCx, id: &str, f: &VariantFields) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE catalog.product SET
           name = coalesce($3, name),
           color = CASE WHEN $4 THEN $5 ELSE color END,
           size = CASE WHEN $6 THEN $7 ELSE size END,
           image = CASE WHEN $8 THEN $9 ELSE image END,
           sale_type = coalesce($10, sale_type),
           is_managed = coalesce($11, is_managed),
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(f.name.as_deref())
    .bind(f.color.is_some())
    .bind(f.color.clone().flatten())
    .bind(f.size.is_some())
    .bind(f.size.clone().flatten())
    .bind(f.image.is_some())
    .bind(f.image.clone().flatten())
    .bind(f.sale_type.as_deref())
    .bind(f.is_managed)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

/// Removes a variant made by a write that is being taken back, with its offer. Fails (as a
/// database error) if anything else points at it, such as a cart line.
pub async fn delete_variant(tx: &mut TxCx, id: &str) -> Result<(), ToolError> {
    for sql in [
        "DELETE FROM catalog.unit_price_specification s USING catalog.offer o
         WHERE s.tenant_id = $1 AND o.tenant_id = $1 AND s.offer_id = o.id AND o.product_id = $2",
        "DELETE FROM catalog.offer WHERE tenant_id = $1 AND product_id = $2",
        "DELETE FROM catalog.product WHERE tenant_id = $1 AND id = $2",
    ] {
        sqlx::query(sql)
            .bind(tx.tenant.as_str())
            .bind(id)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
    }
    Ok(())
}

/// A new offer (`catalog.offer`).
#[derive(Debug, Clone)]
pub struct NewOffer<'a> {
    pub id: &'a str,
    pub variant_id: &'a str,
    pub price: Decimal,
    pub price_currency: &'a str,
    pub availability: &'a str,
    pub inventory_level: i32,
    pub cost: Option<(Decimal, &'a str)>,
}

pub async fn insert_offer(tx: &mut TxCx, o: &NewOffer<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO catalog.offer (tenant_id, id, product_id, price, price_currency,
           availability, inventory_level, cost_value, cost_currency)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(tx.tenant.as_str())
    .bind(o.id)
    .bind(o.variant_id)
    .bind(o.price)
    .bind(o.price_currency)
    .bind(o.availability)
    .bind(o.inventory_level)
    .bind(o.cost.map(|c| c.0))
    .bind(o.cost.map(|c| c.1))
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

pub async fn delete_offer(tx: &mut TxCx, offer_id: &str) -> Result<(), ToolError> {
    for sql in [
        "DELETE FROM catalog.unit_price_specification WHERE tenant_id = $1 AND offer_id = $2",
        "DELETE FROM catalog.offer WHERE tenant_id = $1 AND id = $2",
    ] {
        sqlx::query(sql)
            .bind(tx.tenant.as_str())
            .bind(offer_id)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
    }
    Ok(())
}

pub async fn set_availability(
    tx: &mut TxCx,
    offer_id: &str,
    availability: &str,
) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE catalog.offer SET availability = $3, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(offer_id)
    .bind(availability)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

pub async fn set_safety_stock(tx: &mut TxCx, offer_id: &str, level: i32) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE catalog.offer SET safety_stock = $3, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(offer_id)
    .bind(level)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

/// Which of these variant SKUs are taken, lowercase, with the product each belongs to.
pub async fn taken_skus(
    tx: &mut TxCx,
    skus: &[String],
) -> Result<BTreeMap<String, String>, ToolError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT lower(sku), product_group_id FROM catalog.product
         WHERE tenant_id = $1 AND lower(sku) = ANY($2::text[])",
    )
    .bind(tx.tenant.as_str())
    .bind(skus)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(rows.into_iter().collect())
}

/// Which of these products exist, with their approval status.
pub async fn existing_groups(
    tx: &mut TxCx,
    ids: &[String],
) -> Result<BTreeMap<String, String>, ToolError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, approval_status FROM catalog.product_group
         WHERE tenant_id = $1 AND id = ANY($2::text[])",
    )
    .bind(tx.tenant.as_str())
    .bind(ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(rows.into_iter().collect())
}

/// An uploaded supplier sheet (`catalog.data_feed`).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Feed {
    pub id: String,
    pub name: String,
    pub encoding_format: String,
    pub provider_name: Option<String>,
    pub data_feed_element: Value,
    pub version: i32,
}

pub async fn feed(tx: &mut TxCx, id: &str) -> Result<Option<Feed>, ToolError> {
    sqlx::query_as::<_, Feed>(
        "SELECT id, name, encoding_format, provider_name, data_feed_element, version
         FROM catalog.data_feed WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Imports already run from a sheet.
pub async fn imports_of(tx: &mut TxCx, feed_id: &str) -> Result<Vec<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM catalog.product_import WHERE tenant_id = $1 AND data_feed_id = $2
         ORDER BY id",
    )
    .bind(tx.tenant.as_str())
    .bind(feed_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Products an earlier import made that are still drafts.
pub async fn imported_drafts(tx: &mut TxCx, ids: &[String]) -> Result<BTreeSet<String>, ToolError> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT i.product_group_id FROM catalog.data_feed_item i
         JOIN catalog.product_group g ON g.tenant_id = i.tenant_id AND g.id = i.product_group_id
         WHERE i.tenant_id = $1 AND i.product_group_id = ANY($2::text[])
           AND g.approval_status IN ('draft', 'rejected')",
    )
    .bind(tx.tenant.as_str())
    .bind(ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(rows.into_iter().collect())
}

/// One import run (`catalog.product_import`).
#[derive(Debug, Clone)]
pub struct NewImport<'a> {
    pub id: &'a str,
    pub data_feed_id: &'a str,
    pub import_mode: &'a str,
    pub at: DateTime<Utc>,
}

pub async fn insert_import(tx: &mut TxCx, i: &NewImport<'_>) -> Result<(), ToolError> {
    let agent = tx.principal.subject.clone();
    let change_set = tx.change_set_id.clone();
    sqlx::query(
        "INSERT INTO catalog.product_import (tenant_id, id, data_feed_id, import_mode,
           action_status, agent_id, change_set_id, start_time, end_time)
         VALUES ($1, $2, $3, $4, 'completed', $5, $6, $7, $7)",
    )
    .bind(tx.tenant.as_str())
    .bind(i.id)
    .bind(i.data_feed_id)
    .bind(i.import_mode)
    .bind(agent)
    .bind(change_set)
    .bind(i.at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

pub async fn delete_import(tx: &mut TxCx, id: &str) -> Result<(), ToolError> {
    for sql in [
        "DELETE FROM catalog.data_feed_item WHERE tenant_id = $1 AND product_import_id = $2",
        "DELETE FROM catalog.product_import WHERE tenant_id = $1 AND id = $2",
    ] {
        sqlx::query(sql)
            .bind(tx.tenant.as_str())
            .bind(id)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
    }
    Ok(())
}

/// The sheet row a draft came from (`catalog.data_feed_item`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct DraftItem {
    pub product_import_id: String,
    pub position: i32,
    pub product_group_id: String,
    pub supplier_code: String,
    pub supplier_text: Option<String>,
    pub draft_status: String,
    pub open_fields: Vec<String>,
    pub draft_note: Option<String>,
}

pub async fn insert_item(tx: &mut TxCx, i: &DraftItem) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO catalog.data_feed_item (tenant_id, product_import_id, position,
           product_group_id, supplier_code, supplier_text, draft_status, open_fields, draft_note)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(tx.tenant.as_str())
    .bind(&i.product_import_id)
    .bind(i.position)
    .bind(&i.product_group_id)
    .bind(&i.supplier_code)
    .bind(i.supplier_text.as_deref())
    .bind(&i.draft_status)
    .bind(&i.open_fields)
    .bind(i.draft_note.as_deref())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Sets what is still open on a draft's sheet row (when a person fills a field, or an upsert
/// reads the row again).
pub async fn set_item_state(tx: &mut TxCx, i: &DraftItem) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE catalog.data_feed_item SET supplier_text = $4, draft_status = $5,
           open_fields = $6, draft_note = $7, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND product_import_id = $2 AND position = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(&i.product_import_id)
    .bind(i.position)
    .bind(i.supplier_text.as_deref())
    .bind(&i.draft_status)
    .bind(&i.open_fields)
    .bind(i.draft_note.as_deref())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// The sheet rows these products came from, newest import first per product.
pub async fn items_of(tx: &mut TxCx, product_ids: &[String]) -> Result<Vec<DraftItem>, ToolError> {
    sqlx::query_as::<_, DraftItem>(
        "SELECT DISTINCT ON (product_group_id) product_import_id, position, product_group_id,
                supplier_code, supplier_text, draft_status, open_fields, draft_note
         FROM catalog.data_feed_item
         WHERE tenant_id = $1 AND product_group_id = ANY($2::text[])
         ORDER BY product_group_id, created_at DESC",
    )
    .bind(tx.tenant.as_str())
    .bind(product_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Products made from supplier sheets: those of one import, in one draft state, or both.
pub async fn drafts(
    tx: &mut TxCx,
    import_id: Option<&str>,
    draft_status: Option<&str>,
) -> Result<Vec<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT product_group_id FROM catalog.data_feed_item
         WHERE tenant_id = $1 AND ($2::text IS NULL OR product_import_id = $2)
           AND ($3::text IS NULL OR draft_status = $3)
         ORDER BY product_import_id, position",
    )
    .bind(tx.tenant.as_str())
    .bind(import_id)
    .bind(draft_status)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A scheduled list price (`catalog.unit_price_specification`).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Schedule {
    pub id: String,
    pub offer_id: String,
    pub price: Decimal,
    pub price_currency: String,
    pub valid_from: DateTime<Utc>,
    pub valid_through: Option<DateTime<Utc>>,
}

pub async fn insert_schedule(tx: &mut TxCx, s: &Schedule) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO catalog.unit_price_specification (tenant_id, id, offer_id, price,
           price_currency, price_type, valid_from, valid_through)
         VALUES ($1, $2, $3, $4, $5, 'list_price', $6, $7)",
    )
    .bind(tx.tenant.as_str())
    .bind(&s.id)
    .bind(&s.offer_id)
    .bind(s.price)
    .bind(&s.price_currency)
    .bind(s.valid_from)
    .bind(s.valid_through)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

pub async fn delete_schedule(tx: &mut TxCx, id: &str) -> Result<(), ToolError> {
    sqlx::query("DELETE FROM catalog.unit_price_specification WHERE tenant_id = $1 AND id = $2")
        .bind(tx.tenant.as_str())
        .bind(id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    Ok(())
}

/// Scheduled list prices of these offers, earliest first.
pub async fn schedules(tx: &mut TxCx, offer_ids: &[String]) -> Result<Vec<Schedule>, ToolError> {
    sqlx::query_as::<_, Schedule>(
        "SELECT id, offer_id, price, price_currency::text AS price_currency, valid_from,
                valid_through
         FROM catalog.unit_price_specification
         WHERE tenant_id = $1 AND offer_id = ANY($2::text[]) AND price_type = 'list_price'
         ORDER BY valid_from, id",
    )
    .bind(tx.tenant.as_str())
    .bind(offer_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The scheduled list price in effect at `at` for each of these offers that has one
/// (`pricing.scheduled_change.compute: on_demand`); offers without one keep their own price.
pub async fn scheduled_prices(
    tx: &mut TxCx,
    offer_ids: &[String],
    at: DateTime<Utc>,
) -> Result<BTreeMap<String, Decimal>, ToolError> {
    if offer_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let all = schedules(tx, offer_ids).await?;
    let mut out = BTreeMap::new();
    for offer in offer_ids {
        let mine: Vec<&Schedule> = all.iter().filter(|s| s.offer_id == *offer).collect();
        if mine.is_empty() {
            continue;
        }
        let none = Decimal::MIN;
        let p = crate::ops::schedule::price_at(none, &mine, at);
        if p != none {
            out.insert(offer.clone(), p);
        }
    }
    Ok(out)
}

/// Replaces the list price of variants that have a scheduled price in effect at `at`.
pub async fn apply_scheduled_prices(
    tx: &mut TxCx,
    variants: &mut [crate::model::Variant],
    at: DateTime<Utc>,
) -> Result<(), ToolError> {
    let offers: Vec<String> = variants.iter().filter_map(|v| v.offer_id.clone()).collect();
    let now = scheduled_prices(tx, &offers, at).await?;
    for v in variants.iter_mut() {
        if let Some(p) = v.offer_id.as_ref().and_then(|o| now.get(o)) {
            v.price = Some(*p);
        }
    }
    Ok(())
}

/// A brand as a write sees it.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct BrandRow {
    pub id: String,
    pub name: String,
    pub logo: Option<String>,
    pub version: i32,
}

pub async fn brand_row(tx: &mut TxCx, id: &str) -> Result<Option<BrandRow>, ToolError> {
    sqlx::query_as::<_, BrandRow>(
        "SELECT id, name, logo, version FROM catalog.brand WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The brand with this name (case ignored), if any.
pub async fn brand_named(tx: &mut TxCx, name: &str) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM catalog.brand WHERE tenant_id = $1 AND lower(name) = lower($2)",
    )
    .bind(tx.tenant.as_str())
    .bind(name.trim())
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

pub async fn insert_brand(
    tx: &mut TxCx,
    id: &str,
    name: &str,
    logo: Option<&str>,
) -> Result<(), ToolError> {
    sqlx::query("INSERT INTO catalog.brand (tenant_id, id, name, logo) VALUES ($1, $2, $3, $4)")
        .bind(tx.tenant.as_str())
        .bind(id)
        .bind(name)
        .bind(logo)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    Ok(())
}

pub async fn update_brand(
    tx: &mut TxCx,
    id: &str,
    name: &str,
    logo: Option<&str>,
) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE catalog.brand SET name = $3, logo = $4, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(name)
    .bind(logo)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

/// Removes a brand no product uses; `false` when one does.
pub async fn delete_unused_brand(tx: &mut TxCx, id: &str) -> Result<bool, ToolError> {
    let used: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM catalog.product_group WHERE tenant_id = $1 AND brand_id = $2)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    if used {
        return Ok(false);
    }
    sqlx::query("DELETE FROM catalog.brand WHERE tenant_id = $1 AND id = $2")
        .bind(tx.tenant.as_str())
        .bind(id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    Ok(true)
}

/// A review's moderation state.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ReviewState {
    pub id: String,
    pub product_group_id: String,
    pub creative_work_status: String,
    pub version: i32,
}

pub async fn review_state(tx: &mut TxCx, id: &str) -> Result<Option<ReviewState>, ToolError> {
    sqlx::query_as::<_, ReviewState>(
        "SELECT id, product_group_id, creative_work_status, version FROM catalog.review
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

pub async fn set_review_status(tx: &mut TxCx, id: &str, status: &str) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE catalog.review SET creative_work_status = $3, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(status)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}
