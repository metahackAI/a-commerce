//! The mall's catalog content (`seed/mall/catalog_content.yml`): photos, laid over the generator's
//! rows the way Northwind's are (`crate::generate::content`).
//!
//! A photo replaces the drawn picture (`crate::media`): `image` for the product, and `images` for
//! the variants of each color it shows. Variants of other colors, and products the file does not
//! list, keep their drawing. Nothing else moves: no id, name, price, color or stock.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::SeedRows;

/// The keys a product entry may have. `image_provenance` records how its photos were made.
const KEYS: [&str; 5] = ["id", "name_check", "image", "images", "image_provenance"];

/// A picture URL the storefront can load: a path the web serves, or an https address.
fn picture_url<'a>(v: &'a Value, at: &str) -> Result<&'a str, String> {
    v.as_str()
        .filter(|u| u.starts_with('/') || u.starts_with("https://"))
        .filter(|u| !u.contains(char::is_whitespace))
        .ok_or_else(|| format!("{at}: not a picture URL (a /path or an https:// address)"))
}

/// Lays `yaml` (the text of `catalog_content.yml`) over `rows`, the mall's generated rows.
/// Returns how many products it changed.
pub(crate) fn overlay(rows: &mut SeedRows, yaml: &str) -> Result<usize, String> {
    let doc: Value =
        serde_norway::from_str(yaml).map_err(|e| format!("catalog_content.yml: {e}"))?;
    let products = doc
        .get("products")
        .and_then(Value::as_array)
        .ok_or("catalog_content.yml: no `products:` list")?;

    // What each listed product takes: its picture, and the picture of each color.
    let mut photos: BTreeMap<&str, (&str, BTreeMap<&str, &str>)> = BTreeMap::new();
    let mut names: BTreeMap<&str, &Value> = BTreeMap::new();
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
        let image = picture_url(
            p.get("image").unwrap_or(&Value::Null),
            &format!("{at} image"),
        )?;
        let mut by_color = BTreeMap::new();
        for (color, url) in p
            .get("images")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{at} images: not a map of color to picture"))?
        {
            by_color.insert(
                color.as_str(),
                picture_url(url, &format!("{at} images {color}"))?,
            );
        }
        if !by_color.values().any(|u| *u == image) {
            return Err(format!("{at}: image is not one of its color photos"));
        }
        if !p.get("image_provenance").is_some_and(Value::is_object) {
            return Err(format!("{at} image_provenance: not a map"));
        }
        if photos.insert(id, (image, by_color)).is_some() {
            return Err(format!("{at}: listed twice"));
        }
        names.insert(id, p.get("name_check").unwrap_or(&Value::Null));
    }

    let mut found = BTreeSet::new();
    let mut shown: BTreeSet<(&str, &str)> = BTreeSet::new();
    for (table, row) in rows.iter_mut() {
        match table.as_str() {
            "catalog.product_group" => {
                let Some((&id, (image, _))) = row
                    .get("id")
                    .and_then(Value::as_str)
                    .and_then(|id| photos.get_key_value(id))
                else {
                    continue;
                };
                if row.get("name") != Some(names[id]) {
                    return Err(format!(
                        "catalog_content.yml {id}: drawn for `{}`, but the product is now `{}`; draw the photo again",
                        names[id].as_str().unwrap_or("?"),
                        row.get("name").and_then(Value::as_str).unwrap_or("?")
                    ));
                }
                row["image"] = json!(image);
                found.insert(id);
            }
            "catalog.product" => {
                let Some((&id, (_, by_color))) = row
                    .get("product_group_id")
                    .and_then(Value::as_str)
                    .and_then(|id| photos.get_key_value(id))
                else {
                    continue;
                };
                let color = row.get("color").and_then(Value::as_str).unwrap_or_default();
                if let Some((&color, url)) = by_color.get_key_value(color) {
                    row["image"] = json!(url);
                    shown.insert((id, color));
                }
            }
            _ => {}
        }
    }
    for (id, (_, by_color)) in &photos {
        if !found.contains(id) {
            return Err(format!(
                "catalog_content.yml {id}: the generator made no such product"
            ));
        }
        if let Some(color) = by_color.keys().find(|c| !shown.contains(&(*id, **c))) {
            return Err(format!(
                "catalog_content.yml {id} images: the product has no color `{color}`"
            ));
        }
    }
    Ok(photos.len())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use serde_json::Value;

    use super::overlay;
    use crate::SeedRows;
    use crate::mall::{draw, named_store, rows, spec::Mall};

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// The generator's rows before any content is laid over them.
    fn drawn() -> &'static SeedRows {
        static ROWS: OnceLock<SeedRows> = OnceLock::new();
        ROWS.get_or_init(|| {
            let dir = repo().join("seed/mall");
            let mall = Mall::read(&dir, &repo().join("policies/taxonomy")).unwrap();
            let named = crate::named::load(&dir).unwrap();
            let store = named_store(&named, &mall.store.id).unwrap();
            let products = draw::products(&mall).unwrap();
            rows::rows(&mall, &store, &products)
        })
    }

    /// The first product with two colors or more: its id, name and first two colors.
    fn two_colored() -> (String, String, String, String) {
        let mut colors: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (t, r) in drawn() {
            if t == "catalog.product" {
                let seen = colors
                    .entry(r["product_group_id"].as_str().unwrap())
                    .or_default();
                let color = r["color"].as_str().unwrap();
                if !seen.contains(&color) {
                    seen.push(color);
                }
            }
        }
        let (id, c) = colors.iter().find(|(_, c)| c.len() >= 2).unwrap();
        let name = drawn()
            .iter()
            .find(|(t, r)| t == "catalog.product_group" && r["id"] == *id)
            .map(|(_, r)| r["name"].as_str().unwrap())
            .unwrap();
        (
            (*id).to_owned(),
            name.to_owned(),
            c[0].to_owned(),
            c[1].to_owned(),
        )
    }

    fn entry(id: &str, name: &str, color: &str) -> String {
        format!(
            "products:\n- id: {id}\n  name_check: {}\n  image: /media/mall/{id}-0a1b2c3d.webp\n  \
             images: {{{color}: /media/mall/{id}-0a1b2c3d.webp}}\n  \
             image_provenance: {{model: muse-image-1.0, run: r, response_id: resp_0}}\n",
            serde_json::to_string(name).unwrap()
        )
    }

    #[test]
    fn a_photo_replaces_the_drawing_of_its_color_only() {
        let (id, name, first, second) = two_colored();
        let photo = format!("/media/mall/{id}-0a1b2c3d.webp");
        let mut after = drawn().clone();
        assert_eq!(overlay(&mut after, &entry(&id, &name, &second)), Ok(1));
        for ((table, before), (_, now)) in drawn().iter().zip(&after) {
            let of_product = before["id"] == *id || before["product_group_id"] == *id;
            let photographed = table == "catalog.product_group" || before["color"] == *second;
            for (k, v) in before.as_object().unwrap() {
                if k == "image" && of_product && photographed {
                    assert_eq!(now[k], photo, "{table} {}", before["id"]);
                } else {
                    // Same rows, same order, same names, prices and stock; other colors keep their drawing.
                    assert_eq!(&now[k], v, "{table}.{k} changed: {now}");
                }
            }
        }
        assert!(after.iter().any(|(t, r)| t == "catalog.product"
            && r["product_group_id"] == *id
            && r["color"] == *first
            && r["image"].as_str().unwrap().starts_with("/media/garment/")));
    }

    #[test]
    fn photos_that_do_not_fit_are_refused() {
        let (id, name, color, _) = two_colored();
        let good = entry(&id, &name, &color);
        for (yaml, want) in [
            (
                good.replace(&id, "prd_nobody_999"),
                "the generator made no such product",
            ),
            (
                good.replace(&format!("{{{color}:"), "{No such color:"),
                "the product has no color `No such color`",
            ),
            (
                good.replace("name_check: ", "name_check: Another "),
                "draw the photo again",
            ),
            (
                good.replace("image: /media", "image: media"),
                "not a picture URL",
            ),
            (
                good.replace("images: {", "images: {Other: /media/mall/x.webp}\n  was: {"),
                "unknown key `was`",
            ),
            (
                good.replace("-0a1b2c3d.webp}", "-ffffffff.webp}"),
                "image is not one of its color photos",
            ),
            (
                good.replace("{model: muse-image-1.0", "muse #{model: muse-image-1.0"),
                "image_provenance: not a map",
            ),
            (
                format!("{good}{}", good.trim_start_matches("products:\n")),
                "listed twice",
            ),
        ] {
            let e = overlay(&mut drawn().clone(), &yaml).unwrap_err();
            assert!(e.contains(want), "{yaml}: {e}");
        }
    }

    /// Every photo the content file names is in `web/public` (the web serves it) under the
    /// product's own id, no photo is used for two colors, and no file is left that nothing names.
    #[test]
    fn photos_the_mall_names_are_served_by_the_web() {
        let Ok(text) = std::fs::read_to_string(repo().join("seed/mall/catalog_content.yml")) else {
            return;
        };
        let doc: Value = serde_norway::from_str(&text).unwrap();
        let public = repo().join("web/public");
        let mut named = BTreeSet::new();
        for p in doc["products"].as_array().unwrap() {
            let id = p["id"].as_str().unwrap();
            let by_color = p["images"].as_object().unwrap();
            let urls: BTreeSet<&str> = by_color.values().filter_map(Value::as_str).collect();
            assert_eq!(urls.len(), by_color.len(), "{id}: one photo for two colors");
            for url in urls {
                let file = url.strip_prefix("/media/mall/").unwrap_or_default();
                assert!(
                    file.starts_with(&format!("{id}-")),
                    "{id}: {url} is not its photo"
                );
                assert!(
                    public.join("media/mall").join(file).is_file(),
                    "{id}: {url} is not in web/public"
                );
                named.insert(file.to_owned());
            }
        }
        for f in std::fs::read_dir(public.join("media/mall"))
            .unwrap()
            .flatten()
        {
            let file = f.file_name().to_string_lossy().into_owned();
            assert!(
                named.contains(&file),
                "web/public/media/mall/{file}: nothing names it"
            );
        }
    }

    #[test]
    fn the_content_file_changes_pictures_only() {
        let Ok(text) = std::fs::read_to_string(repo().join("seed/mall/catalog_content.yml")) else {
            return;
        };
        let mut after = drawn().clone();
        let products = overlay(&mut after, &text).unwrap();
        let mut groups = 0;
        for ((table, before), (_, now)) in drawn().iter().zip(&after) {
            for (k, v) in before.as_object().unwrap() {
                if &now[k] != v {
                    assert_eq!(k, "image", "{table}.{k} changed: {now}");
                    groups += usize::from(table == "catalog.product_group");
                }
            }
        }
        assert_eq!(groups, products);
        assert_eq!(after.len(), drawn().len());
    }
}
