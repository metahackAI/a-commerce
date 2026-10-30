//! What catalog and search need from merchandising: where products are placed (with the
//! broader categories above), and which products a category holds.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// A product's place in a category, or in a broader category above it (`direct == false`).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Placement {
    pub product_group_id: String,
    pub category_id: String,
    pub name: String,
    pub is_primary: bool,
    pub direct: bool,
}

/// Placements of these products (every product when `None`) in active categories, each direct
/// placement followed up its broader categories. Category trees are a few levels deep; the walk
/// stops after 16 steps in case of a loop.
pub async fn placements(
    tx: &mut TxCx,
    product_ids: Option<&[String]>,
) -> Result<Vec<Placement>, ToolError> {
    sqlx::query_as::<_, Placement>(
        "WITH RECURSIVE up AS (
           SELECT c.product_group_id, c.category_id, c.is_primary, true AS direct, 0 AS depth
           FROM merchandising.product_group_category c
           WHERE c.tenant_id = $1 AND ($2::text[] IS NULL OR c.product_group_id = ANY($2::text[]))
           UNION ALL
           SELECT up.product_group_id, t.broader_id, false, false, up.depth + 1
           FROM up
           JOIN merchandising.defined_term t ON t.tenant_id = $1 AND t.id = up.category_id
           WHERE t.broader_id IS NOT NULL AND up.depth < 16)
         SELECT DISTINCT ON (up.product_group_id, up.category_id)
                up.product_group_id, up.category_id, d.name, up.is_primary, up.direct
         FROM up
         JOIN merchandising.defined_term d ON d.tenant_id = $1 AND d.id = up.category_id
         WHERE d.is_active
         ORDER BY up.product_group_id, up.category_id, up.direct DESC, up.is_primary DESC",
    )
    .bind(tx.tenant.as_str())
    .bind(product_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Products placed in this category or any category under it; `None` when there is no such
/// category.
pub async fn products_in(
    tx: &mut TxCx,
    category_id: &str,
) -> Result<Option<Vec<String>>, ToolError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM merchandising.defined_term WHERE tenant_id = $1 AND id = $2)",
    )
    .bind(tx.tenant.as_str())
    .bind(category_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    if !exists {
        return Ok(None);
    }
    let ids = sqlx::query_scalar(
        "WITH RECURSIVE down AS (
           SELECT id, 0 AS depth FROM merchandising.defined_term WHERE tenant_id = $1 AND id = $2
           UNION ALL
           SELECT t.id, down.depth + 1
           FROM merchandising.defined_term t JOIN down ON t.broader_id = down.id
           WHERE t.tenant_id = $1 AND down.depth < 16)
         SELECT DISTINCT c.product_group_id
         FROM merchandising.product_group_category c
         WHERE c.tenant_id = $1 AND c.category_id IN (SELECT id FROM down)",
    )
    .bind(tx.tenant.as_str())
    .bind(category_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(Some(ids))
}

/// One category with whether it is active (an inactive category above makes it inactive too).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Term {
    pub id: String,
    pub name: String,
    pub term_code: String,
    /// `standard` or `display`.
    pub term_set: String,
    pub active: bool,
}

/// Every category of the store, with inactive parents taken into account
/// (`category.child_inherits_inactive_parent`). Trees are a few levels deep; the walk stops
/// after 16 steps in case of a loop.
pub async fn terms(tx: &mut TxCx) -> Result<Vec<Term>, ToolError> {
    sqlx::query_as::<_, Term>(
        "WITH RECURSIVE up AS (
           SELECT id AS start, id, broader_id, is_active, 0 AS depth
           FROM merchandising.defined_term WHERE tenant_id = $1
           UNION ALL
           SELECT up.start, t.id, t.broader_id, t.is_active, up.depth + 1
           FROM up JOIN merchandising.defined_term t ON t.tenant_id = $1 AND t.id = up.broader_id
           WHERE up.depth < 16)
         SELECT d.id, d.name, d.term_code, d.in_defined_term_set AS term_set,
                bool_and(up.is_active) AS active
         FROM merchandising.defined_term d JOIN up ON up.start = d.id
         WHERE d.tenant_id = $1
         GROUP BY d.id, d.name, d.term_code, d.in_defined_term_set, d.position
         ORDER BY d.in_defined_term_set DESC, d.position, d.id",
    )
    .bind(tx.tenant.as_str())
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Makes `category_id` the product's primary category: the old primary placement is replaced,
/// other placements stay.
pub async fn place_product(
    tx: &mut TxCx,
    product_id: &str,
    category_id: &str,
) -> Result<(), ToolError> {
    sqlx::query(
        "DELETE FROM merchandising.product_group_category
         WHERE tenant_id = $1 AND product_group_id = $2 AND (is_primary OR category_id = $3)",
    )
    .bind(tx.tenant.as_str())
    .bind(product_id)
    .bind(category_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    sqlx::query(
        "INSERT INTO merchandising.product_group_category
           (tenant_id, product_group_id, category_id, is_primary)
         VALUES ($1, $2, $3, true)",
    )
    .bind(tx.tenant.as_str())
    .bind(product_id)
    .bind(category_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Takes a product out of every category.
pub async fn remove_placements(tx: &mut TxCx, product_id: &str) -> Result<(), ToolError> {
    sqlx::query(
        "DELETE FROM merchandising.product_group_category
         WHERE tenant_id = $1 AND product_group_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(product_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}
