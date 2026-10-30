//! What `search` reads from the catalog: products matching filters (in the query, not after),
//! product cards with the viewer's prices, the text search ranks, and the categories that
//! catalog learns from merchandising.

use acom_contract::ToolError;
use acom_core::Money;
use acom_store::TxCx;

pub use crate::cards::ProductCard;
pub use crate::model::{Summary, Variant};
pub use crate::neighbors::{CategoryLink, SalePrice};
pub use crate::repo::ProductText;
pub use crate::viewer::Viewer;

use crate::repo::{self, Find, Order};
use crate::{cards, neighbors};

/// How products are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductSort {
    /// Best text match first (needs `ranked`).
    Relevance,
    /// Lowest list price first.
    Price,
    /// Highest list price first.
    PriceDesc,
    /// Latest release first.
    Newest,
    /// Least stock first.
    Inventory,
    /// Most stock first.
    InventoryDesc,
}

/// Which products to find. Every filter runs inside the database query, so `total` counts
/// exactly the products that match (`search.yml query.member_tier_filter_at_query_time`).
#[derive(Debug, Clone, PartialEq)]
pub struct ProductQuery<'a> {
    /// Products a text query matched, with their relevance; `None` when there is no query.
    pub ranked: Option<&'a [(String, f64)]>,
    /// Only these products (a category's); `None` for no such filter.
    pub only: Option<&'a [String]>,
    /// Products tagged with all of these.
    pub tags: &'a [String],
    /// Product availability words; empty is any.
    pub availability: &'a [String],
    pub min_inventory: Option<i64>,
    pub max_inventory: Option<i64>,
    /// List price bounds; both in the same currency.
    pub min_price: Option<Money>,
    pub max_price: Option<Money>,
    /// Only active products (shoppers).
    pub active_only: bool,
    pub sort: ProductSort,
    pub limit: i64,
    pub offset: i64,
}

/// The ids of one page of matching products, in order, and how many match in all.
pub async fn find_products(
    tx: &mut TxCx,
    q: &ProductQuery<'_>,
) -> Result<(Vec<String>, i64), ToolError> {
    let currency = q.min_price.or(q.max_price).map(|m| m.currency.to_string());
    let order = match q.sort {
        ProductSort::Relevance => Order::Relevance,
        ProductSort::Price => Order::Price,
        ProductSort::PriceDesc => Order::PriceDesc,
        ProductSort::Newest => Order::Newest,
        ProductSort::Inventory => Order::Inventory,
        ProductSort::InventoryDesc => Order::InventoryDesc,
    };
    repo::find(
        tx,
        &Find {
            ranked: q.ranked,
            only: q.only,
            tags: q.tags,
            availability: q.availability,
            min_inventory: q.min_inventory,
            max_inventory: q.max_inventory,
            min_price: q.min_price.map(|m| m.value),
            max_price: q.max_price.map(|m| m.value),
            price_currency: currency.as_deref(),
            active_only: q.active_only,
            with_drafts: false,
            brand_id: None,
            approval_status: None,
            order,
            limit: q.limit,
            offset: q.offset,
        },
    )
    .await
}

/// Product cards in the order of `ids`, priced for the viewer.
pub async fn product_cards(
    tx: &mut TxCx,
    ids: &[String],
    viewer: &Viewer,
) -> Result<Vec<ProductCard>, ToolError> {
    cards::load(tx, ids, viewer).await
}

/// The searchable text of every product (active ones only for shoppers).
pub async fn product_texts(
    tx: &mut TxCx,
    active_only: bool,
) -> Result<Vec<ProductText>, ToolError> {
    repo::product_texts(tx, active_only).await
}

/// Where products are placed, broader categories included.
pub async fn categories(
    tx: &mut TxCx,
    product_ids: Option<&[String]>,
) -> Result<Vec<CategoryLink>, ToolError> {
    neighbors::categories(tx, product_ids).await
}

/// Products in a category or under it; `None` when the category does not exist.
pub async fn category_products(
    tx: &mut TxCx,
    category_id: &str,
) -> Result<Option<Vec<String>>, ToolError> {
    neighbors::category_products(tx, category_id).await
}

/// Every brand of the store as `(id, name)`.
pub async fn brand_names(tx: &mut TxCx) -> Result<Vec<(String, String)>, ToolError> {
    let (rows, _) = repo::brands(tx, None, i64::from(i32::MAX), 0).await?;
    Ok(rows.into_iter().map(|b| (b.id, b.name)).collect())
}
