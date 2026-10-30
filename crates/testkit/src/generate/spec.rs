//! `seed/demo/generate.yml`, read into typed parts. Values the spec states only in prose are
//! the constants in [`prose`], each next to the sentence it comes from.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{NaiveDate, NaiveDateTime};
use serde_json::Value;

use super::util::{basis_points, cents, parse_date};

/// Values the spec gives in sentences rather than fields.
pub(crate) mod prose {
    /// `catalog.placement`: "20% of summer_outerwear and outerwear also in cat_travel or
    /// cat_fall_layers". Read as: summer outerwear goes to Travel, outerwear to Fall layers.
    pub(crate) const PLACEMENT_PERCENT: i64 = 20;
    pub(crate) const PLACEMENT: &[(&str, &str)] = &[
        ("summer_outerwear", "cat_travel"),
        ("outerwear", "cat_fall_layers"),
    ];
    /// `catalog.autumn_knit_prices`.
    pub(crate) const AUTUMN_KNIT_CATEGORY: &str = "knitwear";
    pub(crate) const AUTUMN_KNIT_PROMOTION: &str = "promo_autumn_knit";
    /// `summer_outerwear.additional_property.packed_size_cm`: "only when packing is own_pocket:
    /// [short, long] with short 12-16, long 16-22".
    pub(crate) const PACKED_WHEN: &str = "own_pocket";
    pub(crate) const PACKED_SHORT: (i64, i64) = (12, 16);
    pub(crate) const PACKED_LONG: (i64, i64) = (16, 22);
    /// `members.addresses`: US north-east states.
    pub(crate) const STATES: &[&str] = &["NY", "NJ", "CT", "MA", "RI", "VT", "ME", "PA"];
    /// `members.gold_coupons`.
    pub(crate) const GOLD_TIER: &str = "gold";
    pub(crate) const GOLD_COUPON: &str = "cpn_gold_10";
    pub(crate) const GOLD_COUPON_MONTH: (i32, u32) = (2026, 11);
    pub(crate) const GOLD_COUPON_STATUS: &str = "available";
    /// `orders.customers`: "weighted: vip 3, gold 2, member 1 per member".
    pub(crate) const TIER_WEIGHTS: &[(&str, u64)] = &[("vip", 3), ("gold", 2), ("member", 1)];
    /// `orders.shipping`: "6.00 when the subtotal before discounts is under 80.00".
    pub(crate) const SHIPPING_FEE: i64 = 600;
    pub(crate) const FREE_SHIPPING_FROM: i64 = 8000;
    /// `orders.delivery`: UPS, "delivered_at 2-4 days after order_date".
    pub(crate) const CARRIER: &str = "UPS";
    pub(crate) const DELIVERY_DAYS: (i64, i64) = (2, 4);
    /// `orders.daily_sales.rule`: "Days in the window that are not listed get 150-260 orders
    /// and revenue at 88-96 USD per order".
    pub(crate) const OTHER_DAY_ORDERS: (i64, i64) = (150, 260);
    pub(crate) const OTHER_DAY_AVERAGE_CENTS: (i64, i64) = (8800, 9600);
    /// `orders.daily_sales.b3_causes.out_of_stock`: over the compare Thursdays, generated lines
    /// sell 37 chore coat units and 47 rain parka + denim jacket units in the sold-out sizes.
    pub(crate) const B3_CHORE_COAT: (&str, i64) = ("prd_chore_coat", 37);
    pub(crate) const B3_PARKA_DENIM: (&[&str], i64) = (&["prd_rain_parka", "prd_denim_jacket"], 47);
    /// `orders.linen_shirt_velocity.rule`.
    pub(crate) const LINEN_PRODUCT: &str = "prd_linen_shirt";
    pub(crate) const LINEN_DAYS: i64 = 7;
    pub(crate) const LINEN_UNITS: &[(&str, i64)] =
        &[("XS", 6), ("S", 16), ("M", 19), ("L", 14), ("XL", 6)];
    /// `orders.other_watchers` with `policies/platform.yml` `watchers.stock_out_forecast`
    /// (lookback 7 days, trips at 5 days of stock or less).
    pub(crate) const FORECAST_LOOKBACK_DAYS: i64 = 7;
    pub(crate) const FORECAST_DAYS: i64 = 5;
}

