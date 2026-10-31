//! The promotion domain's rules as pure functions: the policy values a sale uses, sale prices,
//! the four sale checks (`max_discount`, `price_floor`, `promo_min_stock`, `promotion_overlap`),
//! and promotion status from the period and the clock. No I/O, so the B1 numbers of
//! `docs/scenarios.md` are unit-tested here.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use acom_catalog::service::for_promotion::ProductOffers;
use acom_contract::{Check, ErrorCode, ToolError};
use acom_core::enums::{DiscountLayer, PromotionStatus};
use acom_policy::PolicySet;
use chrono::{DateTime, Duration, Utc};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::Deserialize;
use serde_json::Value;

/// A policy value that should be there and is not: the policy files are broken, not the input.
fn missing(key: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INTERNAL,
        format!("The policy value `{key}` is missing or not readable."),
    )
}

/// A number from a policy value, exactly (ratios such as 0.30 stay 0.30).
pub fn decimal(v: &Value) -> Option<Decimal> {
    match v {
        Value::Number(n) => Decimal::from_str(&n.to_string())
            .or_else(|_| Decimal::from_scientific(&n.to_string()))
            .ok(),
        Value::String(s) => Decimal::from_str(s).ok(),
        _ => None,
    }
}

fn number_at(p: &PolicySet, key: &str) -> Result<Decimal, ToolError> {
    p.lookup(key).and_then(decimal).ok_or_else(|| missing(key))
}

fn word_at(p: &PolicySet, key: &str) -> Result<String, ToolError> {
    p.lookup(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| missing(key))
}

/// The policy values a member or sale price uses (`promotion.yml`, `catalog.yml price_floor`).
#[derive(Debug, Clone, PartialEq)]
pub struct SaleRules {
    /// `max_discount.everyone`, a ratio 0–1.
    pub max_discount_everyone: Decimal,
    /// `max_discount.by_tier`: tier id → ratio.
    pub max_discount_by_tier: BTreeMap<String, Decimal>,
    /// `max_discount.on_unknown_tier` is `use_everyone`.
    pub unknown_tier_uses_everyone: bool,
    /// `promo_min_stock`: products with fewer units (all variants) are left out or flagged.
    pub promo_min_stock: i64,
    /// `catalog.price_floor.multiplier`: the floor is cost × this.
    pub floor_multiplier: Decimal,
    /// `catalog.price_floor.compare` is `gte` (a price equal to the floor passes).
    pub floor_allows_equal: bool,
    /// `sale_price.rounding_mode`.
    pub rounding: RoundingStrategy,
    /// `promotion_overlap.compare_statuses`.
    pub overlap_statuses: Vec<PromotionStatus>,
    /// `promotion_overlap.same_product_same_audience` is `block`.
    pub overlap_blocks: bool,
}

