//! Catalog content (`seed/demo/catalog_content.yml`): product facts, size charts, descriptions
//! and tags for the generated products, laid over the generator's rows.
//!
//! The content never changes an id, a name, a price, stock or anything else the random stream
//! decides, so every number in `docs/scenarios.md` and `eval/` stays the same. Named products
//! are written by hand in `03_catalog.yml` and are not touched here.

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

/// The keys a product entry may have. `summary` waits for a column; `status` and `provenance`
/// record how the entry was made.
const KEYS: [&str; 12] = [
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
    "status",
    "provenance",
];

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

        let weight = present(p.get("weight_g"))
            .map(|w| tenths(w, &format!("{at} weight_g")))
            .transpose()?;
        let by_size = p.get("weight_by_size").and_then(Value::as_object);
        let mut sizes = BTreeSet::new();
        for (t, v) in rows.iter_mut() {
            if t != "catalog.product"
                || v.get("product_group_id").and_then(Value::as_str) != Some(id)
            {
                continue;
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