#[derive(Debug)]
pub(crate) struct Spec {
    pub seed: u64,
    pub store: String,
    pub never_touch: BTreeSet<String>,
    pub catalog: CatalogSpec,
    pub members: MembersSpec,
    pub orders: OrdersSpec,
}

#[derive(Debug)]
pub(crate) struct CatalogSpec {
    pub total_products: usize,
    pub approval_status: String,
    pub brands: Vec<String>,
    pub return_policy: String,
    pub safety_stock: i64,
    /// Summer outerwear first, then the other categories in the order written.
    pub categories: Vec<CategorySpec>,
}

#[derive(Debug)]
pub(crate) struct CategorySpec {
    pub term: String,
    pub count: usize,
    pub price_dollars: (i64, i64),
    pub cost_ratio_bp: (i64, i64),
    pub stock: (i64, i64),
    pub sizes: Vec<Option<String>>,
    pub colors: Vec<String>,
    pub colors_per_product: (i64, i64),
    pub materials: Vec<String>,
    pub nouns: Vec<String>,
    pub weight_g: (i64, i64),
    /// The compare_products attributes; only summer outerwear has them.
    pub attributes: Option<Attributes>,
}

#[derive(Debug)]
pub(crate) struct Attributes {
    pub packing: Vec<String>,
    pub warmth: Vec<String>,
    pub rain: Vec<bool>,
}

#[derive(Debug)]
pub(crate) struct MembersSpec {
    /// (tier, total, named, generate) in the order written.
    pub tiers: Vec<(String, i64, i64, i64)>,
    pub member_state: String,
    pub registered_from: NaiveDateTime,
    pub registered_through: NaiveDateTime,
    pub given_names: Vec<String>,
    pub family_names: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct OrdersSpec {
    pub lines: Vec<(usize, u64)>,
    pub quantities: Vec<(i64, u64)>,
    pub order_number_start: i64,
    pub window_from: NaiveDate,
    pub window_through: NaiveDate,
    /// (first day or None for "before", last day, status).
    pub status_by_date: Vec<(Option<NaiveDate>, NaiveDate, String)>,
    /// Day -> (revenue in cents, orders), named orders included.
    pub targets: BTreeMap<NaiveDate, (i64, i64)>,
    /// Day -> (orders, revenue in cents) of the named orders the spec subtracts.
    pub named_on_target_days: BTreeMap<NaiveDate, (i64, i64)>,
    /// Campaign id -> revenue in cents of the orders that came from it (`campaign_orders`).
    pub campaign_revenue: BTreeMap<String, i64>,
}

fn at<'a>(v: &'a Value, path: &str) -> Result<&'a Value, String> {
    let mut cur = v;
    for key in path.split('.') {
        cur = cur
            .get(key)
            .ok_or_else(|| format!("generate.yml: `{path}` is missing"))?;
    }
    Ok(cur)
}

fn text(v: &Value, path: &str) -> Result<String, String> {
    at(v, path)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("generate.yml: `{path}` is not text"))
}

fn int(v: &Value, path: &str) -> Result<i64, String> {
    as_int(at(v, path)?).ok_or_else(|| format!("generate.yml: `{path}` is not a whole number"))
}