impl SaleRules {
    /// Reads the rules from the store's policies.
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        let by_tier = p
            .lookup("promotion.max_discount.by_tier")
            .and_then(Value::as_object)
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| decimal(v).map(|d| (k.clone(), d)))
                    .collect()
            })
            .unwrap_or_default();
        let statuses = p
            .lookup("promotion.promotion_overlap.compare_statuses")
            .and_then(Value::as_array)
            .ok_or_else(|| missing("promotion.promotion_overlap.compare_statuses"))?
            .iter()
            .filter_map(|s| s.as_str()?.parse().ok())
            .collect();
        let rounding = match word_at(p, "promotion.sale_price.rounding_mode")?.as_str() {
            "half_even" => RoundingStrategy::MidpointNearestEven,
            "down" => RoundingStrategy::ToZero,
            _ => RoundingStrategy::MidpointAwayFromZero,
        };
        Ok(Self {
            max_discount_everyone: number_at(p, "promotion.max_discount.everyone")?,
            max_discount_by_tier: by_tier,
            unknown_tier_uses_everyone: word_at(p, "promotion.max_discount.on_unknown_tier")?
                == "use_everyone",
            promo_min_stock: number_at(p, "promotion.promo_min_stock")?
                .trunc()
                .try_into()
                .map_err(|_| missing("promotion.promo_min_stock"))?,
            floor_multiplier: number_at(p, "catalog.price_floor.multiplier")?,
            floor_allows_equal: word_at(p, "catalog.price_floor.compare")? == "gte",
            rounding,
            overlap_statuses: statuses,
            overlap_blocks: word_at(p, "promotion.promotion_overlap.same_product_same_audience")?
                == "block",
        })
    }

    /// The highest discount (ratio) the tier may get, `None` when the policy sets none.
    pub fn max_discount(&self, tier: Option<&str>) -> Option<Decimal> {
        match tier {
            None => Some(self.max_discount_everyone),
            Some(t) => match self.max_discount_by_tier.get(t) {
                Some(cap) => Some(*cap),
                None if self.unknown_tier_uses_everyone => Some(self.max_discount_everyone),
                None => None,
            },
        }
    }

    /// The sale price: list × (1 − percent / 100), rounded to the cent.
    pub fn sale_price(&self, list: Decimal, percent_off: Decimal) -> Decimal {
        let raw = list * (Decimal::ONE_HUNDRED - percent_off) / Decimal::ONE_HUNDRED;
        cents(raw.round_dp_with_strategy(2, self.rounding))
    }

    /// The price floor of a variant: cost × multiplier, rounded to the cent.
    pub fn floor(&self, cost: Decimal) -> Decimal {
        cents((cost * self.floor_multiplier).round_dp_with_strategy(2, self.rounding))
    }

    /// Whether `price` is allowed for a variant whose floor is `floor`.
    pub fn above_floor(&self, price: Decimal, floor: Decimal) -> bool {
        if self.floor_allows_equal {
            price >= floor
        } else {
            price > floor
        }
    }

    /// The tiers other than `tier` whose limit (`max_discount.by_tier`) allows `asked` percent,
    /// with their limit as a percentage, lowest limit first.
    pub fn tiers_allowing(&self, asked: Decimal, tier: Option<&str>) -> Vec<(String, Decimal)> {
        let mut out: Vec<(String, Decimal)> = self
            .max_discount_by_tier
            .iter()
            .filter(|(t, _)| Some(t.as_str()) != tier)
            .map(|(t, cap)| (t.clone(), *cap * Decimal::ONE_HUNDRED))
            .filter(|(_, cap)| *cap >= asked)
            .collect();
        out.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    /// A failed `max_discount` detail: "25% is over the 20% limit for everyone; vip allows up
    /// to 30%". The tiers that would allow the percent come from `max_discount.by_tier`, so the
    /// agent can offer the discount limited to them; without any, the plain sentence.
    pub fn over_limit_words(&self, asked: Decimal, tier: Option<&str>, cap: Decimal) -> String {
        let plain = format!(
            "{}% is over the {}% limit for {}",
            percent_text(asked),
            percent_text(cap * Decimal::ONE_HUNDRED),
            tier.unwrap_or("everyone")
        );
        let allowing: Vec<String> = self
            .tiers_allowing(asked, tier)
            .iter()
            .map(|(t, cap)| format!("{t} allows up to {}%", percent_text(*cap)))
            .collect();
        if allowing.is_empty() {
            plain
        } else {
            format!("{plain}; {}", and_list(&allowing))
        }
    }

    /// "cost + 15%", from the floor multiplier.
    pub fn floor_words(&self) -> String {
        format!(
            "cost + {}%",
            percent_text((self.floor_multiplier - Decimal::ONE) * Decimal::ONE_HUNDRED)
        )
    }
}

/// Two decimals, so 71.2 prints as 71.20.
pub fn cents(d: Decimal) -> Decimal {
    let mut d = d;
    d.rescale(2);
    d
}

/// `20`, `12.5`: a percentage without trailing zeros.
pub fn percent_text(d: Decimal) -> String {
    d.normalize().to_string()
}

/// `$67.15` for USD, `67.15 EUR` otherwise.
pub fn money_text(d: Decimal, currency: &str) -> String {
    match currency {
        "USD" => format!("${}", cents(d)),
        other => format!("{} {other}", cents(d)),
    }
}

