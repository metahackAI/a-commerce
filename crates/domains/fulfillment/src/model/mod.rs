//! Fulfillment business rules as pure functions: the policy values they read, the small
//! condition evaluator for `when:` blocks, and the sentences shown to people. No I/O here, so
//! every number in scenes B2 and F5 is unit-tested from the real policy files.

pub mod cancel;
pub mod delivery;
pub mod returns;

use std::collections::BTreeMap;
use std::str::FromStr;

use acom_contract::{ErrorCode, ToolError};
use acom_core::{Currency, Money};
use acom_policy::PolicySet;
use chrono::{NaiveDate, NaiveTime, Weekday};
use rust_decimal::Decimal;
use serde_json::Value;

fn policy_error(key: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INTERNAL,
        format!("The store's policy has no usable `{key}`."),
    )
}

/// A JSON number (exact, as written in the policy or the wire) as a decimal.
pub fn decimal(v: &Value) -> Option<Decimal> {
    match v {
        Value::Number(n) => {
            let s = n.to_string();
            Decimal::from_str(&s)
                .ok()
                .or_else(|| Decimal::from_scientific(&s).ok())
        }
        Value::String(s) => Decimal::from_str(s).ok(),
        _ => None,
    }
}

/// A policy amount `{value, currency}`.
pub fn money_of(v: &Value) -> Option<Money> {
    let value = decimal(v.get("value")?)?;
    let currency = Currency::parse(v.get("currency")?.as_str()?).ok()?;
    Some(Money::new(value, currency))
}

/// `$312.00`, the way every sentence shows money.
pub fn money_text(m: Money) -> String {
    let m = m.rounded();
    let amount = format!("{:.2}", m.value);
    let amount = with_thousands(&amount);
    match m.currency.as_str() {
        "USD" => format!("${amount}"),
        other => format!("{amount} {other}"),
    }
}

/// `$200` for whole amounts, `$200.50` otherwise (limits in short sentences).
pub fn money_short(m: Money) -> String {
    let m = m.rounded();
    if m.value.fract().is_zero() {
        let whole = with_thousands(&m.value.trunc().to_string());
        match m.currency.as_str() {
            "USD" => format!("${whole}"),
            other => format!("{whole} {other}"),
        }
    } else {
        money_text(m)
    }
}

fn with_thousands(amount: &str) -> String {
    let (int, frac) = amount.split_once('.').unwrap_or((amount, ""));
    let (sign, digits) = int.strip_prefix('-').map_or(("", int), |d| ("-", d));
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if frac.is_empty() {
        format!("{sign}{out}")
    } else {
        format!("{sign}{out}.{frac}")
    }
}

/// `Tue Nov 10`.
pub fn day_text(d: NaiveDate) -> String {
    d.format("%a %b %-d").to_string()
}

/// `Sep 28`.
pub fn date_text(d: NaiveDate) -> String {
    d.format("%b %-d").to_string()
}

