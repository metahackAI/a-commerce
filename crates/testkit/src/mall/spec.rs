//! `seed/mall/assortment.yml` and `brands.yml`, with the taxonomy files they name
//! (`policies/taxonomy/colors.yml`, `nodes.yml`, `apparel.yml`), read into typed parts. Every
//! name one file uses from another (a material in a type list, a category in a brand's focus, a
//! color on a palette) is checked here, so a drawn product never meets an unknown word.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use chrono::NaiveDate;
use serde_json::{Map, Value};

use crate::generate::util::{basis_points, parse_date};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Gender {
    Female,
    Male,
    Unisex,
}

impl Gender {
    /// The `target_gender` word.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Gender::Female => "female",
            Gender::Male => "male",
            Gender::Unisex => "unisex",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "female" => Some(Gender::Female),
            "male" => Some(Gender::Male),
            "unisex" => Some(Gender::Unisex),
            _ => None,
        }
    }
}

/// A material's class, which the tiers allow by share.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Class {
    Premium,
    Standard,
    Value,
}

impl Class {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "premium" => Some(Class::Premium),
            "standard" => Some(Class::Standard),
            "value" => Some(Class::Value),
            _ => None,
        }
    }
}

/// How a tier's prices end: a step and the remainder every price has (`nines` is $79: step
/// $10, remainder $9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Ending {
    pub step: i64,
    pub rest: i64,
}

impl Ending {
    fn parse(s: &str) -> Option<Self> {
        let (step, rest) = match s {
            "tens" => (1000, 0),
            "fives" => (500, 0),
            "eights" => (1000, 800),
            "nines" => (1000, 900),
            "ninety_cents" => (100, 90),
            "whole" => (100, 0),
            _ => return None,
        };
        Some(Ending { step, rest })
    }

    /// Whether a price in cents ends this way.
    #[cfg(test)]
    pub(crate) fn fits(self, cents: i64) -> bool {
        cents % self.step == self.rest
    }
}

#[derive(Debug)]
pub(crate) struct Mall {
    pub store: Store,
    /// ISO 3166 code -> country name, for the one-line description.
    pub countries: BTreeMap<String, String>,
    pub departments: Vec<Department>,
    /// (fit word, the words of a name that give it), in the order written.
    pub fit_words: Vec<(String, Vec<String>)>,
    /// (fit word, weight) for a name whose words give none.
    pub fit_drawn: Vec<(String, u64)>,
    pub material_words: Vec<String>,
    pub size_systems: BTreeMap<String, Vec<String>>,
    /// Size system -> size -> foot length in millimetres.
    pub foot_length: BTreeMap<String, BTreeMap<String, i64>>,
    pub materials: BTreeMap<String, Material>,
    pub categories: Vec<Category>,
    pub tiers: BTreeMap<String, Tier>,
    pub brands: Vec<Brand>,
    pub colors: BTreeMap<String, Color>,
}

#[derive(Debug)]
pub(crate) struct Store {
    pub id: String,
    pub products: usize,
    pub seed: u64,
    pub released: (NaiveDate, NaiveDate),
    pub safety_stock: i64,
    pub return_policy: String,
}

