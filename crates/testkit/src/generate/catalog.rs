//! Generated products: product groups, variants, offers, category placement and the autumn
//! knitwear sale prices (`generate.yml` `catalog`).

use std::collections::BTreeSet;

use chrono::{NaiveDate, TimeDelta};
use serde_json::{Value, json};

use super::named::Named;
use super::spec::{CategorySpec, Spec, prose};
use super::util::{Dice, decimal, money, round_div, slug};

#[derive(Debug)]
pub(crate) struct Product {
    pub id: String,
    pub term: String,
    pub category_id: String,
    pub name: String,
    pub description: String,
    pub material: String,
    pub brand_id: String,
    pub image: String,
    pub release_date: NaiveDate,
    pub varies_by: Vec<&'static str>,
    pub attributes: Value,
    pub variants: Vec<Variant>,
}

#[derive(Debug)]
pub(crate) struct Variant {
    pub id: String,
    pub offer_id: String,
    pub sku: String,
    pub name: String,
    pub color: String,
    pub size: Option<String>,
    pub weight_g: i64,
    pub price: i64,
    pub cost: i64,
    pub inventory: i64,
}

#[derive(Debug)]
pub(crate) struct Catalog {
    pub products: Vec<Product>,
    /// (product id, display category id, position).
    pub placements: Vec<(String, String, i64)>,
}

/// SKU prefix: the initials of a several-word term code (`summer_outerwear` -> `so`); a
/// one-word code keeps the whole word, because one-letter initials repeat across the 24
/// categories (shirts, shorts, shoes ...) and SKUs must be unique in the store.
pub(crate) fn sku_prefix(term: &str) -> String {
    let words: Vec<&str> = term.split('_').filter(|w| !w.is_empty()).collect();
    if words.len() > 1 {
        words.iter().filter_map(|w| w.chars().next()).collect()
    } else {
        term.to_owned()
    }
}

fn article(phrase: &str) -> &'static str {
    match phrase.chars().next().map(|c| c.to_ascii_lowercase()) {
        Some('a' | 'e' | 'i' | 'o' | 'u') => "An",
        _ => "A",
    }
}

fn release_window(term: &str) -> (NaiveDate, NaiveDate) {
    let d = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap_or_default();
    if term == "summer_outerwear" {
        (d(2026, 2, 1), d(2026, 6, 30))
    } else {
        (d(2024, 9, 1), d(2026, 8, 31))
    }
}

pub(crate) fn generate(spec: &Spec, named: &Named, dice: &mut Dice) -> Result<Catalog, String> {
    let c = &spec.catalog;
    let mut used_names: BTreeSet<String> = named
        .products
        .values()
        .map(|(n, _)| n.to_lowercase())
        .collect();
    let mut skus = named.skus.clone();
    let mut products = Vec::new();
    for cat in &c.categories {
        let category_id = named
            .categories
            .get(&cat.term)
            .cloned()
            .ok_or_else(|| format!("no named category with term_code `{}`", cat.term))?;
        if !named.standard_categories.contains(&category_id) {
            return Err(format!("`{category_id}` is not a standard category"));
        }
        for nn in 1..=cat.count {
            let p = product(spec, cat, &category_id, nn, &mut used_names, dice)?;
            for v in &p.variants {
                if !skus.insert(v.sku.clone()) {
                    return Err(format!("generated SKU `{}` is not unique", v.sku));
                }
            }
            products.push(p);
        }
    }
    let named_count = named.products.len();
    if named_count + products.len() != c.total_products {
        return Err(format!(
            "catalog: {named_count} named + {} generated products, the spec wants {}",
            products.len(),
            c.total_products
        ));
    }

    let mut placements = Vec::new();
    for (term, display) in prose::PLACEMENT {
        let picked: Vec<usize> = {
            let mut idx: Vec<usize> = products
                .iter()
                .enumerate()
                .filter(|(_, p)| p.term == *term)
                .map(|(i, _)| i)
                .collect();
            let n = round_div(idx.len() as i64 * prose::PLACEMENT_PERCENT, 100) as usize;
            dice.shuffle(&mut idx);
            idx.truncate(n);
            idx.sort_unstable();
            idx
        };
        let mut position = named.last_position.get(*display).copied().unwrap_or(0);
        for i in picked {
            position += 1;
            placements.push((products[i].id.clone(), (*display).to_owned(), position));
        }
    }
    Ok(Catalog {
        products,
        placements,
    })
}

