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

/// `review.report_reasons`, `review.report_holds_review`: what a report on a review may say, and
/// whether it puts the review in the moderation queue.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportRules {
    pub reasons: Vec<String>,
    pub holds_review: bool,
}

impl ReportRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            reasons: words_at(p, "catalog.review.report_reasons")?,
            holds_review: lookup(p, "catalog.review.report_holds_review")?
                .as_bool()
                .ok_or_else(|| missing("catalog.review.report_holds_review"))?,
        })
    }
}

/// `question`: what a product question and its answer must pass, and who may answer.
#[derive(Debug, Clone, PartialEq)]
pub struct QuestionRules {
    /// `block` refuses text with a blocked word; `hold_for_review` saves the question hidden.
    pub profanity_filter: String,
    pub blocked_words: Vec<String>,
    pub private_allowed: bool,
    /// `edit_until: answered` — the asker may edit until the store answers.
    pub edit_until_answered: bool,
    /// `operator` (the store and its agent) and/or `shopper` (another signed-in shopper).
    pub answered_by: Vec<String>,
}

impl QuestionRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            profanity_filter: lookup(p, "catalog.question.profanity_filter")?
                .as_str()
                .ok_or_else(|| missing("catalog.question.profanity_filter"))?
                .to_owned(),
            blocked_words: words_at(p, "catalog.question.blocked_words")?
                .into_iter()
                .map(|w| w.to_lowercase())
                .collect(),
            private_allowed: lookup(p, "catalog.question.private_allowed")?
                .as_bool()
                .ok_or_else(|| missing("catalog.question.private_allowed"))?,
            edit_until_answered: lookup(p, "catalog.question.edit_until")?.as_str()
                == Some("answered"),
            answered_by: words_at(p, "catalog.question.answered_by")?,
        })
    }

    /// Whether a blocked word refuses the text (`block`) rather than holding it for review.
    pub fn blocks(&self) -> bool {
        self.profanity_filter == "block"
    }
}

/// `author_masking`: the name a shopper's review, question or answer shows (`Jamie` → `Ja***`).
#[derive(Debug, Clone, PartialEq)]
pub struct AuthorMasking {
    pub enabled: bool,
    pub visible_chars: usize,
    pub mask_char: char,
}

impl AuthorMasking {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        let enabled = lookup(p, "catalog.author_masking.enabled")?
            .as_bool()
            .ok_or_else(|| missing("catalog.author_masking.enabled"))?;
        let visible_chars = lookup(p, "catalog.author_masking.visible_chars")?
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| missing("catalog.author_masking.visible_chars"))?;
        let mask_char = lookup(p, "catalog.author_masking.mask_char")?
            .as_str()
            .and_then(|s| s.chars().next())
            .ok_or_else(|| missing("catalog.author_masking.mask_char"))?;
        Ok(Self {
            enabled,
            visible_chars,
            mask_char,
        })
    }

    /// The name to show: the first `visible_chars` characters and the rest masked. A name that
    /// short keeps its first character only, so some of it is always masked.
    pub fn shown(&self, name: &str) -> String {
        let name = name.trim();
        if !self.enabled || name.is_empty() {
            return name.to_owned();
        }
        let chars: Vec<char> = name.chars().collect();
        let keep = self.visible_chars.min(chars.len().saturating_sub(1)).max(1);
        let mut out: String = chars[..keep].iter().collect();
        out.extend(std::iter::repeat_n(self.mask_char, chars.len() - keep));
        out
    }
}

/// `restock_alert` and `inventory.auto_restore.notify_restock_alerts`: who may ask to hear when
/// a variant is back, how, for how long, and whether the store tells them.
#[derive(Debug, Clone, PartialEq)]
pub struct RestockRules {
    pub channels: Vec<String>,
    pub expires_after_days: u32,
    /// Availability words a variant can be waited for in.
    pub subscribable_availability: Vec<String>,
    /// `transactional`: sent regardless of marketing consent.
    pub message_kind: String,
    /// Whether a variant that comes back in stock by itself tells the shoppers waiting for it.
    pub notify: bool,
}

impl RestockRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            channels: words_at(p, "catalog.restock_alert.channels")?,
            expires_after_days: lookup(p, "catalog.restock_alert.expires_after_days")?
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| missing("catalog.restock_alert.expires_after_days"))?,
            subscribable_availability: words_at(
                p,
                "catalog.restock_alert.subscribable_availability",
            )?,
            message_kind: lookup(p, "catalog.restock_alert.message_kind")?
                .as_str()
                .ok_or_else(|| missing("catalog.restock_alert.message_kind"))?
                .to_owned(),
            notify: lookup(p, "catalog.inventory.auto_restore.notify_restock_alerts")?
                .as_bool()
                .ok_or_else(|| missing("catalog.inventory.auto_restore.notify_restock_alerts"))?,
        })
    }
}

