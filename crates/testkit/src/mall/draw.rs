//! Drawing the mall's products. Each brand splits its products over its categories by its
//! focus, draws one price factor inside its tier's range, then draws every product in turn:
//! gender from its audience, a type its tier can make, a material its tier allows, a name word,
//! the price in the type's slice of the category range, colors from its palette, sizes and stock.

use std::collections::BTreeSet;

use chrono::{Days, NaiveDate};

use super::spec::{Brand, Category, Class, Ending, GarmentType, Gender, Mall, Material, Tier};
use crate::generate::util::{Dice, round_div};

/// Size systems the tier rules switch between (`assortment.yml` `size_systems`).
const ALPHA: &str = "alpha";
const ALPHA_WIDE: &str = "alpha_wide";
const ONE_SIZE: &str = "one_size";

/// How often a product is drawn again when its name is taken in its brand.
const ATTEMPTS: usize = 200;

#[derive(Debug, Clone)]
pub(crate) struct Product {
    pub id: String,
    /// Index into `Mall::brands`.
    pub brand: usize,
    /// Index into `Mall::categories`.
    pub category: usize,
    pub type_name: String,
    pub gender: Gender,
    /// The type's price level; the tests check that a dearer level never costs less.
    #[cfg_attr(not(test), allow(dead_code))]
    pub level: usize,
    pub material: String,
    /// The name word, if any; the tests check that it suits the type and material.
    #[cfg_attr(not(test), allow(dead_code))]
    pub word: Option<String>,
    pub name: String,
    /// Cents.
    pub price: i64,
    pub cost: i64,
    pub origin: String,
    pub fit: Option<String>,
    pub heel_mm: Option<i64>,
    pub released: NaiveDate,
    pub size_system: String,
    pub colors: Vec<String>,
    pub sizes: Vec<String>,
    pub variants: Vec<Variant>,
}

#[derive(Debug, Clone)]
pub(crate) struct Variant {
    pub id: String,
    pub offer_id: String,
    pub sku: String,
    pub name: String,
    pub color: String,
    pub size: String,
    pub stock: i64,
}

/// A brand's own seed: FNV-1a over its id, from the store's seed.
fn own_seed(seed: u64, id: &str) -> u64 {
    id.bytes().fold(seed ^ 0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// `Navy stripe` -> `navy_stripe`, `S/M` -> `s_m`: lowercase, every run of other characters one
/// `sep`.
fn slug(text: &str, sep: char) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with(sep) {
            out.push(sep);
        }
    }
    out.trim_end_matches(sep).to_owned()
}

/// Products per category: the brand's focus shares of its products, rounded so they add up
/// (largest remainder; ties go to the category written first).
pub(crate) fn counts(brand: &Brand) -> Vec<(String, usize)> {
    let n = brand.products as u64;
    let mut out: Vec<(String, usize, u64)> = brand
        .focus
        .iter()
        .map(|(c, pct)| (c.clone(), (n * pct / 100) as usize, n * pct % 100))
        .collect();
    let given: usize = out.iter().map(|(_, k, _)| k).sum();
    let mut order: Vec<usize> = (0..out.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(out[i].2));
    for &i in order.iter().take(brand.products.saturating_sub(given)) {
        out[i].1 += 1;
    }
    out.into_iter().map(|(c, k, _)| (c, k)).collect()
}

/// The material word a type names itself (`leather` in "leather jacket"), if any.
pub(crate) fn material_word<'a>(mall: &'a Mall, type_name: &str) -> Option<&'a str> {
    mall.material_words
        .iter()
        .map(String::as_str)
        .find(|w| type_name.split(' ').any(|t| t == *w))
}

/// The materials `tier` may make `type_name` of in `category`: the type's own list, or the
/// category's materials (only those with the type's material word, when it names one).
pub(crate) fn materials_for<'a>(
    mall: &'a Mall,
    category: &Category,
    tier: &Tier,
    type_name: &str,
) -> Vec<&'a Material> {
    let base: Vec<&Material> = match category.type_materials.get(type_name) {
        Some(list) => list.iter().filter_map(|m| mall.materials.get(m)).collect(),
        None => {
            let word = material_word(mall, type_name);
            mall.materials
                .values()
                .filter(|m| m.fits.contains(&category.key))
                .filter(|m| word.is_none_or(|w| m.name.to_lowercase().split(' ').any(|t| t == w)))
                .collect()
        }
    };
    base.into_iter()
        .filter(|m| tier.allows(m, &category.key))
        .collect()
}

