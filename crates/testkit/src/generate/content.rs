//! Catalog content (`seed/demo/catalog_content.yml`): product facts, size charts, descriptions,
//! tags and photos for the generated products, laid over the generator's rows.
//!
//! The content never changes an id, a name, a price, stock or anything else the random stream
//! decides, so every number in `docs/scenarios.md` and `eval/` stays the same. Named products
//! are written by hand in `03_catalog.yml` and are not touched here.
//!
//! A photo replaces the drawn picture (`crate::media`): `image` for the product, and `images`
//! for the variants of each color it shows. Variants of other colors keep their drawing.
//!
//! `reviews:` adds rows to `catalog.review`, for generated products only. A review with an order
//! comes from the shopper who received that line; one without an order dates from before the
//! store's order history, like the imported reviews of the named products.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::util::decimal;
use crate::SeedRows;

/// The measurement names `catalog.measurement` takes, in the order a size chart lists them.
const MEASUREMENTS: [&str; 10] = [
    "chest",
    "underbust",
    "shoulder",
    "sleeve",
    "length",
    "waist",
    "hip",
    "inseam",
    "rise",
    "foot_length",
];

/// The keys a product entry may have. `summary` waits for a column; `status`, `provenance` and
/// `image_provenance` record how the entry and its photo were made.
const KEYS: [&str; 15] = [
    "id",
    "name_check",
    "description",
    "summary",
    "keywords",
    "additional_property",
    "weight_g",
    "weight_by_size",
    "measurements",
    "measurement_technique",
    "image",
    "images",
    "image_provenance",
    "status",
    "provenance",
];

/// A picture URL the storefront can load: a path the web serves, or an https address.
fn picture_url<'a>(v: &'a Value, at: &str) -> Result<&'a str, String> {
    v.as_str()
        .filter(|u| u.starts_with('/') || u.starts_with("https://"))
        .filter(|u| !u.contains(char::is_whitespace))
        .ok_or_else(|| format!("{at}: not a picture URL (a /path or an https:// address)"))
}

/// A generated product id: `prd_<category term>_<nn>`.
fn generated_id(id: &str) -> bool {
    id.strip_prefix("prd_")
        .and_then(|rest| rest.rsplit_once('_'))
        .is_some_and(|(term, nn)| {
            !term.is_empty() && nn.len() == 2 && nn.bytes().all(|b| b.is_ascii_digit())
        })
}

fn present(v: Option<&Value>) -> Option<&Value> {
    v.filter(|x| !x.is_null())
}

/// One decimal place, as `catalog.measurement.value` and `catalog.product.weight_value` hold.
fn tenths(v: &Value, at: &str) -> Result<i64, String> {
    let x = v.as_f64().ok_or_else(|| format!("{at}: not a number"))?;
    if x <= 0.0 {
        return Err(format!("{at}: must be above zero"));
    }
    Ok((x * 10.0).round() as i64)
}

