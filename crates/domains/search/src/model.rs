//! Search logic as pure functions: turning a query into search terms with the store's synonyms,
//! the values and order of a comparison, facet counts, and autocomplete suggestions.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use acom_catalog::model::{money_text, num, size_key};
use acom_catalog::service::for_search::{CategoryLink, ProductCard};
use acom_core::Money;
use acom_policy::PolicySet;
use rust_decimal::{Decimal, RoundingStrategy};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// The words of a query: lowercase letters and digits only, so they are safe inside a
/// `to_tsquery` expression.
pub fn tokens(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// A rough English stem, only to match a query word to a synonym group (`trips` → `trip`).
/// The database's own stemmer does the real matching.
pub(crate) fn stem(w: &str) -> &str {
    for suffix in ["ing", "es", "s"] {
        if let Some(s) = w.strip_suffix(suffix)
            && s.len() >= 3
        {
            return s;
        }
    }
    w
}

/// The store's synonym groups for a language (`search.yml lexicon.synonyms`).
pub fn synonyms(policy: &PolicySet, language: &str) -> Vec<Vec<String>> {
    policy
        .lookup(&format!("search.lexicon.synonyms.{language}"))
        .and_then(Value::as_array)
        .map(|groups| {
            groups
                .iter()
                .filter_map(Value::as_array)
                .map(|g| {
                    g.iter()
                        .filter_map(Value::as_str)
                        .flat_map(tokens)
                        .collect::<Vec<_>>()
                })
                .filter(|g| !g.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The query's words plus every synonym of each (`query.expansion: synonym_dictionary`), in
/// order, without repeats.
pub fn expand(query: &str, groups: &[Vec<String>]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let push = |w: &str, out: &mut Vec<String>| {
        if !out.iter().any(|x| x == w) {
            out.push(w.to_owned());
        }
    };
    for t in tokens(query) {
        push(&t, &mut out);
        for g in groups {
            if g.iter().any(|w| stem(w) == stem(&t)) {
                g.iter().for_each(|w| push(w, &mut out));
            }
        }
    }
    out
}

/// One `to_tsquery` text per word of the query, each matching the word or its synonyms. Ranking
/// counts how many of these a product matches, so a product that covers more of what the
/// shopper asked for ("light", "jacket", "trips") comes first.
pub fn concepts(query: &str, groups: &[Vec<String>]) -> Vec<String> {
    // Words the database's English configuration drops anyway; a query of only one would match
    // nothing and log a notice for every product.
    const STOP: &[&str] = &[
        "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "he", "her", "his", "i",
        "in", "is", "it", "me", "my", "of", "on", "or", "our", "she", "so", "that", "the", "their",
        "them", "they", "this", "to", "under", "was", "we", "who", "with", "you", "your",
    ];
    let mut out: Vec<String> = Vec::new();
    for t in tokens(query)
        .into_iter()
        .filter(|t| !STOP.contains(&t.as_str()))
    {
        let terms = expand(&t, groups);
        if let Some(q) = tsquery(&terms)
            && !out.contains(&q)
        {
            out.push(q);
        }
    }
    out
}

/// A `to_tsquery` text that matches any of the terms; ranking favors products matching more of
/// them. `None` when the query has no words.
pub fn tsquery(terms: &[String]) -> Option<String> {
    (!terms.is_empty()).then(|| terms.join(" | "))
}

/// One product's text with weights as the search document defines them: name A; brand,
/// categories and tags B; material C; description D.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Doc {
    pub id: String,
    pub name: String,
    pub labels: String,
    pub material: String,
    pub description: String,
}

/// Builds the documents from the catalog's texts and the product placements.
pub fn docs(
    texts: &[acom_catalog::service::for_search::ProductText],
    links: &[CategoryLink],
) -> Vec<Doc> {
    let mut names: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for l in links {
        names
            .entry(l.product_id.as_str())
            .or_default()
            .insert(l.name.as_str());
    }
    texts
        .iter()
        .map(|t| {
            let mut labels: Vec<&str> = Vec::new();
            labels.extend(t.brand_name.as_deref());
            labels.extend(names.get(t.id.as_str()).into_iter().flatten().copied());
            labels.extend(t.keywords.iter().map(String::as_str));
            Doc {
                id: t.id.clone(),
                name: t.name.clone(),
                labels: labels.join(" "),
                material: t.material.clone().unwrap_or_default(),
                description: t.description.clone().unwrap_or_default(),
            }
        })
        .collect()
}

/// An attribute `compare_products` can line up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Attribute {
    /// Weight of size M, in g.
    Weight,
    /// Packed size in cm, short and long side.
    PackedSize,
    /// How it packs.
    Packing,
    /// How warm it is.
    Warmth,
    /// Whether it keeps rain off.
    Rain,
    /// What it is made of.
    Material,
    /// The price the caller pays.
    Price,
}

impl Attribute {
    pub const ALL: [Attribute; 7] = [
        Attribute::Weight,
        Attribute::PackedSize,
        Attribute::Packing,
        Attribute::Warmth,
        Attribute::Rain,
        Attribute::Material,
        Attribute::Price,
    ];

    pub fn word(self) -> &'static str {
        match self {
            Attribute::Weight => "weight",
            Attribute::PackedSize => "packed_size",
            Attribute::Packing => "packing",
            Attribute::Warmth => "warmth",
            Attribute::Rain => "rain",
            Attribute::Material => "material",
            Attribute::Price => "price",
        }
    }

    pub fn from_word(w: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.word() == w)
    }

    fn unit(self) -> Option<&'static str> {
        match self {
            Attribute::Weight => Some("g"),
            Attribute::PackedSize => Some("cm"),
            _ => None,
        }
    }
}

