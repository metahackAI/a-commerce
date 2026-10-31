//! The `product` resource (docs/api/catalog.md#product): what its operations share. The variant
//! helpers (`VariantOptions`, the floor, SKU and gift-card checks, approval) serve `create_product`,
//! `update_product` and `set_variants`: the variants (sizes and colors, one SKU each) of a product,
//! with their prices.

pub mod batch_import;
pub mod create_product;
pub mod discontinue_product;
pub mod get_delivery_policy;
pub mod get_product;
pub mod get_product_measurements;
pub mod get_return_policy;
pub mod set_variants;
pub mod update_product;

use acom_contract::{ApprovalNeed, Check, ErrorClass, ErrorCode, ToolError};
use acom_core::enums::SaleType;
use acom_core::{Currency, Money};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::money_text;
use crate::rules::PriceFloorRule;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProductIdInput {
    /// The product.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_wool_overcoat")]
    pub product_id: String,
}

fn words(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// More option axes than the store allows (`variant.max_option_axes`).
pub const TOO_MANY_OPTION_AXES: ErrorCode =
    ErrorCode::new("too_many_option_axes", ErrorClass::LimitExceeded);
/// More variants than one product may have (`variant.max_variants_per_group`).
pub const TOO_MANY_VARIANTS: ErrorCode =
    ErrorCode::new("too_many_variants", ErrorClass::LimitExceeded);
/// A variant names an option value the product's axes do not have.
pub const INCOMPLETE_COMBINATION: ErrorCode =
    ErrorCode::new("incomplete_combination", ErrorClass::Domain);
/// The sale type cannot be used this way.
pub const INVALID_SALE_TYPE: ErrorCode =
    ErrorCode::new("invalid_sale_type", ErrorClass::InvalidInput);

/// The color and size of one variant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariantOptions {
    /// Color name, for example Slate. Leave out for a one-color product.
    #[schemars(length(min = 1, max = 60), example = &"Slate")]
    pub color: Option<String>,
    /// Size word, for example M, 32 or 9. Leave out for a one-size product.
    #[schemars(length(min = 1, max = 20), example = &"M")]
    pub size: Option<String>,
}

/// A variant's list price.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OfferInput {
    /// List price in major units, for example 89.00.
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    #[schemars(with = "f64", example = 89.00)]
    pub price: rust_decimal::Decimal,
    /// ISO 4217 code of `price`; must be the store's currency, which is the default.
    pub price_currency: Option<Currency>,
}

/// A variant's unit cost.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CostInput {
    /// What one unit costs the store, in major units, for example 36.00.
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    #[schemars(with = "f64", example = 36.00)]
    pub price: rust_decimal::Decimal,
    /// ISO 4217 code of the cost; must be the store's currency, which is the default.
    pub price_currency: Option<Currency>,
}

fn sale_word(s: Option<SaleType>) -> String {
    s.unwrap_or(SaleType::Always).as_str().to_owned()
}

fn variant_name(product: &str, o: &VariantOptions) -> String {
    let mut parts = vec![product.to_owned()];
    parts.extend(o.color.clone());
    parts.extend(o.size.clone());
    parts.join(", ")
}

fn money_in(
    cx: &Cx,
    field: &str,
    value: rust_decimal::Decimal,
    currency: Option<Currency>,
) -> Result<Money, ToolError> {
    let currency = currency.unwrap_or(cx.currency);
    if value.is_sign_negative() {
        return Err(
            ToolError::invalid_input(format!("`{field}` cannot be negative."))
                .detail(json!({"field": field, "reason": "range"})),
        );
    }
    if currency != cx.currency {
        return Err(ToolError::invalid_input(format!(
            "This store sells in {}, not {currency}.",
            cx.currency
        ))
        .detail(json!({"field": field, "reason": "enum", "allowed": [cx.currency.to_string()]})));
    }
    Ok(Money::new(value, currency).rounded())
}

fn floor_check(rule: &PriceFloorRule, priced: &[(String, Money, Option<Money>)]) -> Check {
    let below: Vec<String> = priced
        .iter()
        .filter_map(|(sku, price, cost)| {
            let cost = (*cost)?;
            (!rule.allows(*price, cost)).then(|| {
                format!(
                    "{sku} at {} is under the floor {}",
                    money_text(price),
                    money_text(&rule.floor(cost))
                )
            })
        })
        .collect();
    let with_cost = priced.iter().filter(|p| p.2.is_some()).count();
    Check {
        rule: "price_floor".into(),
        passed: below.is_empty(),
        detail: Some(if !below.is_empty() {
            below.join("; ")
        } else if with_cost == 0 {
            "No cost on file, so there is no floor to check".into()
        } else {
            format!(
                "{with_cost} of {with_cost} prices at or above the floor ({})",
                rule.markup_text()
            )
        }),
        fix_tool: (!below.is_empty()).then(|| "set_price".into()),
    }
}

fn sku_check(taken: &[String], skus: usize) -> Check {
    Check {
        rule: "product.sku_unique".into(),
        passed: taken.is_empty(),
        detail: Some(if taken.is_empty() {
            format!(
                "{skus} SKU{} not used elsewhere in the store",
                if skus == 1 { "" } else { "s" }
            )
        } else {
            format!("Already used in the store: {}", taken.join(", "))
        }),
        fix_tool: (!taken.is_empty()).then(|| "get_product".into()),
    }
}

fn gift_check(p: &acom_policy::PolicySet, sale_types: &[String]) -> Check {
    let disallowed: Vec<String> = p
        .lookup("catalog.gift_card.disallow")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let gift = sale_types.iter().any(|t| t == "gift_card");
    let clash: Vec<&String> = sale_types
        .iter()
        .filter(|t| disallowed.contains(t))
        .collect();
    let ok = !gift || clash.is_empty();
    Check {
        rule: "gift_card.disallow".into(),
        passed: ok,
        detail: Some(if !gift {
            "No gift card variants".into()
        } else if ok {
            "Gift cards are sold on their own".into()
        } else {
            "A gift card cannot be sold with pre-order or subscription variants".into()
        }),
        fix_tool: (!ok).then(|| "update_product".into()),
    }
}

fn customer_facing(status: &str) -> bool {
    matches!(status, "active" | "pending_review")
}

fn approval_from(cx: &Cx, tool: &str, structural: bool, reason: String) -> ApprovalNeed {
    let a = cx.policy.approval_for(
        tool,
        &json!({"change_type": if structural { "structural" } else { "content" }, "new_product": false}),
    );
    if a.needed {
        ApprovalNeed::Needed { reason }
    } else {
        ApprovalNeed::NotNeeded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_floors_of_new_variants() {
        let o = VariantOptions {
            color: Some("Sand".into()),
            size: Some("M".into()),
        };
        assert_eq!(
            variant_name("Packable windbreaker", &o),
            "Packable windbreaker, Sand, M"
        );
        let rule = PriceFloorRule::from_policy(&crate::testing::policies()).unwrap();
        let usd = |s: &str| Money::new(s.parse().unwrap(), Currency::USD);
        let c = floor_check(
            &rule,
            &[("WB-SAND-M".into(), usd("40.00"), Some(usd("36.00")))],
        );
        assert!(!c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("WB-SAND-M at $40.00 is under the floor $41.40")
        );
        assert!(floor_check(&rule, &[("X".into(), usd("40.00"), None)]).passed);
    }
}
