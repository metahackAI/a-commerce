//! What catalog reads need from domains that sit above it in the crate graph.
//!
//! Catalog is the bottom of the domain graph: promotion, merchandising, and sales depend on it,
//! so it cannot call them. A product page still needs the member price (promotion), the
//! category (merchandising), and, to accept a review, the shopper's purchase (sales). Those
//! domains export plain service functions (`service::for_catalog`); a crate above all of them
//! (`acom-search`) wires them into catalog once at start-up with [`install`]. Until then the
//! answers are empty: no member prices, no categories, no purchases.

use std::sync::OnceLock;

use acom_contract::ToolError;
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use rust_decimal::Decimal;

/// A member or time-boxed sale price of one offer (`promotion.unit_price_specification`).
#[derive(Debug, Clone, PartialEq)]
pub struct SalePrice {
    pub offer_id: String,
    pub price: Decimal,
    pub price_currency: String,
    /// `None` means everyone.
    pub valid_for_member_tier: Option<String>,
    pub promotion_id: Option<String>,
    /// The promotion's name, for example "VIP weekend".
    pub name: Option<String>,
    pub valid_through: Option<DateTime<Utc>>,
}

/// A product's place in a category (`merchandising.product_group_category`), or in one of its
/// broader categories (`direct == false`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryLink {
    pub product_id: String,
    pub category_id: String,
    pub name: String,
    pub is_primary: bool,
    pub direct: bool,
}

/// One category of the store (`merchandising.defined_term`), to check and match a product's
/// category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryTerm {
    pub id: String,
    pub name: String,
    pub term_code: String,
    /// `standard` or `display`.
    pub term_set: String,
    /// `false` when it or a category above it is inactive.
    pub active: bool,
}

/// One line of a shopper's order (`sales.order_item`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PurchasedItem {
    pub order_item_id: String,
    pub variant_id: String,
    /// An `order_status` word.
    pub order_item_status: String,
}

/// The lookups catalog makes in other domains. Every method runs in the caller's transaction.
pub trait Neighbors: Send + Sync + 'static {
    /// The member tier of a customer, or `None` when they have none.
    fn customer_tier<'a>(
        &'a self,
        tx: &'a mut TxCx,
        customer_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, ToolError>>;

    /// Whether the store has a member tier with this id.
    fn tier_exists<'a>(
        &'a self,
        tx: &'a mut TxCx,
        tier: &'a str,
    ) -> BoxFuture<'a, Result<bool, ToolError>>;

    /// Sale prices of these offers in effect at `at` for everyone or for `tier`.
    fn sale_prices<'a>(
        &'a self,
        tx: &'a mut TxCx,
        offer_ids: &'a [String],
        tier: Option<&'a str>,
        at: DateTime<Utc>,
    ) -> BoxFuture<'a, Result<Vec<SalePrice>, ToolError>>;

    /// The categories of these products (every product when `None`), with the broader
    /// categories above them.
    fn categories<'a>(
        &'a self,
        tx: &'a mut TxCx,
        product_ids: Option<&'a [String]>,
    ) -> BoxFuture<'a, Result<Vec<CategoryLink>, ToolError>>;

    /// Products in a category or any category under it; `None` when there is no such category.
    fn category_products<'a>(
        &'a self,
        tx: &'a mut TxCx,
        category_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<Vec<String>>, ToolError>>;

    /// The lines of one order, when it belongs to this customer.
    fn purchased_items<'a>(
        &'a self,
        tx: &'a mut TxCx,
        customer_id: &'a str,
        order_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<PurchasedItem>, ToolError>>;

    /// Every category of the store, both term sets.
    fn category_terms<'a>(
        &'a self,
        tx: &'a mut TxCx,
    ) -> BoxFuture<'a, Result<Vec<CategoryTerm>, ToolError>>;

    /// Places a product in a standard category as its primary category, replacing the primary
    /// placement it had.
    fn place_product<'a>(
        &'a self,
        tx: &'a mut TxCx,
        product_id: &'a str,
        category_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ToolError>>;

    /// Takes a product out of every category (undoing a product that is being removed).
    fn remove_placements<'a>(
        &'a self,
        tx: &'a mut TxCx,
        product_id: &'a str,
    ) -> BoxFuture<'a, Result<(), ToolError>>;
}

static NEIGHBORS: OnceLock<Box<dyn Neighbors>> = OnceLock::new();

/// Sets the lookups once. Later calls keep the first ones (registries are built more than once
/// in tests).
pub fn install(n: impl Neighbors) {
    let _ = NEIGHBORS.set(Box::new(n));
}

fn get() -> Option<&'static dyn Neighbors> {
    NEIGHBORS.get().map(|b| b.as_ref())
}

pub(crate) async fn customer_tier(
    tx: &mut TxCx,
    customer_id: &str,
) -> Result<Option<String>, ToolError> {
    match get() {
        Some(n) => n.customer_tier(tx, customer_id).await,
        None => Ok(None),
    }
}

/// `true` when tiers cannot be checked (nothing installed), so the caller's word is taken.
pub(crate) async fn tier_exists(tx: &mut TxCx, tier: &str) -> Result<bool, ToolError> {
    match get() {
        Some(n) => n.tier_exists(tx, tier).await,
        None => Ok(true),
    }
}

pub(crate) async fn sale_prices(
    tx: &mut TxCx,
    offer_ids: &[String],
    tier: Option<&str>,
    at: DateTime<Utc>,
) -> Result<Vec<SalePrice>, ToolError> {
    match get() {
        Some(n) if !offer_ids.is_empty() => n.sale_prices(tx, offer_ids, tier, at).await,
        _ => Ok(Vec::new()),
    }
}

pub(crate) async fn categories(
    tx: &mut TxCx,
    product_ids: Option<&[String]>,
) -> Result<Vec<CategoryLink>, ToolError> {
    match get() {
        Some(n) => n.categories(tx, product_ids).await,
        None => Ok(Vec::new()),
    }
}

pub(crate) async fn category_products(
    tx: &mut TxCx,
    category_id: &str,
) -> Result<Option<Vec<String>>, ToolError> {
    match get() {
        Some(n) => n.category_products(tx, category_id).await,
        None => Ok(None),
    }
}

pub(crate) async fn purchased_items(
    tx: &mut TxCx,
    customer_id: &str,
    order_id: &str,
) -> Result<Vec<PurchasedItem>, ToolError> {
    match get() {
        Some(n) => n.purchased_items(tx, customer_id, order_id).await,
        None => Ok(Vec::new()),
    }
}

pub(crate) async fn category_terms(tx: &mut TxCx) -> Result<Vec<CategoryTerm>, ToolError> {
    match get() {
        Some(n) => n.category_terms(tx).await,
        None => Ok(Vec::new()),
    }
}

fn not_wired() -> ToolError {
    ToolError::new(
        acom_contract::ErrorCode::UPSTREAM_UNAVAILABLE,
        "Categories cannot be changed here: the merchandising lookups are not installed.",
    )
}

pub(crate) async fn place_product(
    tx: &mut TxCx,
    product_id: &str,
    category_id: &str,
) -> Result<(), ToolError> {
    match get() {
        Some(n) => n.place_product(tx, product_id, category_id).await,
        None => Err(not_wired()),
    }
}

pub(crate) async fn remove_placements(tx: &mut TxCx, product_id: &str) -> Result<(), ToolError> {
    match get() {
        Some(n) => n.remove_placements(tx, product_id).await,
        None => Err(not_wired()),
    }
}
