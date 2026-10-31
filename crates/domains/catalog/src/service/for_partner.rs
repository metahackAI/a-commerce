//! What `partner` reads and changes in catalog: a seller's products (their review state, variants
//! and primary category), the product questions shoppers asked about them, the store's categories
//! (to check what a seller applies to sell), and taking a suspended seller's products off sale and
//! back. Partner keeps which products a seller sells; catalog keeps the products.

use std::collections::BTreeMap;

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};

use crate::neighbors;

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// One product a seller sells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SellerProduct {
    /// The product (`prd_…`).
    pub product_id: String,
    pub name: String,
    /// `approval_status` word: `draft`, `pending_review`, `active`, `rejected`, `discontinued`.
    /// Shoppers see only `active` products.
    pub approval_status: String,
    /// Its variants (`var_…`), which order lines name.
    pub variant_ids: Vec<String>,
    /// Its primary standard category (`cat_…`), when merchandising placed it.
    pub category_id: Option<String>,
}

/// The products with these ids that exist, in id order.
pub async fn products(
    tx: &mut TxCx,
    product_ids: &[String],
) -> Result<Vec<SellerProduct>, ToolError> {
    if product_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(String, String, String, Vec<String>)> = sqlx::query_as(
        "SELECT g.id, g.name, g.approval_status,
                COALESCE(array_agg(p.id ORDER BY p.id) FILTER (WHERE p.id IS NOT NULL), '{}')
         FROM catalog.product_group g
         LEFT JOIN catalog.product p ON p.tenant_id = g.tenant_id AND p.product_group_id = g.id
         WHERE g.tenant_id = $1 AND g.id = ANY($2)
         GROUP BY g.id, g.name, g.approval_status
         ORDER BY g.id",
    )
    .bind(tx.tenant.as_str())
    .bind(product_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    let mut primary: BTreeMap<String, String> = BTreeMap::new();
    for link in neighbors::categories(tx, Some(product_ids)).await? {
        if link.is_primary && link.direct {
            primary.insert(link.product_id, link.category_id);
        }
    }
    Ok(rows
        .into_iter()
        .map(
            |(product_id, name, approval_status, variant_ids)| SellerProduct {
                category_id: primary.get(&product_id).cloned(),
                product_id,
                name,
                approval_status,
                variant_ids,
            },
        )
        .collect())
}

/// Product questions about some products.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuestionCounts {
    /// Questions asked in the period.
    pub asked: i64,
    /// Of those, the ones that have an answer.
    pub answered: i64,
    /// Questions without an answer now, whenever they were asked (hidden ones left out).
    pub waiting: i64,
}

/// Questions about these products: asked and answered in `[from, until)`, and all still waiting.
pub async fn questions(
    tx: &mut TxCx,
    product_ids: &[String],
    from: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Result<QuestionCounts, ToolError> {
    if product_ids.is_empty() {
        return Ok(QuestionCounts::default());
    }
    let (asked, answered, waiting): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE q.date_created >= $3 AND q.date_created < $4),
                count(a.id) FILTER (WHERE q.date_created >= $3 AND q.date_created < $4),
                count(*) FILTER (WHERE a.id IS NULL AND q.creative_work_status = 'visible')
         FROM catalog.question q
         LEFT JOIN catalog.answer a ON a.tenant_id = q.tenant_id AND a.question_id = q.id
         WHERE q.tenant_id = $1 AND q.product_group_id = ANY($2)",
    )
    .bind(tx.tenant.as_str())
    .bind(product_ids)
    .bind(from)
    .bind(until)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(QuestionCounts {
        asked,
        answered,
        waiting,
    })
}

/// One category of the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: String,
    pub name: String,
    /// `standard` or `display`.
    pub term_set: String,
}

/// Every category of the store; empty when merchandising is not wired in (unit tests).
pub async fn categories(tx: &mut TxCx) -> Result<Vec<Category>, ToolError> {
    Ok(neighbors::category_terms(tx)
        .await?
        .into_iter()
        .map(|t| Category {
            id: t.id,
            name: t.name,
            term_set: t.term_set,
        })
        .collect())
}

/// Takes these products off sale (`active` → `draft`, so shoppers no longer see them) and
/// returns the ones that moved; products in any other state are left as they are.
pub async fn take_off_sale(
    tx: &mut TxCx,
    product_ids: &[String],
) -> Result<Vec<String>, ToolError> {
    move_status(tx, product_ids, "active", "draft").await
}

/// Puts back on sale the products [`take_off_sale`] moved (`draft` → `active`) and returns the
/// ones that moved; a product someone changed in between is left as it is.
pub async fn put_back_on_sale(
    tx: &mut TxCx,
    product_ids: &[String],
) -> Result<Vec<String>, ToolError> {
    move_status(tx, product_ids, "draft", "active").await
}

async fn move_status(
    tx: &mut TxCx,
    product_ids: &[String],
    from: &str,
    to: &str,
) -> Result<Vec<String>, ToolError> {
    if product_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_scalar(
        "UPDATE catalog.product_group SET approval_status = $4, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = ANY($2) AND approval_status = $3
         RETURNING id",
    )
    .bind(tx.tenant.as_str())
    .bind(product_ids)
    .bind(from)
    .bind(to)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
    .map(|mut ids: Vec<String>| {
        ids.sort();
        ids
    })
}