/// The comparable attributes this store allows (`search.yml compare.attributes`).
pub fn allowed_attributes(policy: &PolicySet) -> Vec<Attribute> {
    policy
        .lookup("search.compare.attributes")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .filter_map(Attribute::from_word)
                .collect()
        })
        .unwrap_or_default()
}

fn packing_text(word: &str) -> Option<&'static str> {
    Some(match word {
        "own_pocket" => "Packs into its own pocket",
        "rolls_into_hood" => "Rolls into its hood",
        "folds_flat" => "Folds flat",
        "none" => "Does not pack small",
        _ => return None,
    })
}

fn warmth_text(word: &str) -> Option<&'static str> {
    Some(match word {
        "wind_layer" => "Wind layer only",
        "light" => "Light",
        "medium" => "Medium",
        "warm" => "Warm",
        _ => return None,
    })
}

fn decimal(v: &Value) -> Option<Decimal> {
    match v {
        Value::Number(n) => Decimal::from_str(&n.to_string()).ok(),
        _ => None,
    }
}

/// What the viewer pays for a product and why: the member price with its name, else the list
/// price.
pub fn paid_price(card: &ProductCard) -> Option<(Money, Option<String>)> {
    let list = card.summary.price?;
    Some(match &card.member_price {
        Some(m) => (Money::new(m.price, list.currency), m.name.clone()),
        None => (list, None),
    })
}

/// One `{attribute, value, unit, display}` of a comparison; a missing value shows "Not listed".
pub fn attribute_value(card: &ProductCard, a: Attribute) -> Value {
    let raw = |name: &str| card.attribute(name).and_then(|p| p.get("value")).cloned();
    let missing =
        || json!({"attribute": a.word(), "value": null, "unit": a.unit(), "display": "Not listed"});
    let found = |value: Value, display: String| json!({"attribute": a.word(), "value": value, "unit": a.unit(), "display": display});
    match a {
        Attribute::Weight => match raw("weight").filter(|v| !v.is_null()) {
            Some(v) => {
                let d = format!("{v} g");
                found(v, d)
            }
            None => missing(),
        },
        Attribute::PackedSize => {
            let packing = raw("packing").and_then(|p| p.as_str().and_then(packing_text));
            match (raw("packed_size").filter(|v| v.is_array()), packing) {
                (Some(v), p) => {
                    let sides: Vec<String> = v
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(Value::to_string)
                        .collect();
                    let size = format!("{} cm", sides.join(" × "));
                    let d = match p {
                        Some(p) => format!("{p}, {size}"),
                        None => size,
                    };
                    found(v, d)
                }
                (None, Some(p)) => found(Value::Null, p.to_owned()),
                (None, None) => missing(),
            }
        }
        Attribute::Packing => match raw("packing").and_then(|v| v.as_str().map(str::to_owned)) {
            Some(w) => {
                let d = packing_text(&w).unwrap_or(w.as_str()).to_owned();
                found(json!(w), d)
            }
            None => missing(),
        },
        Attribute::Warmth => match raw("warmth").and_then(|v| v.as_str().map(str::to_owned)) {
            Some(w) => {
                let d = warmth_text(&w).unwrap_or(w.as_str()).to_owned();
                found(json!(w), d)
            }
            None => missing(),
        },
        Attribute::Rain => match raw("rain").and_then(|v| v.as_bool()) {
            Some(true) => found(json!(true), "Light rain".into()),
            Some(false) => found(json!(false), "No".into()),
            None => missing(),
        },
        Attribute::Material => match &card.material {
            Some(m) => found(json!(m), m.clone()),
            None => missing(),
        },
        Attribute::Price => match paid_price(card) {
            Some((m, Some(name))) => found(json!(m), format!("{} ({name})", money_text(&m))),
            Some((m, None)) => found(json!(m), money_text(&m)),
            None => missing(),
        },
    }
}

