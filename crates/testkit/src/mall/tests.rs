use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::Value;

use super::draw::{self, Product, fit_of, materials_for};
use super::spec::{Gender, Mall};
use crate::SeedRows;
use crate::generate::schema_check;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct Seed {
    mall: Mall,
    products: Vec<Product>,
    rows: SeedRows,
}

fn seed() -> &'static Seed {
    static SEED: OnceLock<Seed> = OnceLock::new();
    SEED.get_or_init(|| {
        let dir = repo().join("seed/mall");
        let taxonomy = repo().join("policies/taxonomy");
        let mall = match Mall::read(&dir, &taxonomy) {
            Ok(m) => m,
            Err(e) => panic!("{e}"),
        };
        let products = match draw::products(&mall) {
            Ok(p) => p,
            Err(e) => panic!("{e}"),
        };
        let named = match crate::named::load(&dir) {
            Ok(r) => r,
            Err(e) => panic!("{e}"),
        };
        let mut rows = named.clone();
        match super::generate(&named, &dir, &taxonomy) {
            Ok(g) => rows.extend(g),
            Err(e) => panic!("{e}"),
        }
        Seed {
            mall,
            products,
            rows,
        }
    })
}

fn tier_of<'a>(s: &'a Seed, p: &Product) -> &'a super::spec::Tier {
    &s.mall.tiers[&s.mall.brands[p.brand].tier]
}

#[test]
fn thirty_brands_sell_five_thousand_products() {
    let s = seed();
    assert_eq!(s.mall.brands.len(), 30);
    assert_eq!(s.products.len(), 5000);
    for (i, b) in s.mall.brands.iter().enumerate() {
        let n = s.products.iter().filter(|p| p.brand == i).count();
        assert_eq!(n, b.products, "{}", b.id);
    }
    let ids: BTreeSet<&str> = s.products.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids.len(), 5000);
}

#[test]
fn a_brand_splits_its_products_by_its_focus() {
    // Maison Orvelle: 100 products, 28% outerwear.
    let s = seed();
    let b = &s.mall.brands[0];
    let counts = draw::counts(b);
    assert_eq!(counts.iter().map(|(_, n)| n).sum::<usize>(), b.products);
    for (c, pct) in &b.focus {
        let n = counts.iter().find(|(k, _)| k == c).map_or(0, |(_, n)| *n);
        let exact = b.products as f64 * *pct as f64 / 100.0;
        assert!(
            (n as f64 - exact).abs() < 1.0,
            "{} {c}: {n} for {exact}",
            b.id
        );
    }
}

#[test]
fn the_same_files_give_the_same_rows() {
    let dir = repo().join("seed/mall");
    let taxonomy = repo().join("policies/taxonomy");
    let named = crate::named::load(&dir).unwrap_or_default();
    let again = super::generate(&named, &dir, &taxonomy).unwrap_or_default();
    let first: SeedRows = seed().rows[named.len()..].to_vec();
    assert_eq!(first.len(), again.len());
    assert!(first == again, "a second run drew other rows");
}

#[test]
fn every_price_is_inside_its_tier_band_and_ends_as_the_tier_says() {
    let s = seed();
    for p in &s.products {
        let tier = tier_of(s, p);
        let (lo, hi) = tier.band(&s.mall.categories[p.category]);
        assert!(
            (lo..=hi).contains(&p.price),
            "{} {} costs {} outside {lo}..{hi}",
            p.id,
            p.name,
            p.price
        );
        assert!(
            tier.ending.fits(p.price),
            "{} ends wrong: {}",
            p.id,
            p.price
        );
        let share = p.cost * 10_000 / p.price;
        assert!(
            (tier.cost.0 - 1..=tier.cost.1 + 1).contains(&share),
            "{}: cost share {share}",
            p.id
        );
    }
}

#[test]
fn a_dearer_type_never_costs_less_in_its_brand() {
    let s = seed();
    let mut by: BTreeMap<(usize, usize, usize), (i64, i64)> = BTreeMap::new();
    for p in &s.products {
        let e = by
            .entry((p.brand, p.category, p.level))
            .or_insert((i64::MAX, i64::MIN));
        e.0 = e.0.min(p.price);
        e.1 = e.1.max(p.price);
    }
    for ((b, c, level), (_, hi)) in &by {
        for upper in level + 1..=3 {
            if let Some((lo, _)) = by.get(&(*b, *c, upper)) {
                assert!(
                    hi <= lo,
                    "{} {}: level {level} up to {hi}, level {upper} from {lo}",
                    s.mall.brands[*b].id,
                    s.mall.categories[*c].key
                );
            }
        }
    }
}

