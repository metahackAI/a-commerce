//! Catalog business logic as pure functions: how a product's price, stock and availability
//! follow from its variants, which sale price a viewer pays, what a stock change does to
//! availability, and how reviews are summarized. The operations load rows and call these;
//! the tests hold them to the numbers in `docs/scenarios.md` and the demo seed.

use std::collections::BTreeMap;
use std::str::FromStr;

use acom_core::{Currency, Money, StoreTime};
use chrono::NaiveDate;
use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::{Value, json};

use crate::neighbors::SalePrice;
use crate::rules::{StockRules, holds};

/// A JSON number with exactly the digits of `d` (`89.00` stays `89.00`).
pub fn num(d: Decimal) -> Value {
    serde_json::Number::from_str(&d.to_string())
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

/// A measurement or height without trailing zeros (`56.0` → `56`, `65.5` stays).
pub fn plain(d: Decimal) -> Value {
    num(d.normalize())
}

/// `$71.20`, `€12.00`, or `1200 KRW`.
pub fn money_text(m: &Money) -> String {
    let digits = m.currency.minor_units() as usize;
    let value = m.rounded().value;
    match m.currency.as_str() {
        "USD" => format!("${value:.digits$}"),
        "EUR" => format!("€{value:.digits$}"),
        other => format!("{value:.digits$} {other}"),
    }
}

/// An amount from a database price and currency code.
pub fn money_of(value: Decimal, currency: &str) -> Option<Money> {
    Currency::parse(currency.trim())
        .ok()
        .map(|c| Money::new(value, c))
}

/// One variant with its offer, as the catalog stores it (`catalog.product` + `catalog.offer`).
/// The offer fields are empty for a draft variant that has no offer yet.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct Variant {
    pub id: String,
    pub product_group_id: String,
    pub sku: String,
    pub name: String,
    pub color: Option<String>,
    pub size: Option<String>,
    pub sale_type: String,
    pub is_managed: bool,
    pub offer_id: Option<String>,
    pub price: Option<Decimal>,
    pub price_currency: Option<String>,
    pub availability: Option<String>,
    pub inventory_level: Option<i32>,
    pub safety_stock: Option<i32>,
    pub cost_value: Option<Decimal>,
    pub cost_currency: Option<String>,
}

impl Variant {
    pub fn list_price(&self) -> Option<Money> {
        money_of(self.price?, self.price_currency.as_deref()?)
    }

    pub fn cost(&self) -> Option<Money> {
        money_of(self.cost_value?, self.cost_currency.as_deref()?)
    }

    pub fn stock(&self) -> i64 {
        i64::from(self.inventory_level.unwrap_or(0))
    }

    fn is_discontinued(&self) -> bool {
        self.availability.as_deref() == Some("discontinued")
    }

    /// The entry in a product's `has_variant[]`.
    pub fn line_json(&self) -> Value {
        json!({"variant_id": self.id, "sku": self.sku, "size": self.size, "color": self.color,
               "inventory_level": self.stock(), "availability": self.availability})
    }
}

/// A product's availability, derived from its variants (`inventory.product_status: derived`):
/// in stock when any variant is, then pre-order, then back-order; discontinued only when every
/// variant is; otherwise out of stock.
pub fn product_availability<'a>(variants: impl IntoIterator<Item = &'a str>) -> &'static str {
    let words: Vec<&str> = variants.into_iter().collect();
    let any = |w: &str| words.contains(&w);
    if any("in_stock") {
        "in_stock"
    } else if any("pre_order") {
        "pre_order"
    } else if any("back_order") {
        "back_order"
    } else if !words.is_empty() && words.iter().all(|w| *w == "discontinued") {
        "discontinued"
    } else {
        "out_of_stock"
    }
}

/// What a product shows for its variants together.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// The lowest list price among the variants still sold (any variant if all ended).
    pub price: Option<Money>,
    /// The cost of the variant that sets `price`.
    pub cost: Option<Money>,
    /// The sum of the variants' stock.
    pub inventory_level: i64,
    pub availability: &'static str,
}