/// Lays `yaml` (the text of `catalog_content.yml`) over `rows`, the named and generated rows in
/// load order, and appends the size-chart rows. Returns how many products it changed.
pub fn overlay(rows: &mut SeedRows, yaml: &str) -> Result<usize, String> {
    let doc: Value =
        serde_norway::from_str(yaml).map_err(|e| format!("catalog_content.yml: {e}"))?;
    let products = doc
        .get("products")
        .and_then(Value::as_array)
        .ok_or("catalog_content.yml: no `products:` list")?;

    let groups: BTreeMap<String, usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, (t, _))| t == "catalog.product_group")
        .filter_map(|(i, (_, r))| {
            r.get("id")
                .and_then(Value::as_str)
                .map(|id| (id.to_owned(), i))
        })
        .collect();
    let mut seen = BTreeSet::new();
    let mut charts = SeedRows::new();

    for (n, p) in products.iter().enumerate() {
        let id = p
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("catalog_content.yml products[{n}]: no `id`"))?;
        let at = format!("catalog_content.yml {id}");
        if let Some(key) = p
            .as_object()
            .and_then(|o| o.keys().find(|k| !KEYS.contains(&k.as_str())))
        {
            return Err(format!("{at}: unknown key `{key}`"));
        }
        if !generated_id(id) {
            return Err(format!(
                "{at}: only generated products take content; named ones are written in 03_catalog.yml"
            ));
        }
        if !seen.insert(id.to_owned()) {
            return Err(format!("{at}: listed twice"));
        }
        let &i = groups
            .get(id)
            .ok_or_else(|| format!("{at}: the generator made no such product"))?;
        let row = &mut rows[i].1;
        if row.get("name") != p.get("name_check") {
            return Err(format!(
                "{at}: written for `{}`, but the product is now `{}`; generate the content again",
                p.get("name_check").and_then(Value::as_str).unwrap_or("?"),
                row.get("name").and_then(Value::as_str).unwrap_or("?")
            ));
        }
        let tenant = row.get("tenant_id").cloned().unwrap_or(Value::Null);
        for key in ["description", "keywords", "additional_property"] {
            if let Some(v) = present(p.get(key)) {
                row[key] = v.clone();
            }
        }
        if let Some(v) = present(p.get("image")) {
            row["image"] = json!(picture_url(v, &format!("{at} image"))?);
        }
        let mut photos = BTreeMap::new();
        if let Some(v) = present(p.get("images")) {
            let by_color = v
                .as_object()
                .ok_or_else(|| format!("{at} images: not a map of color to picture"))?;
            for (color, url) in by_color {
                photos.insert(
                    color.clone(),
                    picture_url(url, &format!("{at} images {color}"))?.to_owned(),
                );
            }
        }
        if present(p.get("image_provenance")).is_some_and(|v| !v.is_object()) {
            return Err(format!("{at} image_provenance: not a map"));
        }

        let weight = present(p.get("weight_g"))
            .map(|w| tenths(w, &format!("{at} weight_g")))
            .transpose()?;
        let by_size = p.get("weight_by_size").and_then(Value::as_object);
        let mut sizes = BTreeSet::new();
        let mut colors = BTreeSet::new();
        for (t, v) in rows.iter_mut() {
            if t != "catalog.product"
                || v.get("product_group_id").and_then(Value::as_str) != Some(id)
            {
                continue;
            }
            if let Some(color) = v.get("color").and_then(Value::as_str).map(str::to_owned) {
                if let Some(url) = photos.get(&color) {
                    v["image"] = json!(url);
                }
                colors.insert(color);
            }
            let size = v.get("size").and_then(Value::as_str).map(str::to_owned);
            let w = match (&size, by_size) {
                (Some(s), Some(m)) if m.contains_key(s) => {
                    Some(tenths(&m[s], &format!("{at} weight_by_size {s}"))?)
                }
                _ => weight,
            };
            if let Some(w) = w {
                v["weight_value"] = decimal(w, 1);
                v["weight_unit_text"] = json!("g");
            }
            if let Some(s) = size {
                sizes.insert(s);
            }
        }
        if let Some(color) = photos.keys().find(|c| !colors.contains(*c)) {
            return Err(format!(
                "{at}: a photo for color `{color}`, which no variant has"
            ));
        }

        let Some(chart) = present(p.get("measurements")).and_then(Value::as_object) else {
            continue;
        };
        let technique = present(p.get("measurement_technique"))
            .cloned()
            .unwrap_or(Value::Null);
        for (size, values) in chart {
            if !sizes.contains(size) {
                return Err(format!(
                    "{at}: the size chart has size `{size}`, which no variant has"
                ));
            }
            let values = values
                .as_object()
                .ok_or_else(|| format!("{at} measurements {size}: not a map"))?;
            if let Some(name) = values.keys().find(|k| !MEASUREMENTS.contains(&k.as_str())) {
                return Err(format!(
                    "{at} measurements {size}: `{name}` is not a measurement name"
                ));
            }
            for name in MEASUREMENTS {
                let Some(v) = values.get(name) else { continue };
                let value = tenths(v, &format!("{at} measurements {size} {name}"))?;
                charts.push((
                    "catalog.measurement".to_owned(),
                    json!({
                        "tenant_id": tenant,
                        "product_group_id": id,
                        "size": size,
                        "name": name,
                        "value": decimal(value, 1),
                        "unit_text": "cm",
                        "measurement_technique": technique,
                    }),
                ));
            }
        }
    }
    rows.extend(charts);
    add_reviews(rows, &doc)?;
    Ok(seen.len())
}