/// The fit the words of `text` give (`assortment.yml` `fit.words`), if any.
pub(crate) fn fit_of(mall: &Mall, text: &str) -> Option<String> {
    let padded = format!(" {} ", text.to_lowercase());
    mall.fit_words
        .iter()
        .find(|(_, words)| words.iter().any(|w| padded.contains(&format!(" {w} "))))
        .map(|(fit, _)| fit.clone())
}

/// Whether a name word and a type or material share a part ("lace-trim" and "lace blouse",
/// "silk-lined" and Silk): the name would say it twice.
fn shares_part(word: &str, text: &str) -> bool {
    let parts = |s: &str| -> Vec<String> {
        s.to_lowercase()
            .split([' ', '-'])
            .filter(|x| !x.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let text = parts(text);
    parts(word).iter().any(|w| text.contains(w))
}

/// A product of `gender` may be of this type: unisex types are sold to everyone, women's and
/// men's types only to their own.
fn sold_to(t: &GarmentType, gender: Gender) -> bool {
    t.gender == Gender::Unisex || t.gender == gender
}

/// What a product is before its price: its type, gender, material and name word.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pick<'a> {
    pub t: &'a GarmentType,
    pub gender: Gender,
    pub material: &'a Material,
    pub word: Option<&'a str>,
}

/// The product name from the tier's template. With `with_material`, a template without the
/// material gets it before the type (a second try when the plain name is taken).
pub(crate) fn compose(
    mall: &Mall,
    tier: &Tier,
    category: &Category,
    pick: Pick<'_>,
    with_material: bool,
) -> String {
    let Pick {
        t,
        gender,
        material,
        word,
    } = pick;
    let lower = material.name.to_lowercase();
    // A type that names its material takes the material's full name in that word's place.
    let (type_text, named) = match material_word(mall, &t.name) {
        Some(w) => (
            t.name
                .split(' ')
                .map(|x| if x == w { lower.as_str() } else { x })
                .collect::<Vec<_>>()
                .join(" "),
            true,
        ),
        None => (t.name.clone(), false),
    };
    let plain = category.plain.as_deref() == Some(material.name.as_str());
    let mut template = tier.name.clone();
    if with_material && !template.contains("{material}") {
        template = template.replace("{type}", "{material} {type}");
    }
    let filled = template
        .replace("{word}", &word.map(str::to_lowercase).unwrap_or_default())
        .replace("{material}", if named || plain { "" } else { &lower })
        .replace("{type}", &type_text);
    let mut name = filled.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.gender == Gender::Unisex {
        match gender {
            Gender::Female => name = format!("women's {name}"),
            Gender::Male => name = format!("men's {name}"),
            Gender::Unisex => {}
        }
    }
    let mut chars = name.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The nearest price with the tier's ending, kept inside the tier's band.
pub(crate) fn ended(raw: i64, ending: Ending, band: (i64, i64)) -> i64 {
    let Ending { step, rest } = ending;
    let mut p = if raw <= rest {
        rest
    } else {
        round_div(raw - rest, step) * step + rest
    };
    while p > band.1 && p - step >= band.0 {
        p -= step;
    }
    while p < band.0 {
        p += step;
    }
    p
}

/// The mid price of a type: uniform in its level's third of the category range.
fn mid_price(category: &Category, level: usize, dice: &mut Dice) -> i64 {
    let (lo, hi) = category.price;
    let span = hi - lo;
    let level = level as i64;
    dice.range(lo + span * (level - 1) / 3, lo + span * level / 3)
}

pub(crate) fn products(mall: &Mall) -> Result<Vec<Product>, String> {
    let mut out = Vec::with_capacity(mall.store.products);
    for (bi, brand) in mall.brands.iter().enumerate() {
        let tier = mall
            .tiers
            .get(&brand.tier)
            .ok_or_else(|| format!("{}: no tier `{}`", brand.id, brand.tier))?;
        let mut dice = Dice::new(own_seed(mall.store.seed, &brand.id));
        // One price position per brand, so a dearer type never costs less than a cheaper one.
        let factor = dice.range(tier.factor.0, tier.factor.1);
        let mut names = BTreeSet::new();
        let mut number = 0;
        for (key, count) in counts(brand) {
            let ci = mall
                .categories
                .iter()
                .position(|c| c.key == key)
                .ok_or_else(|| format!("{}: no category `{key}`", brand.id))?;
            for _ in 0..count {
                number += 1;
                let mut draw = Draw {
                    mall,
                    brand,
                    bi,
                    tier,
                    ci,
                    factor,
                    dice: &mut dice,
                };
                out.push(draw.one(number, &mut names)?);
            }
        }
    }
    Ok(out)
}

struct Draw<'a> {
    mall: &'a Mall,
    brand: &'a Brand,
    bi: usize,
    tier: &'a Tier,
    ci: usize,
    factor: i64,
    dice: &'a mut Dice,
}

impl Draw<'_> {
    fn one(&mut self, number: usize, names: &mut BTreeSet<String>) -> Result<Product, String> {
        let (mall, brand, tier) = (self.mall, self.brand, self.tier);
        let category = &mall.categories[self.ci];
        let by_tier: Vec<(&GarmentType, Vec<&Material>)> = category
            .types
            .iter()
            .map(|t| (t, materials_for(mall, category, tier, &t.name)))
            .filter(|(_, ms)| !ms.is_empty())
            .collect();
        // A brand with its own cloth uses only that, in every category where some of it fits.
        let by_brand: Vec<(&GarmentType, Vec<&Material>)> = match &brand.materials {
            Some(own) => by_tier
                .iter()
                .map(|(t, ms)| {
                    let ms: Vec<&Material> = ms
                        .iter()
                        .copied()
                        .filter(|m| own.contains(&m.name))
                        .collect();
                    (*t, ms)
                })
                .filter(|(_, ms)| !ms.is_empty())
                .collect(),
            None => Vec::new(),
        };
        let makeable = if by_brand.is_empty() {
            by_tier
        } else {
            by_brand
        };
        let genders: Vec<(Gender, u64)> = brand
            .audience
            .iter()
            .copied()
            .filter(|(g, w)| *w > 0 && makeable.iter().any(|(t, _)| sold_to(t, *g)))
            .collect();
        if genders.is_empty() {
            return Err(format!(
                "{}: its tier makes nothing in `{}` for its audience",
                brand.id, category.key
            ));
        }
        let gender_weights: Vec<u64> = genders.iter().map(|(_, w)| *w).collect();
        for _ in 0..ATTEMPTS {
            let gender = genders[self.dice.weighted(&gender_weights)].0;
            let choices: Vec<&(&GarmentType, Vec<&Material>)> = makeable
                .iter()
                .filter(|(t, _)| sold_to(t, gender))
                .collect();
            let level_weights: Vec<u64> = choices
                .iter()
                .map(|(t, _)| tier.type_levels[t.level - 1].max(1))
                .collect();
            let (t, materials) = choices[self.dice.weighted(&level_weights)];
            let classes: Vec<(Class, u64)> = tier
                .classes
                .iter()
                .copied()
                .filter(|(c, w)| *w > 0 && materials.iter().any(|m| m.class == *c))
                .collect();
            let class_weights: Vec<u64> = classes.iter().map(|(_, w)| *w).collect();
            let class = classes[self.dice.weighted(&class_weights)].0;
            let in_class: Vec<&Material> = materials
                .iter()
                .copied()
                .filter(|m| m.class == class)
                .collect();
            let material = *self.dice.pick(&in_class);

            let type_fit = fit_of(mall, &t.name);
            let words: Vec<&str> = tier
                .words
                .iter()
                .filter(|w| {
                    w.fits(&category.key, &t.name, material)
                        && (gender == Gender::Female || !tier.women_only.contains(&w.word))
                        && !shares_part(&w.word, &t.name)
                        && !shares_part(&w.word, &material.name)
                        && match (&type_fit, fit_of(mall, &w.word)) {
                            (Some(a), Some(b)) => *a == b,
                            _ => true,
                        }
                })
                .map(|w| w.word.as_str())
                .collect();
            // No word is one more choice: "Nylon bikini top" reads as well as "Nylon ribbed
            // bikini top", and a brand with many products of few types needs both.
            let word = match self.dice.below(words.len() + 1) {
                i if i < words.len() => Some(words[i]),
                _ => None,
            };

            let pick = Pick {
                t,
                gender,
                material,
                word,
            };
            let mut name = compose(mall, tier, category, pick, false);
            if names.contains(&name) {
                name = compose(mall, tier, category, pick, true);
            }
            if !names.insert(name.clone()) {
                continue;
            }
            return self.rest(number, pick, name);
        }
        Err(format!(
            "{}: no new name in `{}` after {ATTEMPTS} draws",
            brand.id, category.key
        ))
    }

    /// Everything after the name: price, cost, origin, fit, heel, release, colors, sizes, stock.
    fn rest(&mut self, number: usize, pick: Pick<'_>, name: String) -> Result<Product, String> {
        let Pick {
            t,
            gender,
            material,
            word,
        } = pick;
        let (mall, brand, tier) = (self.mall, self.brand, self.tier);
        let category = &mall.categories[self.ci];
        let dice = &mut *self.dice;

        let raw = mid_price(category, t.level, dice) * self.factor / 10_000;
        let price = ended(raw, tier.ending, tier.band(category));
        let cost = round_div(price * dice.range(tier.cost.0, tier.cost.1), 10_000);
        let origin = dice.pick(&brand.origin).clone();

        let fit = category.required.contains("fit").then(|| {
            fit_of(mall, &t.name)
                .or_else(|| word.and_then(|w| fit_of(mall, w)))
                .unwrap_or_else(|| {
                    let weights: Vec<u64> = mall.fit_drawn.iter().map(|(_, w)| *w).collect();
                    mall.fit_drawn[dice.weighted(&weights)].0.clone()
                })
        });
        let heel_mm = category
            .heel_mm
            .get(&t.name)
            .map(|(lo, hi)| (dice.range(*lo, *hi) + 2) / 5 * 5);
        let (from, through) = mall.store.released;
        let days = (through - from).num_days().max(0);
        let released = from + Days::new(dice.range(0, days) as u64);

        let mut usable: Vec<String> = brand
            .colors
            .iter()
            .filter(|c| {
                mall.colors
                    .get(*c)
                    .is_some_and(|x| x.sold_in(&category.key))
            })
            .cloned()
            .collect();
        let wanted = dice.range(tier.colors_per_product.0, tier.colors_per_product.1) as usize;
        dice.shuffle(&mut usable);
        usable.truncate(wanted.max(1));
        let colors = usable;

        let mut system = category
            .sizes
            .get(&gender)
            .cloned()
            .ok_or_else(|| format!("{}: no sizes for {}", category.key, gender.word()))?;
        if category.alpha_types.contains(&t.name) {
            system = ALPHA.to_owned();
        }
        if let Some((women, cats)) = &tier.women_sizes
            && gender == Gender::Female
            && system == ALPHA
            && cats.contains(&category.key)
        {
            system = women.clone();
        }
        if tier.sizes_wide && system == ALPHA {
            system = ALPHA_WIDE.to_owned();
        }
        if let Some((share, cats)) = &tier.one_size
            && cats.contains(&category.key)
            && (dice.below(10_000) as u64) < *share
        {
            system = ONE_SIZE.to_owned();
        }
        let sizes = mall
            .size_systems
            .get(&system)
            .cloned()
            .ok_or_else(|| format!("no size system `{system}`"))?;

        let id = format!("prd_{}_{number:03}", brand.key);
        let stem = format!("{}_{number:03}", brand.key);
        let sku_stem = format!("{}-{number:03}", slug(&brand.key, '-'));
        let mut variants = Vec::new();
        for color in &colors {
            for size in &sizes {
                let tail = format!("{}_{}", slug(color, '_'), slug(size, '_'));
                variants.push(Variant {
                    id: format!("var_{stem}_{tail}"),
                    offer_id: format!("off_{stem}_{tail}"),
                    sku: format!("{sku_stem}-{}-{}", slug(color, '-'), slug(size, '-')),
                    name: format!("{name}, {color}, {size}"),
                    color: color.clone(),
                    size: size.clone(),
                    stock: dice.range(tier.stock.0, tier.stock.1),
                });
            }
        }
        Ok(Product {
            id,
            brand: self.bi,
            category: self.ci,
            type_name: t.name.clone(),
            gender,
            level: t.level,
            material: material.name.clone(),
            word: word.map(str::to_owned),
            name,
            price,
            cost,
            origin,
            fit,
            heel_mm,
            released,
            size_system: system,
            colors,
            sizes,
            variants,
        })
    }
}
