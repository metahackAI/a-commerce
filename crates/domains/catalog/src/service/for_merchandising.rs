//! What the merchandising domain reads from the catalog: a short card per product (name, main
//! image, colors, stock, whether it can be sold), to check a banner's product image and to hide
//! sold-out or unsellable products in a display slot, and brand names to arrange a brand strip.
//! Plain reads over the caller's transaction.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};

/// One product as a display needs it.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ProductCard {
    /// `prd_…`.
    pub product_id: String,
    pub name: String,
    /// `approval_status` word; shoppers only see `active` products.
    pub approval_status: String,
    /// URL of the product's main image.
    pub image: Option<String>,
    /// Colors its variants come in.
    pub colors: Vec<String>,
    /// Units of all variants together.
    pub inventory_level: i64,
}

/// Cards for the products among `product_ids` that exist in this store, in no fixed order.
pub async fn product_cards(
    tx: &mut TxCx,
    product_ids: &[String],
) -> Result<Vec<ProductCard>, ToolError> {
    sqlx::query_as::<_, ProductCard>(
        "SELECT g.id AS product_id, g.name, g.approval_status, g.image,
                coalesce(array_agg(DISTINCT p.color) FILTER (WHERE p.color IS NOT NULL), '{}')
                  AS colors,
                coalesce(sum(o.inventory_level), 0)::bigint AS inventory_level
         FROM catalog.product_group g
         LEFT JOIN catalog.product p ON p.tenant_id = g.tenant_id AND p.product_group_id = g.id
         LEFT JOIN catalog.offer o ON o.tenant_id = p.tenant_id AND o.product_id = p.id
         WHERE g.tenant_id = $1 AND g.id = ANY($2)
         GROUP BY g.id, g.name, g.approval_status, g.image",
    )
    .bind(tx.tenant.as_str())
    .bind(product_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(|e| ToolError::from(StoreError::from(e)))
}

/// `(id, name)` of the brands among `brand_ids` that exist in this store.
pub async fn brand_names(
    tx: &mut TxCx,
    brand_ids: &[String],
) -> Result<Vec<(String, String)>, ToolError> {
    sqlx::query_as(
        "SELECT id, name FROM catalog.brand WHERE tenant_id = $1 AND id = ANY($2) ORDER BY id",
    )
    .bind(tx.tenant.as_str())
    .bind(brand_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(|e| ToolError::from(StoreError::from(e)))
}