/// The ascending sort key of an attribute value; `None` sorts last.
pub fn sort_key(a: Attribute, value: &Value) -> Option<(Decimal, String)> {
    let rank = |words: &[&str], w: &str| {
        words
            .iter()
            .position(|x| *x == w)
            .map(|i| (Decimal::from(i), String::new()))
    };
    match a {
        Attribute::Weight => decimal(value).map(|d| (d, String::new())),
        // Smaller packed area first.
        Attribute::PackedSize => {
            let sides: Vec<Decimal> = value.as_array()?.iter().filter_map(decimal).collect();
            (!sides.is_empty()).then(|| (sides.iter().product(), String::new()))
        }
        Attribute::Packing => rank(
            &["own_pocket", "rolls_into_hood", "folds_flat", "none"],
            value.as_str()?,
        ),
        Attribute::Warmth => rank(&["wind_layer", "light", "medium", "warm"], value.as_str()?),
        Attribute::Rain => value
            .as_bool()
            .map(|b| (Decimal::from(u8::from(b)), String::new())),
        Attribute::Material => value.as_str().map(|s| (Decimal::ZERO, s.to_lowercase())),
        Attribute::Price => decimal(value.get("value")?).map(|d| (d, String::new())),
    }
}

/// One `compare_products` item.
pub fn compare_item(card: &ProductCard, attributes: &[Attribute]) -> Value {
    let list = card.summary.price;
    json!({
        "product_id": card.id,
        "name": card.name,
        "color": card.lead_color(),
        "material": card.material,
        "image": card.image,
        "price": list.map_or(Value::Null, |p| num(p.value)),
        "member_price": card.member_price.as_ref().map_or(Value::Null, |m| num(m.price)),
        "price_currency": list.map(|p| p.currency.to_string()),
        "values": attributes.iter().map(|a| attribute_value(card, *a)).collect::<Vec<_>>(),
    })
}

/// Orders comparison items by one attribute, ascending; products without it go last and ties
/// keep the order the products were given in.
pub fn sort_items(items: &mut [Value], a: Attribute) {
    let key = |item: &Value| {
        item["values"]
            .as_array()
            .and_then(|vs| vs.iter().find(|v| v["attribute"] == a.word()))
            .and_then(|v| sort_key(a, &v["value"]))
    };
    items.sort_by(|x, y| match (key(x), key(y)) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
}

/// A facet a filter panel can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FacetField {
    /// Categories, with products under narrower ones counted too.
    Category,
    /// Brands.
    Brand,
    /// List price bands.
    PriceRange,
    /// Variant colors.
    Color,
    /// Variant sizes.
    Size,
    /// Product availability.
    Availability,
}

impl FacetField {
    pub const ALL: [FacetField; 6] = [
        FacetField::Category,
        FacetField::Brand,
        FacetField::PriceRange,
        FacetField::Color,
        FacetField::Size,
        FacetField::Availability,
    ];

    fn word(self) -> &'static str {
        match self {
            FacetField::Category => "category",
            FacetField::Brand => "brand",
            FacetField::PriceRange => "price_range",
            FacetField::Color => "color",
            FacetField::Size => "size",
            FacetField::Availability => "availability",
        }
    }
}

/// The price band a price falls in, `50.00-100.00` for a width of 50.00 (lower end included).
pub fn price_band(price: Decimal, width: Decimal) -> String {
    if width <= Decimal::ZERO {
        return format!("{price:.2}");
    }
    let low =
        (price / width).round_dp_with_strategy(0, RoundingStrategy::ToNegativeInfinity) * width;
    format!("{:.2}-{:.2}", low, low + width)
}

