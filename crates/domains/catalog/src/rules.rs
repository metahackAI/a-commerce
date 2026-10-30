//! The catalog rules the operations apply, read from the store's policies
//! (`policies/catalog.yml` plus overrides). Nothing here is a constant: a missing key is an
//! error, so a broken policy file shows up at the first call instead of as a silent default.

use std::str::FromStr;

use acom_contract::{ErrorCode, ToolError};
use acom_core::Money;
use acom_policy::PolicySet;
use rust_decimal::Decimal;
use serde_json::Value;

fn missing(path: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INTERNAL,
        format!("The store policy `{path}` is missing or malformed."),
    )
}

/// An exact decimal from a policy number (`1.15`), a numeric string, or `{value, currency}`.
pub fn decimal(v: &Value) -> Option<Decimal> {
    match v {
        Value::Number(n) => {
            let s = n.to_string();
            Decimal::from_str(&s)
                .or_else(|_| Decimal::from_scientific(&s))
                .ok()
        }
        Value::String(s) => Decimal::from_str(s).ok(),
        Value::Object(o) => o.get("value").and_then(decimal),
        _ => None,
    }
}

/// A policy amount `{value, currency}`, written to the currency's minor unit (`80.00`; the YAML
/// reader keeps only `80.0`).
pub fn money(v: &Value) -> Option<Money> {
    let mut value = decimal(v.get("value")?)?;
    let currency = acom_core::Currency::parse(v.get("currency")?.as_str()?).ok()?;
    value.rescale(currency.minor_units());
    Some(Money::new(value, currency))
}

pub(crate) fn lookup<'a>(p: &'a PolicySet, path: &str) -> Result<&'a Value, ToolError> {
    p.lookup(path).ok_or_else(|| missing(path))
}

/// `price_floor`: a price must stay at or above cost × multiplier, rounded to the cent.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceFloorRule {
    pub multiplier: Decimal,
    /// `compare: gt` — the price must be above the floor, not equal to it.
    pub strict: bool,
}

impl PriceFloorRule {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        let multiplier = decimal(lookup(p, "catalog.price_floor.multiplier")?)
            .ok_or_else(|| missing("catalog.price_floor.multiplier"))?;
        let strict = lookup(p, "catalog.price_floor.compare")?.as_str() == Some("gt");
        Ok(Self { multiplier, strict })
    }

    /// The markup over cost in words, for example `cost + 15%`.
    pub fn markup_text(&self) -> String {
        let percent = ((self.multiplier - Decimal::ONE) * Decimal::ONE_HUNDRED).normalize();
        format!("cost + {percent}%")
    }

    /// The lowest allowed price for a cost, rounded to the currency's minor unit.
    pub fn floor(&self, cost: Money) -> Money {
        cost.times(self.multiplier).rounded()
    }

    /// Whether `price` respects the floor of `cost`.
    pub fn allows(&self, price: Money, cost: Money) -> bool {
        let floor = self.floor(cost);
        if price.currency != floor.currency {
            return false;
        }
        if self.strict {
            price.value > floor.value
        } else {
            price.value >= floor.value
        }
    }
}

/// `inventory.auto_out_of_stock` and `inventory.auto_restore`: when a stock change moves a
/// variant's availability by itself.
#[derive(Debug, Clone, PartialEq)]
pub struct StockRules {
    pub out_of_stock_when: Value,
    pub out_of_stock_to: String,
    pub restore_when: Value,
    pub restore_to: String,
}

impl StockRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        let word = |path: &str| -> Result<String, ToolError> {
            lookup(p, path)?
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| missing(path))
        };
        Ok(Self {
            out_of_stock_when: lookup(p, "catalog.inventory.auto_out_of_stock.when")?.clone(),
            out_of_stock_to: word("catalog.inventory.auto_out_of_stock.set_availability")?,
            restore_when: lookup(p, "catalog.inventory.auto_restore.when")?.clone(),
            restore_to: word("catalog.inventory.auto_restore.set_availability")?,
        })
    }
}

/// `review`: what a new review must pass.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewRules {
    pub verified_purchase_only: bool,
    pub one_per_order_item: bool,
    /// `block` refuses text with a blocked word; `hold_for_review` accepts it as reported.
    pub profanity_filter: String,
    pub blocked_words: Vec<String>,
}

