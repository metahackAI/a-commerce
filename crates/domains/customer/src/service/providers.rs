//! Readers that domains depending on `customer` install at start-up, so customer reads can show
//! their data without a dependency cycle: sales (orders and products, for tier progress and
//! the wishlist), promotion (what refers to a tier), fulfillment (return claims). Each reader is
//! a plain function over the caller's transaction. A read whose reader is missing leaves those
//! fields out rather than guessing.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use acom_contract::ToolError;
use acom_core::Money;
use acom_store::TxCx;
use chrono::{DateTime, Utc};

/// What a reader returns: a boxed future borrowing the transaction.
pub type Reading<'a, T> = Pin<Box<dyn Future<Output = Result<T, ToolError>> + Send + 'a>>;

/// One order of a member that counts towards tier requirements.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberOrder {
    pub order_id: String,
    pub order_number: String,
    /// What the member paid, net of returns and cancellations.
    pub amount: Money,
    pub sales_channel: String,
    /// When every line was delivered; `None` while the order is still on its way.
    pub delivered_at: Option<DateTime<Utc>>,
}

/// A member's orders placed at or after the given time.
pub type MemberOrdersReader =
    for<'a> fn(&'a mut TxCx, &'a str, DateTime<Utc>) -> Reading<'a, Vec<MemberOrder>>;

/// A product as the wishlist shows it: current price and whether it can be bought.
#[derive(Debug, Clone, PartialEq)]
pub struct ProductCard {
    pub product_id: String,
    pub name: String,
    /// The lowest list price among its variants.
    pub price: Money,
    /// `in_stock` when any variant can be bought, else the first variant's availability word.
    pub availability: String,
}

/// The given products that exist (unknown ids are left out).
pub type ProductCardsReader =
    for<'a> fn(&'a mut TxCx, &'a [String]) -> Reading<'a, Vec<ProductCard>>;

/// How many running or scheduled promotions and coupons name a tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TierReferences {
    pub active_promotions: i64,
    pub active_coupons: i64,
}

pub type TierReferencesReader = for<'a> fn(&'a mut TxCx, &'a str) -> Reading<'a, TierReferences>;

/// One return claim of a customer.
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnClaim {
    pub id: String,
    pub claim_type: String,
    pub order_id: String,
    pub order_number: String,
    /// Name of the (first) item returned.
    pub item: String,
    pub amount: Money,
    pub reason: Option<String>,
    pub status: String,
    pub requested_at: DateTime<Utc>,
}

/// A customer's return claims requested at or after the given time, newest first.
pub type ReturnClaimsReader =
    for<'a> fn(&'a mut TxCx, &'a str, DateTime<Utc>) -> Reading<'a, Vec<ReturnClaim>>;

static MEMBER_ORDERS: OnceLock<MemberOrdersReader> = OnceLock::new();
static PRODUCT_CARDS: OnceLock<ProductCardsReader> = OnceLock::new();
static TIER_REFERENCES: OnceLock<TierReferencesReader> = OnceLock::new();
static RETURN_CLAIMS: OnceLock<ReturnClaimsReader> = OnceLock::new();

/// Sales installs this (tier progress counts orders).
pub fn provide_member_orders(f: MemberOrdersReader) {
    let _ = MEMBER_ORDERS.set(f);
}

/// Sales installs this, reading products through catalog (the wishlist shows prices).
pub fn provide_product_cards(f: ProductCardsReader) {
    let _ = PRODUCT_CARDS.set(f);
}

/// Promotion installs this (`get_tier` shows what refers to the tier).
pub fn provide_tier_references(f: TierReferencesReader) {
    let _ = TIER_REFERENCES.set(f);
}

/// Fulfillment installs this (`get_return_history`).
pub fn provide_return_claims(f: ReturnClaimsReader) {
    let _ = RETURN_CLAIMS.set(f);
}

pub(crate) fn member_orders() -> Option<MemberOrdersReader> {
    MEMBER_ORDERS.get().copied()
}

pub(crate) fn product_cards() -> Option<ProductCardsReader> {
    PRODUCT_CARDS.get().copied()
}

pub(crate) fn tier_references() -> Option<TierReferencesReader> {
    TIER_REFERENCES.get().copied()
}

pub(crate) fn return_claims() -> Option<ReturnClaimsReader> {
    RETURN_CLAIMS.get().copied()
}