fn weekday(word: &str) -> Option<Weekday> {
    match word {
        "mon" => Some(Weekday::Mon),
        "tue" => Some(Weekday::Tue),
        "wed" => Some(Weekday::Wed),
        "thu" => Some(Weekday::Thu),
        "fri" => Some(Weekday::Fri),
        "sat" => Some(Weekday::Sat),
        "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

/// The values of `fulfillment.yml` (and the few keys of other files) the operations use, read
/// once per request. A missing or malformed key is the store's configuration error.
#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    /// The whole `fulfillment` file: `when:` conditions look values up in it.
    pub file: Value,
    pub return_window_days: i64,
    pub defect_refund_type: String,
    pub defect_return_fees: String,
    pub changed_mind_return_fees: String,
    pub damage_claim_needs_photo: bool,
    /// `return_suggestions.order`.
    pub suggestion_order: Vec<String>,
    pub flag_returns: i64,
    pub flag_days: i64,
    pub refund_to: String,
    pub coupon_deducted: bool,
    pub money_back_days: (i64, i64),
    /// Payment method → days the original payment can still be reversed; `None` is unlimited.
    pub reversal_window_days: BTreeMap<String, Option<i64>>,
    /// Claim reason → default liability.
    pub liability_default: BTreeMap<String, String>,
    /// Liability → what that party pays.
    pub liability_pays: BTreeMap<String, Vec<String>>,
    pub non_returnable_when: Vec<String>,
    pub replacement_option: String,
    pub send_back_methods: Vec<String>,
    pub pickup_days_after: i64,
    pub pickup_start: NaiveTime,
    pub pickup_end: NaiveTime,
    pub return_withdraw_states: Vec<String>,
    pub exchange_withdraw_states: Vec<String>,
    pub ship_cutoff: NaiveTime,
    pub ship_days: Vec<Weekday>,
    pub business_days: Vec<Weekday>,
    pub delivery_methods: Vec<String>,
    pub zone_name: String,
    pub zone_countries: Vec<String>,
    pub shipping_rate: Money,
    pub zone_surcharge: Money,
    pub transit_business_days: i64,
    /// `order.yml free_shipping_min_subtotal.amount`.
    pub free_shipping_min: Money,
    /// `promotion.yml loyalty.accrual_base_rate` × the default tier multiplier.
    pub points_rate: Decimal,
}

impl Rules {
    pub fn load(policy: &PolicySet) -> Result<Self, ToolError> {
        let file = policy
            .raw("fulfillment")
            .cloned()
            .ok_or_else(|| policy_error("fulfillment"))?;
        let at = |path: &str| -> Result<&Value, ToolError> {
            let mut v = &file;
            for p in path.split('.') {
                v = v.get(p).ok_or_else(|| policy_error(path))?;
            }
            Ok(v)
        };
        let text = |path: &str| -> Result<String, ToolError> {
            at(path)?
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| policy_error(path))
        };
        let int = |path: &str| -> Result<i64, ToolError> {
            at(path)?.as_i64().ok_or_else(|| policy_error(path))
        };
        let words = |path: &str| -> Result<Vec<String>, ToolError> {
            at(path)?
                .as_array()
                .ok_or_else(|| policy_error(path))?
                .iter()
                .map(|w| {
                    w.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| policy_error(path))
                })
                .collect()
        };
        let days = |path: &str| -> Result<Vec<Weekday>, ToolError> {
            words(path)?
                .iter()
                .map(|w| weekday(w).ok_or_else(|| policy_error(path)))
                .collect()
        };
        let time = |path: &str| -> Result<NaiveTime, ToolError> {
            NaiveTime::parse_from_str(&text(path)?, "%H:%M").map_err(|_| policy_error(path))
        };
        let amount = |path: &str| -> Result<Money, ToolError> {
            money_of(at(path)?).ok_or_else(|| policy_error(path))
        };
        let map_of_words = |path: &str| -> Result<BTreeMap<String, String>, ToolError> {
            at(path)?
                .as_object()
                .ok_or_else(|| policy_error(path))?
                .iter()
                .map(|(k, v)| {
                    v.as_str()
                        .map(|s| (k.clone(), s.to_owned()))
                        .ok_or_else(|| policy_error(path))
                })
                .collect()
        };
        let reversal = at("refund.reversal_window_days")?
            .as_object()
            .ok_or_else(|| policy_error("refund.reversal_window_days"))?
            .iter()
            .map(|(k, v)| (k.clone(), v.as_i64()))
            .collect();
        let pays = at("liability.pays")?
            .as_object()
            .ok_or_else(|| policy_error("liability.pays"))?
            .keys()
            .map(|k| Ok((k.clone(), words(&format!("liability.pays.{k}"))?)))
            .collect::<Result<_, ToolError>>()?;
        let lands = at("refund.lands_in_business_days")?;
        let free_shipping_min = policy
            .lookup("order.free_shipping_min_subtotal.amount")
            .and_then(money_of)
            .ok_or_else(|| policy_error("order.free_shipping_min_subtotal.amount"))?;
        let base_rate = policy
            .lookup("promotion.loyalty.accrual_base_rate")
            .and_then(decimal)
            .ok_or_else(|| policy_error("promotion.loyalty.accrual_base_rate"))?;
        let multiplier = policy
            .lookup("promotion.loyalty.tier_multiplier.default")
            .and_then(decimal)
            .unwrap_or(Decimal::ONE);
        Ok(Self {
            return_window_days: int("return_window_days")?,
            defect_refund_type: text("defect_refund.refund_type")?,
            defect_return_fees: text("defect_refund.return_fees")?,
            changed_mind_return_fees: text("changed_mind_return_shipping")?,
            damage_claim_needs_photo: at("damage_claim_needs_photo")?
                .as_bool()
                .ok_or_else(|| policy_error("damage_claim_needs_photo"))?,
            suggestion_order: words("return_suggestions.order")?,
            flag_returns: int("return_suggestions.flag_return_history.returns")?,
            flag_days: int("return_suggestions.flag_return_history.days")?,
            refund_to: text("refund.to")?,
            coupon_deducted: at("refund.coupon_deducted")?
                .as_bool()
                .ok_or_else(|| policy_error("refund.coupon_deducted"))?,
            money_back_days: (
                lands["min"]
                    .as_i64()
                    .ok_or_else(|| policy_error("refund.lands_in_business_days"))?,
                lands["max"]
                    .as_i64()
                    .ok_or_else(|| policy_error("refund.lands_in_business_days"))?,
            ),
            reversal_window_days: reversal,
            liability_default: map_of_words("liability.claim_reason_default")?,
            liability_pays: pays,
            non_returnable_when: words("return.non_returnable_when")?,
            replacement_option: text("return.replacement_option")?,
            send_back_methods: words("return.send_back_methods")?,
            pickup_days_after: int("return.pickup.business_days_after_request")?,
            pickup_start: time("return.pickup.window.start")?,
            pickup_end: time("return.pickup.window.end")?,
            return_withdraw_states: words("return.withdraw.allowed_states")?,
            exchange_withdraw_states: words("exchange.withdraw.allowed_states")?,
            ship_cutoff: time("delivery.estimate.ship_cutoff_local_time")?,
            ship_days: days("delivery.estimate.ship_days")?,
            business_days: days("delivery.estimate.business_days")?,
            delivery_methods: words("delivery.methods")?,
            zone_name: text("delivery.default_zone.name")?,
            zone_countries: words("delivery.default_zone.countries")?,
            shipping_rate: amount("delivery.default_zone.shipping_rate")?,
            zone_surcharge: amount("delivery.default_zone.surcharge")?,
            transit_business_days: int("delivery.default_zone.transit_business_days")?,
            free_shipping_min,
            points_rate: base_rate * multiplier,
            file,
        })
    }

    /// The liability a claim reason starts with (`liability.claim_reason_default`).
    pub fn liability_for(&self, reason: &str) -> String {
        self.liability_default
            .get(reason)
            .cloned()
            .unwrap_or_else(|| "none".to_owned())
    }

    /// True when the store side pays return shipping for this liability.
    pub fn store_pays_return(&self, liability: &str) -> bool {
        liability != "customer_fault"
            && self
                .liability_pays
                .get(liability)
                .is_some_and(|p| p.iter().any(|x| x == "return_shipping"))
    }

    /// Who pays return shipping (`return_fees` word) for a liability: the defect rule for the
    /// store side, the change-of-mind rule for the shopper.
    pub fn return_fees_for(&self, liability: &str) -> String {
        if liability == "customer_fault" {
            self.changed_mind_return_fees.clone()
        } else {
            self.defect_return_fees.clone()
        }
    }

    /// The `when:` block of one return suggestion.
    pub fn suggestion(&self, name: &str) -> Option<&Value> {
        self.file.get("return_suggestions")?.get(name)
    }

    /// A value in the fulfillment file by dotted path (`cancel.self_service_states`).
    pub fn lookup(&self, path: &str) -> Option<&Value> {
        let mut v = &self.file;
        for p in path.split('.') {
            v = v.get(p)?;
        }
        Some(v)
    }

    /// The words of a list in the fulfillment file, empty when it is missing.
    pub fn list(&self, path: &str) -> Vec<String> {
        self.lookup(path)
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(|w| w.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Whether a structured `when:` condition holds for the facts (`fulfillment.yml` header):
/// `{field, op, value}` or `value_from: <key>`, `{all: [...]}`, `{any: [...]}`. A fact that is
/// missing never matches.
pub fn holds(when: &Value, facts: &Value, rules: &Rules) -> bool {
    if let Some(all) = when.get("all").and_then(Value::as_array) {
        return all.iter().all(|c| holds(c, facts, rules));
    }
    if let Some(any) = when.get("any").and_then(Value::as_array) {
        return any.iter().any(|c| holds(c, facts, rules));
    }
    let (Some(field), Some(op)) = (
        when.get("field").and_then(Value::as_str),
        when.get("op").and_then(Value::as_str),
    ) else {
        return false;
    };
    let expected = match when.get("value_from").and_then(Value::as_str) {
        Some(path) => match rules.lookup(path) {
            Some(v) => v,
            None => return false,
        },
        None => match when.get("value") {
            Some(v) => v,
            None => return false,
        },
    };
    let Some(actual) = facts.get(field).filter(|v| !v.is_null()) else {
        return false;
    };
    match op {
        "eq" => same(actual, expected),
        "ne" => !same(actual, expected),
        "in" => expected
            .as_array()
            .is_some_and(|l| l.iter().any(|e| same(actual, e))),
        _ => {
            let Some(order) = compare(actual, expected) else {
                return false;
            };
            match op {
                "gt" => order.is_gt(),
                "gte" => order.is_ge(),
                "lt" => order.is_lt(),
                "lte" => order.is_le(),
                _ => false,
            }
        }
    }
}

fn same(a: &Value, b: &Value) -> bool {
    a == b || compare(a, b).is_some_and(|o| o.is_eq())
}

/// Orders two numbers or two amounts in the same currency.
fn compare(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    if let (Some(x), Some(y)) = (money_of(a), money_of(b)) {
        return (x.currency == y.currency).then(|| x.value.cmp(&y.value));
    }
    Some(decimal(a)?.cmp(&decimal(b)?))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// The default policies from `policies/`, with the demo store's overrides where they
    /// differ (none for fulfillment: the demo store keeps the defaults, docs/scenarios.md §5).
    pub(crate) fn policy() -> PolicySet {
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

    pub(crate) fn rules() -> Rules {
        Rules::load(&policy()).unwrap()
    }

    pub(crate) fn usd(s: &str) -> Money {
        Money::new(Decimal::from_str(s).unwrap(), Currency::USD)
    }

    #[test]
    fn reads_the_policy_files() {
        let r = rules();
        assert_eq!(r.return_window_days, 30);
        assert_eq!(r.suggestion_order, ["decline", "ask_for_photos", "refund"]);
        assert_eq!((r.flag_returns, r.flag_days), (4, 90));
        assert_eq!(r.money_back_days, (3, 5));
        assert_eq!(r.shipping_rate, usd("6.00"));
        assert_eq!(r.free_shipping_min, usd("80.00"));
        assert_eq!(r.points_rate, Decimal::from_str("0.01").unwrap());
        assert_eq!(r.reversal_window_days["loyalty_points"], None);
        assert_eq!(r.liability_for("not_as_described"), "store_fault");
        assert_eq!(
            r.return_fees_for("customer_fault"),
            "return_fees_customer_responsibility"
        );
        assert_eq!(r.return_fees_for("store_fault"), "free_return");
    }

    #[test]
    fn evaluates_structured_conditions() {
        let r = rules();
        let window =
            json!({"field": "days_since_delivery", "op": "gt", "value_from": "return_window_days"});
        assert!(holds(&window, &json!({"days_since_delivery": 39}), &r));
        assert!(!holds(&window, &json!({"days_since_delivery": 30}), &r));
        assert!(!holds(&window, &json!({"days_since_delivery": null}), &r));
        let photos = json!({"all": [
            {"field": "reason", "op": "eq", "value": "damaged_in_transit"},
            {"field": "photos", "op": "eq", "value": 0}]});
        assert!(holds(
            &photos,
            &json!({"reason": "damaged_in_transit", "photos": 0}),
            &r
        ));
        assert!(!holds(
            &photos,
            &json!({"reason": "damaged_in_transit", "photos": 2}),
            &r
        ));
        let states =
            json!({"field": "line_state", "op": "in", "value_from": "cancel.self_service_states"});
        assert!(holds(&states, &json!({"line_state": "awaiting_stock"}), &r));
        assert!(!holds(&states, &json!({"line_state": "packed"}), &r));
        let zero = json!({"field": "extra_charge", "op": "eq", "value": {"value": 0.00, "currency": "USD"}});
        assert!(holds(&zero, &json!({"extra_charge": usd("0")}), &r));
        assert!(!holds(&zero, &json!({"extra_charge": usd("6.00")}), &r));
    }

    #[test]
    fn writes_money_and_days_for_people() {
        assert_eq!(money_text(usd("312")), "$312.00");
        assert_eq!(money_text(usd("2463")), "$2,463.00");
        assert_eq!(money_short(usd("200.00")), "$200");
        assert_eq!(money_short(usd("200.50")), "$200.50");
        let d = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
        assert_eq!(day_text(d), "Tue Nov 10");
        assert_eq!(date_text(d), "Nov 10");
    }
}