impl ReviewRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        let flag = |path: &str| -> Result<bool, ToolError> {
            lookup(p, path)?.as_bool().ok_or_else(|| missing(path))
        };
        Ok(Self {
            verified_purchase_only: flag("catalog.review.verified_purchase_only")?,
            one_per_order_item: flag("catalog.review.one_per_order_item")?,
            profanity_filter: lookup(p, "catalog.review.profanity_filter")?
                .as_str()
                .unwrap_or("block")
                .to_owned(),
            blocked_words: p
                .lookup("catalog.review.blocked_words")
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_lowercase)
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

/// Whether a structured policy condition holds for the facts: `{field, op, value}`,
/// `{all: [...]}`, `{any: [...]}` (the same form as `approval.yml`).
pub fn holds(cond: &Value, facts: &Value) -> bool {
    if let Some(all) = cond.get("all").and_then(Value::as_array) {
        return all.iter().all(|c| holds(c, facts));
    }
    if let Some(any) = cond.get("any").and_then(Value::as_array) {
        return any.iter().any(|c| holds(c, facts));
    }
    let (Some(field), Some(op), Some(expected)) = (
        cond.get("field").and_then(Value::as_str),
        cond.get("op").and_then(Value::as_str),
        cond.get("value"),
    ) else {
        return false;
    };
    let Some(actual) = facts.get(field) else {
        return false;
    };
    let numbers = decimal(actual).zip(decimal(expected));
    match op {
        "eq" => actual == expected || numbers.is_some_and(|(a, b)| a == b),
        "ne" => !(actual == expected || numbers.is_some_and(|(a, b)| a == b)),
        "in" => expected.as_array().is_some_and(|l| l.contains(actual)),
        "gt" => numbers.is_some_and(|(a, b)| a > b),
        "gte" => numbers.is_some_and(|(a, b)| a >= b),
        "lt" => numbers.is_some_and(|(a, b)| a < b),
        "lte" => numbers.is_some_and(|(a, b)| a <= b),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::policies;
    use acom_core::Currency;
    use serde_json::json;

    fn usd(s: &str) -> Money {
        Money::new(Decimal::from_str(s).unwrap_or_default(), Currency::USD)
    }

    #[test]
    fn floor_is_cost_plus_fifteen_percent_rounded_to_the_cent() {
        let rule = PriceFloorRule::from_policy(&policies()).unwrap_or_else(|e| panic!("{e}"));
        // scenarios.md §3.1: overshirt cost $58.00 → floor $66.70; 20% off $79.00 fails.
        assert_eq!(rule.floor(usd("58.00")), usd("66.70"));
        assert!(!rule.allows(usd("63.20"), usd("58.00")));
        assert!(rule.allows(usd("67.15"), usd("58.00")));
        // Equal to the floor passes (compare: gte); the windbreaker floor is $41.40.
        assert_eq!(rule.floor(usd("36.00")), usd("41.40"));
        assert!(rule.allows(usd("41.40"), usd("36.00")));
        // The 08:31 fix raised the Breton tee from $19.00 to $23.00 on a $20.00 cost.
        assert!(rule.allows(usd("23.00"), usd("20.00")));
        assert!(!rule.allows(usd("19.00"), usd("20.00")));
    }

    #[test]
    fn stock_conditions_come_from_the_policy() {
        let rules = StockRules::from_policy(&policies()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(rules.out_of_stock_to, "out_of_stock");
        assert!(holds(
            &rules.out_of_stock_when,
            &json!({"inventory_level": 0})
        ));
        assert!(!holds(
            &rules.out_of_stock_when,
            &json!({"inventory_level": 3})
        ));
        assert!(holds(
            &rules.restore_when,
            &json!({"inventory_level": 5, "auto_out_of_stock": true})
        ));
        assert!(!holds(
            &rules.restore_when,
            &json!({"inventory_level": 5, "auto_out_of_stock": false})
        ));
    }

    #[test]
    fn review_rules_read_the_policy() {
        let r = ReviewRules::from_policy(&policies()).unwrap_or_else(|e| panic!("{e}"));
        assert!(r.verified_purchase_only && r.one_per_order_item);
        assert_eq!(r.profanity_filter, "block");
    }

    #[test]
    fn reads_policy_numbers_exactly() {
        let v: Value = serde_json::from_str("1.15").unwrap_or_default();
        assert_eq!(decimal(&v), Decimal::from_str("1.15").ok());
        let m = money(&json!({"value": 80.00, "currency": "USD"}));
        assert_eq!(m.map(|m| m.value.to_string()), Some("80.00".into()));
    }
}