/// `a`, `a and b`, `a, b and c`.
pub fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A promotion's status now: `draft`, `paused` and `ended` are set by people and stay; the rest
/// follows the period (both ends inclusive, minute precision) and the store clock.
pub fn promotion_status(
    stored: &str,
    valid_from: DateTime<Utc>,
    valid_through: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> PromotionStatus {
    match stored.parse::<PromotionStatus>() {
        Ok(s @ (PromotionStatus::Draft | PromotionStatus::Paused | PromotionStatus::Ended)) => s,
        _ if now < valid_from => PromotionStatus::Scheduled,
        _ if valid_through.is_some_and(|t| now >= t + Duration::minutes(1)) => {
            PromotionStatus::Ended
        }
        _ => PromotionStatus::Running,
    }
}

/// `promotion.applies_to`: what a promotion covers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, serde::Serialize)]
pub struct AppliesTo {
    /// `products`, `categories` or `order`.
    #[serde(default)]
    pub scope: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub product_ids: Vec<String>,
    #[serde(default)]
    pub category_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_product_ids: Vec<String>,
}

impl AppliesTo {
    pub fn read(v: &Value) -> Self {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }
}

/// One variant's sale price.
#[derive(Debug, Clone, PartialEq)]
pub struct PricedOffer {
    pub offer_id: String,
    pub list_price: Decimal,
    pub sale_price: Decimal,
    pub currency: String,
    pub cost: Option<Decimal>,
    /// The floor, when the cost is known.
    pub floor: Option<Decimal>,
}

/// A product in the sale.
#[derive(Debug, Clone, PartialEq)]
pub struct PricedProduct {
    pub product_id: String,
    pub name: String,
    pub percent_off: Decimal,
    pub stock: i64,
    pub offers: Vec<PricedOffer>,
}

impl PricedProduct {
    /// The variant shown on the change row: the lowest list price.
    pub fn shown(&self) -> Option<&PricedOffer> {
        self.offers.iter().min_by_key(|o| o.list_price)
    }

    /// The variant with the least room above its floor.
    fn tightest(&self) -> Option<&PricedOffer> {
        self.offers
            .iter()
            .filter(|o| o.floor.is_some())
            .min_by_key(|o| o.sale_price - o.floor.unwrap_or_default())
    }
}

/// Why a product in scope is left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftOut {
    /// Fewer units than `promo_min_stock`.
    LowStock,
    /// Not `active` in the catalog.
    NotSellable,
    /// No variant has an offer.
    NoOffer,
}