/// The keys a review may have: its columns, apart from the store.
const REVIEW_KEYS: [&str; 13] = [
    "id",
    "product_group_id",
    "product_id",
    "author_id",
    "author_name",
    "order_id",
    "rating_value",
    "review_body",
    "fit_vote",
    "author_height_value",
    "author_height_unit_text",
    "keywords",
    "date_published",
];

/// Appends the rows of `reviews:` and refuses one no shopper could have written: a review with an
/// order must come from its buyer, for a line of that order, after the parcel arrived, once.
fn add_reviews(rows: &mut SeedRows, doc: &Value) -> Result<(), String> {
    let Some(reviews) = doc.get("reviews") else {
        return Ok(());
    };
    let reviews = reviews
        .as_array()
        .ok_or("catalog_content.yml: `reviews:` is not a list")?;
    let text = |r: &Value, key: &str| r.get(key).and_then(Value::as_str).map(str::to_owned);
    let mut tenant = None;
    let mut variants = BTreeMap::new();
    let mut buyers = BTreeMap::new();
    let mut lines = BTreeSet::new();
    let mut delivered = BTreeMap::new();
    for (table, r) in rows.iter() {
        match table.as_str() {
            "catalog.product_group" => tenant = tenant.or_else(|| r.get("tenant_id").cloned()),
            "catalog.product" => {
                if let (Some(id), Some(group)) = (text(r, "id"), text(r, "product_group_id")) {
                    variants.insert(id, group);
                }
            }
            "sales.sales_order" => {
                if let (Some(id), Some(buyer)) = (text(r, "id"), text(r, "customer_id")) {
                    buyers.insert(id, buyer);
                }
            }
            "sales.order_item" => {
                if let (Some(order), Some(variant)) = (text(r, "order_id"), text(r, "product_id")) {
                    lines.insert((order, variant));
                }
            }
            "fulfillment.parcel_delivery" => {
                if let (Some(order), Some(at)) = (text(r, "order_id"), text(r, "delivered_at")) {
                    delivered.insert(order, at.chars().take(10).collect::<String>());
                }
            }
            _ => {}
        }
    }
    let tenant = tenant.ok_or("catalog_content.yml: reviews, but no products")?;
    let mut ids = BTreeSet::new();
    let mut reviewed = BTreeSet::new();
    let mut out = SeedRows::new();
    for (n, r) in reviews.iter().enumerate() {
        let at = format!("catalog_content.yml reviews[{n}]");
        let map = r.as_object().ok_or_else(|| format!("{at}: not a map"))?;
        if let Some(key) = map.keys().find(|k| !REVIEW_KEYS.contains(&k.as_str())) {
            return Err(format!("{at}: unknown key `{key}`"));
        }
        let id = text(r, "id").ok_or_else(|| format!("{at}: no `id`"))?;
        if !ids.insert(id.clone()) {
            return Err(format!("{at}: `{id}` is listed twice"));
        }
        let group = text(r, "product_group_id").unwrap_or_default();
        if !generated_id(&group) {
            return Err(format!("{at}: `{group}` is not a generated product"));
        }
        let variant = text(r, "product_id").unwrap_or_default();
        if variants.get(&variant) != Some(&group) {
            return Err(format!("{at}: `{variant}` is not a variant of `{group}`"));
        }
        let date = text(r, "date_published").unwrap_or_default();
        if let Some(order) = text(r, "order_id") {
            if buyers.get(&order) != text(r, "author_id").as_ref() {
                return Err(format!("{at}: the author did not place `{order}`"));
            }
            if !lines.contains(&(order.clone(), variant.clone())) {
                return Err(format!("{at}: `{order}` has no line for `{variant}`"));
            }
            if delivered.get(&order).is_none_or(|d| *d > date) {
                return Err(format!("{at}: published before `{order}` arrived"));
            }
            if !reviewed.insert((order.clone(), variant)) {
                return Err(format!("{at}: `{order}` has a review of this line already"));
            }
        }
        let mut row = r.clone();
        row["tenant_id"] = tenant.clone();
        out.push(("catalog.review".to_owned(), row));
    }
    rows.extend(out);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::super::{generate, schema_check};
    use super::*;

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn demo() -> SeedRows {
        let dir = repo().join("seed/demo");
        let mut rows = crate::named::load(&dir).unwrap();
        let spec = std::fs::read_to_string(dir.join("generate.yml")).unwrap();
        let generated = generate(&rows, &spec).unwrap();
        rows.extend(generated);
        rows
    }

    fn group<'a>(rows: &'a SeedRows, id: &str) -> &'a Value {
        rows.iter()
            .find(|(t, r)| t == "catalog.product_group" && r["id"] == id)
            .map(|(_, r)| r)
            .unwrap()
    }

    const SHACKET: &str = r#"