#[test]
fn dearer_tiers_ask_more_for_what_they_sell() {
    // A raw median per category compares a designer brand's liner gloves with a contemporary
    // brand's leather gloves. Each price is read against its category's mid range instead (1.0
    // is the middle of the mid tier's range). Spa and boutique bands overlap (x0.35-0.6 and
    // x0.3-0.55), so both are only compared with mid.
    let s = seed();
    let median = |tier: &str| {
        let mut index: Vec<i64> = s
            .products
            .iter()
            .filter(|p| s.mall.brands[p.brand].tier == tier)
            .map(|p| {
                let (lo, hi) = s.mall.categories[p.category].price;
                p.price * 20_000 / (lo + hi)
            })
            .collect();
        index.sort_unstable();
        index.get(index.len() / 2).copied().unwrap_or_default()
    };
    let order = ["luxury", "designer", "contemporary", "mid", "spa"];
    for w in order.windows(2) {
        let (a, b) = (median(w[0]), median(w[1]));
        assert!(a > b, "{} median {a} is not above {} {b}", w[0], w[1]);
    }
    assert!(median("mid") > median("boutique"));
}

#[test]
fn every_material_is_one_the_tier_may_use_for_that_type() {
    let s = seed();
    for p in &s.products {
        let c = &s.mall.categories[p.category];
        let allowed = materials_for(&s.mall, c, tier_of(s, p), &p.type_name);
        assert!(
            allowed.iter().any(|m| m.name == p.material),
            "{} {}: {} is not allowed",
            p.id,
            p.name,
            p.material
        );
    }
}

#[test]
fn a_brand_with_its_own_cloth_uses_only_that() {
    // Ironbridge Workwear sells canvas, twill and denim; it drew crepe pants before brands
    // could list their materials.
    let s = seed();
    for p in &s.products {
        let b = &s.mall.brands[p.brand];
        let Some(own) = &b.materials else {
            continue;
        };
        let c = &s.mall.categories[p.category];
        let some_fits = c.types.iter().any(|t| {
            materials_for(&s.mall, c, tier_of(s, p), &t.name)
                .iter()
                .any(|m| own.contains(&m.name))
        });
        if some_fits {
            assert!(
                own.contains(&p.material),
                "{} {}: {}",
                b.id,
                p.name,
                p.material
            );
        }
    }
    let ironbridge = s
        .products
        .iter()
        .filter(|p| s.mall.brands[p.brand].id == "brn_ironbridge");
    assert!(ironbridge.clone().all(|p| p.material != "Crepe"));
    assert!(ironbridge.count() > 0);
}

#[test]
fn a_name_word_suits_its_type_and_material() {
    // "Ribbed wool blend fedora" and "Fuzzy polyester satin scarf" read wrong.
    let s = seed();
    for p in &s.products {
        let Some(word) = &p.word else {
            continue;
        };
        let w = tier_of(s, p).words.iter().find(|w| &w.word == word);
        let m = &s.mall.materials[&p.material];
        let c = &s.mall.categories[p.category].key;
        assert!(w.is_some_and(|w| w.fits(c, &p.type_name, m)), "{}", p.name);
    }
}

#[test]
fn origins_colors_sizes_and_stock_follow_brand_and_tier() {
    let s = seed();
    for p in &s.products {
        let b = &s.mall.brands[p.brand];
        let tier = tier_of(s, p);
        let c = &s.mall.categories[p.category];
        assert!(
            b.origin.contains(&p.origin) && tier.origin.contains(&p.origin),
            "{}",
            p.id
        );
        assert!(!p.colors.is_empty(), "{} has no color", p.id);
        assert!(
            p.colors.len() as i64 <= tier.colors_per_product.1,
            "{}: {} colors",
            p.id,
            p.colors.len()
        );
        for col in &p.colors {
            assert!(
                b.colors.contains(col),
                "{}: {col} is not on {}'s palette",
                p.id,
                b.id
            );
            assert!(
                s.mall.colors[col].sold_in(&c.key),
                "{}: {col} not sold in {}",
                p.id,
                c.key
            );
        }
        assert_eq!(p.variants.len(), p.colors.len() * p.sizes.len(), "{}", p.id);
        for v in &p.variants {
            assert!((tier.stock.0..=tier.stock.1).contains(&v.stock), "{}", v.id);
        }
    }
}

#[test]
fn most_products_get_the_colors_their_tier_asks_for() {
    // A palette short of a tier's minimum in one category gives fewer colors; it stays rare.
    let s = seed();
    let short = s
        .products
        .iter()
        .filter(|p| (p.colors.len() as i64) < tier_of(s, p).colors_per_product.0)
        .count();
    assert!(
        short * 100 < s.products.len(),
        "{short} products have too few colors"
    );
}

