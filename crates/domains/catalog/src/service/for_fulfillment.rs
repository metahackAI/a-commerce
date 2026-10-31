//! What `fulfillment` reads and changes in catalog: a variant with its stock (replacements,
//! delivery dates, whether an item can be returned) and stock moves when a cancel puts units
//! back or an exchange holds a replacement.

use acom_contract::{ErrorCode, ToolError};
use acom_core::{Currency, Money};
use acom_store::{StoreError, TxCx};
use rust_decimal::Decimal;

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// The offer of one variant: list price and stock.
#[derive(Debug, Clone, PartialEq)]
pub struct Stock {
    pub offer_id: String,
    pub price: Money,
    /// `in_stock`, `out_of_stock`, `pre_order`, `back_order` or `discontinued`.
    pub availability: String,
    pub inventory_level: i32,
    pub version: i32,
}

/// One variant (`var_…`) and the product it belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub id: String,
    /// The product (`prd_…`).
    pub product_id: String,
    pub product_name: String,
    pub name: String,
    pub size: Option<String>,
    pub color: Option<String>,
    /// `always`, `pre_order`, `subscription` or `gift_card`.
    pub sale_type: String,
    /// True when stock is tracked.
    pub is_managed: bool,
    /// The product's tags (for example `final_sale`).
    pub keywords: Vec<String>,
    pub stock: Option<Stock>,
}

type VariantRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    bool,
    Vec<String>,
    Option<String>,
    Option<Decimal>,
    Option<String>,
    Option<String>,
    Option<i32>,
    Option<i32>,
);

const VARIANT_SELECT: &str = "SELECT p.id, p.product_group_id, g.name, p.name, p.size, p.color,
       p.sale_type, p.is_managed, g.keywords, o.id, o.price, o.price_currency::text,
       o.availability, o.inventory_level, o.version
     FROM catalog.product p
     JOIN catalog.product_group g ON g.tenant_id = p.tenant_id AND g.id = p.product_group_id
     LEFT JOIN catalog.offer o ON o.tenant_id = p.tenant_id AND o.product_id = p.id";

fn variant_from(r: VariantRow) -> Result<Variant, ToolError> {
    let stock = match (r.9, r.10, r.11, r.12, r.13, r.14) {
        (Some(offer_id), Some(price), Some(currency), Some(availability), Some(level), Some(v)) => {
            let currency = Currency::parse(currency.trim())
                .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?;
            Some(Stock {
                offer_id,
                price: Money::new(price, currency),
                availability,
                inventory_level: level,
                version: v,
            })
        }
        _ => None,
    };
    Ok(Variant {
        id: r.0,
        product_id: r.1,
        product_name: r.2,
        name: r.3,
        size: r.4,
        color: r.5,
        sale_type: r.6,
        is_managed: r.7,
        keywords: r.8,
        stock,
    })
}

/// One variant with its stock, or `None` when the store has no such variant.
pub async fn variant(tx: &mut TxCx, variant_id: &str) -> Result<Option<Variant>, ToolError> {
    let row: Option<VariantRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{VARIANT_SELECT} WHERE p.tenant_id = $1 AND p.id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(variant_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    row.map(variant_from).transpose()
}

/// Several variants at once; missing ids are left out.
pub async fn variants(tx: &mut TxCx, variant_ids: &[String]) -> Result<Vec<Variant>, ToolError> {
    let rows: Vec<VariantRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{VARIANT_SELECT} WHERE p.tenant_id = $1 AND p.id = ANY($2) ORDER BY p.id"
    )))
    .bind(tx.tenant.as_str())
    .bind(variant_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    rows.into_iter().map(variant_from).collect()
}

/// Every variant of a product, or `None` when the store has no such product.
pub async fn product_variants(
    tx: &mut TxCx,
    product_id: &str,
) -> Result<Option<Vec<Variant>>, ToolError> {
    let exists: Option<String> =
        sqlx::query_scalar("SELECT id FROM catalog.product_group WHERE tenant_id = $1 AND id = $2")
            .bind(tx.tenant.as_str())
            .bind(product_id)
            .fetch_optional(&mut *tx.tx)
            .await
            .map_err(db)?;
    if exists.is_none() {
        return Ok(None);
    }
    let rows: Vec<VariantRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "{VARIANT_SELECT} WHERE p.tenant_id = $1 AND p.product_group_id = $2 ORDER BY p.id"
    )))
    .bind(tx.tenant.as_str())
    .bind(product_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    rows.into_iter()
        .map(variant_from)
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// A stock move on one offer: the level before and after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StockMove {
    pub from: i32,
    pub to: i32,
}

/// Adds `delta` units (negative to hold units) to a tracked variant's sellable stock, and keeps
/// `availability` in step when the level crosses zero. Untracked variants and variants without
/// an offer are left alone (`None`). Never goes below zero.
pub async fn move_stock(
    tx: &mut TxCx,
    variant_id: &str,
    delta: i32,
) -> Result<Option<StockMove>, ToolError> {
    let row: Option<(i32, bool)> = sqlx::query_as(
        "SELECT o.inventory_level, p.is_managed
         FROM catalog.offer o
         JOIN catalog.product p ON p.tenant_id = o.tenant_id AND p.id = o.product_id
         WHERE o.tenant_id = $1 AND o.product_id = $2 FOR UPDATE OF o",
    )
    .bind(tx.tenant.as_str())
    .bind(variant_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    let Some((from, managed)) = row else {
        return Ok(None);
    };
    if !managed || delta == 0 {
        return Ok(None);
    }
    let to = (from + delta).max(0);
    let now = tx.clock.now();
    sqlx::query(
        "UPDATE catalog.offer SET inventory_level = $3,
           availability = CASE
             WHEN $3 > 0 AND availability = 'out_of_stock' THEN 'in_stock'
             WHEN $3 = 0 AND availability = 'in_stock' THEN 'out_of_stock'
             ELSE availability END,
           sold_out_at = CASE WHEN $3 = 0 THEN $4 WHEN $3 > 0 THEN NULL ELSE sold_out_at END,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND product_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(variant_id)
    .bind(to)
    .bind(now)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    // A returned unit can bring a sold-out size back: the shoppers waiting for it hear
    // (`inventory.auto_restore.notify_restock_alerts`).
    if from == 0 && to > 0 {
        crate::restock_alert::notify_back_in_stock(tx, variant_id).await?;
    }
    Ok(Some(StockMove { from, to }))
}
