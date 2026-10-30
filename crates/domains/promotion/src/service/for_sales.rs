//! What the cart, checkout and order placement use from promotion: the benefit calculation
//! (`calculate_benefits` itself, so every surface shows the same numbers), redeeming the coupons
//! a placed order records, and the slot where sales installs its cart reader for the tool.

pub use crate::benefit::calc::Rules;
pub use crate::benefit::calc::{
    Breakdown, DiscountLine, ItemBenefit, Line, LineResult, RecordedCoupon, RuleViolation, money,
};
pub use crate::benefit::{
    BenefitCart, CartReader, NOT_ELIGIBLE, Reading, Request, breakdown_json, provide_cart_reader,
    rules,
};

use acom_contract::ToolError;
use acom_store::TxCx;

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
