//! Exchanges (`fulfillment.yml exchange`): who is at fault for a reason, which variants of the
//! same product a line can be swapped for, what the shopper pays for the two legs, and whether
//! a person at the store confirms it. `get_exchange_options` shows it; the person-only
//! `POST /v1/fulfillment/exchanges` computes it again and acts on it.

use acom_core::Money;
use serde::Serialize;
use serde_json::Value;

use super::zone::{Zone, leg_cost};
use super::{Rules, money_of};

/// The liability an exchange reason carries (`exchange.reasons`), or `None` when an exchange
/// cannot have that reason (for example `changed_mind`: that is a return).
pub fn exchange_liability(rules: &Rules, reason: &str) -> Option<String> {
    let reasons = rules.lookup("exchange.reasons")?.as_object()?;
    reasons.iter().find_map(|(liability, list)| {
        list.as_array()?
            .iter()
            .any(|r| r.as_str() == Some(reason))
            .then(|| liability.clone())
    })
}

/// Every reason an exchange can have, in policy order.
pub fn exchange_reasons(rules: &Rules) -> Vec<String> {
    rules
        .lookup("exchange.reasons")
        .and_then(Value::as_object)
        .map(|m| {
            m.values()
                .filter_map(Value::as_array)
                .flatten()
                .filter_map(|r| r.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Why the line itself cannot be exchanged (`exchange.not_exchangeable_when`), from the
/// variant's sale type and the product's tags.
pub fn not_exchangeable(rules: &Rules, sale_type: &str, keywords: &[String]) -> Option<String> {
    let blocked = rules.list("exchange.not_exchangeable_when");
    if blocked.iter().any(|b| b == sale_type) {
        return Some(sale_type.to_owned());
    }
    keywords
        .iter()
        .find(|k| blocked.contains(k))
        .map(ToOwned::to_owned)
}

/// A variant of the same product, as the exchange sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub variant_id: String,
    pub size: Option<String>,
    pub color: Option<String>,
    /// The offer's availability; `discontinued` when there is no offer.
    pub availability: String,
    /// Sellable units; `None` when stock is not tracked.
    pub available_quantity: Option<i32>,
    pub price: Option<Money>,
    pub sale_type: String,
}

/// One variant the shopper may (or may not) swap to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Choice {
    pub variant_id: String,
    pub size: Option<String>,
    pub color: Option<String>,
    pub availability: String,
    pub available_quantity: Option<i32>,
    pub price: Option<rust_decimal::Decimal>,
    pub price_currency: Option<String>,
    pub price_difference: Option<Money>,
    pub selectable: bool,
    /// `out_of_stock`, `price_differs` or `not_sold`, when not selectable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_tool: Option<String>,
}

/// The variants `current` can be swapped for (`exchange.replacement`): the same product only;
/// the same variant only when the store is at fault; actively sold, in stock for `quantity`,
/// and at the same price unless the policy allows a difference. `reference` is the price the
/// difference is measured from: the current price of the variant bought.
pub fn choices(
    rules: &Rules,
    liability: &str,
    current_variant: &str,
    reference: Money,
    quantity: i32,
    candidates: &[Candidate],
) -> Vec<Choice> {
    let same_allowed = rules
        .lookup("exchange.replacement.same_variant_allowed_when")
        .and_then(Value::as_str)
        .is_some_and(|l| l == liability);
    let constraints = rules.list("exchange.replacement.constraints");
    let price_must_match = rules
        .lookup("exchange.replacement.price_difference")
        .and_then(Value::as_str)
        .is_none_or(|p| p == "not_allowed");
    let no_ship = rules.list("delivery.no_ship_for");
    candidates
        .iter()
        .filter(|c| c.variant_id != current_variant || same_allowed)
        .map(|c| {
            let selling = c.availability != "discontinued"
                && c.price.is_some()
                && !no_ship.contains(&c.sale_type);
            let stocked = c.available_quantity.is_none_or(|n| n >= quantity);
            let difference = c
                .price
                .map(|p| Money::new(p.value - reference.value, p.currency).rounded());
            let differs = difference.is_some_and(|d| !d.value.is_zero());
            let why = if constraints.iter().any(|x| x == "actively_selling") && !selling {
                Some("not_sold")
            } else if constraints.iter().any(|x| x == "in_stock_for_quantity") && !stocked {
                Some("out_of_stock")
            } else if price_must_match && differs {
                Some("price_differs")
            } else {
                None
            };
            Choice {
                variant_id: c.variant_id.clone(),
                size: c.size.clone(),
                color: c.color.clone(),
                availability: c.availability.clone(),
                available_quantity: c.available_quantity,
                price: c.price.map(|p| p.value),
                price_currency: c.price.map(|p| p.currency.as_str().to_owned()),
                price_difference: difference,
                selectable: why.is_none(),
                next_tool: (why == Some("out_of_stock")).then(|| "subscribe_restock".to_owned()),
                why: why.map(str::to_owned),
            }
        })
        .collect()
}

/// What the shopper pays for an exchange: each leg (pickup, reshipment) and a flat fee.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ExchangeCost {
    pub pickup: Money,
    pub reship: Money,
    pub fee: Money,
    pub you_pay: Money,
}

/// An amount written in the currency's minor units (`12.00`, `0.00`).
fn cents(m: Money) -> Money {
    let mut value = m.rounded().value;
    value.rescale(m.currency.minor_units());
    Money::new(value, m.currency)
}

/// `exchange.shipping_cost`: the store pays both legs when it is at fault; otherwise a size
/// exchange costs `size_exchange_fee` plus any remote-area surcharge, and any other exchange
/// costs a round trip (each leg at its zone's rate and surcharge).
pub fn exchange_cost(
    rules: &Rules,
    reason: &str,
    liability: &str,
    pickup_zone: &Zone,
    reship_zone: &Zone,
) -> ExchangeCost {
    let currency = rules.shipping_rate.currency;
    let zero = Money::zero(currency);
    let store_pays = rules
        .lookup(&format!("exchange.shipping_cost.{liability}"))
        .and_then(Value::as_str)
        .is_some_and(|w| w == "store_pays_both_legs")
        || (liability != "customer_fault" && rules.store_pays_return(liability));
    if store_pays {
        return ExchangeCost {
            pickup: cents(zero),
            reship: cents(zero),
            fee: cents(zero),
            you_pay: cents(zero),
        };
    }
    let size_fee = (reason == "wrong_size")
        .then(|| {
            rules
                .lookup("exchange.shipping_cost.wrong_size_fee")
                .and_then(Value::as_str)
                .and_then(|key| rules.lookup(key))
                .and_then(money_of)
        })
        .flatten();
    let (pickup, reship, fee) = match size_fee {
        Some(fee) => (pickup_zone.surcharge, reship_zone.surcharge, fee),
        None => (leg_cost(pickup_zone), leg_cost(reship_zone), zero),
    };
    let you_pay = Money::new(pickup.value + reship.value + fee.value, currency).rounded();
    ExchangeCost {
        pickup: cents(pickup),
        reship: cents(reship),
        fee: cents(fee),
        you_pay: cents(you_pay),
    }
}

/// Whether a person at the store confirms the exchange before the pickup
/// (`exchange.pickup.operator_approval_when`): a damage claim that needs photos.
pub fn needs_person(rules: &Rules, reason: &str) -> bool {
    rules
        .list("exchange.pickup.operator_approval_when")
        .iter()
        .any(|w| w == "damage_claim_needs_photo")
        && reason == "damaged_in_transit"
        && rules.damage_claim_needs_photo
}

/// The status a new exchange starts in: `charge_due` while the shopper owes shipping
/// (`exchange.shipping_cost.collect: pay_before_accept`), `requested` when a person confirms
/// it, else `pickup_instructed` (`exchange.pickup.instruction: automatic`).
pub fn first_status(rules: &Rules, you_pay: Money, person: bool) -> &'static str {
    let automatic = rules
        .lookup("exchange.pickup.instruction")
        .and_then(Value::as_str)
        .is_some_and(|i| i == "automatic");
    if !you_pay.value.is_zero() {
        "charge_due"
    } else if person || !automatic {
        "requested"
    } else {
        "pickup_instructed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::{rules, usd};
    use crate::model::zone::default_zone;

    fn variant(id: &str, size: &str, color: &str, stock: i32, price: &str) -> Candidate {
        Candidate {
            variant_id: id.into(),
            size: Some(size.into()),
            color: Some(color.into()),
            availability: if stock > 0 {
                "in_stock"
            } else {
                "out_of_stock"
            }
            .into(),
            available_quantity: Some(stock),
            price: Some(usd(price)),
            sale_type: "always".into(),
        }
    }

    #[test]
    fn reasons_carry_their_fault() {
        let r = rules();
        assert_eq!(
            exchange_liability(&r, "wrong_size").as_deref(),
            Some("customer_fault")
        );
        assert_eq!(
            exchange_liability(&r, "defect").as_deref(),
            Some("store_fault")
        );
        assert_eq!(exchange_liability(&r, "changed_mind"), None);
        assert!(exchange_reasons(&r).contains(&"missing_parts".to_owned()));
    }

    #[test]
    fn a_size_swap_offers_same_priced_variants_in_stock() {
        let r = rules();
        let all = vec![
            variant("var_runner_9_slate", "9", "Slate", 4, "128.00"),
            variant("var_runner_10_slate", "10", "Slate", 7, "128.00"),
            variant("var_runner_10_5_slate", "10.5", "Slate", 0, "128.00"),
            variant("var_runner_10_ember", "10", "Ember", 3, "138.00"),
        ];
        let c = choices(
            &r,
            "customer_fault",
            "var_runner_9_slate",
            usd("128.00"),
            1,
            &all,
        );
        assert_eq!(c.len(), 3);
        assert!(c[0].selectable);
        assert_eq!(c[1].why.as_deref(), Some("out_of_stock"));
        assert_eq!(c[1].next_tool.as_deref(), Some("subscribe_restock"));
        assert_eq!(c[2].why.as_deref(), Some("price_differs"));
        assert_eq!(c[2].price_difference, Some(usd("10.00")));
        // A defect may be swapped for the same variant, new.
        let d = choices(
            &r,
            "store_fault",
            "var_runner_9_slate",
            usd("128.00"),
            1,
            &all,
        );
        assert!(
            d.iter()
                .any(|x| x.variant_id == "var_runner_9_slate" && x.selectable)
        );
    }

    #[test]
    fn who_pays_for_the_two_legs() {
        let r = rules();
        let z = default_zone(&r);
        let size = exchange_cost(&r, "wrong_size", "customer_fault", &z, &z);
        assert_eq!(size.you_pay, usd("0.00"));
        let other = exchange_cost(&r, "different_option_wanted", "customer_fault", &z, &z);
        assert_eq!(other.you_pay, usd("12.00"));
        assert_eq!(other.pickup, usd("6.00"));
        let defect = exchange_cost(&r, "defect", "store_fault", &z, &z);
        assert_eq!(defect.you_pay, usd("0.00"));
        assert_eq!(first_status(&r, other.you_pay, false), "charge_due");
        assert_eq!(first_status(&r, size.you_pay, false), "pickup_instructed");
        assert!(needs_person(&r, "damaged_in_transit"));
        assert_eq!(first_status(&r, size.you_pay, true), "requested");
    }

    #[test]
    fn gift_cards_and_final_sale_items_are_not_exchanged() {
        let r = rules();
        assert_eq!(
            not_exchangeable(&r, "gift_card", &[]).as_deref(),
            Some("gift_card")
        );
        assert_eq!(
            not_exchangeable(&r, "always", &["final_sale".into()]).as_deref(),
            Some("final_sale")
        );
        assert_eq!(not_exchangeable(&r, "always", &["new".into()]), None);
    }
}