products:
  - id: prd_summer_outerwear_13
    name_check: Poplin shacket
    description: A relaxed shirt jacket in 100% cotton poplin.
    keywords: [everyday, layering, summer]
    additional_property:
      - {name: fiber_content, value: 100% cotton, unit_text: null}
      - {name: weight, value: 395, unit_text: g, size: M}
    weight_g: 395
    weight_by_size: {XS: 365, XL: 425}
    measurements:
      M: {chest: 61, length: 74}
      XS: {chest: 54.5}
    measurement_technique: Garment laid flat.
"#;

    #[test]
    fn content_changes_only_text_facts_weights_and_charts() {
        let before = demo();
        let mut after = before.clone();
        assert_eq!(overlay(&mut after, SHACKET).unwrap(), 1);
        let g = group(&after, "prd_summer_outerwear_13");
        assert_eq!(
            g["description"],
            "A relaxed shirt jacket in 100% cotton poplin."
        );
        assert_eq!(g["keywords"], json!(["everyday", "layering", "summer"]));
        assert_eq!(g["name"], group(&before, "prd_summer_outerwear_13")["name"]);
        let weights: BTreeMap<String, String> = after
            .iter()
            .filter(|(t, r)| {
                t == "catalog.product" && r["product_group_id"] == "prd_summer_outerwear_13"
            })
            .map(|(_, r)| {
                (
                    r["size"].as_str().unwrap().to_owned(),
                    r["weight_value"].to_string(),
                )
            })
            .collect();
        assert_eq!(weights["XS"], "365.0");
        assert_eq!(weights["M"], "395.0");
        assert_eq!(weights["XL"], "425.0");
        let chart: Vec<_> = after[before.len()..]
            .iter()
            .map(|(t, r)| (t.as_str(), r))
            .collect();
        assert_eq!(chart.len(), 3);
        assert!(chart.iter().all(|(t, _)| *t == "catalog.measurement"));
        // Offers (prices, stock) are the same rows as before.
        let offers = |rows: &SeedRows| -> Vec<Value> {
            rows.iter()
                .filter(|(t, _)| t == "catalog.offer")
                .map(|(_, r)| r.clone())
                .collect()
        };
        assert!(offers(&before) == offers(&after));
    }

    #[test]
    fn content_rows_fit_the_migrations() {
        let mut rows = demo();
        overlay(&mut rows, SHACKET).unwrap();
        let path = repo().join("seed/demo/catalog_content.yml");
        if let Ok(text) = std::fs::read_to_string(&path) {
            let mut rows = demo();
            let n = overlay(&mut rows, &text).unwrap_or_else(|e| panic!("{e}"));
            assert!(n > 0);
            let problems =
                schema_check::check(&schema_check::parse(&repo().join("migrations")), &rows);
            assert!(
                problems.is_empty(),
                "{:#?}",
                &problems[..problems.len().min(20)]
            );
        }
        let problems = schema_check::check(&schema_check::parse(&repo().join("migrations")), &rows);
        assert!(
            problems.is_empty(),
            "{:#?}",
            &problems[..problems.len().min(20)]
        );
    }

    /// One order line of a generated member, delivered, with its product and buyer.
    fn delivered_line(rows: &SeedRows) -> (String, String, String, String, String) {
        let variant_group: BTreeMap<&str, &str> = rows
            .iter()
            .filter(|(t, _)| t == "catalog.product")
            .map(|(_, r)| {
                (
                    r["id"].as_str().unwrap(),
                    r["product_group_id"].as_str().unwrap(),
                )
            })
            .collect();
        let delivered: BTreeMap<&str, &str> = rows
            .iter()
            .filter(|(t, r)| t == "fulfillment.parcel_delivery" && r["delivered_at"].is_string())
            .map(|(_, r)| {
                (
                    r["order_id"].as_str().unwrap(),
                    r["delivered_at"].as_str().unwrap(),
                )
            })
            .collect();
        let buyer: BTreeMap<&str, &str> = rows
            .iter()
            .filter(|(t, _)| t == "sales.sales_order")
            .map(|(_, r)| {
                (
                    r["id"].as_str().unwrap(),
                    r["customer_id"].as_str().unwrap(),
                )
            })
            .collect();
        let (_, line) = rows
            .iter()
            .find(|(t, r)| {
                t == "sales.order_item"
                    && delivered.contains_key(r["order_id"].as_str().unwrap())
                    && generated_id(variant_group[r["product_id"].as_str().unwrap()])
            })
            .unwrap();
        let order = line["order_id"].as_str().unwrap();
        let variant = line["product_id"].as_str().unwrap();
        (
            variant_group[variant].to_owned(),
            variant.to_owned(),
            order.to_owned(),
            buyer[order].to_owned(),
            delivered[order].chars().take(10).collect(),
        )
    }

    fn review_yaml(rows: &SeedRows, change: impl Fn(&mut Value)) -> String {
        let (group, variant, order, buyer, date) = delivered_line(rows);
        let mut review = json!({
            "id": "rev_test_01", "product_group_id": group, "product_id": variant, "author_id": buyer,
            "author_name": "Ke*****", "order_id": order, "rating_value": 4,
            "review_body": "Holds up well.", "keywords": [], "date_published": date,
        });
        change(&mut review);
        serde_norway::to_string(&json!({"version": 1, "products": [], "reviews": [review]}))
            .unwrap()
    }

    #[test]
    fn a_review_comes_from_the_buyer_of_a_delivered_line() {
        let mut rows = demo();
        let good = review_yaml(&rows, |_| {});
        overlay(&mut rows, &good).unwrap();
        let added: Vec<&Value> = rows
            .iter()
            .filter(|(t, r)| t == "catalog.review" && r["id"] == "rev_test_01")
            .map(|(_, r)| r)
            .collect();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0]["tenant_id"], "store_northwind");
    }

    #[test]
    fn a_review_no_shopper_could_write_is_refused() {
        let rows = demo();
        for (change, words) in [
            (
                Box::new(|r: &mut Value| r["author_id"] = json!("cus_m000001"))
                    as Box<dyn Fn(&mut Value)>,
                "did not place",
            ),
            (
                Box::new(|r: &mut Value| r["date_published"] = json!("2026-01-01")),
                "before",
            ),
            (
                Box::new(|r: &mut Value| r["product_group_id"] = json!("prd_wool_overcoat")),
                "not a generated",
            ),
            (
                Box::new(|r: &mut Value| r["surprise"] = json!(1)),
                "unknown key",
            ),
        ] {
            let e = overlay(&mut rows.clone(), &review_yaml(&rows, change)).unwrap_err();
            assert!(e.contains(words), "{e}");
        }
    }

    #[test]
    fn reviews_in_the_content_file_fit_the_migrations_and_the_named_products_keep_theirs() {
        let Ok(text) = std::fs::read_to_string(repo().join("seed/demo/catalog_content.yml")) else {
            return;
        };
        let mut rows = demo();
        let overcoat_before = rows
            .iter()
            .filter(|(t, r)| t == "catalog.review" && r["product_group_id"] == "prd_wool_overcoat")
            .count();
        overlay(&mut rows, &text).unwrap_or_else(|e| panic!("{e}"));
        let overcoat = rows
            .iter()
            .filter(|(t, r)| t == "catalog.review" && r["product_group_id"] == "prd_wool_overcoat")
            .count();
        assert_eq!(overcoat, overcoat_before);
        let problems = schema_check::check(&schema_check::parse(&repo().join("migrations")), &rows);
        assert!(
            problems.is_empty(),
            "{:#?}",
            &problems[..problems.len().min(20)]
        );
    }

    #[test]
    fn bra_size_charts_take_the_underbust() {
        let mut rows = demo();
        let bra = r#"
products:
  - id: prd_activewear_03
    name_check: Mesh sports bra
    measurements:
      S: {chest: 84, underbust: 70}
      M: {chest: 89, underbust: 75}
    measurement_technique: Body measurements.
"#;
        assert_eq!(overlay(&mut rows, bra).unwrap(), 1);
        let problems = schema_check::check(&schema_check::parse(&repo().join("migrations")), &rows);
        assert!(problems.is_empty(), "{problems:#?}");
    }

    const CARDIGAN: &str = r#"