/// Facet values and how many products have each. `labels` names ids (brands, categories).
pub fn facets(
    fields: &[FacetField],
    cards: &[ProductCard],
    links: &[CategoryLink],
    labels: &BTreeMap<String, String>,
    band_width: Decimal,
) -> Vec<Value> {
    let mut out = Vec::new();
    for f in fields {
        // Facet value → the products that have it.
        let mut pairs: Vec<(String, String)> = Vec::new();
        match f {
            FacetField::Category => {
                for l in links {
                    if cards.iter().any(|c| c.id == l.product_id) {
                        pairs.push((l.category_id.clone(), l.product_id.clone()));
                    }
                }
            }
            FacetField::Brand => {
                for c in cards {
                    if let Some(b) = &c.brand_id {
                        pairs.push((b.clone(), c.id.clone()));
                    }
                }
            }
            FacetField::PriceRange => {
                for c in cards {
                    if let Some(p) = c.summary.price {
                        pairs.push((price_band(p.value, band_width), c.id.clone()));
                    }
                }
            }
            FacetField::Color | FacetField::Size => {
                for c in cards {
                    for v in &c.variants {
                        let value = if *f == FacetField::Color {
                            &v.color
                        } else {
                            &v.size
                        };
                        if let Some(value) = value {
                            pairs.push((value.clone(), c.id.clone()));
                        }
                    }
                }
            }
            FacetField::Availability => {
                for c in cards {
                    pairs.push((c.summary.availability.to_owned(), c.id.clone()));
                }
            }
        }
        let mut counts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (value, product) in pairs {
            counts.entry(value).or_default().insert(product);
        }
        let mut values: Vec<(String, usize)> =
            counts.into_iter().map(|(k, v)| (k, v.len())).collect();
        match f {
            FacetField::Size => values.sort_by_key(|a| size_key(Some(&a.0))),
            FacetField::PriceRange => values.sort_by(|a, b| {
                let low = |s: &str| {
                    s.split('-')
                        .next()
                        .and_then(|x| Decimal::from_str(x).ok())
                        .unwrap_or_default()
                };
                low(&a.0).cmp(&low(&b.0))
            }),
            _ => values.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0))),
        }
        for (value, count) in values {
            let mut item = json!({"name": f.word(), "value": value, "count": count});
            if let Some(label) = labels.get(&value) {
                item["label"] = json!(label);
            }
            out.push(item);
        }
    }
    out
}

/// What an autocomplete suggests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SuggestType {
    /// A search word.
    Keyword,
    /// A product name.
    Product,
    /// A brand name.
    Brand,
    /// A category name.
    Category,
}

impl SuggestType {
    pub fn word(self) -> &'static str {
        match self {
            SuggestType::Keyword => "keyword",
            SuggestType::Product => "product",
            SuggestType::Brand => "brand",
            SuggestType::Category => "category",
        }
    }
}

