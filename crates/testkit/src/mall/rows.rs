//! The mall's generated rows in load order: brands, departments and categories, products
//! (`catalog.product_group`), their variants and offers, and each product's category.
//!
//! Facts go into `additional_property` as `{name, value, unit_text}` under the attribute names
//! of `policies/taxonomy/` (product_type, material_family, care_method, fit, ...), with values
//! from its word lists. `keywords` repeat the words a shopper may search by that the name does
//! not carry: who it is for, the color families, the material family and the fit.

use serde_json::{Value, json};

use super::NamedStore;
use super::draw::Product;
use super::spec::{Gender, Mall};
use crate::SeedRows;
use crate::generate::util::money;

fn prop(name: &str, value: Value) -> Value {
    json!({"name": name, "value": value, "unit_text": null})
}

fn measure(name: &str, value: i64, unit: &str) -> Value {
    json!({"name": name, "value": value, "unit_text": unit})
}

/// The product's facts: what its taxonomy node needs, and fabric where the material is a weave
/// or knit of its own.
pub(crate) fn group_properties(mall: &Mall, p: &Product) -> Vec<Value> {
    let category = &mall.categories[p.category];
    let required = &category.required;
    let Some(m) = mall.materials.get(&p.material) else {
        return Vec::new();
    };
    let mut out = vec![
        prop("product_type", json!(p.type_name)),
        prop("target_gender", json!(p.gender.word())),
        prop("age_group", json!("adult")),
        prop("material_family", json!(m.family)),
        prop("care_method", json!(m.care)),
        prop("country_of_origin", json!(p.origin)),
    ];
    if let Some(fit) = &p.fit {
        out.push(prop("fit", json!(fit)));
    }
    if required.contains("thickness") {
        out.push(prop("thickness", json!(m.thickness)));
    }
    if let Some(fabric) = category.fabric.as_ref().or(m.fabric.as_ref()) {
        out.push(prop("fabric_type", json!(fabric)));
    }
    if let Some(w) = category.warmth.get(&p.type_name) {
        out.push(prop("warmth", json!(w)));
    }
    if required.contains("water_resistance") {
        let w = category
            .water
            .get(&p.type_name)
            .map_or("none", String::as_str);
        out.push(prop("water_resistance", json!(w)));
    }
    if let Some(mm) = p.heel_mm {
        out.push(measure("heel_height", mm, "mm"));
    }
    out
}

/// A variant's facts: its color families and pattern, and its foot length for shoes.
pub(crate) fn variant_properties(mall: &Mall, p: &Product, color: &str, size: &str) -> Vec<Value> {
    let c = mall.colors.get(color);
    let mut out = vec![
        prop(
            "color_family",
            json!(c.map(|c| c.families.clone()).unwrap_or_default()),
        ),
        prop(
            "pattern",
            json!(c.and_then(|c| c.pattern.as_deref()).unwrap_or("solid")),
        ),
    ];
    if let Some(mm) = mall
        .foot_length
        .get(&p.size_system)
        .and_then(|t| t.get(size))
    {
        out.push(measure("foot_length", *mm, "mm"));
    }
    out
}

/// Words to search by that the name does not carry.
fn keywords(mall: &Mall, p: &Product) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |w: &str| {
        let w = w.replace('_', " ");
        if !out.contains(&w) {
            out.push(w);
        }
    };
    add(match p.gender {
        Gender::Female => "women",
        Gender::Male => "men",
        Gender::Unisex => "unisex",
    });
    for c in &p.colors {
        for f in mall
            .colors
            .get(c)
            .map(|c| c.families.as_slice())
            .unwrap_or_default()
        {
            add(f);
        }
    }
    if let Some(m) = mall.materials.get(&p.material) {
        add(&m.family);
    }
    if let Some(fit) = &p.fit {
        add(fit);
    }
    out
}

/// The placeholder description until the text model writes one: brand, material, origin.
fn description(mall: &Mall, p: &Product) -> String {
    let brand = &mall.brands[p.brand].name;
    let country = mall
        .countries
        .get(&p.origin)
        .map_or(p.origin.as_str(), String::as_str);
    format!("By {brand}. {}, made in {country}.", p.material)
}

pub(crate) fn rows(mall: &Mall, store: &NamedStore, products: &[Product]) -> SeedRows {
    let tenant = &mall.store.id;
    let currency = &store.currency;
    let mut out = SeedRows::new();
    for b in &mall.brands {
        out.push((
            "catalog.brand".to_owned(),
            json!({"tenant_id": tenant, "id": b.id, "name": b.name, "description": b.identity}),
        ));
    }
    for (i, d) in mall.departments.iter().enumerate() {
        out.push((
            "merchandising.defined_term".to_owned(),
            json!({
                "tenant_id": tenant,
                "id": format!("cat_{}", d.key),
                "name": d.name,
                "term_code": d.key,
                "in_defined_term_set": "standard",
                "position": i + 1,
                "is_active": true,
            }),
        ));
    }
    for d in &mall.departments {
        for (i, key) in d.categories.iter().enumerate() {
            let name = mall.category(key).map_or(key.as_str(), |c| c.name.as_str());
            out.push((
                "merchandising.defined_term".to_owned(),
                json!({
                    "tenant_id": tenant,
                    "id": format!("cat_{key}"),
                    "name": name,
                    "term_code": key,
                    "in_defined_term_set": "standard",
                    "broader_id": format!("cat_{}", d.key),
                    "position": i + 1,
                    "is_active": true,
                }),
            ));
        }
    }
    for p in products {
        let first = p.colors.first().map_or("", String::as_str);
        let varies_by = if p.sizes.len() > 1 {
            json!(["color", "size"])
        } else {
            json!(["color"])
        };
        out.push((
            "catalog.product_group".to_owned(),
            json!({
                "tenant_id": tenant,
                "id": p.id,
                "name": p.name,
                "description": description(mall, p),
                "brand_id": mall.brands[p.brand].id,
                "approval_status": "active",
                "varies_by": varies_by,
                "material": p.material,
                "keywords": keywords(mall, p),
                "additional_property": group_properties(mall, p),
                "image": crate::media::garment_image(&p.name, first),
                "has_merchant_return_policy_id": mall.store.return_policy,
                "release_date": p.released.format("%Y-%m-%d").to_string(),
            }),
        ));
    }
    for p in products {
        for v in &p.variants {
            out.push((
                "catalog.product".to_owned(),
                json!({
                    "tenant_id": tenant,
                    "id": v.id,
                    "product_group_id": p.id,
                    "sku": v.sku,
                    "name": v.name,
                    "color": v.color,
                    "size": v.size,
                    "image": crate::media::garment_image(&p.name, &v.color),
                    "additional_property": variant_properties(mall, p, &v.color, &v.size),
                }),
            ));
        }
    }
    for p in products {
        for v in &p.variants {
            out.push((
                "catalog.offer".to_owned(),
                json!({
                    "tenant_id": tenant,
                    "id": v.offer_id,
                    "product_id": v.id,
                    "price": money(p.price),
                    "price_currency": currency,
                    "availability": if v.stock > 0 { "in_stock" } else { "out_of_stock" },
                    "inventory_level": v.stock,
                    "safety_stock": mall.store.safety_stock,
                    "cost_value": money(p.cost),
                    "cost_currency": currency,
                }),
            ));
        }
    }
    for p in products {
        out.push((
            "merchandising.product_group_category".to_owned(),
            json!({
                "tenant_id": tenant,
                "product_group_id": p.id,
                "category_id": format!("cat_{}", mall.categories[p.category].key),
                "is_primary": true,
            }),
        ));
    }
    out
}
