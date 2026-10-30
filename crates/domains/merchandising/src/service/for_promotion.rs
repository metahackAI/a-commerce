//! What the promotion domain reads from merchandising: the products placed in a category or
//! any category under it (a sale on "summer outerwear", a promotion scoped to knitwear).
//!
//! Merchandising depends on promotion, so promotion cannot call this directly; `register`
//! hands [`lookup`] to promotion (`provide_category_products`).

use std::future::Future;
use std::pin::Pin;

use acom_contract::ToolError;
use acom_promotion::service::for_merchandising::CategoryProducts;
use acom_store::TxCx;

use crate::repo;

/// The products of each category among `category_ids` that exists, in placement order.
pub async fn category_products(
    tx: &mut TxCx,
    category_ids: &[String],
) -> Result<Vec<CategoryProducts>, ToolError> {
    let names = repo::category_names(tx, category_ids).await?;
    let placed = repo::placed_products(tx, category_ids).await?;
    Ok(category_ids
        .iter()
        .filter_map(|id| {
            let name = names.iter().find(|(n, _)| n == id)?.1.clone();
            Some(CategoryProducts {
                category_id: id.clone(),
                name,
                product_ids: placed
                    .iter()
                    .filter(|(root, _)| root == id)
                    .map(|(_, p)| p.clone())
                    .collect(),
            })
        })
        .collect())
}

/// [`category_products`] in the shape promotion stores.
pub fn lookup<'a>(
    tx: &'a mut TxCx,
    category_ids: &'a [String],
) -> Pin<Box<dyn Future<Output = Result<Vec<CategoryProducts>, ToolError>> + Send + 'a>> {
    Box::pin(category_products(tx, category_ids))
}
