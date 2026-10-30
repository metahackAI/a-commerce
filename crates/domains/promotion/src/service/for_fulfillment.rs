//! What `fulfillment` reads and changes in promotion: the coupons an order used, and putting
//! them back in the member's wallet when a cancel restores them (`fulfillment.yml`
//! `cancel.restore`).

use acom_contract::{ErrorCode, ToolError};
use acom_core::{Currency, Money};
use acom_store::{StoreError, TxCx};
use rust_decimal::Decimal;

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

fn money(value: Option<Decimal>, currency: Option<String>) -> Result<Option<Money>, ToolError> {
    match (value, currency) {
        (Some(v), Some(c)) => Currency::parse(c.trim())
            .map(|c| Some(Money::new(v, c)))
            .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string())),
        _ => Ok(None),
    }
}

/// A coupon a member used on an order.
#[derive(Debug, Clone, PartialEq)]
pub struct UsedCoupon {
    /// The issued coupon (`mcp_…`).
    pub member_coupon_id: String,
    /// The coupon definition (`cpn_…`).
    pub coupon_id: String,
    pub discount_code: String,
    pub promotion_id: String,
    /// `item_price` (an item coupon), `order_discount` (an order coupon) or `add_on`.
    pub discount_layer: String,
    /// Discount recorded on the order when it was used.
    pub benefit: Option<Money>,
    /// The promotion's minimum item subtotal, when it has one.
    pub min_subtotal: Option<Money>,
    pub version: i32,
}

/// The coupons used on an order and not yet restored.
pub async fn coupons_used(tx: &mut TxCx, order_id: &str) -> Result<Vec<UsedCoupon>, ToolError> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        String,
        String,
        String,
        String,
        String,
        Option<Decimal>,
        Option<String>,
        Option<Decimal>,
        Option<String>,
        i32,
    )> = sqlx::query_as(
        "SELECT m.id, m.coupon_id, c.discount_code, c.promotion_id, p.discount_layer,
                m.benefit_recorded_value, m.benefit_recorded_currency::text,
                p.eligible_transaction_volume_value, p.eligible_transaction_volume_currency::text,
                m.version
         FROM promotion.member_coupon m
         JOIN promotion.coupon c ON c.tenant_id = m.tenant_id AND c.id = m.coupon_id
         JOIN promotion.promotion p ON p.tenant_id = c.tenant_id AND p.id = c.promotion_id
         WHERE m.tenant_id = $1 AND m.order_id = $2 AND m.coupon_status = 'used'
         ORDER BY m.id",
    )
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    rows.into_iter()
        .map(|r| {
            Ok(UsedCoupon {
                member_coupon_id: r.0,
                coupon_id: r.1,
                discount_code: r.2,
                promotion_id: r.3,
                discount_layer: r.4,
                benefit: money(r.5, r.6)?,
                min_subtotal: money(r.7, r.8)?,
                version: r.9,
            })
        })
        .collect()
}

/// Puts used coupons back in their holders' wallets (`used` → `restored`). Coupons that are no
/// longer `used` are left alone, so running it twice changes nothing. Returns the ids restored.
pub async fn restore_coupons(
    tx: &mut TxCx,
    member_coupon_ids: &[String],
) -> Result<Vec<String>, ToolError> {
    if member_coupon_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_scalar(
        "UPDATE promotion.member_coupon SET coupon_status = 'restored', updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = ANY($2) AND coupon_status = 'used'
         RETURNING id",
    )
    .bind(tx.tenant.as_str())
    .bind(member_coupon_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}