/// `bundle`: what a bundle's components must share, and its limits.
#[derive(Debug, Clone, PartialEq)]
pub struct BundleRules {
    pub same_seller: bool,
    pub same_age_restriction: bool,
    pub components_must_be_active: bool,
    pub min_components: usize,
    /// The largest bundle discount, as a rate (0–1).
    pub max_discount: Decimal,
}

impl BundleRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        let flag = |path: &str| -> Result<bool, ToolError> {
            lookup(p, path)?.as_bool().ok_or_else(|| missing(path))
        };
        Ok(Self {
            same_seller: flag("catalog.bundle.same_seller")?,
            same_age_restriction: flag("catalog.bundle.same_age_restriction")?,
            components_must_be_active: flag("catalog.bundle.components_must_be_active")?,
            min_components: lookup(p, "catalog.bundle.min_components")?
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| missing("catalog.bundle.min_components"))?,
            max_discount: decimal(lookup(p, "catalog.bundle.max_discount")?)
                .ok_or_else(|| missing("catalog.bundle.max_discount"))?,
        })
    }
}

/// `content`: generated copy and images (`generate_copy`, `generate_image`).
#[derive(Debug, Clone, PartialEq)]
pub struct ContentRules {
    /// `placeholder` while no generator is connected.
    pub generator: String,
    /// `ai_content_disclosure: required` — drafts carry `acom:ai_generated: true`.
    pub disclosure_required: bool,
    /// `brand_guideline: enforced` — prompts with a blocked word are refused.
    pub brand_guideline_enforced: bool,
    pub prompt_blocked_words: Vec<String>,
    pub max_images_per_request: u32,
    pub title_max_chars: usize,
    pub seo_max_chars: usize,
}

impl ContentRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        let count = |path: &str| -> Result<u64, ToolError> {
            lookup(p, path)?.as_u64().ok_or_else(|| missing(path))
        };
        let word = |path: &str| -> Result<String, ToolError> {
            lookup(p, path)?
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| missing(path))
        };
        Ok(Self {
            generator: word("catalog.content.generator")?,
            disclosure_required: word("catalog.content.ai_content_disclosure")? == "required",
            brand_guideline_enforced: word("catalog.content.brand_guideline")? == "enforced",
            prompt_blocked_words: words_at(p, "catalog.content.prompt_blocked_words")?
                .into_iter()
                .map(|w| w.to_lowercase())
                .collect(),
            max_images_per_request: u32::try_from(count("catalog.content.max_images_per_request")?)
                .map_err(|_| missing("catalog.content.max_images_per_request"))?,
            title_max_chars: usize::try_from(count("catalog.content.copy.title_max_chars")?)
                .map_err(|_| missing("catalog.content.copy.title_max_chars"))?,
            seo_max_chars: usize::try_from(count("catalog.content.copy.seo_max_chars")?)
                .map_err(|_| missing("catalog.content.copy.seo_max_chars"))?,
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
    fn engagement_rules_read_the_policy() {
        let p = policies();
        let q = QuestionRules::from_policy(&p).unwrap_or_else(|e| panic!("{e}"));
        assert!(q.blocks() && q.private_allowed && q.edit_until_answered);
        assert_eq!(q.answered_by, ["operator"]);
        let r = ReportRules::from_policy(&p).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(r.reasons.len(), 7);
        assert!(r.holds_review);
        let s = RestockRules::from_policy(&p).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(s.expires_after_days, 90);
        assert!(s.notify && s.channels.contains(&"email".to_owned()));
        let b = BundleRules::from_policy(&p).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(b.min_components, 2);
        assert_eq!(
            b.max_discount,
            Decimal::from_str("0.30").unwrap_or_default()
        );
        let c = ContentRules::from_policy(&p).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(c.generator, "placeholder");
        assert_eq!(
            (c.max_images_per_request, c.title_max_chars, c.seo_max_chars),
            (4, 70, 155)
        );
        assert!(c.disclosure_required && c.brand_guideline_enforced);
    }

    #[test]
    fn author_names_are_masked_as_the_policy_says() {
        let m = AuthorMasking::from_policy(&policies()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(m.shown("Jamie"), "Ja***");
        assert_eq!(m.shown("Al"), "A*");
        assert_eq!(m.shown(" Jo "), "J*");
        let off = AuthorMasking {
            enabled: false,
            ..m
        };
        assert_eq!(off.shown("Jamie"), "Jamie");
    }

    #[test]
    fn reads_policy_numbers_exactly() {
        let v: Value = serde_json::from_str("1.15").unwrap_or_default();
        assert_eq!(decimal(&v), Decimal::from_str("1.15").ok());
        let m = money(&json!({"value": 80.00, "currency": "USD"}));
        assert_eq!(m.map(|m| m.value.to_string()), Some("80.00".into()));
    }
}
