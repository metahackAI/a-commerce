//! The order policy values sales uses (`policies/order.yml`, and the delivery estimate in
//! `policies/fulfillment.yml`), read once per request into plain values.

use acom_contract::{ErrorCode, ToolError};
use acom_core::Money;
use acom_policy::PolicySet;
use chrono::{NaiveTime, Weekday};
use serde_json::Value;

fn missing(path: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INTERNAL,
        format!("policy {path} is missing or malformed"),
    )
}

fn words(policy: &PolicySet, path: &str) -> Vec<String> {
    policy
        .lookup(path)
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn count(policy: &PolicySet, path: &str) -> Result<u32, ToolError> {
    policy
        .lookup(path)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| missing(path))
}

/// `cart.*`: how big a cart may get.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CartLimits {
    pub max_lines: u32,
    pub max_quantity_per_line: u32,
}

pub(crate) fn cart_limits(policy: &PolicySet) -> Result<CartLimits, ToolError> {
    Ok(CartLimits {
        max_lines: count(policy, "order.cart.max_lines")?,
        max_quantity_per_line: count(policy, "order.cart.max_quantity_per_line")?,
    })
}

/// When parcels leave and how long they travel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ShipCalendar {
    /// Orders before this local time ship the same day.
    pub cutoff: NaiveTime,
    /// The days parcels leave; transit is counted on these days too.
    pub ship_days: Vec<Weekday>,
    pub transit_days: u32,
}

fn weekday(word: &str) -> Option<Weekday> {
    Some(match word {
        "mon" => Weekday::Mon,
        "tue" => Weekday::Tue,
        "wed" => Weekday::Wed,
        "thu" => Weekday::Thu,
        "fri" => Weekday::Fri,
        "sat" => Weekday::Sat,
        "sun" => Weekday::Sun,
        _ => return None,
    })
}

pub(crate) fn ship_calendar(policy: &PolicySet) -> Result<ShipCalendar, ToolError> {
    let cutoff_path = "fulfillment.delivery.estimate.ship_cutoff_local_time";
    let cutoff = policy
        .lookup(cutoff_path)
        .and_then(Value::as_str)
        .and_then(|s| NaiveTime::parse_from_str(s, "%H:%M").ok())
        .ok_or_else(|| missing(cutoff_path))?;
    let ship_days: Vec<Weekday> = words(policy, "fulfillment.delivery.estimate.ship_days")
        .iter()
        .filter_map(|w| weekday(w))
        .collect();
    if ship_days.is_empty() {
        return Err(missing("fulfillment.delivery.estimate.ship_days"));
    }
    Ok(ShipCalendar {
        cutoff,
        ship_days,
        transit_days: count(policy, "order.shipping.transit_business_days")?,
    })
}

/// `order.shipping.ships_to`: countries parcels go to.
pub(crate) fn ships_to(policy: &PolicySet) -> Vec<String> {
    words(policy, "order.shipping.ships_to")
}

/// `order.checkout_validation`: the checks marked `required`, in policy order.
pub(crate) fn checkout_checks(policy: &PolicySet) -> Vec<String> {
    policy
        .lookup("order.checkout_validation")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter(|(_, v)| v.as_str() == Some("required"))
                .map(|(k, _)| k.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// `order.payment`: which methods the store takes and how.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PaymentRules {
    pub methods: Vec<String>,
    pub loyalty_as_secondary: bool,
    pub guest_allows_loyalty: bool,
    pub installment_methods: Vec<String>,
    pub installment_min: Option<Money>,
    pub installment_months: Vec<u32>,
}

pub(crate) fn payment_rules(policy: &PolicySet) -> Result<PaymentRules, ToolError> {
    let methods = words(policy, "order.payment.methods");
    if methods.is_empty() {
        return Err(missing("order.payment.methods"));
    }
    let flag = |p: &str| policy.lookup(p).and_then(Value::as_bool).unwrap_or(false);
    let installment_min = policy
        .lookup("order.payment.installment.min_amount")
        .map(|v| serde_json::from_value::<Money>(v.clone()))
        .transpose()
        .map_err(|_| missing("order.payment.installment.min_amount"))?;
    let installment_months = policy
        .lookup("order.payment.installment.months")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_u64)
                .filter_map(|n| u32::try_from(n).ok())
                .collect()
        })
        .unwrap_or_default();
    Ok(PaymentRules {
        methods,
        loyalty_as_secondary: flag("order.payment.loyalty_as_secondary"),
        guest_allows_loyalty: flag("order.payment.guest_allows_loyalty"),
        installment_methods: words(policy, "order.payment.installment.methods"),
        installment_min,
        installment_months,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn defaults() -> PolicySet {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../policies");
        let texts: Vec<(&str, String)> = acom_policy::FILES
            .iter()
            .map(|f| {
                (
                    *f,
                    std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap(),
                )
            })
            .collect();
        PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str()))).unwrap()
    }

    #[test]
    fn reads_the_order_policy() {
        let p = defaults();
        assert_eq!(
            cart_limits(&p).unwrap(),
            CartLimits {
                max_lines: 50,
                max_quantity_per_line: 20
            }
        );
        let cal = ship_calendar(&p).unwrap();
        assert_eq!(cal.cutoff, NaiveTime::from_hms_opt(14, 0, 0).unwrap());
        assert_eq!(cal.ship_days.len(), 6);
        assert_eq!(cal.transit_days, 2);
        assert_eq!(ships_to(&p), vec!["US"]);
        assert_eq!(
            checkout_checks(&p),
            vec![
                "common_payment_method_exists",
                "stock_available",
                "product_sellable",
                "no_domestic_international_mix",
                "server_total_matches_client",
                "price_floor_respected"
            ]
        );
        let pay = payment_rules(&p).unwrap();
        assert_eq!(pay.installment_months, vec![0, 3, 6]);
        assert!(pay.loyalty_as_secondary && !pay.guest_allows_loyalty);
    }
}
