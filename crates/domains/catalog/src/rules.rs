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

fn words_at(p: &PolicySet, path: &str) -> Result<Vec<String>, ToolError> {
    lookup(p, path)?
        .as_array()
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .ok_or_else(|| missing(path))
}

/// `pricing_rule`: the list price a new draft gets from its cost — cost × multiplier, then the
/// nearest whole price ending in 9 (`round_to: nearest_ending_in_9`), ties going up or down.
#[derive(Debug, Clone, PartialEq)]
pub struct PricingRule {
    pub multiplier: Decimal,
    /// `true` for `round_to: nearest_ending_in_9`; `false` rounds to the cent only.
    pub ending_in_9: bool,
    /// `tie: up` — the higher of two equally near prices.
    pub tie_up: bool,
}

impl PricingRule {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        let multiplier = decimal(lookup(p, "catalog.pricing_rule.multiplier")?)
            .ok_or_else(|| missing("catalog.pricing_rule.multiplier"))?;
        let ending_in_9 = match lookup(p, "catalog.pricing_rule.round_to")?.as_str() {
            Some("nearest_ending_in_9") => true,
            Some("cent") => false,
            _ => return Err(missing("catalog.pricing_rule.round_to")),
        };
        let tie_up = p
            .lookup("catalog.pricing_rule.tie")
            .and_then(Value::as_str)
            .is_none_or(|t| t == "up");
        Ok(Self {
            multiplier,
            ending_in_9,
            tie_up,
        })
    }

    /// The price for a cost. With `nearest_ending_in_9` the candidates are whole amounts
    /// 9, 19, 29, …: $96 × 2.5 = $240 → $239; $52 × 2.5 = $130 → $129; $33.60 × 2.5 = $84 is as
    /// near $79 as $89, and `tie: up` picks $89.
    pub fn price(&self, cost: Money) -> Money {
        let target = cost.value * self.multiplier;
        if !self.ending_in_9 {
            return Money::new(target, cost.currency).rounded();
        }
        let ten = Decimal::TEN;
        let nine = Decimal::from(9);
        let below = ((target - nine) / ten).floor() * ten + nine;
        let above = below + ten;
        let pick = if below < Decimal::ZERO {
            above
        } else {
            let (down, up) = (target - below, above - target);
            if down < up || (down == up && !self.tie_up) {
                below
            } else {
                above
            }
        };
        let mut value = pick;
        value.rescale(cost.currency.minor_units());
        Money::new(value, cost.currency)
    }

    /// The rule in words, for the "rules used" of a preview.
    pub fn text(&self) -> String {
        let m = self.multiplier.normalize();
        if self.ending_in_9 {
            format!(
                "Cost × {m}, to the nearest price ending in 9 (a tie goes {})",
                if self.tie_up { "up" } else { "down" }
            )
        } else {
            format!("Cost × {m}, to the cent")
        }
    }
}

/// `import`: how `batch_import` reads a supplier sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportRules {
    pub max_rows: usize,
    /// Fields a draft must have before it can be published; missing ones are flagged.
    pub required_fields: Vec<String>,
    /// `save_as_draft` keeps a row with open fields as a flagged draft; `reject_row` refuses it.
    pub save_open_rows: bool,
    /// Two rows with the same photo make that photo unclear for both.
    pub one_product_per_image: bool,
    /// Category fields the sheet's category word is matched against (`name`, `term_code`).
    pub category_match: Vec<String>,
}

impl ImportRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            max_rows: lookup(p, "catalog.import.max_rows")?
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| missing("catalog.import.max_rows"))?,
            required_fields: words_at(p, "catalog.import.required_fields")?,
            save_open_rows: lookup(p, "catalog.import.on_open_field")?.as_str()
                != Some("reject_row"),
            one_product_per_image: lookup(p, "catalog.import.one_product_per_image")?
                .as_bool()
                .ok_or_else(|| missing("catalog.import.one_product_per_image"))?,
            category_match: words_at(p, "catalog.import.category_match")?,
        })
    }
}

/// `product.required_on_create`, `product.publish_requires`: what a new product must have, and
/// what it must have before it is published.
#[derive(Debug, Clone, PartialEq)]
pub struct ProductRules {
    pub required_on_create: Vec<String>,
    pub publish_requires: Vec<String>,
}