products:
  - id: prd_knitwear_11
    name_check: Wool cardigan
    image: /media/products/prd_knitwear_11-a40cad40.webp
    images: {White: /media/products/prd_knitwear_11-a40cad40.webp}
    image_provenance: {model: muse-image-1.0, run: 20261031-213410-full, response_id: resp_0}
"#;

    #[test]
    fn a_photo_replaces_the_drawing_of_its_color_only() {
        let before = demo();
        let mut after = before.clone();
        assert_eq!(overlay(&mut after, CARDIGAN).unwrap(), 1);
        let photo = "/media/products/prd_knitwear_11-a40cad40.webp";
        assert_eq!(group(&after, "prd_knitwear_11")["image"], photo);
        let variants: Vec<&Value> = after
            .iter()
            .filter(|(t, r)| t == "catalog.product" && r["product_group_id"] == "prd_knitwear_11")
            .map(|(_, r)| r)
            .collect();
        assert!(variants.iter().any(|v| v["color"] == "Olive"));
        for v in variants {
            if v["color"] == "White" {
                assert_eq!(v["image"], photo);
            } else {
                assert_eq!(v["image"], "/media/garment/knit-olive.svg");
            }
        }
        // Only image fields change: same rows, same order, same names, prices and stock.
        assert_eq!(before.len(), after.len());
        for ((table, was), (_, now)) in before.iter().zip(&after) {
            for (k, v) in now.as_object().unwrap() {
                if was.get(k) != Some(v) {
                    assert_eq!(k, "image", "{table}.{k} changed: {now}");
                }
            }
        }
    }

    #[test]
    fn photos_that_do_not_fit_are_refused() {
        let photo = "/media/products/prd_knitwear_11-a40cad40.webp";
        let cases = [
            (CARDIGAN.replace("{White: ", "{Red: "), "no variant has"),
            (
                CARDIGAN.replace("image: /media", "image: media"),
                "not a picture URL",
            ),
            (
                CARDIGAN.replace(&format!("{{White: {photo}}}"), &format!("[{photo}]")),
                "not a map of color",
            ),
            (
                CARDIGAN.replace("{model: muse-image-1.0", "muse #{model: muse-image-1.0"),
                "image_provenance: not a map",
            ),
        ];
        for (yaml, want) in cases {
            let e = overlay(&mut demo(), &yaml).unwrap_err();
            assert!(e.contains(want), "{yaml}: {e}");
        }
    }

    /// Once the seed is loaded, a variant shows a photo of its own color: one photo is never used
    /// for two colors of a product, and a photo made for another color (`prd_x--olive-….webp`)
    /// sits only on the variants of that color.
    #[test]
    fn every_variant_photo_shows_its_own_color() {
        let Ok(text) = std::fs::read_to_string(repo().join("seed/demo/catalog_content.yml")) else {
            return;
        };
        let mut rows = demo();
        overlay(&mut rows, &text).unwrap();
        let mut colors: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
        for (t, r) in &rows {
            let Some(url) = r["image"]
                .as_str()
                .filter(|u| u.starts_with("/media/products/"))
            else {
                continue;
            };
            if t != "catalog.product" {
                continue;
            }
            let group = r["product_group_id"].as_str().unwrap_or_default();
            let color = r["color"].as_str().unwrap_or_default();
            colors
                .entry((group.to_owned(), url.to_owned()))
                .or_default()
                .insert(color.to_owned());
            if let Some((pid, rest)) = url.trim_start_matches("/media/products/").split_once("--") {
                let slug = color.to_lowercase().replace(' ', "-");
                assert_eq!(pid, group, "{}: {url} is another product's photo", r["id"]);
                assert!(
                    rest.starts_with(&format!("{slug}-")),
                    "{}: {url} is not a {color} photo",
                    r["id"]
                );
            }
        }
        assert!(!colors.is_empty());
        for ((group, url), shown) in &colors {
            assert_eq!(shown.len(), 1, "{group}: {url} is used for {shown:?}");
        }
    }

    /// Every photo the seed names (the content file, and the named products of `03_catalog.yml`)
    /// is in `web/public` (the web serves it), a product's photo is one of its color photos, and
    /// no photo file is left that nothing names.
    #[test]
    fn photos_the_seed_names_are_served_by_the_web() {
        let Ok(text) = std::fs::read_to_string(repo().join("seed/demo/catalog_content.yml")) else {
            return;
        };
        let doc: Value = serde_norway::from_str(&text).unwrap();
        let public = repo().join("web/public");
        let mut named = BTreeSet::new();
        let catalog: Value = serde_norway::from_str(
            &std::fs::read_to_string(repo().join("seed/demo/03_catalog.yml")).unwrap(),
        )
        .unwrap();
        for table in ["catalog.product_group", "catalog.product"] {
            for row in catalog["tables"][table].as_array().unwrap() {
                let url = row["image"].as_str().unwrap_or_default();
                if let Some(path) = url
                    .strip_prefix('/')
                    .filter(|u| u.starts_with("media/products/"))
                {
                    assert!(
                        public.join(path).is_file(),
                        "{}: {url} is not in web/public",
                        row["id"]
                    );
                    named.insert(path.to_owned());
                }
            }
        }
        for p in doc["products"].as_array().unwrap() {
            let by_color: Vec<&Value> = p["images"]
                .as_object()
                .map(|m| m.values().collect())
                .unwrap_or_default();
            if let (Some(image), false) = (p["image"].as_str(), by_color.is_empty()) {
                assert!(
                    by_color.iter().any(|u| u.as_str() == Some(image)),
                    "{}: image is not one of its color photos",
                    p["id"]
                );
            }
            for url in by_color
                .into_iter()
                .chain(p.get("image"))
                .filter_map(Value::as_str)
            {
                if let Some(path) = url
                    .strip_prefix('/')
                    .filter(|u| u.starts_with("media/products/"))
                {
                    assert!(
                        public.join(path).is_file(),
                        "{}: {url} is not in web/public",
                        p["id"]
                    );
                    named.insert(path.to_owned());
                }
            }
        }
        if let Ok(files) = std::fs::read_dir(public.join("media/products")) {
            for f in files.flatten() {
                let path = format!("media/products/{}", f.file_name().to_string_lossy());
                assert!(
                    named.contains(&path),
                    "{path} is in web/public but no product names it"
                );
            }
        }
    }

    #[test]
    fn content_that_does_not_fit_is_refused() {
        let cases = [
            (
                "prd_summer_outerwear_13",
                "Linen shacket",
                "now `Poplin shacket`",
            ),
            (
                "prd_windbreaker",
                "Packable windbreaker",
                "named ones are written",
            ),
            ("prd_summer_outerwear_99", "Ghost", "no such product"),
        ];
        for (id, name, want) in cases {
            let yaml = format!("products:\n  - {{id: {id}, name_check: {name}}}\n");
            let e = overlay(&mut demo(), &yaml).unwrap_err();
            assert!(e.contains(want), "{e}");
        }
        let bad_name = SHACKET.replace("chest: 54.5", "neck: 40");
        assert!(
            overlay(&mut demo(), &bad_name)
                .unwrap_err()
                .contains("not a measurement name")
        );
        let bad_size = SHACKET.replace("XS: {chest: 54.5}", "XXL: {chest: 70}");
        assert!(
            overlay(&mut demo(), &bad_size)
                .unwrap_err()
                .contains("no variant has")
        );
        let bad_key = SHACKET.replace("weight_g: 395", "weight_g: 395\n    colour: red");
        assert!(
            overlay(&mut demo(), &bad_key)
                .unwrap_err()
                .contains("unknown key `colour`")
        );
    }
}