impl LeftOut {
    /// The `why` word on the `exclude` change row (the policy key for stock).
    pub fn word(self) -> &'static str {
        match self {
            LeftOut::LowStock => "promo_min_stock",
            LeftOut::NotSellable => "not_sellable",
            LeftOut::NoOffer => "no_offer",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Excluded {
    pub product_id: String,
    pub name: String,
    pub why: LeftOut,
    pub stock: i64,
}

/// What `set_sale_prices` asks for, after the input was read.
#[derive(Debug, Clone, PartialEq)]
pub struct SaleAsk {
    /// `None` for everyone.
    pub tier: Option<String>,
    pub percent_off: Decimal,
    /// Product id → its own percent off.
    pub overrides: BTreeMap<String, Decimal>,
    pub exclude_low_stock: bool,
}

impl SaleAsk {
    pub fn percent_for(&self, product_id: &str) -> Decimal {
        self.overrides
            .get(product_id)
            .copied()
            .unwrap_or(self.percent_off)
    }

    /// `vip`, or `everyone`.
    pub fn audience(&self) -> &str {
        self.tier.as_deref().unwrap_or("everyone")
    }
}

/// Products priced and left out, before the overlap check (which needs other promotions).
#[derive(Debug, Clone, PartialEq)]
pub struct SalePlan {
    pub included: Vec<PricedProduct>,
    pub excluded: Vec<Excluded>,
    /// Included products under `promo_min_stock` (only when `exclude_low_stock` is false).
    pub low_stock_included: Vec<(String, i64)>,
}

/// Prices every product in scope and decides which ones are left out.
pub fn plan_sale(rules: &SaleRules, ask: &SaleAsk, products: &[ProductOffers]) -> SalePlan {
    let mut plan = SalePlan {
        included: Vec::new(),
        excluded: Vec::new(),
        low_stock_included: Vec::new(),
    };
    for p in products {
        let stock = p.stock();
        let left_out = if p.approval_status != "active" {
            Some(LeftOut::NotSellable)
        } else if p.offers.is_empty() {
            Some(LeftOut::NoOffer)
        } else if stock < rules.promo_min_stock && ask.exclude_low_stock {
            Some(LeftOut::LowStock)
        } else {
            None
        };
        if let Some(why) = left_out {
            plan.excluded.push(Excluded {
                product_id: p.product_id.clone(),
                name: p.name.clone(),
                why,
                stock,
            });
            continue;
        }
        if stock < rules.promo_min_stock {
            plan.low_stock_included.push((p.name.clone(), stock));
        }
        let percent = ask.percent_for(&p.product_id);
        let offers = p
            .offers
            .iter()
            .map(|o| {
                let floor = o.cost_value.map(|c| rules.floor(c));
                PricedOffer {
                    offer_id: o.offer_id.clone(),
                    list_price: cents(o.price),
                    sale_price: rules.sale_price(o.price, percent),
                    currency: o.price_currency.clone(),
                    cost: o.cost_value,
                    floor,
                }
            })
            .collect();
        plan.included.push(PricedProduct {
            product_id: p.product_id.clone(),
            name: p.name.clone(),
            percent_off: percent,
            stock,
            offers,
        });
    }
    plan
}

/// `max_discount`: the highest percent asked stays within the tier's limit.
pub fn check_max_discount(rules: &SaleRules, ask: &SaleAsk, plan: &SalePlan) -> Check {
    let audience = ask.audience();
    let highest = plan.included.iter().max_by_key(|p| p.percent_off);
    let Some(cap) = rules.max_discount(ask.tier.as_deref()) else {
        return Check {
            rule: "max_discount".into(),
            passed: false,
            detail: Some(format!("The policy sets no discount limit for {audience}")),
            fix_tool: Some("get_policy".into()),
        };
    };
    let cap_percent = cap * Decimal::ONE_HUNDRED;
    let Some(highest) = highest else {
        return Check {
            rule: "max_discount".into(),
            passed: true,
            detail: Some("No prices change".into()),
            fix_tool: None,
        };
    };
    let passed = highest.percent_off <= cap_percent;
    let detail = if passed {
        format!(
            "Highest is {}%; policy allows {}% for {audience}",
            percent_text(highest.percent_off),
            percent_text(cap_percent)
        )
    } else {
        rules.over_limit_words(highest.percent_off, ask.tier.as_deref(), cap)
    };
    Check {
        rule: "max_discount".into(),
        passed,
        detail: Some(detail),
        fix_tool: (!passed).then(|| "set_sale_prices".into()),
    }
}

/// `price_floor`: no variant sells under cost × multiplier.
pub fn check_price_floor(rules: &SaleRules, ask: &SaleAsk, plan: &SalePlan) -> Check {
    let floor_words = rules.floor_words();
    let mut failures = Vec::new();
    let mut rescued = Vec::new();
    for p in &plan.included {
        let Some(tight) = p.tightest() else {
            continue;
        };
        let (Some(floor), Some(cost)) = (tight.floor, tight.cost) else {
            continue;
        };
        if !rules.above_floor(tight.sale_price, floor) {
            failures.push(format!(
                "{} at {}% would be {}, under the floor {} (cost {} + {}%)",
                p.name,
                percent_text(p.percent_off),
                money_text(tight.sale_price, &tight.currency),
                money_text(floor, &tight.currency),
                money_text(cost, &tight.currency),
                percent_text((rules.floor_multiplier - Decimal::ONE) * Decimal::ONE_HUNDRED),
            ));
        } else if p.percent_off < ask.percent_off {
            // Its own, smaller discount keeps it above the floor the base discount would break.
            let at_base = rules.sale_price(tight.list_price, ask.percent_off);
            if !rules.above_floor(at_base, floor) {
                rescued.push(format!(
                    "{} moved to {}% ({}) to stay at {floor_words} ({})",
                    p.name,
                    percent_text(p.percent_off),
                    money_text(tight.sale_price, &tight.currency),
                    money_text(floor, &tight.currency),
                ));
            }
        }
    }
    let passed = failures.is_empty();
    let detail = if !passed {
        failures.join("; ")
    } else if !rescued.is_empty() {
        rescued.join("; ")
    } else {
        format!("No price goes under {floor_words}")
    };
    Check {
        rule: "price_floor".into(),
        passed,
        detail: Some(detail),
        fix_tool: (!passed).then(|| "set_sale_prices".into()),
    }
}

/// `promo_min_stock`: products short of stock are left out (and named) or flagged.
pub fn check_min_stock(rules: &SaleRules, plan: &SalePlan) -> Check {
    let min = rules.promo_min_stock;
    let low_excluded: Vec<String> = plan
        .excluded
        .iter()
        .filter(|e| e.why == LeftOut::LowStock)
        .map(|e| format!("{} ({})", e.name, e.stock))
        .collect();
    if !plan.low_stock_included.is_empty() {
        let names: Vec<String> = plan
            .low_stock_included
            .iter()
            .map(|(n, s)| format!("{n} ({s})"))
            .collect();
        let verb = if names.len() == 1 { "is" } else { "are" };
        return Check {
            rule: "promo_min_stock".into(),
            passed: false,
            detail: Some(format!("{} {verb} under {min} units", and_list(&names))),
            fix_tool: Some("set_sale_prices".into()),
        };
    }
    let detail = match low_excluded.len() {
        0 => format!("Every product has at least {min} units"),
        1 => format!("1 product under {min} units excluded: {}", low_excluded[0]),
        n => format!(
            "{n} products under {min} units excluded: {}",
            low_excluded.join(", ")
        ),
    };
    Check {
        rule: "promo_min_stock".into(),
        passed: true,
        detail: Some(detail),
        fix_tool: None,
    }
}

/// A running or scheduled promotion the new sale is compared with.
#[derive(Debug, Clone, PartialEq)]
pub struct OtherPromotion {
    pub id: String,
    pub name: String,
    pub status: PromotionStatus,
    pub layer: DiscountLayer,
    /// `None` for everyone.
    pub tier: Option<String>,
    /// The products it covers; `None` when it covers every product (an order-wide scope).
    pub products: Option<BTreeSet<String>>,
}

/// `promotion_overlap`: no promotion with the same layer covers the same product for an
/// overlapping audience in the same period. `others` are the promotions in the compared
/// statuses whose period overlaps; `scope_words` names the sale's products ("summer outerwear").
pub fn check_overlap(
    rules: &SaleRules,
    tier: Option<&str>,
    products: &BTreeSet<String>,
    others: &[OtherPromotion],
    scope_words: &str,
) -> Check {
    let mut clashes = Vec::new();
    for o in others {
        let same_layer = o.layer == DiscountLayer::ItemPrice;
        let audiences_meet = tier.is_none() || o.tier.is_none() || o.tier.as_deref() == tier;
        let shared = match &o.products {
            None => products.len(),
            Some(theirs) => theirs.intersection(products).count(),
        };
        if same_layer && audiences_meet && shared > 0 {
            clashes.push(format!(
                "{} ({}) already gives an item price on {shared} of these products",
                o.name,
                o.tier.as_deref().unwrap_or("everyone")
            ));
        }
    }
    let counted = |s: PromotionStatus| others.iter().filter(|o| o.status == s).count();
    let parts: Vec<String> = rules
        .overlap_statuses
        .iter()
        .map(|s| (s, counted(*s)))
        .filter(|(_, n)| *n > 0)
        .map(|(s, n)| format!("{n} {s}"))
        .collect();
    let checked = if parts.is_empty() {
        "No running or scheduled promotion overlaps this period".to_owned()
    } else {
        let plural = if others.len() == 1 { "" } else { "s" };
        format!("Checked {} promotion{plural}", and_list(&parts))
    };
    if clashes.is_empty() {
        let tail = if others.is_empty() {
            String::new()
        } else {
            format!("; none gives an item price on {scope_words}")
        };
        return Check {
            rule: "promotion_overlap".into(),
            passed: true,
            detail: Some(format!("{checked}{tail}")),
            fix_tool: None,
        };
    }
    Check {
        rule: "promotion_overlap".into(),
        passed: !rules.overlap_blocks,
        detail: Some(clashes.join("; ")),
        fix_tool: rules
            .overlap_blocks
            .then(|| "list_active_promotions".into()),
    }
}

/// The policy check `member_benefits_per_order` for a coupon with this stacking rule.
pub fn check_member_benefits(per_order: i64, stacking: &str) -> Check {
    let stacks = stacking == "with_tier_coupons";
    let passed = !stacks || per_order > 1;
    let detail = match (stacks, stacking) {
        (false, "no_other_coupons") => {
            format!("Doesn't stack with any other coupon; {per_order} member benefit per order")
        }
        (false, _) => {
            format!("Doesn't stack with tier coupons; {per_order} member benefit per order")
        }
        (true, _) if passed => format!(
            "Stacks with tier coupons; the policy allows {per_order} member benefits per order"
        ),
        (true, _) => format!(
            "Stacks with tier coupons, but the policy allows {per_order} member benefit per order"
        ),
    };
    Check {
        rule: "member_benefits_per_order".into(),
        passed,
        detail: Some(detail),
        fix_tool: (!passed).then(|| "create_coupon".into()),
    }
}

/// `1,284`.
pub fn count_text(n: i64) -> String {
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::policies;
    use acom_catalog::service::for_promotion::VariantOffer;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn product(id: &str, name: &str, price: &str, cost: &str, stock: &[i32]) -> ProductOffers {
        ProductOffers {
            product_id: id.into(),
            name: name.into(),
            approval_status: "active".into(),
            offers: stock
                .iter()
                .enumerate()
                .map(|(i, s)| VariantOffer {
                    offer_id: format!("off_{}_{i}", &id[4..]),
                    variant_id: format!("var_{}_{i}", &id[4..]),
                    color: None,
                    size: None,
                    price: d(price),
                    price_currency: "USD".into(),
                    cost_value: Some(d(cost)),
                    cost_currency: Some("USD".into()),
                    inventory_level: *s,
                    availability: "in_stock".into(),
                    version: 1,
                })
                .collect(),
        }
    }

    #[test]
    fn reads_the_demo_rules() {
        let r = SaleRules::from_policy(&policies(true)).unwrap();
        assert_eq!(r.max_discount(Some("vip")), Some(d("0.3")));
        assert_eq!(r.max_discount(Some("gold")), Some(d("0.2")));
        assert_eq!(r.max_discount(None), Some(d("0.2")));
        assert_eq!(r.promo_min_stock, 10);
        assert_eq!(r.floor_words(), "cost + 15%");
        assert_eq!(
            r.overlap_statuses,
            vec![PromotionStatus::Running, PromotionStatus::Scheduled]
        );
        let plain = SaleRules::from_policy(&policies(false)).unwrap();
        assert_eq!(plain.max_discount(Some("vip")), Some(d("0.2")));
    }

    #[test]
    fn prices_round_half_up_to_the_cent() {
        let r = SaleRules::from_policy(&policies(true)).unwrap();
        assert_eq!(r.sale_price(d("128.00"), d("20")).to_string(), "102.40");
        assert_eq!(r.sale_price(d("89.00"), d("20")).to_string(), "71.20");
        assert_eq!(r.sale_price(d("79.00"), d("15")).to_string(), "67.15");
        assert_eq!(r.sale_price(d("118.00"), d("20")).to_string(), "94.40");
        assert_eq!(r.sale_price(d("79.00"), d("20")).to_string(), "63.20");
        // 0.5 of a cent goes up.
        assert_eq!(r.sale_price(d("10.05"), d("50")).to_string(), "5.03");
        assert_eq!(r.floor(d("58.00")).to_string(), "66.70");
        assert!(r.above_floor(d("66.70"), d("66.70")));
    }

    #[test]
    fn overshirt_fails_the_floor_at_20_and_passes_at_15() {
        let r = SaleRules::from_policy(&policies(true)).unwrap();
        let products = [product(
            "prd_seersucker_overshirt",
            "Seersucker overshirt",
            "79.00",
            "58.00",
            &[20, 33],
        )];
        let mut ask = SaleAsk {
            tier: Some("vip".into()),
            percent_off: d("20"),
            overrides: BTreeMap::new(),
            exclude_low_stock: false,
        };
        let plan = plan_sale(&r, &ask, &products);
        let c = check_price_floor(&r, &ask, &plan);
        assert!(!c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some(
                "Seersucker overshirt at 20% would be $63.20, under the floor $66.70 (cost $58.00 + 15%)"
            )
        );
        assert_eq!(c.fix_tool.as_deref(), Some("set_sale_prices"));

        ask.overrides
            .insert("prd_seersucker_overshirt".into(), d("15"));
        let plan = plan_sale(&r, &ask, &products);
        let c = check_price_floor(&r, &ask, &plan);
        assert!(c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("Seersucker overshirt moved to 15% ($67.15) to stay at cost + 15% ($66.70)")
        );
    }

    #[test]
    fn low_stock_is_flagged_or_left_out() {
        let r = SaleRules::from_policy(&policies(true)).unwrap();
        let products = [
            product(
                "prd_chore_coat",
                "Cotton chore coat",
                "142.00",
                "60.00",
                &[3, 0, 0, 4],
            ),
            product(
                "prd_rain_parka",
                "Light rain parka",
                "118.00",
                "50.00",
                &[2, 0, 0, 2],
            ),
            product(
                "prd_windbreaker",
                "Packable windbreaker",
                "89.00",
                "36.00",
                &[100, 114],
            ),
        ];
        let mut ask = SaleAsk {
            tier: Some("vip".into()),
            percent_off: d("20"),
            overrides: BTreeMap::new(),
            exclude_low_stock: false,
        };
        let plan = plan_sale(&r, &ask, &products);
        assert_eq!(plan.included.len(), 3);
        let c = check_min_stock(&r, &plan);
        assert!(!c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("Cotton chore coat (7) and Light rain parka (4) are under 10 units")
        );

        ask.exclude_low_stock = true;
        let plan = plan_sale(&r, &ask, &products);
        assert_eq!(plan.included.len(), 1);
        assert_eq!(plan.excluded.len(), 2);
        assert_eq!(plan.excluded[0].why.word(), "promo_min_stock");
        let c = check_min_stock(&r, &plan);
        assert!(c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("2 products under 10 units excluded: Cotton chore coat (7), Light rain parka (4)")
        );
    }

    #[test]
    fn max_discount_uses_the_tier_limit() {
        let r = SaleRules::from_policy(&policies(true)).unwrap();
        let products = [product(
            "prd_windbreaker",
            "Packable windbreaker",
            "89.00",
            "36.00",
            &[214],
        )];
        let mut ask = SaleAsk {
            tier: Some("vip".into()),
            percent_off: d("20"),
            overrides: BTreeMap::new(),
            exclude_low_stock: true,
        };
        let c = check_max_discount(&r, &ask, &plan_sale(&r, &ask, &products));
        assert!(c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("Highest is 20%; policy allows 30% for vip")
        );
        // Over the limit: the detail names the tiers whose limit would allow it.
        ask.tier = Some("gold".into());
        ask.percent_off = d("25");
        let c = check_max_discount(&r, &ask, &plan_sale(&r, &ask, &products));
        assert!(!c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("25% is over the 20% limit for gold; vip allows up to 30%")
        );
        ask.tier = None;
        let c = check_max_discount(&r, &ask, &plan_sale(&r, &ask, &products));
        assert!(!c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("25% is over the 20% limit for everyone; vip allows up to 30%")
        );
        // No tier allows it: the plain sentence.
        ask.percent_off = d("35");
        let c = check_max_discount(&r, &ask, &plan_sale(&r, &ask, &products));
        assert_eq!(
            c.detail.as_deref(),
            Some("35% is over the 20% limit for everyone")
        );
        // The asked tier itself is never offered as the way out.
        ask.tier = Some("vip".into());
        let c = check_max_discount(&r, &ask, &plan_sale(&r, &ask, &products));
        assert_eq!(c.detail.as_deref(), Some("35% is over the 30% limit for vip"));
        // Several tiers that would allow it, lowest limit first.
        let mut more = r.clone();
        more.max_discount_by_tier.insert("gold".into(), d("0.25"));
        assert_eq!(
            more.over_limit_words(d("25"), None, d("0.20")),
            "25% is over the 20% limit for everyone; gold allows up to 25% and vip allows up to 30%"
        );
    }

    #[test]
    fn overlap_needs_layer_audience_and_product_in_common() {
        let r = SaleRules::from_policy(&policies(true)).unwrap();
        let ours: BTreeSet<String> = ["prd_windbreaker".to_owned()].into();
        let knit = OtherPromotion {
            id: "promo_autumn_knit".into(),
            name: "Autumn knitwear 15%".into(),
            status: PromotionStatus::Running,
            layer: DiscountLayer::ItemPrice,
            tier: None,
            products: Some(["prd_cashmere_sweater".to_owned()].into()),
        };
        let gold = OtherPromotion {
            id: "promo_gold_10".into(),
            name: "Gold tier 10%".into(),
            status: PromotionStatus::Running,
            layer: DiscountLayer::OrderDiscount,
            tier: Some("gold".into()),
            products: None,
        };
        let c = check_overlap(
            &r,
            Some("vip"),
            &ours,
            &[knit.clone(), gold],
            "summer outerwear",
        );
        assert!(c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("Checked 2 running promotions; none gives an item price on summer outerwear")
        );
        let mut clash = knit;
        clash.products = Some(ours.clone());
        let c = check_overlap(&r, Some("vip"), &ours, &[clash], "summer outerwear");
        assert!(!c.passed);
        assert_eq!(c.fix_tool.as_deref(), Some("list_active_promotions"));
    }

    #[test]
    fn status_follows_the_clock_unless_set_by_a_person() {
        let t = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let from = t("2026-11-07T00:00:00-05:00");
        let through = Some(t("2026-11-08T23:59:00-05:00"));
        let fri = t("2026-11-06T11:00:00-05:00");
        let sat = t("2026-11-07T10:00:00-05:00");
        let late = t("2026-11-08T23:59:30-05:00");
        let mon = t("2026-11-09T00:00:00-05:00");
        assert_eq!(
            promotion_status("scheduled", from, through, fri),
            PromotionStatus::Scheduled
        );
        assert_eq!(
            promotion_status("scheduled", from, through, sat),
            PromotionStatus::Running
        );
        assert_eq!(
            promotion_status("running", from, through, late),
            PromotionStatus::Running
        );
        assert_eq!(
            promotion_status("running", from, through, mon),
            PromotionStatus::Ended
        );
        assert_eq!(
            promotion_status("paused", from, through, sat),
            PromotionStatus::Paused
        );
    }

    #[test]
    fn coupon_stacking_follows_member_benefits_per_order() {
        let c = check_member_benefits(1, "no_tier_coupons");
        assert!(c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("Doesn't stack with tier coupons; 1 member benefit per order")
        );
        assert!(!check_member_benefits(1, "with_tier_coupons").passed);
        assert!(check_member_benefits(2, "with_tier_coupons").passed);
    }

    #[test]
    fn writes_counts_and_lists_for_people() {
        assert_eq!(count_text(1284), "1,284");
        assert_eq!(count_text(9816), "9,816");
        assert_eq!(count_text(320), "320");
        assert_eq!(
            and_list(&["a".into(), "b".into(), "c".into()]),
            "a, b and c"
        );
        assert_eq!(money_text(d("71.2"), "USD"), "$71.20");
    }
}