fn as_int(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn texts(v: &Value, path: &str) -> Result<Vec<String>, String> {
    let list = at(v, path)?
        .as_array()
        .ok_or_else(|| format!("generate.yml: `{path}` is not a list"))?;
    let out: Vec<String> = list
        .iter()
        .map(|x| match x {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .collect::<Option<_>>()
        .ok_or_else(|| format!("generate.yml: `{path}` has a value that is not text"))?;
    if out.is_empty() {
        return Err(format!("generate.yml: `{path}` is empty"));
    }
    Ok(out)
}

fn sizes(v: &Value, path: &str) -> Result<Vec<Option<String>>, String> {
    let list = at(v, path)?
        .as_array()
        .ok_or_else(|| format!("generate.yml: `{path}` is not a list"))?;
    let out: Vec<Option<String>> = list
        .iter()
        .map(|x| match x {
            Value::Null => Ok(None),
            Value::String(s) => Ok(Some(s.clone())),
            Value::Number(n) => Ok(Some(n.to_string())),
            _ => Err(format!(
                "generate.yml: `{path}` has a size that is not text"
            )),
        })
        .collect::<Result<_, _>>()?;
    if out.is_empty() {
        return Err(format!("generate.yml: `{path}` is empty"));
    }
    Ok(out)
}

fn min_max(v: &Value, path: &str) -> Result<(i64, i64), String> {
    let x = at(v, path)?;
    let pair = match x {
        Value::Array(a) if a.len() == 2 => (as_int(&a[0]), as_int(&a[1])),
        Value::Object(_) => (x.get("min").and_then(as_int), x.get("max").and_then(as_int)),
        _ => (None, None),
    };
    match pair {
        (Some(lo), Some(hi)) if lo <= hi => Ok((lo, hi)),
        _ => Err(format!("generate.yml: `{path}` is not a min/max pair")),
    }
}

fn ratio_bp(v: &Value, path: &str) -> Result<(i64, i64), String> {
    let x = at(v, path)?;
    let pair = match x {
        Value::Array(a) if a.len() == 2 => (a[0].as_f64(), a[1].as_f64()),
        _ => (x.as_f64(), x.as_f64()),
    };
    match pair {
        (Some(lo), Some(hi)) if lo <= hi && lo > 0.0 && hi < 1.0 => {
            Ok((basis_points(lo), basis_points(hi)))
        }
        _ => Err(format!("generate.yml: `{path}` is not a cost ratio")),
    }
}

/// Weights of a `{1: 0.62, 2: 0.28}` map as (key, weight in 1/10,000).
fn weights(v: &Value, path: &str) -> Result<Vec<(i64, u64)>, String> {
    let map = at(v, path)?
        .as_object()
        .ok_or_else(|| format!("generate.yml: `{path}` is not a map"))?;
    let mut out = Vec::new();
    for (k, w) in map {
        let key: i64 = k
            .parse()
            .map_err(|_| format!("generate.yml: `{path}.{k}` is not a number"))?;
        let w = w
            .as_f64()
            .ok_or_else(|| format!("generate.yml: `{path}.{k}` is not a share"))?;
        out.push((key, basis_points(w).max(0) as u64));
    }
    if out.is_empty() || out.iter().all(|(_, w)| *w == 0) {
        return Err(format!("generate.yml: `{path}` has no weights"));
    }
    Ok(out)
}

fn wall_clock(v: &Value, path: &str) -> Result<NaiveDateTime, String> {
    let t = text(v, path)?;
    NaiveDateTime::parse_from_str(&t, "%Y-%m-%dT%H:%M")
        .map_err(|_| format!("generate.yml: `{path}` = `{t}` is not YYYY-MM-DDTHH:MM"))
}

/// A category's words: term code, materials and nouns for "<material> <noun>" names, and the
/// weight range of a variant in grams.
type Words = (
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
    (i64, i64),
);

/// Per-category words the spec leaves open for the standard categories.
#[rustfmt::skip]
const WORDS: &[Words] = &[
    ("outerwear", &["Wool", "Waxed cotton", "Quilted", "Melton", "Tweed", "Recycled nylon", "Corduroy", "Down"],
        &["overcoat", "parka", "peacoat", "puffer jacket", "trench coat", "car coat", "shell jacket", "duffle coat", "liner jacket", "field coat"], (600, 1800)),
    ("knitwear", &["Merino", "Lambswool", "Cotton", "Alpaca blend", "Mohair blend", "Wool", "Ribbed cotton", "Cashmere blend"],
        &["crew sweater", "cardigan", "turtleneck", "V-neck sweater", "half-zip sweater", "knit vest", "cable sweater", "polo knit", "mock neck", "fisherman sweater"], (250, 700)),
    ("shirts", &["Oxford", "Poplin", "Flannel", "Chambray", "Linen", "Twill", "Corduroy", "Seersucker"],
        &["shirt", "button-down", "work shirt", "camp shirt", "band-collar shirt", "utility shirt", "popover shirt", "western shirt"], (180, 400)),
    ("tees", &["Cotton", "Slub cotton", "Organic cotton", "Pima cotton", "Jersey", "Heavyweight cotton", "Linen blend", "Waffle"],
        &["tee", "pocket tee", "long-sleeve tee", "henley", "raglan tee", "boxy tee", "crew tee", "V-neck tee"], (140, 260)),
    ("sweatshirts", &["Loopback", "Fleece", "French terry", "Brushed cotton", "Organic cotton", "Heavyweight", "Waffle", "Recycled cotton"],
        &["sweatshirt", "hoodie", "zip hoodie", "crewneck", "quarter-zip", "raglan sweatshirt", "half-zip sweatshirt"], (400, 800)),
    ("pants", &["Chino", "Twill", "Corduroy", "Linen", "Wool", "Ripstop", "Canvas", "Seersucker"],
        &["trousers", "pants", "cargo pants", "pleated trousers", "carpenter pants", "drawstring pants", "straight pants", "tapered pants"], (300, 700)),
    ("denim", &["Rigid", "Selvedge", "Washed", "Stretch", "Black", "Raw", "Light-wash", "Organic"],
        &["jeans", "straight jeans", "slim jeans", "wide jeans", "denim shirt", "carpenter jeans", "denim overshirt", "denim vest"], (500, 900)),
    ("shorts", &["Chino", "Linen", "Ripstop", "Twill", "Seersucker", "Nylon", "Cotton", "Terry"],
        &["shorts", "cargo shorts", "trail shorts", "pull-on shorts", "pleated shorts", "camp shorts"], (150, 350)),
    ("dresses", &["Linen", "Poplin", "Jersey", "Silk blend", "Cotton", "Satin", "Knit", "Gauze"],
        &["midi dress", "shirt dress", "slip dress", "wrap dress", "maxi dress", "tiered dress", "sweater dress", "tank dress"], (200, 500)),
    ("skirts", &["Linen", "Wool", "Satin", "Denim", "Crepe", "Corduroy", "Cotton", "Knit"],
        &["midi skirt", "mini skirt", "wrap skirt", "A-line skirt", "slip skirt", "maxi skirt"], (150, 400)),
    ("activewear", &["Recycled polyester", "Merino", "Stretch", "Nylon", "Mesh", "Seamless", "Quick-dry", "Ribbed"],
        &["leggings", "running tee", "training shorts", "sports bra", "track jacket", "running tights", "tank top", "half-zip top"], (120, 400)),
    ("loungewear", &["Modal", "Waffle", "Brushed cotton", "Fleece", "Terry", "Jersey", "Flannel", "Cashmere blend"],
        &["lounge pants", "robe", "pajama set", "lounge top", "joggers", "sleep shirt", "lounge shorts"], (200, 600)),
    ("swimwear", &["Recycled nylon", "Ribbed", "Textured", "Quick-dry", "Printed", "Solid", "Sculpt", "Seersucker"],
        &["swim trunks", "one-piece", "bikini top", "bikini bottom", "swim shorts", "rash guard"], (80, 250)),
    ("underwear", &["Cotton", "Modal", "Merino", "Organic cotton", "Bamboo", "Mesh", "Stretch cotton", "Micromodal"],
        &["boxer briefs", "briefs", "trunks", "bralette", "undershirt", "boxers"], (50, 120)),
    ("shoes", &["Leather", "Suede", "Nubuck", "Waxed leather", "Canvas", "Pebbled leather"],
        &["derby shoes", "loafers", "desert boots", "hiking boots", "work boots", "brogues", "boat shoes", "ankle boots"], (700, 1400)),
    ("sneakers", &["Leather", "Canvas", "Suede", "Knit", "Mesh", "Recycled"],
        &["court sneakers", "runners", "high-tops", "low-tops", "trail runners", "slip-on sneakers", "retro runners"], (600, 1000)),
    ("sandals", &["Leather", "Cork", "Rubber", "Suede", "Recycled", "Webbing"],
        &["slides", "sandals", "trail sandals", "flip-flops", "clogs", "fisherman sandals"], (300, 700)),
    ("accessories", &["Leather", "Wool", "Canvas", "Cotton", "Silk", "Recycled"],
        &["card holder", "wallet", "gloves", "keyring", "glasses case", "phone sleeve", "umbrella"], (50, 300)),
    ("bags", &["Canvas", "Leather", "Nylon", "Waxed canvas", "Recycled polyester", "Ripstop"],
        &["backpack", "tote", "crossbody bag", "duffel", "sling bag", "weekender", "messenger bag", "belt bag"], (300, 1500)),
    ("hats", &["Wool", "Cotton", "Corduroy", "Fleece", "Canvas", "Ribbed knit"],
        &["cap", "beanie", "bucket hat", "trail cap", "wool cap", "sun hat"], (60, 150)),
    ("socks", &["Merino", "Cotton", "Wool blend", "Bamboo", "Recycled", "Cushioned"],
        &["crew socks", "ankle socks", "trail socks", "dress socks", "boot socks", "no-show socks"], (40, 100)),
    ("scarves", &["Wool", "Cashmere blend", "Cotton", "Linen", "Silk", "Lambswool"],
        &["scarf", "wrap", "bandana", "snood", "stole"], (80, 300)),
    ("belts", &["Leather", "Webbing", "Suede", "Canvas", "Braided leather", "Elastic"],
        &["belt", "dress belt", "casual belt", "trail belt", "reversible belt"], (100, 250)),
];

fn owned(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_owned()).collect()
}

impl Spec {
    pub(crate) fn parse(yaml: &str) -> Result<Spec, String> {
        let v: Value = serde_norway::from_str(yaml).map_err(|e| format!("generate.yml: {e}"))?;
        let seed = at(&v, "random_seed")?
            .as_u64()
            .ok_or("generate.yml: `random_seed` is not a whole number")?;
        let never_touch = texts(&v, "never_touch.customers")?.into_iter().collect();
        Ok(Spec {
            seed,
            store: text(&v, "store")?,
            never_touch,
            catalog: Self::catalog(&v)?,
            members: Self::members(&v)?,
            orders: Self::orders(&v)?,
        })
    }

    fn catalog(v: &Value) -> Result<CatalogSpec, String> {
        let summer = at(v, "catalog.summer_outerwear")?;
        let props = at(summer, "additional_property")?;
        let mut categories = vec![CategorySpec {
            term: "summer_outerwear".to_owned(),
            count: int(summer, "count")? as usize,
            price_dollars: min_max(summer, "list_price")?,
            cost_ratio_bp: ratio_bp(summer, "cost_ratio")?,
            stock: min_max(summer, "stock_per_product")?,
            sizes: sizes(summer, "sizes")?,
            colors: texts(summer, "colors")?,
            colors_per_product: min_max(summer, "colors_per_product")?,
            materials: texts(summer, "materials")?,
            nouns: texts(summer, "nouns")?,
            weight_g: min_max(props, "weight_g")?,
            attributes: Some(Attributes {
                packing: texts(props, "packing")?,
                warmth: texts(props, "warmth")?,
                rain: at(props, "rain")?
                    .as_array()
                    .map(|a| a.iter().filter_map(Value::as_bool).collect())
                    .filter(|a: &Vec<bool>| !a.is_empty())
                    .ok_or("generate.yml: `rain` is not a list of booleans")?,
            }),
        }];
        let summer_colors = categories[0].colors.clone();
        let summer_cpp = categories[0].colors_per_product;
        let others_value = at(v, "catalog.other_categories")?;
        let others = others_value
            .as_object()
            .ok_or("generate.yml: `catalog.other_categories` is not a map")?;
        let default_sizes = sizes(others_value, "default_sizes")?;
        for (term, c) in others {
            if term == "default_sizes" {
                continue;
            }
            let (_, materials, nouns, weight) = WORDS
                .iter()
                .find(|(t, ..)| t == term)
                .ok_or_else(|| format!("generate.yml: no words for category `{term}`"))?;
            categories.push(CategorySpec {
                term: term.clone(),
                count: int(c, "count")? as usize,
                price_dollars: min_max(c, "price")?,
                cost_ratio_bp: ratio_bp(c, "cost_ratio")?,
                stock: min_max(c, "stock")?,
                sizes: if c.get("sizes").is_some() {
                    sizes(c, "sizes")?
                } else {
                    default_sizes.clone()
                },
                colors: summer_colors.clone(),
                colors_per_product: summer_cpp,
                materials: owned(materials),
                nouns: owned(nouns),
                weight_g: *weight,
                attributes: None,
            });
        }
        Ok(CatalogSpec {
            total_products: int(v, "catalog.total_products")? as usize,
            approval_status: text(v, "catalog.all_generated.approval_status")?,
            brands: texts(v, "catalog.all_generated.brand_id")?,
            return_policy: text(v, "catalog.all_generated.has_merchant_return_policy_id")?,
            safety_stock: int(v, "catalog.all_generated.safety_stock")?,
            categories,
        })
    }

    fn members(v: &Value) -> Result<MembersSpec, String> {
        let tiers = at(v, "members.tiers")?
            .as_object()
            .ok_or("generate.yml: `members.tiers` is not a map")?
            .iter()
            .map(|(tier, t)| {
                Ok((
                    tier.clone(),
                    int(t, "total")?,
                    int(t, "named")?,
                    int(t, "generate")?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(MembersSpec {
            tiers,
            member_state: text(v, "members.member_state")?,
            registered_from: wall_clock(v, "members.registered_at.from")?,
            registered_through: wall_clock(v, "members.registered_at.through")?,
            given_names: texts(v, "members.given_names")?,
            family_names: texts(v, "members.family_names")?,
        })
    }

    fn orders(v: &Value) -> Result<OrdersSpec, String> {
        let lines = weights(v, "orders.lines_per_order")?
            .into_iter()
            .map(|(k, w)| (k.max(1) as usize, w))
            .collect();
        let mut status_by_date = Vec::new();
        for (key, status) in at(v, "orders.status_by_date")?
            .as_object()
            .ok_or("generate.yml: `orders.status_by_date` is not a map")?
        {
            let status = status
                .as_str()
                .ok_or_else(|| format!("generate.yml: status for `{key}` is not a word"))?
                .to_owned();
            if let Some(day) = key.strip_prefix("before_") {
                let through = parse_date(day)?
                    .pred_opt()
                    .ok_or("generate.yml: date out of range")?;
                status_by_date.push((None, through, status));
            } else if let Some((from, through)) = key.split_once("_to_") {
                status_by_date.push((Some(parse_date(from)?), parse_date(through)?, status));
            } else {
                return Err(format!(
                    "generate.yml: status range `{key}` is not understood"
                ));
            }
        }
        let mut targets = BTreeMap::new();
        for (day, pair) in at(v, "orders.daily_sales.targets")?
            .as_object()
            .ok_or("generate.yml: `daily_sales.targets` is not a map")?
        {
            let (rev, n) = match pair.as_array().map(Vec::as_slice) {
                Some([r, n]) => (cents(r), n.as_i64()),
                _ => (None, None),
            };
            let (Some(rev), Some(n)) = (rev, n) else {
                return Err(format!(
                    "generate.yml: target `{day}` is not [revenue, orders]"
                ));
            };
            targets.insert(parse_date(day)?, (rev, n));
        }
        let mut named_on_target_days = BTreeMap::new();
        for (day, x) in at(v, "orders.daily_sales.named_orders_on_target_days")?
            .as_object()
            .ok_or("generate.yml: `named_orders_on_target_days` is not a map")?
        {
            let n = int(x, "orders")?;
            let rev = cents(at(x, "revenue")?)
                .ok_or_else(|| format!("generate.yml: revenue of `{day}` is not money"))?;
            named_on_target_days.insert(parse_date(day)?, (n, rev));
        }
        let mut campaign_revenue = BTreeMap::new();
        for (campaign, x) in at(v, "orders.campaign_orders.revenue")?
            .as_object()
            .ok_or("generate.yml: `campaign_orders.revenue` is not a map")?
        {
            let rev = cents(x).ok_or_else(|| {
                format!("generate.yml: revenue of campaign `{campaign}` is not money")
            })?;
            campaign_revenue.insert(campaign.clone(), rev);
        }
        Ok(OrdersSpec {
            lines,
            quantities: weights(v, "orders.quantity_per_line")?,
            order_number_start: int(v, "orders.order_number.start")?,
            window_from: wall_clock(v, "orders.window.from")?.date(),
            window_through: wall_clock(v, "orders.window.through")?.date(),
            status_by_date,
            targets,
            named_on_target_days,
            campaign_revenue,
        })
    }
}
