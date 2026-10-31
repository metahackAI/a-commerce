//! Catalog content (`seed/demo/catalog_content.yml`): product facts, size charts, descriptions,
//! tags and photos for the generated products, laid over the generator's rows.
//!
//! The content never changes an id, a name, a price, stock or anything else the random stream
//! decides, so every number in `docs/scenarios.md` and `eval/` stays the same. Named products
//! are written by hand in `03_catalog.yml` and are not touched here.
//!
//! A photo replaces the drawn picture (`crate::media`): `image` for the product, and `images`
//! for the variants of each color it shows. Variants of other colors keep their drawing.

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
    Ok(seen.len())
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
