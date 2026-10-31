//! What the cart, checkout and order placement use from promotion: the benefit calculation
//! (`calculate_benefits` itself, so every surface shows the same numbers), redeeming the coupons
//! a placed order records, and the slots where sales installs its cart reader for the tool and
//! its order readers (`provide_order_history`).

pub use crate::benefit::calc::Rules;
pub use crate::benefit::calc::{
    Breakdown, DiscountLine, ItemBenefit, Line, LineResult, RecordedCoupon, RuleViolation, money,
};
pub use crate::benefit::{
    BenefitCart, CartReader, NOT_ELIGIBLE, Reading, Request, breakdown_json, provide_cart_reader,
    rules,
};

use std::sync::OnceLock;

use acom_contract::{ErrorCode, ToolError};
use acom_core::Money;
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use serde_json::json;

/// Runs `calculate_benefits` for priced lines inside the caller's transaction.
pub async fn calculate(tx: &mut TxCx, request: &Request) -> Result<Breakdown, ToolError> {
    crate::benefit::calculate(tx, request).await
}

/// Marks the coupons a placed order records as used on it. Returns the member coupon ids.
/// Safe to call again for the same order.
pub async fn redeem_coupons(
    tx: &mut TxCx,
    customer_id: &str,
    order_id: &str,
    recorded: &[RecordedCoupon],
) -> Result<Vec<String>, ToolError> {
    crate::benefit::redeem(tx, customer_id, order_id, recorded).await
}

/// Promotion names by id, for orders placed before their discounts were recorded in full.
pub async fn promotion_names(
    tx: &mut TxCx,
    ids: &[String],
) -> Result<Vec<(String, String)>, ToolError> {
    crate::benefit::repo::promotion_names(tx, ids).await
}

/// A coupon recorded on an order: its code and promotion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CouponFacts {
    pub coupon_id: String,
    pub code: String,
    pub promotion_id: String,
}

/// The code and promotion of these coupons.
pub async fn coupon_facts(tx: &mut TxCx, ids: &[String]) -> Result<Vec<CouponFacts>, ToolError> {
    Ok(crate::benefit::repo::coupon_facts(tx, ids)
        .await?
        .into_iter()
        .map(|(coupon_id, code, promotion_id)| CouponFacts {
            coupon_id,
            code,
            promotion_id,
        })
        .collect())
}

/// One order as promotion needs it: who placed it, when, for how much, and what it holds. Used
/// by `redeem_coupon` (whose coupon, what the order recorded) and `enter_event` (an order that
/// enters a purchase-triggered event).
#[derive(Debug, Clone, PartialEq)]
pub struct OrderFacts {
    pub order_id: String,
    /// The member who placed it; `None` for a guest order.
    pub customer_id: Option<String>,
    pub order_date: DateTime<Utc>,
    /// `order_status` word.
    pub order_status: String,
    /// `sales_channel` word.
    pub sales_channel: String,
    /// Item subtotal before member benefits.
    pub subtotal: Money,
    /// Every discount recorded on the order.
    pub discount: Money,
    /// The coupon the order recorded, if any.
    pub coupon_id: Option<String>,
    /// The variants (`var_…`) on the order that were not cancelled.
    pub variant_ids: Vec<String>,
}

/// What one member bought over every order that was not cancelled. Used by segments.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberPurchases {
    pub customer_id: String,
    pub last_order_at: DateTime<Utc>,
    /// What they paid, net of returned and cancelled items.
    pub spend: Money,
    /// Every variant (`var_…`) they bought and did not cancel or return.
    pub variant_ids: Vec<String>,
}

/// The orders among the ids that exist.
pub type OrdersReader = for<'a> fn(&'a mut TxCx, &'a [String]) -> Reading<'a, Vec<OrderFacts>>;

/// Every member's purchases.
pub type PurchasesReader = for<'a> fn(&'a mut TxCx) -> Reading<'a, Vec<MemberPurchases>>;

static ORDERS: OnceLock<OrdersReader> = OnceLock::new();
static PURCHASES: OnceLock<PurchasesReader> = OnceLock::new();

/// Sales owns orders and depends on promotion, so it hands its order readers over once at start
/// (`acom-registry` does it after registering both domains). Later calls are ignored.
pub fn provide_order_history(orders: OrdersReader, purchases: PurchasesReader) {
    let _ = ORDERS.set(orders);
    let _ = PURCHASES.set(purchases);
}

fn not_connected() -> ToolError {
    ToolError::new(
        ErrorCode::UPSTREAM_UNAVAILABLE,
        "Order history is not connected to promotions; try again later.",
    )
    .detail(json!({"upstream": "sales", "retryable": true}))
}

/// The orders among `order_ids`, read through sales.
pub(crate) async fn orders(
    tx: &mut TxCx,
    order_ids: &[String],
) -> Result<Vec<OrderFacts>, ToolError> {
    let f = ORDERS.get().copied().ok_or_else(not_connected)?;
    f(tx, order_ids).await
}

/// Every member's purchases, read through sales.
pub(crate) async fn purchases(tx: &mut TxCx) -> Result<Vec<MemberPurchases>, ToolError> {
    let f = PURCHASES.get().copied().ok_or_else(not_connected)?;
    f(tx).await
}