#[derive(Debug)]
pub(crate) struct Department {
    pub key: String,
    pub name: String,
    pub categories: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct Material {
    pub name: String,
    pub class: Class,
    pub family: String,
    pub care: String,
    pub thickness: String,
    pub fabric: Option<String>,
    pub fits: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct GarmentType {
    pub name: String,
    pub gender: Gender,
    /// 1, 2 or 3: the slice of the category's price range.
    pub level: usize,
}

#[derive(Debug)]
pub(crate) struct Category {
    pub key: String,
    pub name: String,
    /// The mid tier's range in cents.
    pub price: (i64, i64),
    /// Size system per gender.
    pub sizes: BTreeMap<Gender, String>,
    pub types: Vec<GarmentType>,
    pub type_materials: BTreeMap<String, Vec<String>>,
    pub alpha_types: BTreeSet<String>,
    pub plain: Option<String>,
    pub fabric: Option<String>,
    pub warmth: BTreeMap<String, String>,
    pub water: BTreeMap<String, String>,
    pub heel_mm: BTreeMap<String, (i64, i64)>,
    /// The attributes its taxonomy node needs, its parents' included.
    pub required: BTreeSet<String>,
}

#[derive(Debug)]
pub(crate) struct Tier {
    /// The price factor's range in basis points (×7.0 is 70,000).
    pub factor: (i64, i64),
    pub ending: Ending,
    /// Cost as a share of the price, in basis points.
    pub cost: (i64, i64),
    /// Weight of type levels 1, 2 and 3.
    pub type_levels: [u64; 3],
    /// Weight of each class the tier uses.
    pub classes: Vec<(Class, u64)>,
    pub value_only_in: Option<BTreeSet<String>>,
    pub premium_allowed: Option<BTreeSet<String>>,
    pub origin: Vec<String>,
    pub colors_per_product: (i64, i64),
    pub stock: (i64, i64),
    /// The size system women's products take in these categories, instead of alpha.
    pub women_sizes: Option<(String, BTreeSet<String>)>,
    pub sizes_wide: bool,
    /// Share in basis points of products sold as One size in these categories.
    pub one_size: Option<(u64, BTreeSet<String>)>,
    pub name: String,
    /// In the order written.
    pub words: Vec<Word>,
    pub women_only: BTreeSet<String>,
}

/// A word a tier's names may carry.
#[derive(Debug)]
pub(crate) struct Word {
    pub word: String,
    pub categories: BTreeSet<String>,
    pub types: BTreeSet<String>,
    /// Material names or families it suits; any when none are written.
    pub materials: Option<BTreeSet<String>>,
}

impl Word {
    pub(crate) fn fits(&self, category: &str, type_name: &str, material: &Material) -> bool {
        (self.categories.contains(category) || self.types.contains(type_name))
            && self
                .materials
                .as_ref()
                .is_none_or(|m| m.contains(&material.name) || m.contains(&material.family))
    }
}

impl Tier {
    /// Whether this tier makes products of `material` in `category`.
    pub(crate) fn allows(&self, material: &Material, category: &str) -> bool {
        let share = self
            .classes
            .iter()
            .find(|(c, _)| *c == material.class)
            .map_or(0, |(_, w)| *w);
        if share == 0 {
            return false;
        }
        if material.class == Class::Value
            && self
                .value_only_in
                .as_ref()
                .is_some_and(|only| !only.contains(category))
        {
            return false;
        }
        if material.class == Class::Premium
            && self
                .premium_allowed
                .as_ref()
                .is_some_and(|only| !only.contains(&material.name))
        {
            return false;
        }
        true
    }

    /// The lowest and highest price this tier may ask in `category`, in cents.
    pub(crate) fn band(&self, category: &Category) -> (i64, i64) {
        (
            category.price.0 * self.factor.0 / 10_000,
            category.price.1 * self.factor.1 / 10_000,
        )
    }
}

#[derive(Debug)]
pub(crate) struct Brand {
    pub id: String,
    /// The id without `brn_`, used in product ids and SKUs.
    pub key: String,
    pub name: String,
    pub tier: String,
    pub products: usize,
    pub identity: String,
    pub audience: Vec<(Gender, u64)>,
    /// (category, percent of the brand's products), in the order written.
    pub focus: Vec<(String, u64)>,
    /// Palette colors, then extra colors, without repeats.
    pub colors: Vec<String>,
    pub origin: Vec<String>,
    /// Its own cloth: the only materials it uses, where one of them fits.
    pub materials: Option<BTreeSet<String>>,
}

#[derive(Debug)]
pub(crate) struct Color {
    pub families: Vec<String>,
    pub pattern: Option<String>,
    pub only_for: Option<BTreeSet<String>>,
}

impl Color {
    pub(crate) fn sold_in(&self, category: &str) -> bool {
        self.only_for
            .as_ref()
            .is_none_or(|only| only.contains(category))
    }
}

fn read(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_norway::from_str(&text).map_err(|e| format!("{}: not valid YAML: {e}", path.display()))
}

fn get<'a>(v: &'a Value, key: &str, at: &str) -> Result<&'a Value, String> {
    v.get(key).ok_or_else(|| format!("{at}.{key} is missing"))
}

