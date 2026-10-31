//! The `order` resource (`docs/api/sales.md` §order): `prepare_order`, the person-only
//! `POST /v1/sales/orders` that places it, `get_my_orders`, `get_order`, `search_orders`. What
//! they share lives here: delivery choices, the prepared order, the checkout view, and how an
//! order and its items read.

pub mod get_my_orders;
pub mod get_order;
pub mod place_order;
pub mod prepare_order;
pub mod search_orders;

use std::collections::HashMap;

use acom_catalog::service::for_sales as catalog;
use acom_contract::{Change, ChangeOp, ErrorCode, ToolError};
use acom_core::Money;
use acom_promotion::service::for_sales as promotion;
use acom_store::{Cx, TxCx};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::delivery::{self, Address};
use crate::owner::{self, Viewer};
use crate::pricing::{self, CartView, Carts, DeliveryChoice};
use crate::repo::{self, OrderRow};
use crate::rules;
use crate::{
    ADDRESS_NOT_DELIVERABLE, ADDRESS_NOT_FOUND, MIXED_ORDER_TYPE, ORDER_NOT_FOUND,
    address_not_found, cart_empty, not_signed_in,
};

/// How the shopper names where the order goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    /// A saved address (`delivery_address_id`).
    Saved,
    /// An address for this order only (`delivery_address`), not saved.
    OneTime,
}

/// A postal address given for one order.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PostalAddressInput {
    /// Who receives the parcel.
    #[schemars(length(max = 100), example = &"Jamie")]
    pub name: Option<String>,
    /// Street, number, apartment.
    #[schemars(length(min = 1, max = 200), example = &"12 Harbor St, Apt 3")]
    pub street_address: String,
    /// City.
    #[schemars(length(min = 1, max = 100), example = &"Brooklyn")]
    pub address_locality: String,
    /// State or region.
    #[schemars(length(max = 100), example = &"NY")]
    pub address_region: Option<String>,
    /// Postal code.
    #[schemars(length(min = 1, max = 20), example = &"11201")]
    pub postal_code: String,
    /// ISO 3166-1 country code.
    #[schemars(regex(pattern = r"^[A-Z]{2}$"), example = &"US")]
    pub address_country: String,
}

/// One line of a prepared order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PlannedLine {
    pub cart_line_id: String,
    pub variant_id: String,
    pub quantity: u32,
}

/// What `prepare_order` hands to the shopper's "Place order" button.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PreparedOrder {
    pub order_id: String,
    pub customer_id: String,
    pub lines: Vec<PlannedLine>,
    pub delivery: DeliveryChoice,
    pub payment_method: String,
    /// The total the checkout card showed.
    pub total: Money,
    /// Checks that failed on the card; such an order cannot be placed.
    pub failed_checks: Vec<String>,
}

pub(crate) fn planned_lines(v: &CartView) -> Vec<PlannedLine> {
    v.lines
        .iter()
        .map(|l| PlannedLine {
            cart_line_id: l.line_id.clone(),
            variant_id: l.variant.variant_id.clone(),
            quantity: l.quantity,
        })
        .collect()
}

pub(crate) fn create_order_change(v: &CartView) -> Change {
    Change {
        op: ChangeOp::Create,
        entity: "order".into(),
        id: String::new(),
        from: Value::Null,
        to: pricing::order_json(v),
        note: None,
    }
}

pub(crate) fn floor_multiplier(tx_policy: &acom_policy::PolicySet) -> Option<Decimal> {
    promotion::rules(tx_policy).ok()?.price_floor_multiplier
}

/// The delivery a prepare or change names: a saved address, or one for this order only.
pub(crate) fn delivery_choice(
    mode: DeliveryMode,
    address_id: Option<&String>,
    address: Option<&PostalAddressInput>,
) -> Result<DeliveryChoice, ToolError> {
    match mode {
        DeliveryMode::Saved => address_id
            .cloned()
            .map(DeliveryChoice::Saved)
            .ok_or_else(|| {
                ToolError::invalid_input("`saved` needs `delivery_address_id`.")
                    .detail(json!({"field": "delivery_address_id", "reason": "missing"}))
            }),
        DeliveryMode::OneTime => {
            let a = address.ok_or_else(|| {
                ToolError::invalid_input("`one_time` needs `delivery_address`.")
                    .detail(json!({"field": "delivery_address", "reason": "missing"}))
            })?;
            Ok(DeliveryChoice::OneTime(Address {
                name: a.name.clone(),
                street_address: a.street_address.trim().to_owned(),
                address_locality: a.address_locality.trim().to_owned(),
                address_region: a.address_region.clone(),
                postal_code: a.postal_code.trim().to_owned(),
                address_country: a.address_country.clone(),
            }))
        }
    }
}