impl ProductRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            required_on_create: words_at(p, "catalog.product.required_on_create")?,
            publish_requires: words_at(p, "catalog.product.publish_requires")?,
        })
    }
}

/// `content.copy`: the store's tone for descriptions.
#[derive(Debug, Clone, PartialEq)]
pub struct CopyRules {
    pub tone: String,
    pub sentences: u64,
    pub care_line: bool,
}

impl CopyRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            tone: lookup(p, "catalog.content.copy.tone")?
                .as_str()
                .ok_or_else(|| missing("catalog.content.copy.tone"))?
                .to_owned(),
            sentences: lookup(p, "catalog.content.copy.sentences")?
                .as_u64()
                .ok_or_else(|| missing("catalog.content.copy.sentences"))?,
            care_line: lookup(p, "catalog.content.copy.care_line")?
                .as_bool()
                .unwrap_or(false),
        })
    }

    /// "2 sentences + care line".
    pub fn shape_text(&self) -> String {
        format!(
            "{} sentence{}{}",
            self.sentences,
            if self.sentences == 1 { "" } else { "s" },
            if self.care_line { " + care line" } else { "" }
        )
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

/// `reorder`: how a reorder to the supplier is planned (`create_reorder`).
#[derive(Debug, Clone, PartialEq)]
pub struct ReorderRules {
    /// Calendar days from the order date to the expected arrival.
    pub lead_time_days: u32,
    /// The most units of one variant in one reorder.
    pub max_units_per_line: i64,
    /// Availability words a variant can be reordered in (a discontinued one cannot).
    pub orderable_availability: Vec<String>,
}

impl ReorderRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        let count = |path: &str| -> Result<u64, ToolError> {
            lookup(p, path)?.as_u64().ok_or_else(|| missing(path))
        };
        Ok(Self {
            lead_time_days: u32::try_from(count("catalog.reorder.lead_time_days")?)
                .map_err(|_| missing("catalog.reorder.lead_time_days"))?,
            max_units_per_line: i64::try_from(count("catalog.reorder.max_units_per_line")?)
                .map_err(|_| missing("catalog.reorder.max_units_per_line"))?,
            orderable_availability: words_at(p, "catalog.reorder.orderable_availability")?,
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
    fn pricing_rule_rounds_to_the_nearest_price_ending_in_9() {
        let rule = PricingRule::from_policy(&policies()).unwrap_or_else(|e| panic!("{e}"));
        // scenarios.md §3.3: the six named drafts.
        for (cost, price) in [
            ("96.00", "239.00"),
            ("38.00", "99.00"),
            ("30.00", "79.00"),
            ("52.00", "129.00"),
            ("34.00", "89.00"),
            ("44.00", "109.00"),
        ] {
            assert_eq!(rule.price(usd(cost)), usd(price), "cost {cost}");
        }
        // $33.60 × 2.5 = $84.00 is as near $79 as $89: a tie goes up.
        assert_eq!(rule.price(usd("33.60")), usd("89.00"));
        // An exact x9 stays; a target under $9 goes up to $9.
        assert_eq!(rule.price(usd("27.60")), usd("69.00"));
        assert_eq!(rule.price(usd("2.00")), usd("9.00"));
        assert_eq!(rule.price(usd("96.00")).value.to_string(), "239.00");
        assert_eq!(
            rule.text(),
            "Cost × 2.5, to the nearest price ending in 9 (a tie goes up)"
        );
    }

    #[test]
    fn import_product_and_copy_rules_read_the_policy() {
        let i = ImportRules::from_policy(&policies()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(i.required_fields, ["material", "image"]);
        assert!(i.save_open_rows && i.one_product_per_image);
        let p = ProductRules::from_policy(&policies()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(p.required_on_create, ["seller", "category", "name"]);
        assert!(p.publish_requires.iter().any(|r| r == "no_open_fields"));
        let c = CopyRules::from_policy(&policies()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c.shape_text(), "2 sentences + care line");
    }

    #[test]
    fn reads_policy_numbers_exactly() {
        let v: Value = serde_json::from_str("1.15").unwrap_or_default();
        assert_eq!(decimal(&v), Decimal::from_str("1.15").ok());
        let m = money(&json!({"value": 80.00, "currency": "USD"}));
        assert_eq!(m.map(|m| m.value.to_string()), Some("80.00".into()));
    }
}