fn product(
    spec: &Spec,
    cat: &CategorySpec,
    category_id: &str,
    nn: usize,
    used_names: &mut BTreeSet<String>,
    dice: &mut Dice,
) -> Result<Product, String> {
    let c = &spec.catalog;
    let suffix = format!("{}_{nn:02}", cat.term);
    let mut material = String::new();
    let mut noun = String::new();
    let mut name = String::new();
    for attempt in 0..64 {
        material = dice.pick(&cat.materials).clone();
        noun = dice.pick(&cat.nouns).clone();
        name = format!("{material} {noun}");
        if attempt == 63 {
            name = format!("{name} {nn}");
        }
        if used_names.insert(name.to_lowercase()) {
            break;
        }
    }
    let brand_id = dice.pick(&c.brands).clone();
    let prices: Vec<i64> = (cat.price_dollars.0..=cat.price_dollars.1)
        .filter(|p| p % 10 == 9)
        .collect();
    if prices.is_empty() {
        return Err(format!("{}: no x9 price in the range", cat.term));
    }
    let price = dice.pick(&prices) * 100;
    let ratio = dice.range(cat.cost_ratio_bp.0, cat.cost_ratio_bp.1);
    let cost = round_div(price * ratio, 10_000);
    if round_div(cost * 115, 100) > price {
        return Err(format!(
            "{suffix}: cost {cost} x 1.15 is above the list price {price}"
        ));
    }
    let stock = dice.range(cat.stock.0, cat.stock.1);
    let n_colors = dice
        .range(cat.colors_per_product.0, cat.colors_per_product.1)
        .clamp(1, cat.colors.len() as i64) as usize;
    let mut palette = cat.colors.clone();
    // Partial Fisher-Yates: the first n are a uniform sample, in draw order.
    for i in 0..n_colors {
        let j = i + dice.below(palette.len() - i);
        palette.swap(i, j);
    }
    palette.truncate(n_colors);
    let weight_g = dice.range(cat.weight_g.0, cat.weight_g.1);

    let mut attributes = json!([]);
    if let Some(a) = &cat.attributes {
        let packing = dice.pick(&a.packing).clone();
        let warmth = dice.pick(&a.warmth).clone();
        let rain = *dice.pick(&a.rain);
        let packed = if packing == prose::PACKED_WHEN {
            let short = dice.range(prose::PACKED_SHORT.0, prose::PACKED_SHORT.1);
            let long = dice.range(prose::PACKED_LONG.0, prose::PACKED_LONG.1);
            json!({"name": "packed_size", "value": [short, long], "unit_text": "cm"})
        } else {
            json!({"name": "packed_size", "value": null, "unit_text": null})
        };
        attributes = json!([
            {"name": "weight", "value": weight_g, "unit_text": "g", "size": "M"},
            packed,
            {"name": "packing", "value": packing, "unit_text": null},
            {"name": "warmth", "value": warmth, "unit_text": null},
            {"name": "rain", "value": rain, "unit_text": null},
        ]);
    }

    let mut variants = Vec::new();
    for color in &palette {
        for size in &cat.sizes {
            let mut vsuffix = suffix.clone();
            let mut sku = format!("{}-{nn:02}-{}", sku_prefix(&cat.term), slug(color, '-'));
            let mut vname = format!("{name}, {color}");
            if let Some(size) = size {
                vsuffix.push('_');
                vsuffix.push_str(&slug(size, '_'));
                sku.push('-');
                sku.push_str(&slug(size, '-'));
                vname.push_str(", ");
                vname.push_str(size);
            }
            vsuffix.push('_');
            vsuffix.push_str(&slug(color, '_'));
            variants.push(Variant {
                id: format!("var_{vsuffix}"),
                offer_id: format!("off_{vsuffix}"),
                sku,
                name: vname,
                color: color.clone(),
                size: size.clone(),
                weight_g,
                price,
                cost,
                inventory: 0,
            });
        }
    }
    // Stock split at random: every variant gets one unit first unless there are fewer units
    // than variants, then each remaining unit goes to a variant drawn uniformly.
    let n = variants.len();
    if stock as usize >= n {
        for v in &mut variants {
            v.inventory = 1;
        }
        for _ in 0..(stock as usize - n) {
            variants[dice.below(n)].inventory += 1;
        }
    } else {
        let mut idx: Vec<usize> = (0..n).collect();
        dice.shuffle(&mut idx);
        for &i in idx.iter().take(stock as usize) {
            variants[i].inventory = 1;
        }
    }
    let (from, through) = release_window(&cat.term);
    let days = (through - from).num_days();
    let release_date = from + TimeDelta::days(dice.range(0, days));

    Ok(Product {
        id: format!("prd_{suffix}"),
        term: cat.term.clone(),
        category_id: category_id.to_owned(),
        description: format!(
            "{} {} {}.",
            article(&material),
            material.to_lowercase(),
            noun
        ),
        name,
        material,
        brand_id,
        image: format!("https://cdn.northwind.example/p/{suffix}.webp"),
        release_date,
        varies_by: if cat.sizes.iter().any(Option::is_some) {
            vec!["color", "size"]
        } else {
            vec!["color"]
        },
        attributes,
        variants,
    })
}