pub fn summarize(variants: &[Variant]) -> Summary {
    let offered: Vec<&Variant> = variants.iter().filter(|v| v.offer_id.is_some()).collect();
    let selling: Vec<&Variant> = offered
        .iter()
        .copied()
        .filter(|v| !v.is_discontinued())
        .collect();
    let pool = if selling.is_empty() {
        &offered
    } else {
        &selling
    };
    let cheapest = pool
        .iter()
        .filter(|v| v.list_price().is_some())
        .min_by(|a, b| a.price.cmp(&b.price));
    Summary {
        price: cheapest.and_then(|v| v.list_price()),
        cost: cheapest.and_then(|v| v.cost()),
        inventory_level: offered.iter().map(|v| v.stock()).sum(),
        availability: product_availability(
            offered.iter().filter_map(|v| v.availability.as_deref()),
        ),
    }
}

const LETTER_SIZES: [&str; 8] = ["XXS", "XS", "S", "M", "L", "XL", "XXL", "XXXL"];

/// Sort key for sizes: letter sizes in wearing order, then numeric sizes, then the rest.
pub fn size_key(size: Option<&str>) -> (u8, Decimal, String) {
    match size {
        None => (3, Decimal::ZERO, String::new()),
        Some(s) => {
            if let Some(i) = LETTER_SIZES.iter().position(|x| x.eq_ignore_ascii_case(s)) {
                (0, Decimal::from(i), String::new())
            } else if let Ok(n) = Decimal::from_str(s) {
                (1, n, String::new())
            } else {
                (2, Decimal::ZERO, s.to_lowercase())
            }
        }
    }
}

/// Orders variants for display: the color with the most stock first, then by size.
pub fn order_variants(variants: &mut [Variant]) {
    let mut by_color: BTreeMap<String, i64> = BTreeMap::new();
    for v in variants.iter() {
        *by_color
            .entry(v.color.clone().unwrap_or_default())
            .or_default() += v.stock();
    }
    variants.sort_by(|a, b| {
        let ca = a.color.clone().unwrap_or_default();
        let cb = b.color.clone().unwrap_or_default();
        let (sa, sb) = (by_color[&ca], by_color[&cb]);
        sb.cmp(&sa)
            .then_with(|| ca.cmp(&cb))
            .then_with(|| size_key(a.size.as_deref()).cmp(&size_key(b.size.as_deref())))
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// The sale price a viewer pays for one offer: the lowest one under the list price; a price for
/// the viewer's tier wins a tie with an everyone price (`pricing.sale_price.compute: on_demand`,
/// so the candidates are the prices in effect at that moment).
pub fn best_sale_price<'a>(
    list_price: Decimal,
    candidates: impl IntoIterator<Item = &'a SalePrice>,
) -> Option<&'a SalePrice> {
    candidates
        .into_iter()
        .filter(|p| p.price < list_price)
        .min_by(|a, b| {
            a.price.cmp(&b.price).then_with(|| {
                b.valid_for_member_tier
                    .is_some()
                    .cmp(&a.valid_for_member_tier.is_some())
            })
        })
}

/// A product's member price: the best sale price over its variants still sold.
pub fn member_price<'a>(variants: &[Variant], prices: &'a [SalePrice]) -> Option<&'a SalePrice> {
    variants
        .iter()
        .filter(|v| !v.is_discontinued())
        .filter_map(|v| {
            let offer = v.offer_id.as_deref()?;
            best_sale_price(v.price?, prices.iter().filter(|p| p.offer_id == offer))
        })
        .min_by(|a, b| a.price.cmp(&b.price))
}

/// `member_price` as products show it.
pub fn member_price_json(p: &SalePrice, time: &StoreTime) -> Value {
    json!({"price": num(p.price), "name": p.name,
           "valid_for_member_tier": p.valid_for_member_tier.as_deref().unwrap_or("everyone"),
           "promotion_id": p.promotion_id,
           "valid_through": p.valid_through.map(|t| time.format(t))})
}

/// `additional_property[]` for shoppers: the material, then the comparison attributes with
/// `unit_text` shown as `unit_code`.
pub fn properties_json(material: Option<&str>, additional: &Value) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(m) = material {
        out.push(json!({"name": "material", "value": m}));
    }
    for p in additional.as_array().into_iter().flatten() {
        let Some(name) = p.get("name").and_then(Value::as_str) else {
            continue;
        };
        if name == "material" && material.is_some() {
            continue;
        }
        let mut item =
            json!({"name": name, "value": p.get("value").cloned().unwrap_or(Value::Null)});
        if let Some(unit) = p.get("unit_text").filter(|u| !u.is_null()) {
            item["unit_code"] = unit.clone();
        }
        if let Some(size) = p.get("size").filter(|s| !s.is_null()) {
            item["size"] = size.clone();
        }
        out.push(item);
    }
    out
}

