//! The `offer` resource (docs/api/promotion.md#offer): tier and period sale prices. What its
//! operations share: the policy error codes of a sale, input checks, and the promotions an
//! overlap check compares with.

pub(crate) mod set_sale_prices;

use std::collections::{BTreeMap, BTreeSet};

use acom_contract::{Basis, ErrorClass, ErrorCode, ToolError};
use acom_core::enums::DiscountLayer;
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::json;

use crate::model::{AppliesTo, OtherPromotion, SaleRules, promotion_status};
use crate::repo::{self, PromotionFilter};
use crate::service::for_merchandising::category_products;

pub const BELOW_PRICE_FLOOR: ErrorCode =
    ErrorCode::new("below_price_floor", ErrorClass::PolicyViolation);
pub const OVER_MAX_DISCOUNT: ErrorCode =
    ErrorCode::new("over_max_discount", ErrorClass::PolicyViolation);
pub const LOW_STOCK: ErrorCode = ErrorCode::new("low_stock", ErrorClass::PolicyViolation);
pub const PROMOTION_OVERLAP: ErrorCode =
    ErrorCode::new("promotion_overlap", ErrorClass::PolicyViolation);

fn field_error(field: &str, reason: &str, message: impl Into<String>) -> ToolError {
    ToolError::invalid_input(message).detail(json!({"field": field, "reason": reason}))
}

/// A percent input: above 0, below 100, at most two decimals.
fn valid_percent(field: &str, p: Decimal) -> Result<Decimal, ToolError> {
    if p <= Decimal::ZERO || p >= Decimal::ONE_HUNDRED || p.normalize().scale() > 2 {
        return Err(field_error(
            field,
            "range",
            format!("`{field}` must be above 0 and below 100, with at most two decimals; got {p}."),
        ));
    }
    Ok(p.normalize())
}

/// The promotions in the compared statuses whose period overlaps, with the products each covers.
async fn others(
    tx: &mut TxCx,
    rules: &SaleRules,
    from: DateTime<Utc>,
    through: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(Vec<OtherPromotion>, Vec<Basis>), ToolError> {
    let filter = PromotionFilter {
        statuses: rules
            .overlap_statuses
            .iter()
            .map(|s| s.as_str().to_owned())
            .collect(),
        from: Some(from),
        through: Some(through),
        ..PromotionFilter::default()
    };
    let (_, rows) = repo::promotions(tx, &filter, now, None).await?;
    let mut wanted: Vec<String> = rows
        .iter()
        .map(|r| AppliesTo::read(&r.applies_to))
        .filter(|a| a.scope == "categories")
        .flat_map(|a| a.category_ids)
        .collect();
    wanted.sort();
    wanted.dedup();
    let by_category: BTreeMap<String, Vec<String>> = category_products(tx, &wanted)
        .await?
        .into_iter()
        .map(|c| (c.category_id, c.product_ids))
        .collect();
    let out = rows
        .iter()
        .map(|r| as_other(r, now, &by_category))
        .collect();
    let basis = rows
        .iter()
        .map(|r| Basis {
            entity: "promotion.promotion".into(),
            id: r.id.clone(),
            version: i64::from(r.version),
        })
        .collect();
    Ok((out, basis))
}

/// Another promotion as the overlap check sees it: its status now and the products it covers
/// (`by_category` holds the products of the categories it names).
pub(crate) fn as_other(
    r: &repo::PromotionRow,
    now: DateTime<Utc>,
    by_category: &BTreeMap<String, Vec<String>>,
) -> OtherPromotion {
    let a = AppliesTo::read(&r.applies_to);
    let excluded: BTreeSet<&String> = a.exclude_product_ids.iter().collect();
    let products = match a.scope.as_str() {
        "products" => Some(
            a.product_ids
                .iter()
                .filter(|p| !excluded.contains(p))
                .cloned()
                .collect(),
        ),
        "categories" => Some(
            a.category_ids
                .iter()
                .filter_map(|c| by_category.get(c))
                .flatten()
                .filter(|p| !excluded.contains(p))
                .cloned()
                .collect(),
        ),
        _ => None,
    };
    OtherPromotion {
        id: r.id.clone(),
        name: r.name.clone(),
        status: promotion_status(&r.promotion_status, r.valid_from, r.valid_through, now),
        layer: r
            .discount_layer
            .parse()
            .unwrap_or(DiscountLayer::OrderDiscount),
        tier: r.valid_for_member_tier.clone(),
        products,
    }
}