/// Rows in load order: product groups, variants, offers, category placement, then the
/// autumn knitwear sale prices.
pub(crate) fn rows(
    spec: &Spec,
    named: &Named,
    catalog: &Catalog,
    autumn_knit: Option<&super::named::ItemPromotion>,
    out: &mut crate::SeedRows,
) -> Result<(), String> {
    let tenant = &spec.store;
    let currency = &named.currency;
    let c = &spec.catalog;
    for p in &catalog.products {
        out.push((
            "catalog.product_group".to_owned(),
            json!({
                "tenant_id": tenant,
                "id": p.id,
                "name": p.name,
                "description": p.description,
                "brand_id": p.brand_id,
                "approval_status": c.approval_status,
                "varies_by": p.varies_by,
                "material": p.material,
                "additional_property": p.attributes,
                "image": p.image,
                "has_merchant_return_policy_id": c.return_policy,
                "release_date": p.release_date.format("%Y-%m-%d").to_string(),
            }),
        ));
    }
    for p in &catalog.products {
        for v in &p.variants {
            let mut row = json!({
                "tenant_id": tenant,
                "id": v.id,
                "product_group_id": p.id,
                "sku": v.sku,
                "name": v.name,
                "color": v.color,
            });
            if let (Some(size), Some(map)) = (&v.size, row.as_object_mut()) {
                map.insert("size".to_owned(), json!(size));
            }
            if let Some(map) = row.as_object_mut() {
                map.insert("weight_value".to_owned(), decimal(v.weight_g * 10, 1));
                map.insert("weight_unit_text".to_owned(), json!("g"));
            }
            out.push(("catalog.product".to_owned(), row));
        }
    }
    for p in &catalog.products {
        for v in &p.variants {
            out.push((
                "catalog.offer".to_owned(),
                json!({
                    "tenant_id": tenant,
                    "id": v.offer_id,
                    "product_id": v.id,
                    "price": money(v.price),
                    "price_currency": currency,
                    "availability": if v.inventory > 0 { "in_stock" } else { "out_of_stock" },
                    "inventory_level": v.inventory,
                    "safety_stock": c.safety_stock,
                    "cost_value": money(v.cost),
                    "cost_currency": currency,
                }),
            ));
        }
    }
    for p in &catalog.products {
        out.push((
            "merchandising.product_group_category".to_owned(),
            json!({
                "tenant_id": tenant,
                "product_group_id": p.id,
                "category_id": p.category_id,
                "is_primary": true,
            }),
        ));
    }
    for (product, category, position) in &catalog.placements {
        out.push((
            "merchandising.product_group_category".to_owned(),
            json!({
                "tenant_id": tenant,
                "product_group_id": product,
                "category_id": category,
                "is_primary": false,
                "position": position,
            }),
        ));
    }

    let promo = autumn_knit.ok_or_else(|| {
        format!(
            "named rows: no running item_price promotion `{}`",
            prose::AUTUMN_KNIT_PROMOTION
        )
    })?;
    let tz = named.tz;
    for p in catalog
        .products
        .iter()
        .filter(|p| p.term == prose::AUTUMN_KNIT_CATEGORY)
    {
        for v in &p.variants {
            let suffix = v.id.trim_start_matches("var_");
            let mut row = json!({
                "tenant_id": tenant,
                "id": format!("ups_{suffix}_autumn_knit"),
                "offer_id": v.offer_id,
                "promotion_id": promo.id,
                "price": money(v.price - round_div(v.price * promo.percent_bp, 10_000)),
                "price_currency": currency,
                "price_type": "sale_price",
                "valid_for_member_tier": null,
                "valid_from": super::util::rfc3339(&promo.from.with_timezone(&tz)),
            });
            if let Some(map) = row.as_object_mut() {
                map.insert(
                    "valid_through".to_owned(),
                    promo
                        .through
                        .map(|t| json!(super::util::rfc3339(&t)))
                        .unwrap_or(Value::Null),
                );
                map.insert("percent_off".to_owned(), decimal(promo.percent_bp, 2));
            }
            out.push(("promotion.unit_price_specification".to_owned(), row));
        }
    }
    Ok(())
}