/// Words that start with the prefix, scored by how many products use them (the most used
/// scores 1.00).
pub fn keyword_suggestions(prefix: &str, texts: &[String]) -> Vec<(String, Decimal)> {
    let p = prefix.trim().to_lowercase();
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for t in texts {
        let words: BTreeSet<String> = tokens(t).into_iter().filter(|w| w.len() >= 2).collect();
        for w in words {
            if w.starts_with(&p) {
                *counts.entry(w).or_default() += 1;
            }
        }
    }
    let top = counts.values().copied().max().unwrap_or(1).max(1);
    let mut out: Vec<(String, Decimal)> = counts
        .into_iter()
        .map(|(w, n)| (w, (Decimal::from(n) / Decimal::from(top)).round_dp(2)))
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// Names that start with the prefix (score 1.00) or have a word that does (0.80).
pub fn name_suggestions(
    prefix: &str,
    names: &[(String, String)],
) -> Vec<(String, String, Decimal)> {
    let p = prefix.trim().to_lowercase();
    let mut out: Vec<(String, String, Decimal)> = names
        .iter()
        .filter_map(|(id, name)| {
            let lower = name.to_lowercase();
            let score = if lower.starts_with(&p) {
                Decimal::new(100, 2)
            } else if tokens(&lower).iter().any(|w| w.starts_with(&p)) {
                Decimal::new(80, 2)
            } else {
                return None;
            };
            Some((id.clone(), name.clone(), score))
        })
        .collect();
    out.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.cmp(&b.1)));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_word_is_one_concept_with_its_synonyms() {
        let groups = vec![
            vec!["jacket".to_owned(), "windbreaker".to_owned()],
            vec!["light".to_owned(), "packable".to_owned()],
        ];
        assert_eq!(
            concepts("A light jacket for trips", &groups),
            ["light | packable", "jacket | windbreaker", "trips"]
        );
    }
    use acom_catalog::model::{Summary, Variant};
    use acom_catalog::neighbors::SalePrice;
    use acom_core::Currency;

    fn policies() -> PolicySet {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../policies");
        let texts: Vec<(&str, String)> = acom_policy::FILES
            .iter()
            .map(|f| {
                (
                    *f,
                    std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap_or_default(),
                )
            })
            .collect();
        PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str())))
            .unwrap_or_else(|e| panic!("{e}"))
    }

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap_or_default()
    }

    #[test]
    fn spring_trip_query_reaches_jackets_and_travel_words() {
        let groups = synonyms(&policies(), "en");
        let terms = expand("Light jacket for spring trips!", &groups);
        for w in [
            "light",
            "jacket",
            "spring",
            "trips",
            "travel",
            "windbreaker",
            "overshirt",
            "packable",
        ] {
            assert!(terms.iter().any(|t| t == w), "{w} missing from {terms:?}");
        }
        let q = tsquery(&terms).unwrap_or_default();
        assert!(q.starts_with("light | "), "{q}");
        assert!(
            q.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '|')
        );
        assert_eq!(tsquery(&expand("!!", &groups)), None);
    }

    fn card(id: &str, name: &str, list: &str, member: Option<&str>, props: Value) -> ProductCard {
        let price = Money::new(d(list), Currency::USD);
        ProductCard {
            id: id.into(),
            name: name.into(),
            description: None,
            brand_id: Some("brn_northwind".into()),
            category_id: Some("cat_summer_outerwear".into()),
            approval_status: "active".into(),
            varies_by: vec!["color".into(), "size".into()],
            material: Some("Recycled nylon".into()),
            keywords: vec![],
            additional_property: props,
            image: None,
            release_date: None,
            variants: vec![Variant {
                id: format!("var_{id}_m"),
                product_group_id: id.into(),
                sku: "x".into(),
                name: name.into(),
                color: Some("Slate".into()),
                size: Some("M".into()),
                sale_type: "always".into(),
                is_managed: true,
                offer_id: Some(format!("off_{id}_m")),
                price: Some(d(list)),
                price_currency: Some("USD".into()),
                availability: Some("in_stock".into()),
                inventory_level: Some(18),
                safety_stock: Some(2),
                cost_value: None,
                cost_currency: None,
            }],
            summary: Summary {
                price: Some(price),
                cost: None,
                inventory_level: 18,
                availability: "in_stock",
            },
            member_price: member.map(|m| SalePrice {
                offer_id: format!("off_{id}_m"),
                price: d(m),
                price_currency: "USD".into(),
                valid_for_member_tier: Some("vip".into()),
                promotion_id: Some("promo_1042".into()),
                name: Some("VIP weekend".into()),
                valid_through: None,
            }),
        }
    }

    /// F2: Jamie compares the windbreaker, field jacket and overshirt by packed size.
    fn jamie_compares() -> Vec<ProductCard> {
        vec![
            card(
                "prd_field_jacket",
                "Linen blend field jacket",
                "128.00",
                Some("102.40"),
                json!([
                {"name": "weight", "value": 340, "unit_text": "g", "size": "M"},
                {"name": "packed_size", "value": null, "unit_text": null},
                {"name": "packing", "value": "folds_flat", "unit_text": null},
                {"name": "rain", "value": false, "unit_text": null}]),
            ),
            card(
                "prd_windbreaker",
                "Packable windbreaker",
                "89.00",
                Some("71.20"),
                json!([
                {"name": "weight", "value": 190, "unit_text": "g", "size": "M"},
                {"name": "packed_size", "value": [14, 18], "unit_text": "cm"},
                {"name": "packing", "value": "own_pocket", "unit_text": null},
                {"name": "warmth", "value": "wind_layer", "unit_text": null},
                {"name": "rain", "value": true, "unit_text": null}]),
            ),
            card(
                "prd_seersucker_overshirt",
                "Seersucker overshirt",
                "79.00",
                Some("67.15"),
                json!([
                {"name": "weight", "value": 360, "unit_text": "g", "size": "M"},
                {"name": "packed_size", "value": null, "unit_text": null},
                {"name": "packing", "value": "folds_flat", "unit_text": null}]),
            ),
        ]
    }

    #[test]
    fn compares_like_the_spec_example() {
        let attrs = [
            Attribute::PackedSize,
            Attribute::Weight,
            Attribute::Rain,
            Attribute::Price,
        ];
        let mut items: Vec<Value> = jamie_compares()
            .iter()
            .map(|c| compare_item(c, &attrs))
            .collect();
        sort_items(&mut items, Attribute::PackedSize);
        assert_eq!(items[0]["product_id"], "prd_windbreaker");
        assert_eq!(
            items[0]["values"][0],
            json!({"attribute": "packed_size", "value": [14, 18], "unit": "cm",
                   "display": "Packs into its own pocket, 14 × 18 cm"})
        );
        assert_eq!(items[0]["values"][1]["display"], "190 g");
        assert_eq!(items[0]["values"][2]["display"], "Light rain");
        assert_eq!(items[0]["values"][3]["display"], "$71.20 (VIP weekend)");
        assert_eq!(items[0]["member_price"].to_string(), "71.20");
        // The field jacket stays ahead of the overshirt: no packed size, input order kept.
        assert_eq!(items[1]["product_id"], "prd_field_jacket");
        assert_eq!(items[1]["values"][0]["display"], "Folds flat");
        assert_eq!(
            items[2]["values"][2],
            json!({"attribute": "rain", "value": null, "unit": null, "display": "Not listed"})
        );
        let mut by_price = items.clone();
        sort_items(&mut by_price, Attribute::Price);
        assert_eq!(by_price[0]["product_id"], "prd_seersucker_overshirt");
    }

    #[test]
    fn the_policy_allows_every_comparison_attribute() {
        assert_eq!(allowed_attributes(&policies()), Attribute::ALL.to_vec());
    }

    #[test]
    fn facets_count_products_not_variants() {
        let cards = jamie_compares();
        let links = vec![CategoryLink {
            product_id: "prd_windbreaker".into(),
            category_id: "cat_travel".into(),
            name: "Travel".into(),
            is_primary: false,
            direct: true,
        }];
        let mut labels = BTreeMap::new();
        labels.insert("cat_travel".to_owned(), "Travel".to_owned());
        let f = facets(
            &[
                FacetField::PriceRange,
                FacetField::Category,
                FacetField::Availability,
            ],
            &cards,
            &links,
            &labels,
            d("50.00"),
        );
        assert_eq!(
            f[0],
            json!({"name": "price_range", "value": "50.00-100.00", "count": 2})
        );
        assert_eq!(
            f[1],
            json!({"name": "price_range", "value": "100.00-150.00", "count": 1})
        );
        assert_eq!(
            f[2],
            json!({"name": "category", "value": "cat_travel", "count": 1, "label": "Travel"})
        );
        assert_eq!(
            f[3],
            json!({"name": "availability", "value": "in_stock", "count": 3})
        );
        assert_eq!(price_band(d("89.00"), d("50.00")), "50.00-100.00");
        assert_eq!(price_band(d("100.00"), d("50.00")), "100.00-150.00");
    }

    #[test]
    fn suggests_words_and_names_by_prefix() {
        let texts = vec![
            "Packable windbreaker".to_owned(),
            "Wool overcoat".into(),
            "Windproof shell".into(),
            "Packable windbreaker travel".into(),
        ];
        let words = keyword_suggestions("win", &texts);
        assert_eq!(words[0], ("windbreaker".to_owned(), d("1.00")));
        assert_eq!(words[1], ("windproof".to_owned(), d("0.50")));
        let names = name_suggestions(
            "wind",
            &[
                ("prd_windbreaker".into(), "Packable windbreaker".into()),
                ("prd_shell".into(), "Windproof shell".into()),
            ],
        );
        assert_eq!(names[0].1, "Windproof shell");
        assert_eq!(names[1].2, d("0.80"));
    }
}