fn map<'a>(v: &'a Value, at: &str) -> Result<&'a Map<String, Value>, String> {
    v.as_object().ok_or_else(|| format!("{at} is not a map"))
}

fn text(v: &Value, at: &str) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        _ => Err(format!("{at} is not text")),
    }
}

fn texts(v: &Value, at: &str) -> Result<Vec<String>, String> {
    v.as_array()
        .ok_or_else(|| format!("{at} is not a list"))?
        .iter()
        .enumerate()
        .map(|(i, x)| text(x, &format!("{at}[{i}]")))
        .collect()
}

fn int(v: &Value, at: &str) -> Result<i64, String> {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .ok_or_else(|| format!("{at} is not a whole number"))
}

fn share(v: &Value, at: &str) -> Result<u64, String> {
    v.as_f64()
        .filter(|x| *x >= 0.0)
        .map(|x| basis_points(x) as u64)
        .ok_or_else(|| format!("{at} is not a share"))
}

fn pair<T>(v: &Value, at: &str, one: impl Fn(&Value) -> Option<T>) -> Result<(T, T), String>
where
    T: PartialOrd,
{
    match v.as_array().map(Vec::as_slice) {
        Some([a, b]) => match (one(a), one(b)) {
            (Some(a), Some(b)) if a <= b => Ok((a, b)),
            _ => Err(format!("{at} is not [low, high]")),
        },
        _ => Err(format!("{at} is not [low, high]")),
    }
}

fn int_pair(v: &Value, at: &str) -> Result<(i64, i64), String> {
    pair(v, at, Value::as_i64)
}

fn bp_pair(v: &Value, at: &str) -> Result<(i64, i64), String> {
    pair(v, at, |x| x.as_f64().map(basis_points))
}

fn set(v: Option<&Value>, at: &str) -> Result<BTreeSet<String>, String> {
    v.map_or(Ok(BTreeSet::new()), |v| {
        texts(v, at).map(|t| t.into_iter().collect())
    })
}

/// The type -> value maps written by value (`warm: [parka, peacoat]`).
fn by_value(v: Option<&Value>, at: &str) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    if let Some(v) = v {
        for (value, types) in map(v, at)? {
            for t in texts(types, &format!("{at}.{value}"))? {
                if out.insert(t.clone(), value.clone()).is_some() {
                    return Err(format!("{at}: `{t}` has two values"));
                }
            }
        }
    }
    Ok(out)
}

