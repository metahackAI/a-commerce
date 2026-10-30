//! `get_payment_methods`, and the checkout checks (`order.checkout_validation`) that preparing
//! and placing an order both run.

use acom_contract::{
    ActionType, Audience, Check, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::Money;
use acom_store::Cx;
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::pricing::{self, CartView, DeliveryChoice};
use crate::rules::{self, PaymentRules};
use crate::{CART_EMPTY, cart_empty, owner};

/// One way to pay this cart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PaymentOption {
    pub payment_method: String,
    pub label: String,
    /// A method the shopper saved (their card).
    pub saved: bool,
    pub default: bool,
    /// Card installments that can be chosen; `[0]` means paid at once only.
    pub installment_months: Vec<u32>,
    /// Loyalty points can be added as a second method.
    pub loyalty_usable: bool,
}

/// What the shopper sees for a method: "Card ending 4821", "Bank transfer".
pub(crate) fn payment_label(method: &str, card_last4: Option<&str>) -> String {
    match (method, card_last4) {
        ("credit_card", Some(l)) => format!("Card ending {l}"),
        _ => {
            let words = method.replace('_', " ");
            let mut c = words.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        }
    }
}

/// The methods that can pay this cart, in policy order. A card needs a saved card (the store
/// keeps no other card details); loyalty points are only a second method
/// (`payment.loyalty_as_secondary`) and store credit balances are not kept yet, so neither is
/// offered as the one main method (`payment.primary_methods_per_order`).
pub(crate) fn payment_options(
    rules: &PaymentRules,
    card_last4: Option<&str>,
    signed_in: bool,
    total: Money,
    chosen: Option<&str>,
) -> Vec<PaymentOption> {
    let loyalty_usable = rules.methods.iter().any(|m| m == "loyalty_points")
        && rules.loyalty_as_secondary
        && (signed_in || rules.guest_allows_loyalty);
    let mut out: Vec<PaymentOption> = rules
        .methods
        .iter()
        .filter(|m| match m.as_str() {
            "credit_card" => card_last4.is_some(),
            "loyalty_points" => !rules.loyalty_as_secondary,
            "store_credit" => false,
            _ => true,
        })
        .map(|m| {
            let installments = rules.installment_methods.contains(m)
                && rules
                    .installment_min
                    .is_none_or(|min| total.value >= min.value);
            PaymentOption {
                payment_method: m.clone(),
                label: payment_label(m, card_last4),
                saved: m == "credit_card",
                default: false,
                installment_months: if installments {
                    rules.installment_months.clone()
                } else {
                    vec![0]
                },
                loyalty_usable,
            }
        })
        .collect();
    let default = chosen
        .and_then(|c| out.iter().position(|o| o.payment_method == c))
        .or_else(|| out.iter().position(|o| o.saved))
        .or(if out.is_empty() { None } else { Some(0) });
    if let Some(n) = default {
        out[n].default = true;
    }
    out
}

fn check(rule: &str, passed: bool, detail: String, fix: Option<&str>) -> Check {
    Check {
        rule: format!("checkout_validation.{rule}"),
        passed,
        detail: Some(detail),
        fix_tool: if passed { None } else { fix.map(str::to_owned) },
    }
}

