//! The `bundle` resource (docs/api/catalog.md#bundle): a product sold as a set of other variants.
//! Its components are `catalog.type_and_quantity_node` rows; its price and stock come from them
//! (`bundle.derive_price_from`, `bundle.derive_stock_from`).

pub mod create_bundle;

use acom_contract::ToolError;
use acom_store::TxCx;

use crate::db;

/// Writes a bundle's components.
pub(crate) async fn insert_components(
    tx: &mut TxCx,
    bundle_id: &str,
    components: &[(String, i32)],
) -> Result<(), ToolError> {
    for (variant_id, quantity) in components {
        sqlx::query(
            "INSERT INTO catalog.type_and_quantity_node (tenant_id, product_group_id, product_id,
               amount_of_this_good)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT DO NOTHING",
        )
        .bind(tx.tenant.as_str())
        .bind(bundle_id)
        .bind(variant_id)
        .bind(quantity)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    Ok(())
}

/// A bundle's components, `(variant_id, units)`.
pub(crate) async fn components(
    tx: &mut TxCx,
    bundle_id: &str,
) -> Result<Vec<(String, i32)>, ToolError> {
    sqlx::query_as(
        "SELECT product_id, amount_of_this_good FROM catalog.type_and_quantity_node
         WHERE tenant_id = $1 AND product_group_id = $2 ORDER BY product_id",
    )
    .bind(tx.tenant.as_str())
    .bind(bundle_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}