#[test]
fn a_product_is_for_a_gender_its_brand_sells_to_and_its_type_allows() {
    let s = seed();
    for p in &s.products {
        let b = &s.mall.brands[p.brand];
        assert!(
            b.audience.iter().any(|(g, w)| *g == p.gender && *w > 0),
            "{} is {} but {} sells to {:?}",
            p.id,
            p.gender.word(),
            b.id,
            b.audience
        );
        let t = s.mall.categories[p.category]
            .types
            .iter()
            .find(|t| t.name == p.type_name);
        let t = t.map(|t| t.gender);
        assert!(
            t == Some(Gender::Unisex) || t == Some(p.gender),
            "{}: {} type for a {} product",
            p.id,
            p.type_name,
            p.gender.word()
        );
    }
}

#[test]
fn no_brand_sells_two_products_of_one_name() {
    let s = seed();
    let mut seen = BTreeSet::new();
    for p in &s.products {
        assert!(
            seen.insert((p.brand, p.name.clone())),
            "{} repeats {}",
            p.id,
            p.name
        );
    }
}

#[test]
fn names_read_as_one_sentence_without_repeats_or_clashing_fits() {
    let s = seed();
    for p in &s.products {
        assert!(p.name.starts_with(|c: char| c.is_uppercase()), "{}", p.name);
        let words: Vec<String> = p
            .name
            .to_lowercase()
            .split(' ')
            .map(str::to_owned)
            .collect();
        for w in words.windows(2) {
            assert_ne!(w[0], w[1], "{}", p.name);
        }
        if let Some(word) = &p.word {
            let a = fit_of(&s.mall, &p.type_name);
            let b = fit_of(&s.mall, word);
            assert!(a.is_none() || b.is_none() || a == b, "{} clashes", p.name);
            if p.gender != Gender::Female {
                assert!(
                    !tier_of(s, p).women_only.contains(word),
                    "{} is not for women",
                    p.name
                );
            }
        }
        if let Some(fit) = &p.fit
            && let Some(named) = fit_of(&s.mall, &p.name)
        {
            assert_eq!(fit, &named, "{} has fit {fit}", p.name);
        }
    }
}

#[test]
fn every_product_has_the_attributes_its_taxonomy_node_needs() {
    let s = seed();
    let names = |v: &Value| -> BTreeSet<String> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.get("name").and_then(Value::as_str).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut variant_props: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    for (t, r) in &s.rows {
        if t == "catalog.product" {
            let g = r["product_group_id"].as_str().unwrap_or_default();
            let n = names(&r["additional_property"]);
            variant_props
                .entry(g)
                .and_modify(|x| *x = x.intersection(&n).cloned().collect())
                .or_insert(n);
        }
    }
    let by_id: BTreeMap<&str, &Product> = s.products.iter().map(|p| (p.id.as_str(), p)).collect();
    for (t, r) in &s.rows {
        if t != "catalog.product_group" {
            continue;
        }
        let id = r["id"].as_str().unwrap_or_default();
        let p = by_id[id];
        let mut have = names(&r["additional_property"]);
        have.extend(variant_props.get(id).cloned().unwrap_or_default());
        let needed = &s.mall.categories[p.category].required;
        let missing: Vec<&String> = needed.difference(&have).collect();
        assert!(missing.is_empty(), "{id} {} lacks {missing:?}", p.name);
    }
}

#[test]
fn heels_and_flats_carry_a_heel_height_in_their_type_range() {
    let s = seed();
    let heels = s.mall.category("heels_flats").map(|c| &c.heel_mm);
    for p in s
        .products
        .iter()
        .filter(|p| s.mall.categories[p.category].key == "heels_flats")
    {
        let (lo, hi) = heels
            .and_then(|h| h.get(&p.type_name))
            .copied()
            .unwrap_or((0, 0));
        let mm = p.heel_mm.unwrap_or(-1);
        assert!((lo..=hi).contains(&mm) && mm % 5 == 0, "{} {mm}", p.name);
    }
}

#[test]
fn the_rows_fit_the_migrations() {
    let tables = schema_check::parse(&repo().join("migrations"));
    let problems = schema_check::check(&tables, &seed().rows);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn prices_end_where_the_tier_says() {
    // The nearest price with the ending, kept inside the band.
    use super::spec::Ending;
    let nines = Ending {
        step: 1000,
        rest: 900,
    };
    assert_eq!(draw::ended(7_640, nines, (0, 100_000)), 7_900);
    assert_eq!(draw::ended(7_340, nines, (0, 100_000)), 6_900);
    assert_eq!(draw::ended(10_100, nines, (0, 10_000)), 9_900);
    let ninety_cents = Ending {
        step: 100,
        rest: 90,
    };
    assert_eq!(draw::ended(2_461, ninety_cents, (0, 100_000)), 2_490);
}