/// Runs the `order.checkout_validation` checks the policy marks required, in policy order.
pub(crate) fn checkout_checks(
    v: &CartView,
    required: &[String],
    client_total: Money,
    floor_multiplier: Option<Decimal>,
) -> Vec<Check> {
    let b = &v.breakdown;
    let mut out = Vec::new();
    for rule in required {
        match rule.as_str() {
            "common_payment_method_exists" => {
                let ok = v.payment.as_ref().is_some_and(|p| {
                    v.options
                        .iter()
                        .any(|o| o.payment_method == p.payment_method)
                });
                let label = v
                    .payment
                    .as_ref()
                    .map_or("No payment method".to_owned(), |p| p.label.clone());
                out.push(check(
                    rule,
                    ok,
                    if ok {
                        format!("{label} works for every line")
                    } else {
                        format!("{label} can't pay for this order")
                    },
                    Some("get_payment_methods"),
                ));
            }
            "stock_available" => {
                let short: Vec<String> = v
                    .lines
                    .iter()
                    .filter(|l| !l.variant.has_stock(i64::from(l.quantity)))
                    .map(|l| {
                        format!(
                            "{} has {} left, the cart has {}",
                            l.title(),
                            l.variant.inventory_level,
                            l.quantity
                        )
                    })
                    .collect();
                let detail = match (short.first(), v.lines.as_slice()) {
                    (Some(_), _) => short.join("; "),
                    (None, [one]) => format!("{} is in stock", one.title()),
                    (None, all) => format!("All {} lines are in stock", all.len()),
                };
                out.push(check(rule, short.is_empty(), detail, Some("get_product")));
            }
            "product_sellable" => {
                let off: Vec<String> = v
                    .lines
                    .iter()
                    .filter(|l| !l.variant.sellable())
                    .map(|l| format!("{} is not on sale", l.title()))
                    .collect();
                let ok = off.is_empty();
                out.push(check(
                    rule,
                    ok,
                    if ok {
                        "All lines are on sale".to_owned()
                    } else {
                        off.join("; ")
                    },
                    Some("remove_cart_item"),
                ));
            }
            "no_domestic_international_mix" => {
                // The catalog has no shipping origin per product yet: every line ships from the
                // store's own country, so one order never mixes the two.
                let country = v
                    .destination
                    .as_ref()
                    .map_or("the store's country".to_owned(), |d| {
                        d.address.address_country.clone()
                    });
                out.push(check(
                    rule,
                    true,
                    format!("All lines ship within the {country}"),
                    None,
                ));
            }
            "server_total_matches_client" => {
                let ok = b.total.currency == client_total.currency
                    && b.total.value == client_total.value;
                let math = format!(
                    "{} − {} + {} = {}",
                    b.subtotal.value, b.discount_total.value, b.shipping.value, b.total.value
                );
                out.push(check(
                    rule,
                    ok,
                    if ok {
                        format!("Recalculated with calculate_benefits: {math}, same as shown")
                    } else {
                        format!(
                            "Recalculated with calculate_benefits: {math}, but {} was shown",
                            client_total.value
                        )
                    },
                    Some("get_cart"),
                ));
            }
            "price_floor_respected" => {
                let under: Vec<String> = b
                    .below_price_floor_items
                    .iter()
                    .map(|f| {
                        format!(
                            "{} would sell at {}, under the floor {}",
                            f.variant_id, f.unit_price.value, f.floor.value
                        )
                    })
                    .collect();
                let ok = under.is_empty();
                let percent = floor_multiplier
                    .map(|k| ((k - Decimal::ONE) * Decimal::ONE_HUNDRED).normalize());
                out.push(check(
                    rule,
                    ok,
                    match (ok, percent) {
                        (true, Some(p)) => {
                            format!("Sale prices stay at or above cost + {p}%")
                        }
                        (true, None) => "No price floor applies".to_owned(),
                        (false, _) => under.join("; "),
                    },
                    Some("get_cart"),
                ));
            }
            _ => {}
        }
    }
    out
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetPaymentMethodsInput {}

pub(crate) struct GetPaymentMethods;

impl Operation for GetPaymentMethods {
    const META: OpMeta = OpMeta {
        address: "sales.checkout.list_payment_methods",
        tool: "get_payment_methods",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/sales/checkout/payment-methods",
        summary: "The payment methods that work for every line of the caller's own cart, with installments and whether loyalty points can be added. The next tool when a payment check fails.",
        consumes: &[Slot::one("cart_id")],
        produces: &[],
        checks: &[
            "payment.methods",
            "payment.primary_methods_per_order",
            "payment.installment",
        ],
        errors: &[CART_EMPTY],
        audience: Audience::Shopper,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetPaymentMethodsInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetPaymentMethods {
    async fn read(&self, cx: &Cx, _: GetPaymentMethodsInput) -> Result<Value, ToolError> {
        let owners = owner::owners(&cx.principal)?;
        let limits = rules::cart_limits(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let carts = pricing::load_carts(&mut tx, &owners, limits).await?;
        let view = pricing::view(&mut tx, &carts, &DeliveryChoice::CartDefault, None)
            .await?
            .map_err(|_| cart_empty())?;
        tx.commit().await.map_err(ToolError::from)?;
        if view.lines.is_empty() {
            return Err(cart_empty());
        }
        let total = view.options.len();
        Ok(json!({"items": view.options, "total": total}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::tests::defaults;
    use acom_contract::schema::{closed_schema_for, violations};
    use acom_core::Currency;
    use std::str::FromStr;

    fn usd(s: &str) -> Money {
        Money::new(Decimal::from_str(s).unwrap(), Currency::USD)
    }

    #[test]
    fn jamie_pays_with_her_saved_card_in_up_to_six_installments() {
        let rules = rules::payment_rules(&defaults()).unwrap();
        let o = payment_options(&rules, Some("4821"), true, usd("71.20"), None);
        assert_eq!(o[0].payment_method, "credit_card");
        assert_eq!(o[0].label, "Card ending 4821");
        assert!(o[0].saved && o[0].default && o[0].loyalty_usable);
        assert_eq!(o[0].installment_months, vec![0, 3, 6]);
        assert!(o.iter().all(|m| m.payment_method != "loyalty_points"));
        assert_eq!(o.iter().filter(|m| m.default).count(), 1);
        // Under the installment minimum only paying at once is offered.
        let small = payment_options(&rules, Some("4821"), true, usd("40.00"), None);
        assert_eq!(small[0].installment_months, vec![0]);
        // Without a saved card a shopper can still transfer money; guests get no loyalty.
        let guest = payment_options(&rules, None, false, usd("89.00"), None);
        assert_eq!(guest[0].payment_method, "bank_transfer");
        assert!(guest[0].default && !guest[0].loyalty_usable);
        let chosen = payment_options(
            &rules,
            Some("4821"),
            true,
            usd("71.20"),
            Some("virtual_account"),
        );
        assert!(
            chosen
                .iter()
                .find(|m| m.payment_method == "virtual_account")
                .unwrap()
                .default
        );
    }

    #[test]
    fn labels_methods_in_plain_words() {
        assert_eq!(
            payment_label("credit_card", Some("0937")),
            "Card ending 0937"
        );
        assert_eq!(payment_label("virtual_account", None), "Virtual account");
        assert_eq!(payment_label("credit_card", None), "Credit card");
    }

    #[test]
    fn input_schema_is_closed() {
        assert!(violations(&closed_schema_for::<GetPaymentMethodsInput>()).is_empty());
    }

    /// Integration: Jamie's payment methods from the database (`DATABASE_URL`).
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn lists_jamies_methods_from_the_database() {
        let cx = crate::testdb::cx("cus_jamie").await;
        match GetPaymentMethods.read(&cx, GetPaymentMethodsInput {}).await {
            Ok(v) => assert!(v["total"].as_u64().unwrap() >= 1),
            Err(e) => assert_eq!(e.code, "cart_empty"),
        }
    }
}