/// Where a variant's stock and availability stand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StockState {
    pub inventory_level: i64,
    pub availability: String,
    /// The variant ran out by itself (`sold_out_at` is set), so it may come back by itself.
    pub auto_out_of_stock: bool,
}

/// What setting the stock to `level` does (`inventory.auto_out_of_stock`, `auto_restore`).
/// Untracked stock (`managed == false`) never moves availability; only a variant on sale runs
/// out by itself, and only one that ran out by itself comes back by itself.
pub fn apply_stock(rules: &StockRules, now: &StockState, level: i64, managed: bool) -> StockState {
    let unchanged = StockState {
        inventory_level: level,
        ..now.clone()
    };
    if !managed {
        return unchanged;
    }
    let facts = json!({"inventory_level": level, "auto_out_of_stock": now.auto_out_of_stock});
    if now.availability == "in_stock" && holds(&rules.out_of_stock_when, &facts) {
        return StockState {
            inventory_level: level,
            availability: rules.out_of_stock_to.clone(),
            auto_out_of_stock: true,
        };
    }
    if holds(&rules.restore_when, &facts) {
        return StockState {
            inventory_level: level,
            availability: rules.restore_to.clone(),
            auto_out_of_stock: false,
        };
    }
    unchanged
}

/// Whether one change in a product's history touched content or structure
/// (`catalog.yml product.approval`: prices, categories, variants and discontinuing are
/// structural; descriptions, images, copy and stock are not).
pub fn change_type(tool: &str, to: &Value) -> &'static str {
    const STRUCTURAL_TOOLS: [&str; 11] = [
        "set_price",
        "schedule_price_change",
        "set_variants",
        "discontinue_product",
        "approve_product",
        "reject_product",
        "submit_for_review",
        "create_product",
        "create_product_group",
        "create_bundle",
        "batch_import",
    ];
    const STRUCTURAL_FIELDS: [&str; 5] = [
        "category_id",
        "varies_by",
        "price",
        "sale_type",
        "approval_status",
    ];
    if STRUCTURAL_TOOLS.contains(&tool)
        || STRUCTURAL_FIELDS
            .iter()
            .any(|f| to.get(*f).is_some_and(|v| !v.is_null()))
    {
        "structural"
    } else {
        "content"
    }
}

/// A `LIKE` pattern that matches `text` anywhere, with `%`, `_` and `\` taken literally.
pub fn like_pattern(text: &str) -> String {
    let mut out = String::from("%");
    for c in text.trim().chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// The first sentence of a review, at most 160 characters.
pub fn excerpt(body: &str) -> String {
    let body = body.trim();
    let end = body
        .char_indices()
        .find(|(i, c)| {
            matches!(c, '.' | '!' | '?')
                && body[i + c.len_utf8()..]
                    .chars()
                    .next()
                    .is_none_or(char::is_whitespace)
        })
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(body.len());
    let sentence = &body[..end];
    if sentence.chars().count() <= 160 {
        return sentence.to_owned();
    }
    let cut: String = sentence.chars().take(159).collect();
    match cut.rfind(' ') {
        Some(i) => format!("{}…", &cut[..i]),
        None => format!("{cut}…"),
    }
}

/// The first blocked word in a text, matched as a whole word in any case.
pub fn blocked_word(text: &str, blocked: &[String]) -> Option<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    blocked.iter().find(|b| words.contains(b)).cloned()
}

/// The product's known mention phrases that a new review uses (case-insensitive).
pub fn mentions_in(text: &str, known: &[String]) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut out: Vec<String> = known
        .iter()
        .filter(|k| !k.is_empty() && lower.contains(&k.to_lowercase()))
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

/// What a review contributes to a summary.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewFacts {
    pub id: String,
    pub rating: i16,
    pub body: String,
    pub fit: Option<String>,
    pub height_cm: Option<Decimal>,
    pub size: Option<String>,
    pub keywords: Vec<String>,
    pub date_published: NaiveDate,
}

/// The words reviewers use for each fit, most typical first. The label of a fit vote is the
/// phrase its reviews use most; with no match, the first phrase.
const FIT_PHRASES: [(&str, &[&str]); 3] = [
    (
        "true_to_size",
        &["True to size", "Fits as expected", "Perfect fit"],
    ),
    ("runs_small", &["A bit snug", "Runs small", "Snug", "Tight"]),
    (
        "runs_large",
        &["Runs large", "A bit roomy", "Roomy", "Loose"],
    ),
];