/// Prices the caller's cart for checkout and refuses what can never be ordered. A gift whose
/// recipient gives the address later (`DeliveryChoice::Later`) needs no address yet.
pub(crate) async fn checkout_view(
    tx: &mut TxCx,
    delivery: &DeliveryChoice,
    payment_method: Option<&str>,
) -> Result<(Carts, CartView), ToolError> {
    let owners = owner::owners(&tx.principal)?;
    if owners.owner.customer_id.is_none() {
        return Err(not_signed_in());
    }
    let limits = rules::cart_limits(&tx.policy)?;
    let carts = pricing::load_carts(tx, &owners, limits).await?;
    let view = pricing::view(tx, &carts, delivery, payment_method)
        .await?
        .map_err(|id| address_not_found(&id))?;
    if view.lines.is_empty() {
        return Err(cart_empty());
    }
    let kinds: Vec<bool> = view
        .lines
        .iter()
        .map(|l| l.variant.sale_type == "subscription")
        .collect();
    if kinds.iter().any(|k| *k != kinds[0]) {
        return Err(ToolError::new(
            MIXED_ORDER_TYPE,
            "Subscriptions and one-time purchases are ordered separately.",
        )
        .detail(json!({"policy_key": "cart.separate_by"}))
        .next("remove_cart_item"));
    }
    if *delivery == DeliveryChoice::Later {
        return Ok((carts, view));
    }
    let Some(d) = &view.destination else {
        return Err(ToolError::new(
            ADDRESS_NOT_FOUND,
            "There is no delivery address yet; save one or give one for this order.",
        )
        .detail(json!({"entity": "postal_address"}))
        .next("get_my_profile"));
    };
    let ships_to = rules::ships_to(&tx.policy);
    if !delivery::deliverable(&d.address.address_country, &ships_to) {
        return Err(ToolError::new(
            ADDRESS_NOT_DELIVERABLE,
            format!("The store does not ship to {}.", d.address.address_country),
        )
        .detail(
            json!({"country": d.address.address_country, "ships_to": ships_to,
                       "policy_key": "shipping.ships_to"}),
        )
        .next("get_my_profile"));
    }
    Ok((carts, view))
}

/// Facts the order reads add to each item.
pub(crate) struct ItemFacts {
    product_id: Option<String>,
    size: Option<String>,
    color: Option<String>,
}

pub(crate) async fn item_facts(
    tx: &mut TxCx,
    items: &[repo::ItemRow],
) -> Result<HashMap<String, ItemFacts>, ToolError> {
    let mut ids: Vec<String> = items.iter().map(|i| i.product_id.clone()).collect();
    ids.sort_unstable();
    ids.dedup();
    Ok(catalog::variants(tx, &ids)
        .await?
        .into_iter()
        .map(|v| {
            (
                v.variant_id.clone(),
                ItemFacts {
                    product_id: Some(v.product_id),
                    size: v.size,
                    color: v.color,
                },
            )
        })
        .collect())
}

pub(crate) fn item_json(
    i: &repo::ItemRow,
    facts: &HashMap<String, ItemFacts>,
    full: bool,
) -> Result<Value, ToolError> {
    let f = facts.get(&i.product_id);
    let mut v = json!({
        "id": i.id,
        "product_id": f.and_then(|f| f.product_id.clone()),
        "variant_id": i.product_id,
        "name": i.name,
        "size": f.and_then(|f| f.size.clone()),
        "color": f.and_then(|f| f.color.clone()),
        "order_quantity": i.order_quantity,
    });
    if full {
        v["price"] = json!(repo::money(i.price, &i.price_currency)?);
        v["discount"] = json!(repo::money(i.discount_value, &i.discount_currency)?);
    }
    v["order_item_status"] = json!(i.order_item_status);
    Ok(v)
}

pub(crate) fn order_not_found(id: &str) -> ToolError {
    ToolError::new(ORDER_NOT_FOUND, format!("There is no order {id}."))
        .detail(json!({"entity": "order", "id": id}))
        .next("search_orders")
}

/// Loads an order the caller may see.
pub(crate) async fn visible_order(cx: &Cx, tx: &mut TxCx, id: &str) -> Result<OrderRow, ToolError> {
    let o = repo::order(tx, id)
        .await?
        .ok_or_else(|| order_not_found(id))?;
    match owner::viewer(&cx.principal) {
        Viewer::Staff => Ok(o),
        Viewer::Shopper(c) if o.customer_id.as_deref() == Some(c.as_str()) => Ok(o),
        _ => Err(
            ToolError::new(ErrorCode::FORBIDDEN, "This is not your order.")
                .detail(json!({"reason": "owner"}))
                .next("get_my_orders"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn a_one_time_address_is_closed() {
        let s = closed_schema_for::<PostalAddressInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["additionalProperties"], json!(false));
    }
}