impl Mall {
    pub(crate) fn read(dir: &Path, taxonomy: &Path) -> Result<Mall, String> {
        let a = read(&dir.join("assortment.yml"))?;
        let b = read(&dir.join("brands.yml"))?;
        let colors = colors(&read(&taxonomy.join("colors.yml"))?)?;
        let nodes = read(&taxonomy.join("nodes.yml"))?;
        let apparel = read(&taxonomy.join("apparel.yml"))?;

        let s = get(&a, "store", "assortment.yml")?;
        let at = "assortment.yml store";
        let released = get(s, "released", at)?;
        let released = (
            parse_date(&text(&released[0], at)?)?,
            parse_date(&text(&released[1], at)?)?,
        );
        let store = Store {
            id: text(get(s, "id", at)?, at)?,
            products: int(get(s, "products", at)?, at)? as usize,
            seed: int(get(s, "random_seed", at)?, at)? as u64,
            released,
            safety_stock: int(get(s, "safety_stock", at)?, at)?,
            return_policy: text(get(s, "return_policy", at)?, at)?,
        };

        let countries = map(get(&a, "countries", "assortment.yml")?, "countries")?
            .iter()
            .map(|(k, v)| Ok((k.clone(), text(v, &format!("countries.{k}"))?)))
            .collect::<Result<BTreeMap<_, _>, String>>()?;

        let mut departments = Vec::new();
        for (key, d) in map(get(&a, "departments", "assortment.yml")?, "departments")? {
            let at = format!("departments.{key}");
            departments.push(Department {
                key: key.clone(),
                name: text(get(d, "name", &at)?, &at)?,
                categories: texts(get(d, "categories", &at)?, &at)?,
            });
        }

        let fit = get(&a, "fit", "assortment.yml")?;
        let fit_words = map(get(fit, "words", "fit")?, "fit.words")?
            .iter()
            .map(|(k, v)| Ok((k.clone(), texts(v, &format!("fit.words.{k}"))?)))
            .collect::<Result<Vec<_>, String>>()?;
        let fit_drawn = map(get(fit, "drawn", "fit")?, "fit.drawn")?
            .iter()
            .map(|(k, v)| Ok((k.clone(), share(v, &format!("fit.drawn.{k}"))?)))
            .collect::<Result<Vec<_>, String>>()?;
        let material_words = texts(
            get(&a, "material_words", "assortment.yml")?,
            "material_words",
        )?;

        let size_systems = map(get(&a, "size_systems", "assortment.yml")?, "size_systems")?
            .iter()
            .map(|(k, v)| Ok((k.clone(), texts(v, &format!("size_systems.{k}"))?)))
            .collect::<Result<BTreeMap<_, _>, String>>()?;

        let tables = apparel["attributes"]["foot_length"]["from_size"]
            .as_object()
            .ok_or("apparel.yml: attributes.foot_length.from_size is not a map")?;
        let mut foot_length = BTreeMap::new();
        for (system, table) in map(
            get(&a, "foot_length_from", "assortment.yml")?,
            "foot_length_from",
        )? {
            let table = text(table, &format!("foot_length_from.{system}"))?;
            let sizes = tables
                .get(&table)
                .and_then(Value::as_object)
                .ok_or_else(|| format!("apparel.yml: no foot_length table `{table}`"))?;
            let mm = sizes
                .iter()
                .map(|(size, mm)| {
                    Ok((
                        size.clone(),
                        int(mm, &format!("foot_length.{table}.{size}"))?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, String>>()?;
            foot_length.insert(system.clone(), mm);
        }

        let mut materials = BTreeMap::new();
        for (name, m) in map(get(&a, "materials", "assortment.yml")?, "materials")? {
            let at = format!("materials.{name}");
            let class = text(get(m, "class", &at)?, &at)?;
            materials.insert(
                name.clone(),
                Material {
                    name: name.clone(),
                    class: Class::parse(&class).ok_or_else(|| format!("{at}: class `{class}`"))?,
                    family: text(get(m, "family", &at)?, &at)?,
                    care: text(get(m, "care", &at)?, &at)?,
                    thickness: text(get(m, "thickness", &at)?, &at)?,
                    fabric: m.get("fabric").map(|f| text(f, &at)).transpose()?,
                    fits: set(m.get("fits"), &at)?,
                },
            );
        }

        let department_of: BTreeMap<&str, &str> = departments
            .iter()
            .flat_map(|d| d.categories.iter().map(|c| (c.as_str(), d.key.as_str())))
            .collect();
        let node_map = nodes["nodes"]
            .as_object()
            .ok_or("nodes.yml: nodes is not a map")?;
        let mut categories = Vec::new();
        for (key, c) in map(get(&a, "categories", "assortment.yml")?, "categories")? {
            let at = format!("categories.{key}");
            let node = text(get(c, "node", &at)?, &at)?;
            if !department_of.contains_key(key.as_str()) {
                return Err(format!("assortment.yml {at}: in no department"));
            }
            categories.push(category(key, c, &at, &node, node_map, &size_systems)?);
        }

        let mut tiers = BTreeMap::new();
        for (key, t) in map(get(&b, "tiers", "brands.yml")?, "tiers")? {
            let at = format!("tiers.{key}");
            tiers.insert(
                key.clone(),
                tier(t, &at, &departments, &categories, &materials)?,
            );
        }

        let palettes = map(get(&b, "palettes", "brands.yml")?, "palettes")?;
        let mut brands = Vec::new();
        for (id, br) in map(get(&b, "brands", "brands.yml")?, "brands")? {
            let at = format!("brands.{id}");
            let mut cs: Vec<String> = Vec::new();
            for p in texts(get(br, "palette", &at)?, &at)? {
                let list = palettes
                    .get(&p)
                    .ok_or_else(|| format!("{at}: no palette `{p}`"))?;
                cs.extend(texts(list, &format!("palettes.{p}"))?);
            }
            if let Some(extra) = br.get("extra_colors") {
                cs.extend(texts(extra, &at)?);
            }
            let mut seen = BTreeSet::new();
            cs.retain(|c| seen.insert(c.clone()));
            let audience = map(get(br, "audience", &at)?, &at)?
                .iter()
                .map(|(g, s)| {
                    let gender = Gender::parse(g).ok_or_else(|| format!("{at}: gender `{g}`"))?;
                    Ok((gender, share(s, &at)?))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let focus = map(get(br, "focus", &at)?, &at)?
                .iter()
                .map(|(c, p)| Ok((c.clone(), int(p, &at)? as u64)))
                .collect::<Result<Vec<_>, String>>()?;
            brands.push(Brand {
                id: id.clone(),
                key: id
                    .strip_prefix("brn_")
                    .ok_or_else(|| format!("{at}: a brand id starts with `brn_`"))?
                    .to_owned(),
                name: text(get(br, "name", &at)?, &at)?,
                tier: text(get(br, "tier", &at)?, &at)?,
                products: int(get(br, "products", &at)?, &at)? as usize,
                identity: text(get(br, "identity", &at)?, &at)?,
                audience,
                focus,
                colors: cs,
                origin: texts(get(br, "origin", &at)?, &at)?,
                materials: br.get("materials").map(|m| set(Some(m), &at)).transpose()?,
            });
        }

        let mall = Mall {
            store,
            countries,
            departments,
            fit_words,
            fit_drawn,
            material_words,
            size_systems,
            foot_length,
            materials,
            categories,
            tiers,
            brands,
            colors,
        };
        mall.check()?;
        Ok(mall)
    }

    pub(crate) fn category(&self, key: &str) -> Option<&Category> {
        self.categories.iter().find(|c| c.key == key)
    }

    /// Every name one part uses from another exists.
    fn check(&self) -> Result<(), String> {
        let mut problems = Vec::new();
        for c in &self.categories {
            for (t, list) in &c.type_materials {
                if !c.types.iter().any(|x| &x.name == t) {
                    problems.push(format!("{}: type_materials names no type `{t}`", c.key));
                }
                for m in list {
                    if !self.materials.contains_key(m) {
                        problems.push(format!("{} `{t}`: no material `{m}`", c.key));
                    }
                }
            }
            if c.plain
                .as_ref()
                .is_some_and(|m| !self.materials.contains_key(m))
            {
                problems.push(format!("{}: plain material is not a material", c.key));
            }
            for system in c.sizes.values() {
                if !self.size_systems.contains_key(system) {
                    problems.push(format!("{}: no size system `{system}`", c.key));
                }
            }
            for (k, keys) in [("warmth", &c.warmth), ("water_resistance", &c.water)] {
                for t in keys.keys() {
                    if !c.types.iter().any(|x| &x.name == t) {
                        problems.push(format!("{} {k}: no type `{t}`", c.key));
                    }
                }
            }
            if c.required.contains("warmth") {
                for t in &c.types {
                    if !c.warmth.contains_key(&t.name) {
                        problems.push(format!("{} `{}`: no warmth", c.key, t.name));
                    }
                }
            }
            if c.required.contains("heel_height") {
                for t in &c.types {
                    if !c.heel_mm.contains_key(&t.name) {
                        problems.push(format!("{} `{}`: no heel height", c.key, t.name));
                    }
                }
            }
        }
        for m in self.materials.values() {
            for c in &m.fits {
                if self.category(c).is_none() {
                    problems.push(format!("material {}: fits no category `{c}`", m.name));
                }
            }
        }
        let total: usize = self.brands.iter().map(|b| b.products).sum();
        if total != self.store.products {
            problems.push(format!(
                "brands sell {total} products; the store says {}",
                self.store.products
            ));
        }
        for b in &self.brands {
            let Some(tier) = self.tiers.get(&b.tier) else {
                problems.push(format!("{}: no tier `{}`", b.id, b.tier));
                continue;
            };
            if b.focus.iter().map(|(_, p)| p).sum::<u64>() != 100 {
                problems.push(format!("{}: focus does not add up to 100", b.id));
            }
            for (c, _) in &b.focus {
                if self.category(c).is_none() {
                    problems.push(format!("{}: focus on no category `{c}`", b.id));
                }
            }
            for m in b.materials.iter().flatten() {
                if !self.materials.contains_key(m) {
                    problems.push(format!("{}: no material `{m}`", b.id));
                }
            }
            for c in &b.colors {
                if !self.colors.contains_key(c) {
                    problems.push(format!("{}: color `{c}` is not in colors.yml", b.id));
                }
            }
            for o in &b.origin {
                if !tier.origin.contains(o) {
                    problems.push(format!(
                        "{}: origin {o} is not on the {} list",
                        b.id, b.tier
                    ));
                }
                if !self.countries.contains_key(o) {
                    problems.push(format!("{}: origin {o} has no country name", b.id));
                }
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(format!("seed/mall: {}", problems.join("; ")))
        }
    }
}

fn colors(doc: &Value) -> Result<BTreeMap<String, Color>, String> {
    let mut out = BTreeMap::new();
    for (name, c) in map(get(doc, "colors", "colors.yml")?, "colors.yml colors")? {
        let at = format!("colors.yml {name}");
        out.insert(
            name.clone(),
            Color {
                families: texts(get(c, "family", &at)?, &at)?,
                pattern: c.get("pattern").map(|p| text(p, &at)).transpose()?,
                only_for: c.get("for").map(|f| set(Some(f), &at)).transpose()?,
            },
        );
    }
    Ok(out)
}

/// The attributes a node needs: its own `required` and its parents'.
fn required(nodes: &Map<String, Value>, node: &str) -> Result<BTreeSet<String>, String> {
    let mut out = BTreeSet::new();
    let mut cur = Some(node.to_owned());
    let mut depth = 0;
    while let Some(n) = cur {
        let v = nodes
            .get(&n)
            .ok_or_else(|| format!("nodes.yml: no node `{n}`"))?;
        out.extend(set(v.get("required"), &format!("nodes.{n}.required"))?);
        cur = v.get("parent").and_then(Value::as_str).map(str::to_owned);
        depth += 1;
        if depth > 16 {
            return Err(format!("nodes.yml: `{node}` has a parent loop"));
        }
    }
    Ok(out)
}

fn category(
    key: &str,
    c: &Value,
    at: &str,
    node: &str,
    nodes: &Map<String, Value>,
    size_systems: &BTreeMap<String, Vec<String>>,
) -> Result<Category, String> {
    let (lo, hi) = int_pair(get(c, "price", at)?, at)?;
    let mut sizes = BTreeMap::new();
    match get(c, "sizes", at)? {
        Value::String(system) => {
            for g in [Gender::Female, Gender::Male, Gender::Unisex] {
                sizes.insert(g, system.clone());
            }
        }
        Value::Object(by) => {
            for (g, system) in by {
                let g = Gender::parse(g).ok_or_else(|| format!("{at}.sizes: gender `{g}`"))?;
                sizes.insert(g, text(system, at)?);
            }
            // A unisex product takes the men's sizes where the category names none for it.
            if !sizes.contains_key(&Gender::Unisex)
                && let Some(m) = sizes.get(&Gender::Male).cloned()
            {
                sizes.insert(Gender::Unisex, m);
            }
        }
        _ => return Err(format!("{at}.sizes is not a size system or a map")),
    }
    if sizes.values().any(|s| !size_systems.contains_key(s)) {
        return Err(format!("{at}.sizes names an unknown size system"));
    }

    let mut types = Vec::new();
    if let Some(levels) = c.get("types") {
        for (level, list) in map(levels, at)? {
            let level: usize = level
                .parse()
                .ok()
                .filter(|l| (1..=3).contains(l))
                .ok_or_else(|| format!("{at}.types: level `{level}`"))?;
            for name in texts(list, at)? {
                types.push(GarmentType {
                    name,
                    gender: Gender::Unisex,
                    level,
                });
            }
        }
    }
    for (gender, key) in [(Gender::Female, "women"), (Gender::Male, "men")] {
        if let Some(list) = c.get(key) {
            let list = texts(list, &format!("{at}.{key}"))?;
            let n = list.len();
            // Written cheapest first: the first third is level 1, the last third level 3.
            for (i, name) in list.into_iter().enumerate() {
                types.push(GarmentType {
                    name,
                    gender,
                    level: 1 + (3 * i) / n,
                });
            }
        }
    }
    let mut seen = BTreeSet::new();
    if let Some(t) = types.iter().find(|t| !seen.insert(t.name.clone())) {
        return Err(format!("{at}: type `{}` is listed twice", t.name));
    }

    let mut type_materials = BTreeMap::new();
    if let Some(tm) = c.get("type_materials") {
        for (t, list) in map(tm, at)? {
            type_materials.insert(t.clone(), texts(list, &format!("{at}.type_materials.{t}"))?);
        }
    }
    let mut heel_mm = BTreeMap::new();
    if let Some(h) = c.get("heel_mm") {
        for (t, range) in map(h, at)? {
            heel_mm.insert(t.clone(), int_pair(range, &format!("{at}.heel_mm.{t}"))?);
        }
    }
    let label = nodes
        .get(node)
        .and_then(|n| n.get("label"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(Category {
        key: key.to_owned(),
        name: label.unwrap_or_else(|| key.to_owned()),
        required: required(nodes, node)?,
        price: (lo * 100, hi * 100),
        sizes,
        types,
        type_materials,
        alpha_types: set(c.get("alpha_types"), at)?,
        plain: c.get("plain").map(|p| text(p, at)).transpose()?,
        fabric: c.get("fabric").map(|f| text(f, at)).transpose()?,
        warmth: by_value(c.get("warmth"), &format!("{at}.warmth"))?,
        water: by_value(c.get("water_resistance"), &format!("{at}.water_resistance"))?,
        heel_mm,
    })
}

fn tier(
    t: &Value,
    at: &str,
    departments: &[Department],
    categories: &[Category],
    materials: &BTreeMap<String, Material>,
) -> Result<Tier, String> {
    let ending = text(get(t, "price_ending", at)?, at)?;
    let levels = map(get(t, "type_levels", at)?, at)?;
    let mut type_levels = [0; 3];
    for (l, w) in levels {
        let i: usize = l
            .parse()
            .ok()
            .filter(|l| (1..=3).contains(l))
            .ok_or_else(|| format!("{at}.type_levels: level `{l}`"))?;
        type_levels[i - 1] = int(w, at)? as u64;
    }
    let classes = map(get(t, "materials", at)?, at)?
        .iter()
        .map(|(c, w)| {
            let class = Class::parse(c).ok_or_else(|| format!("{at}.materials: `{c}`"))?;
            Ok((class, share(w, at)?))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let women_sizes = match t.get("women_sizes") {
        Some(system) => Some((text(system, at)?, set(t.get("women_sizes_in"), at)?)),
        None => None,
    };
    let one_size = match t.get("one_size_share") {
        Some(s) => Some((share(s, at)?, set(t.get("one_size_in"), at)?)),
        None => None,
    };
    let word_materials = match t.get("word_materials") {
        Some(m) => map(m, at)?.clone(),
        None => Map::new(),
    };
    let mut words = Vec::new();
    if let Some(w) = t.get("words") {
        for (word, keys) in map(w, at)? {
            let at = format!("{at}.words.{word}");
            // A word fits categories, departments, garment types, or `all`.
            let (mut cats, mut types) = (BTreeSet::new(), BTreeSet::new());
            for k in texts(keys, &at)? {
                if k == "all" {
                    cats.extend(categories.iter().map(|c| c.key.clone()));
                } else if let Some(d) = departments.iter().find(|d| d.key == k) {
                    cats.extend(d.categories.iter().cloned());
                } else if categories.iter().any(|c| c.key == k) {
                    cats.insert(k);
                } else if categories
                    .iter()
                    .any(|c| c.types.iter().any(|t| t.name == k))
                {
                    types.insert(k);
                } else {
                    return Err(format!("{at}: `{k}` is no category, department or type"));
                }
            }
            let suits = match word_materials.get(word) {
                Some(list) => {
                    let list = set(Some(list), &format!("{at} word_materials"))?;
                    if let Some(m) = list.iter().find(|m| {
                        !materials.contains_key(*m) && !materials.values().any(|x| &x.family == *m)
                    }) {
                        return Err(format!("{at}: `{m}` is no material or material family"));
                    }
                    Some(list)
                }
                None => None,
            };
            words.push(Word {
                word: word.clone(),
                categories: cats,
                types,
                materials: suits,
            });
        }
    }
    if let Some(w) = word_materials
        .keys()
        .find(|w| !words.iter().any(|x| &x.word == *w))
    {
        return Err(format!(
            "{at}.word_materials: `{w}` is not one of the tier's words"
        ));
    }
    let women_only = set(t.get("women_only"), at)?;
    if let Some(w) = women_only
        .iter()
        .find(|w| !words.iter().any(|x| &x.word == *w))
    {
        return Err(format!(
            "{at}.women_only: `{w}` is not one of the tier's words"
        ));
    }
    Ok(Tier {
        factor: bp_pair(get(t, "price_factor", at)?, at)?,
        ending: Ending::parse(&ending).ok_or_else(|| format!("{at}: price_ending `{ending}`"))?,
        cost: bp_pair(get(t, "cost_share", at)?, at)?,
        type_levels,
        classes,
        value_only_in: t
            .get("value_only_in")
            .map(|v| set(Some(v), at))
            .transpose()?,
        premium_allowed: t
            .get("premium_allowed")
            .map(|v| set(Some(v), at))
            .transpose()?,
        origin: texts(get(t, "origin", at)?, at)?,
        colors_per_product: int_pair(get(t, "colors_per_product", at)?, at)?,
        stock: int_pair(get(t, "stock_per_variant", at)?, at)?,
        women_sizes,
        sizes_wide: t
            .get("sizes_wide")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        one_size,
        name: text(get(t, "name", at)?, at)?,
        words,
        women_only,
    })
}