fn fit_label(phrases: &[&str], bodies: &[String]) -> String {
    let mut best: Option<(&str, usize)> = None;
    for p in phrases {
        let needle = p.to_lowercase();
        let n = bodies.iter().filter(|b| b.contains(&needle)).count();
        if n > 0 && best.is_none_or(|(_, m)| n > m) {
            best = Some((p, n));
        }
    }
    best.map_or(phrases[0], |(p, _)| p).to_owned()
}

/// `review_count`, `rating_value`, `fit_votes`, `mentions` and `sample` of a review summary.
/// Ratings average to one decimal; every fit appears, with zero counts too; mentions are
/// counted once per review, most frequent first; the sample prefers reviews that use the top
/// mention, newest first.
pub fn review_summary(reviews: &[ReviewFacts], sample_limit: usize) -> Value {
    let count = reviews.len();
    let rating = if count == 0 {
        Value::Null
    } else {
        let sum: i64 = reviews.iter().map(|r| i64::from(r.rating)).sum();
        let avg = (Decimal::from(sum) / Decimal::from(count as i64))
            .round_dp_with_strategy(1, RoundingStrategy::MidpointAwayFromZero);
        num(avg)
    };
    let fit_votes: Vec<Value> = FIT_PHRASES
        .iter()
        .map(|(fit, phrases)| {
            let bodies: Vec<String> = reviews
                .iter()
                .filter(|r| r.fit.as_deref() == Some(*fit))
                .map(|r| r.body.to_lowercase())
                .collect();
            json!({"fit": fit, "label": fit_label(phrases, &bodies), "count": bodies.len()})
        })
        .collect();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for r in reviews {
        let mut seen: Vec<&str> = r.keywords.iter().map(String::as_str).collect();
        seen.sort_unstable();
        seen.dedup();
        for k in seen {
            *counts.entry(k).or_default() += 1;
        }
    }
    let mut mentions: Vec<(&str, usize)> = counts.into_iter().collect();
    mentions.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let top = mentions.first().map(|(t, _)| *t);
    let mut sample: Vec<&ReviewFacts> = reviews.iter().collect();
    sample.sort_by(|a, b| {
        let has = |r: &ReviewFacts| top.is_some_and(|t| r.keywords.iter().any(|k| k == t));
        has(b)
            .cmp(&has(a))
            .then_with(|| b.date_published.cmp(&a.date_published))
            .then_with(|| b.id.cmp(&a.id))
    });
    let sample: Vec<Value> = sample
        .into_iter()
        .take(sample_limit)
        .map(|r| {
            json!({"review_id": r.id, "review_rating": r.rating, "size_bought": r.size,
                   "height_cm": r.height_cm.map(plain), "excerpt": excerpt(&r.body),
                   "date_published": r.date_published.to_string()})
        })
        .collect();
    json!({
        "review_count": count,
        "rating_value": rating,
        "fit_votes": fit_votes,
        "mentions": mentions.iter().map(|(t, n)| json!({"text": t, "count": n})).collect::<Vec<_>>(),
        "sample": sample,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::StockRules;
    use crate::testing::policies;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap_or_default()
    }

    fn variant(id: &str, color: &str, size: &str, stock: i32, availability: &str) -> Variant {
        Variant {
            id: id.into(),
            product_group_id: "prd_x".into(),
            sku: id.replace("var_", ""),
            name: id.into(),
            color: Some(color.into()),
            size: Some(size.into()),
            sale_type: "always".into(),
            is_managed: true,
            offer_id: Some(id.replace("var_", "off_")),
            price: Some(d("142.00")),
            price_currency: Some("USD".into()),
            availability: Some(availability.into()),
            inventory_level: Some(stock),
            safety_stock: Some(2),
            cost_value: Some(d("60.00")),
            cost_currency: Some("USD".into()),
        }
    }

    /// The chore coat on Friday: S 3, M 0, L 0, XL 4 (scenarios.md §3.1).
    fn chore_coat() -> Vec<Variant> {
        vec![
            variant("var_chore_coat_l_olive", "Olive", "L", 0, "out_of_stock"),
            variant("var_chore_coat_xl_olive", "Olive", "XL", 4, "in_stock"),
            variant("var_chore_coat_s_olive", "Olive", "S", 3, "in_stock"),
            variant("var_chore_coat_m_olive", "Olive", "M", 0, "out_of_stock"),
        ]
    }

    #[test]
    fn product_stock_and_status_follow_the_variants() {
        let s = summarize(&chore_coat());
        assert_eq!(s.inventory_level, 7);
        assert_eq!(s.availability, "in_stock");
        assert_eq!(s.price.map(|p| p.value), Some(d("142.00")));
        assert_eq!(s.cost.map(|c| c.value), Some(d("60.00")));
        assert_eq!(
            product_availability(["out_of_stock", "discontinued"]),
            "out_of_stock"
        );
        assert_eq!(
            product_availability(["discontinued", "discontinued"]),
            "discontinued"
        );
        assert_eq!(
            product_availability(["out_of_stock", "back_order"]),
            "back_order"
        );
    }

    #[test]
    fn variants_show_the_best_stocked_color_first_then_sizes_in_order() {
        let mut v = chore_coat();
        v.push(variant("var_chore_coat_m_sand", "Sand", "M", 1, "in_stock"));
        order_variants(&mut v);
        let sizes: Vec<_> = v
            .iter()
            .map(|x| x.size.clone().unwrap_or_default())
            .collect();
        assert_eq!(sizes, ["S", "M", "L", "XL", "M"]);
        assert!(size_key(Some("8")) < size_key(Some("10")));
        assert!(size_key(Some("XL")) < size_key(Some("8")));
    }

    fn sale(offer: &str, price: &str, tier: Option<&str>, name: &str) -> SalePrice {
        SalePrice {
            offer_id: offer.into(),
            price: d(price),
            price_currency: "USD".into(),
            valid_for_member_tier: tier.map(str::to_owned),
            promotion_id: Some("promo_1042".into()),
            name: Some(name.into()),
            valid_through: None,
        }
    }

    #[test]
    fn vip_weekend_price_is_the_member_price() {
        // B1: windbreaker $89.00 → $71.20 for VIP; an everyone price at the same level loses
        // the tie to the tier price.
        let mut wb = variant("var_windbreaker_m_slate", "Slate", "M", 18, "in_stock");
        wb.price = Some(d("89.00"));
        let prices = vec![
            sale("off_windbreaker_m_slate", "71.20", None, "Everyone"),
            sale(
                "off_windbreaker_m_slate",
                "71.20",
                Some("vip"),
                "VIP weekend",
            ),
            sale("off_windbreaker_m_slate", "95.00", None, "Not a discount"),
        ];
        let best = member_price(std::slice::from_ref(&wb), &prices);
        assert_eq!(best.map(|p| p.price), Some(d("71.20")));
        assert_eq!(
            best.and_then(|p| p.name.clone()).as_deref(),
            Some("VIP weekend")
        );
        assert!(member_price(&[wb], &prices[2..]).is_none());
    }

    #[test]
    fn properties_show_material_then_comparison_attributes() {
        let additional = json!([
            {"name": "weight", "value": 190, "unit_text": "g", "size": "M"},
            {"name": "packed_size", "value": [14, 18], "unit_text": "cm"},
            {"name": "packing", "value": "own_pocket", "unit_text": null}]);
        let p = properties_json(Some("Recycled nylon"), &additional);
        assert_eq!(p[0], json!({"name": "material", "value": "Recycled nylon"}));
        assert_eq!(
            p[1],
            json!({"name": "weight", "value": 190, "unit_code": "g", "size": "M"})
        );
        assert_eq!(p[3], json!({"name": "packing", "value": "own_pocket"}));
    }

    #[test]
    fn stock_moves_availability_only_by_the_policy() {
        let rules = StockRules::from_policy(&policies()).unwrap_or_else(|e| panic!("{e}"));
        let on_sale = StockState {
            inventory_level: 3,
            availability: "in_stock".into(),
            auto_out_of_stock: false,
        };
        let out = apply_stock(&rules, &on_sale, 0, true);
        assert_eq!(out.availability, "out_of_stock");
        assert!(out.auto_out_of_stock);
        let back = apply_stock(&rules, &out, 12, true);
        assert_eq!(back.availability, "in_stock");
        assert!(!back.auto_out_of_stock);
        // Untracked stock, and stock that ran out by hand, stay where they are.
        assert_eq!(
            apply_stock(&rules, &on_sale, 0, false).availability,
            "in_stock"
        );
        let manual = StockState {
            auto_out_of_stock: false,
            ..out.clone()
        };
        assert_eq!(
            apply_stock(&rules, &manual, 5, true).availability,
            "out_of_stock"
        );
    }

    #[test]
    fn prices_are_structural_and_stock_is_not() {
        assert_eq!(
            change_type("set_price", &json!({"price": 23.00})),
            "structural"
        );
        assert_eq!(
            change_type("set_variant_stock", &json!({"inventory_level": 4})),
            "content"
        );
        assert_eq!(
            change_type("update_product_group", &json!({"category_id": "cat_x"})),
            "structural"
        );
        assert_eq!(
            change_type("update_product_group", &json!({"description": "New"})),
            "content"
        );
    }

    #[test]
    fn text_helpers() {
        assert_eq!(like_pattern("50%_off"), "%50\\%\\_off%");
        assert_eq!(
            excerpt("Size up if you wear a hoodie underneath. Warm, sharp, and solid."),
            "Size up if you wear a hoodie underneath."
        );
        assert_eq!(excerpt("No period here"), "No period here");
        assert_eq!(money_text(&Money::new(d("71.2"), Currency::USD)), "$71.20");
        assert_eq!(
            blocked_word("Total JUNK, avoid", &["junk".into()]),
            Some("junk".into())
        );
        assert_eq!(blocked_word("Junkyard style", &["junk".into()]), None);
        assert_eq!(
            mentions_in(
                "Size up for a hoodie, really.",
                &["size up for a hoodie".into()]
            ),
            vec!["size up for a hoodie".to_owned()]
        );
    }

    /// Reviews shaped like the 41 wool overcoat reviews of the seed: 29 true to size, 12 a bit
    /// snug, 9 mentioning sizing up for a hoodie; 28 fives, 10 fours, 3 threes (sum 189).
    fn overcoat_reviews() -> Vec<ReviewFacts> {
        let mut out = Vec::new();
        for i in 0..41u32 {
            let snug = i < 12;
            let hoodie = (12..39).contains(&i) && (i - 12) % 3 == 0;
            let rating = if i < 3 {
                3
            } else if i < 13 {
                4
            } else {
                5
            };
            out.push(ReviewFacts {
                id: format!("rev_{}", 8772 + i),
                rating,
                body: if snug {
                    "A bit snug across the chest and shoulders, fine over a shirt.".into()
                } else if hoodie {
                    "True to size over a shirt; size up for a hoodie.".into()
                } else {
                    "True to size. Warm without bulk.".into()
                },
                fit: Some(if snug { "runs_small" } else { "true_to_size" }.into()),
                height_cm: Some(Decimal::from(175 + i % 8)),
                size: Some(if i % 2 == 0 { "L" } else { "M" }.into()),
                keywords: if hoodie {
                    vec!["size up for a hoodie".into()]
                } else {
                    vec![]
                },
                date_published: NaiveDate::from_ymd_opt(2025, 11, 1)
                    .unwrap_or_default()
                    .checked_add_days(chrono::Days::new(u64::from(i) * 8))
                    .unwrap_or_default(),
            });
        }
        out
    }

    #[test]
    fn summarizes_the_overcoat_reviews_like_the_fit_check() {
        let reviews = overcoat_reviews();
        let snug = reviews
            .iter()
            .filter(|r| r.fit.as_deref() == Some("runs_small"))
            .count();
        let hoodie = reviews.iter().filter(|r| !r.keywords.is_empty()).count();
        assert_eq!((reviews.len(), snug, hoodie), (41, 12, 9));
        let s = review_summary(&reviews, 3);
        assert_eq!(s["review_count"], json!(41));
        assert_eq!(s["rating_value"].to_string(), "4.6");
        assert_eq!(
            s["fit_votes"],
            json!([
                {"fit": "true_to_size", "label": "True to size", "count": 29},
                {"fit": "runs_small", "label": "A bit snug", "count": 12},
                {"fit": "runs_large", "label": "Runs large", "count": 0}])
        );
        assert_eq!(
            s["mentions"],
            json!([{"text": "size up for a hoodie", "count": 9}])
        );
        let sample = s["sample"].as_array().cloned().unwrap_or_default();
        assert_eq!(sample.len(), 3);
        assert!(
            sample
                .iter()
                .all(|r| r["excerpt"].as_str().is_some_and(|e| e.contains("hoodie")))
        );
        assert_eq!(review_summary(&[], 3)["rating_value"], Value::Null);
    }
}
