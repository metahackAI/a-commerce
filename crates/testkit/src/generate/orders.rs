//! Generated sales history: orders, items, payments, events and deliveries
//! (`generate.yml` `orders`).
//!
//! Each day in the window gets a fixed number of generated orders and a fixed revenue (the
//! spec target minus that day's named orders). Orders are composed one by one toward the
//! revenue still open. Line discounts can leave cents (20% of 68.00, 15% of 59.00), so some
//! forty orders before the end of the day one or two orders of discounted lines square the
//! cents, and the orders after them use whole-dollar lines only. The last two orders are
//! chosen together from every one- and two-line basket so that the day closes to the cent;
//! the spec's "last order absorbs the remaining cents" is this pair.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, TimeDelta};
use chrono_tz::Tz;
use serde_json::json;

use super::members::Member;
use super::named::{ItemPromotion, Named};
use super::spec::{Spec, prose};
use super::util::{Dice, at, money, rfc3339, round_div};

/// A variant that can be sold: named or generated.
#[derive(Debug)]
pub(crate) struct Sku {
    pub id: String,
    pub product_id: String,
    pub product_name: String,
    pub category_id: String,
    pub size: Option<String>,
    pub price: i64,
    pub inventory: i64,
    pub sold_out_at: Option<DateTime<Tz>>,
    pub old_price: Option<(i64, DateTime<Tz>)>,
}

#[derive(Debug, Clone)]
pub(crate) struct Line {
    pub sku: usize,
    pub qty: i64,
    pub price: i64,
    pub discount: i64,
    pub promotion: Option<usize>,
}

impl Line {
    fn gross(&self) -> i64 {
        self.price * self.qty
    }
}

#[derive(Debug)]
pub(crate) struct Order {
    pub lines: Vec<Line>,
    pub subtotal: i64,
    pub discount: i64,
    pub shipping: i64,
    pub total: i64,
    pub at: DateTime<Tz>,
    pub customer: usize,
    pub number: i64,
    pub status: String,
    pub shipped_at: Option<DateTime<Tz>>,
    pub delivered_at: Option<DateTime<Tz>>,
    pub expected: Option<(DateTime<Tz>, DateTime<Tz>)>,
}

fn shipping(subtotal: i64) -> i64 {
    if subtotal < prose::FREE_SHIPPING_FROM {
        prose::SHIPPING_FEE
    } else {
        0
    }
}

impl Order {
    fn new(start: DateTime<Tz>, lines: Vec<Line>) -> Order {
        let subtotal: i64 = lines.iter().map(Line::gross).sum();
        let discount: i64 = lines.iter().map(|l| l.discount).sum();
        let shipping = shipping(subtotal);
        Order {
            lines,
            subtotal,
            discount,
            shipping,
            total: subtotal - discount + shipping,
            at: start,
            customer: 0,
            number: 0,
            status: String::new(),
            shipped_at: None,
            delivered_at: None,
            expected: None,
        }
    }
}

/// Variants open to picks, cheapest net unit price first.
#[derive(Debug, Default)]
struct Pool {
    skus: Vec<usize>,
    net: Vec<i64>,
}

/// One store day: the price and line promotion of every variant, and the variants open to
/// random picks.
#[derive(Debug)]
struct Day {
    start: DateTime<Tz>,
    price: Vec<i64>,
    promotion: Vec<Option<usize>>,
    /// Every variant open to random picks.
    all: Pool,
    /// Those whose line totals are whole dollars (no discount with cents).
    whole: Pool,
    /// Those whose discount leaves cents, for the order that squares the day's cents.
    cents: Vec<usize>,
    /// Inside the stock-out forecast lookback: picks must not trip the watcher.
    capped: bool,
}

/// Variants of one price and promotion.
#[derive(Debug)]
struct Class {
    members: Vec<usize>,
}

/// `qty` units of a class as one line, with its amounts before and after discount.
#[derive(Debug)]
struct Basket {
    class: usize,
    qty: i64,
    gross: i64,
    net: i64,
}

const NONE: u16 = u16::MAX;

/// Every order of one or two lines over the whole-dollar variants, by total.
#[derive(Debug)]
struct Closing {
    classes: Vec<Class>,
    baskets: Vec<Basket>,
    combos: Vec<[u16; 2]>,
    totals: Vec<i64>,
    by_total: BTreeMap<i64, Vec<u32>>,
}

/// Orders before the end of a day at which the day's cents are squared.
const SQUARE_CENTS_AT: usize = 40;

/// Hour-of-day weights for order times (store time).
const HOURS: [u64; 24] = [
    3, 2, 1, 1, 1, 2, 4, 7, 9, 10, 11, 12, 13, 12, 11, 11, 12, 13, 15, 16, 15, 12, 9, 5,
];

fn total_of(baskets: &[Basket], picked: &[usize]) -> i64 {
    let gross: i64 = picked.iter().map(|&b| baskets[b].gross).sum();
    let net: i64 = picked.iter().map(|&b| baskets[b].net).sum();
    net + shipping(gross)
}

#[derive(Debug)]
struct Shop<'a> {
    skus: &'a [Sku],
    promotions: &'a [ItemPromotion],
    tz: Tz,
    sold7: Vec<i64>,
    lines: Vec<(usize, u64)>,
    quantities: Vec<(i64, u64)>,
}

impl Shop<'_> {
    fn discount(&self, price: i64, qty: i64, promotion: Option<usize>) -> i64 {
        promotion.map_or(0, |p| {
            round_div(price * qty * self.promotions[p].percent_bp, 10_000)
        })
    }

    fn day(&self, date: NaiveDate, forced_only: &BTreeSet<usize>, capped: bool) -> Day {
        let start = at(self.tz, date, 0, 0);
        let end = at(self.tz, date, 23, 59);
        let mut price = Vec::with_capacity(self.skus.len());
        let mut promotion = Vec::with_capacity(self.skus.len());
        for s in self.skus {
            price.push(match s.old_price {
                Some((old, until)) if end < until => old,
                _ => s.price,
            });
            promotion.push(self.promotions.iter().position(|p| {
                p.from <= start
                    && p.through.is_none_or(|t| end <= t)
                    && (p.products.contains(&s.product_id) || p.categories.contains(&s.category_id))
            }));
        }
        let mut all: Vec<(i64, usize)> = Vec::new();
        let mut whole: Vec<(i64, usize)> = Vec::new();
        let mut cents = Vec::new();
        for (i, s) in self.skus.iter().enumerate() {
            if forced_only.contains(&i) || s.sold_out_at.is_some_and(|t| t <= end) || price[i] <= 0
            {
                continue;
            }
            let net = price[i] - self.discount(price[i], 1, promotion[i]);
            all.push((net, i));
            let has_cents = self
                .quantities
                .iter()
                .any(|&(q, _)| self.discount(price[i], q, promotion[i]) % 100 != 0);
            if has_cents {
                cents.push(i);
            } else {
                whole.push((net, i));
            }
        }
        let pool = |mut v: Vec<(i64, usize)>| {
            v.sort_unstable();
            Pool {
                net: v.iter().map(|(n, _)| *n).collect(),
                skus: v.iter().map(|(_, i)| *i).collect(),
            }
        };
        Day {
            start,
            price,
            promotion,
            all: pool(all),
            whole: pool(whole),
            cents,
            capped,
        }
    }

    fn line(&self, day: &Day, sku: usize, qty: i64) -> Line {
        let price = day.price[sku];
        let promotion = day.promotion[sku];
        Line {
            sku,
            qty,
            price,
            discount: self.discount(price, qty, promotion),
            promotion,
        }
    }

    /// Selling `qty` more keeps the variant above the stock-out forecast threshold.
    fn cap_ok(&self, day: &Day, sku: usize, qty: i64) -> bool {
        !day.capped
            || prose::FORECAST_DAYS * (self.sold7[sku] + qty)
                < prose::FORECAST_LOOKBACK_DAYS * self.skus[sku].inventory
    }

    fn apply(&mut self, day: &Day, lines: &[Line], sign: i64) {
        if day.capped {
            for l in lines {
                self.sold7[l.sku] += sign * l.qty;
            }
        }
    }

    fn pick_near(
        &self,
        dice: &mut Dice,
        day: &Day,
        pool: &Pool,
        target: i64,
        qty: i64,
        taken: &[Line],
    ) -> Option<usize> {
        let ok = |s: usize| !taken.iter().any(|l| l.sku == s) && self.cap_ok(day, s, qty);
        let lo = pool.net.partition_point(|&n| n < target * 4 / 5);
        let hi = pool.net.partition_point(|&n| n <= target * 5 / 4);
        if lo < hi {
            for _ in 0..8 {
                let s = pool.skus[lo + dice.below(hi - lo)];
                if ok(s) {
                    return Some(s);
                }
            }
        }
        let mid = pool.net.partition_point(|&n| n < target);
        let n = pool.skus.len();
        for d in 0..=n {
            if mid + d < n && ok(pool.skus[mid + d]) {
                return Some(pool.skus[mid + d]);
            }
            if d < mid && ok(pool.skus[mid - d - 1]) {
                return Some(pool.skus[mid - d - 1]);
            }
        }
        None
    }

    /// One order worth about `target` cents (shipping included), with the forced lines first.
    fn compose(
        &mut self,
        dice: &mut Dice,
        day: &Day,
        whole_only: bool,
        target: i64,
        forced: &[(usize, i64)],
    ) -> Option<Order> {
        let pool = if whole_only { &day.whole } else { &day.all };
        let mut lines: Vec<Line> = forced.iter().map(|&(s, q)| self.line(day, s, q)).collect();
        let weights: Vec<u64> = self.lines.iter().map(|(_, w)| *w).collect();
        let want = self.lines[dice.weighted(&weights)].0;
        let extra = want
            .saturating_sub(lines.len())
            .max(usize::from(lines.is_empty()));
        let qweights: Vec<u64> = self.quantities.iter().map(|(_, w)| *w).collect();
        let qtys: Vec<i64> = (0..extra)
            .map(|_| self.quantities[dice.weighted(&qweights)].0)
            .collect();
        let mut budget = target - lines.iter().map(|l| l.gross() - l.discount).sum::<i64>();
        if target - prose::SHIPPING_FEE < prose::FREE_SHIPPING_FROM {
            budget -= prose::SHIPPING_FEE;
        }
        let mut units: i64 = qtys.iter().sum();
        for q in qtys {
            let per_unit = (budget / units.max(1)).max(100);
            let t = per_unit * dice.range(750, 1333) / 1000;
            let s = self.pick_near(dice, day, pool, t, q, &lines)?;
            let l = self.line(day, s, q);
            budget -= l.gross() - l.discount;
            units -= q;
            lines.push(l);
        }
        self.apply(day, &lines, 1);
        Some(Order::new(day.start, lines))
    }

    /// Classes (same price and promotion) and one- or two-unit baskets of them.
    fn baskets(&self, day: &Day, skus: &[usize]) -> (Vec<Class>, Vec<Basket>) {
        let mut by_class: BTreeMap<(i64, Option<usize>), Vec<usize>> = BTreeMap::new();
        for &s in skus {
            by_class
                .entry((day.price[s], day.promotion[s]))
                .or_default()
                .push(s);
        }
        let mut classes = Vec::new();
        let mut baskets = Vec::new();
        for ((price, promotion), members) in by_class {
            for &(qty, _) in &self.quantities {
                let gross = price * qty;
                baskets.push(Basket {
                    class: classes.len(),
                    qty,
                    gross,
                    net: gross - self.discount(price, qty, promotion),
                });
            }
            classes.push(Class { members });
        }
        (classes, baskets)
    }

    fn closing(&self, day: &Day) -> Closing {
        let (classes, baskets) = self.baskets(day, &day.whole.skus);
        let mut combos = Vec::new();
        let mut totals = Vec::new();
        let mut by_total: BTreeMap<i64, Vec<u32>> = BTreeMap::new();
        for a in 0..baskets.len() {
            for b in a..baskets.len() {
                let pair = a != b;
                if pair
                    && baskets[a].class == baskets[b].class
                    && classes[baskets[a].class].members.len() < 2
                {
                    continue;
                }
                let picked: &[usize] = if pair { &[a, b] } else { &[a] };
                let total = total_of(&baskets, picked);
                by_total.entry(total).or_default().push(combos.len() as u32);
                combos.push([a as u16, if pair { b as u16 } else { NONE }]);
                totals.push(total);
            }
        }
        Closing {
            classes,
            baskets,
            combos,
            totals,
            by_total,
        }
    }

    /// Lines for the given baskets on distinct variants, or None when the variants run out.
    fn realize(
        &mut self,
        dice: &mut Dice,
        day: &Day,
        classes: &[Class],
        baskets: &[Basket],
        picked: &[usize],
    ) -> Option<Order> {
        let mut lines: Vec<Line> = Vec::new();
        for &b in picked {
            let basket = &baskets[b];
            let members = &classes[basket.class].members;
            let start = dice.below(members.len());
            let sku = (0..members.len())
                .map(|k| members[(start + k) % members.len()])
                .find(|&s| !lines.iter().any(|l| l.sku == s) && self.cap_ok(day, s, basket.qty))?;
            lines.push(self.line(day, sku, basket.qty));
        }
        self.apply(day, &lines, 1);
        Some(Order::new(day.start, lines))
    }

    fn realize_combo(
        &mut self,
        dice: &mut Dice,
        day: &Day,
        c: &Closing,
        combo: usize,
    ) -> Option<Order> {
        let picked: Vec<usize> = c.combos[combo]
            .iter()
            .filter(|&&b| b != NONE)
            .map(|&b| b as usize)
            .collect();
        self.realize(dice, day, &c.classes, &c.baskets, &picked)
    }

    /// An order that squares the cents still open, so the rest of the day is whole dollars:
    /// one to three lines of discounted variants, as close to `average` as the cents allow.
    /// When no such order has the open cents exactly, it takes cents that one more order can
    /// finish.
    fn square_cents(
        &mut self,
        dice: &mut Dice,
        day: &Day,
        open: i64,
        average: i64,
    ) -> Option<Order> {
        let (classes, baskets) = self.baskets(day, &day.cents);
        let want = open.rem_euclid(100);
        let n = baskets.len();
        let mut by_cents: BTreeMap<i64, Vec<(i64, Vec<usize>)>> = BTreeMap::new();
        let mut consider = |picked: &[usize]| {
            let mut uses: BTreeMap<usize, usize> = BTreeMap::new();
            for &b in picked {
                *uses.entry(baskets[b].class).or_default() += 1;
            }
            let total = total_of(&baskets, picked);
            if total <= open / 8 && uses.iter().all(|(c, k)| classes[*c].members.len() >= *k) {
                by_cents
                    .entry(total.rem_euclid(100))
                    .or_default()
                    .push(((total - average).abs(), picked.to_vec()));
            }
        };
        for a in 0..n {
            consider(&[a]);
            for b in a..n {
                consider(&[a, b]);
                for c in b..n {
                    consider(&[a, b, c]);
                }
            }
        }
        let cents = if by_cents.contains_key(&want) {
            want
        } else {
            by_cents
                .keys()
                .copied()
                .find(|c| by_cents.contains_key(&(want - c).rem_euclid(100)))?
        };
        let mut found = by_cents.remove(&cents)?;
        found.sort();
        found.truncate(8);
        let start = dice.below(found.len());
        for k in 0..found.len() {
            let picked = &found[(start + k) % found.len()].1;
            if let Some(order) = self.realize(dice, day, &classes, &baskets, picked) {
                return Some(order);
            }
        }
        None
    }

    /// Two orders that add up to `remaining` exactly.
    fn close(
        &mut self,
        dice: &mut Dice,
        day: &Day,
        c: &Closing,
        remaining: i64,
    ) -> Option<(Order, Order)> {
        let n = c.combos.len();
        for pass in 0..2 {
            let start = dice.below(n);
            for step in 0..n {
                let first = (start + step) % n;
                let t1 = c.totals[first];
                let t2 = remaining - t1;
                if t2 <= 0 || (pass == 0 && (t1 * 10 < remaining * 3 || t1 * 10 > remaining * 7)) {
                    continue;
                }
                let Some(list) = c.by_total.get(&t2) else {
                    continue;
                };
                let j0 = dice.below(list.len());
                for k in 0..list.len().min(8) {
                    let second = list[(j0 + k) % list.len()] as usize;
                    if let Some(a) = self.realize_combo(dice, day, c, first) {
                        if let Some(b) = self.realize_combo(dice, day, c, second) {
                            return Some((a, b));
                        }
                        self.apply(day, &a.lines, -1);
                    }
                }
            }
        }
        None
    }
}

/// Every variant that can be sold, named first, then generated, in catalog order.
pub(crate) fn skus(named: &Named, catalog: &super::catalog::Catalog) -> Vec<Sku> {
    let mut out = Vec::new();
    for v in &named.variants {
        let (name, category) = named
            .products
            .get(&v.product_id)
            .cloned()
            .unwrap_or_default();
        out.push(Sku {
            id: v.id.clone(),
            product_id: v.product_id.clone(),
            product_name: name,
            category_id: category,
            size: v.size.clone(),
            price: v.price,
            inventory: v.inventory,
            sold_out_at: v.sold_out_at,
            old_price: v.old_price,
        });
    }
    for p in &catalog.products {
        for v in &p.variants {
            out.push(Sku {
                id: v.id.clone(),
                product_id: p.id.clone(),
                product_name: p.name.clone(),
                category_id: p.category_id.clone(),
                size: v.size.clone(),
                price: v.price,
                inventory: v.inventory,
                sold_out_at: None,
                old_price: None,
            });
        }
    }
    out
}

/// A day's generated order count and revenue in cents.
fn plan(
    spec: &Spec,
    named: &Named,
    date: NaiveDate,
    dice: &mut Dice,
) -> Result<(usize, i64), String> {
    let o = &spec.orders;
    let (named_n, named_rev) = named.sales_by_day.get(&date).copied().unwrap_or((0, 0));
    let (n, rev) = if let Some(&(rev, n)) = o.targets.get(&date) {
        let listed = o.named_on_target_days.get(&date).copied().unwrap_or((0, 0));
        if listed != (named_n, named_rev) {
            return Err(format!(
                "{date}: generate.yml subtracts {listed:?} named (orders, cents), the named rows have {:?}",
                (named_n, named_rev)
            ));
        }
        (n, rev)
    } else {
        let n = dice.range(prose::OTHER_DAY_ORDERS.0, prose::OTHER_DAY_ORDERS.1);
        let average = dice.range(
            prose::OTHER_DAY_AVERAGE_CENTS.0,
            prose::OTHER_DAY_AVERAGE_CENTS.1,
        );
        (n, round_div(n * average, 100) * 100)
    };
    let gen_n = n - named_n;
    if gen_n < 3 {
        return Err(format!("{date}: fewer than 3 generated orders"));
    }
    Ok((gen_n as usize, rev - named_rev))
}

/// Units the spec places on given days (the linen shirt velocity and the B3 sold-out sizes),
/// and the variants that sell only as such units on those days.
#[derive(Debug, Default)]
struct Forced {
    units: BTreeMap<NaiveDate, Vec<usize>>,
    only: BTreeMap<NaiveDate, BTreeSet<usize>>,
}

fn forced_units(spec: &Spec, skus: &[Sku], dice: &mut Dice) -> Result<Forced, String> {
    let o = &spec.orders;
    let mut units: BTreeMap<NaiveDate, Vec<usize>> = BTreeMap::new();
    let mut forced_only: BTreeMap<NaiveDate, BTreeSet<usize>> = BTreeMap::new();

    // B3: the compare days are the target days on the B3 weekday before the B3 day.
    let b3_day = *o.targets.keys().last().ok_or("no daily_sales targets")?;
    let compare: Vec<NaiveDate> = o
        .targets
        .keys()
        .filter(|d| d.weekday() == b3_day.weekday() && **d < b3_day)
        .copied()
        .collect();
    let sold_out = |products: &[&str]| -> Vec<usize> {
        skus.iter()
            .enumerate()
            .filter(|(_, s)| s.sold_out_at.is_some() && products.contains(&s.product_id.as_str()))
            .map(|(i, _)| i)
            .collect()
    };
    let chore = sold_out(&[prose::B3_CHORE_COAT.0]);
    let parka_denim = sold_out(prose::B3_PARKA_DENIM.0);
    if chore.is_empty() || parka_denim.is_empty() || compare.is_empty() {
        return Err("B3: no sold-out variants or compare days".to_owned());
    }
    let days = compare.len() as i64;
    for (i, day) in compare.iter().enumerate() {
        let i = i as i64;
        let (c, p) = (prose::B3_CHORE_COAT.1, prose::B3_PARKA_DENIM.1);
        let n_chore = c / days + i64::from(i >= days - c % days);
        let n_parka = p / days + i64::from(i < p % days);
        let list = units.entry(*day).or_default();
        for _ in 0..n_chore {
            list.push(*dice.pick(&chore));
        }
        for _ in 0..n_parka {
            list.push(*dice.pick(&parka_denim));
        }
        forced_only
            .entry(*day)
            .or_default()
            .extend(chore.iter().chain(&parka_denim));
    }

    // Linen shirt: exact units per size over the last days of the window, dealt evenly.
    let mut linen = Vec::new();
    for (size, n) in prose::LINEN_UNITS {
        let sku = skus
            .iter()
            .position(|s| s.product_id == prose::LINEN_PRODUCT && s.size.as_deref() == Some(size))
            .ok_or_else(|| format!("no {} variant in size {size}", prose::LINEN_PRODUCT))?;
        linen.extend(std::iter::repeat_n(sku, *n as usize));
    }
    dice.shuffle(&mut linen);
    let all_linen: BTreeSet<usize> = skus
        .iter()
        .enumerate()
        .filter(|(_, s)| s.product_id == prose::LINEN_PRODUCT)
        .map(|(i, _)| i)
        .collect();
    let first = o.window_through - TimeDelta::days(prose::LINEN_DAYS - 1);
    for (i, sku) in linen.into_iter().enumerate() {
        let day = first + TimeDelta::days(i as i64 % prose::LINEN_DAYS);
        units.entry(day).or_default().push(sku);
    }
    for k in 0..prose::LINEN_DAYS {
        forced_only
            .entry(first + TimeDelta::days(k))
            .or_default()
            .extend(&all_linen);
    }
    Ok(Forced {
        units,
        only: forced_only,
    })
}

pub(crate) fn generate(
    spec: &Spec,
    named: &Named,
    skus: &[Sku],
    members: &[Member],
    dice: &mut Dice,
) -> Result<Vec<Order>, String> {
    let o = &spec.orders;
    let tz = named.tz;
    let mut shop = Shop {
        skus,
        promotions: &named.promotions,
        tz,
        sold7: vec![0; skus.len()],
        lines: o.lines.clone(),
        quantities: o.quantities.clone(),
    };
    let Forced {
        mut units,
        only: forced_only,
    } = forced_units(spec, skus, dice)?;
    let lookback_from = o.window_through - TimeDelta::days(prose::FORECAST_LOOKBACK_DAYS - 1);
    let seed_day = o.window_through + TimeDelta::days(1);

    let weight_of = |tier: &str| {
        prose::TIER_WEIGHTS
            .iter()
            .find(|(t, _)| *t == tier)
            .map_or(1, |(_, w)| *w)
    };
    let mut cumulative = Vec::with_capacity(members.len());
    let mut weight_total = 0_u64;
    for m in members {
        weight_total += weight_of(&m.tier);
        cumulative.push(weight_total);
    }
    if members.is_empty() {
        return Err("no generated members to place orders".to_owned());
    }

    let mut orders: Vec<Order> = Vec::new();
    let mut date = o.window_from;
    while date <= o.window_through {
        let (n, revenue) = plan(spec, named, date, dice)?;
        let none = BTreeSet::new();
        let day = shop.day(
            date,
            forced_only.get(&date).unwrap_or(&none),
            date >= lookback_from,
        );

        // Forced units go to random orders, away from the closing pair.
        let mut forced: Vec<Vec<(usize, i64)>> = vec![Vec::new(); n];
        let square_at = n.saturating_sub(SQUARE_CENTS_AT);
        let reach = square_at.max(1);
        for sku in units.remove(&date).unwrap_or_default() {
            for attempt in 0..64 {
                let slot = &mut forced[dice.below(reach)];
                match slot.iter_mut().find(|(s, _)| *s == sku) {
                    Some((_, q)) if *q < 2 || attempt == 63 => {
                        *q += 1;
                        break;
                    }
                    Some(_) => {}
                    None => {
                        slot.push((sku, 1));
                        break;
                    }
                }
            }
        }

        let mut day_orders: Vec<Order> = Vec::with_capacity(n);
        let mut total = 0;
        for (i, f) in forced.iter().enumerate().take(n - 2) {
            let left = (n - i) as i64;
            let open = revenue - total;
            let order = if i >= square_at && open % 100 != 0 && f.is_empty() {
                shop.square_cents(dice, &day, open, open / left)
                    .ok_or_else(|| format!("{date}: cannot square {open} cents"))?
            } else {
                // Per mille of the average still open: mostly near it, now and then a big
                // basket (the expensive categories), which later orders make up for.
                let factor = match left {
                    0..=3 => 1000,
                    _ if dice.below(20) == 0 => dice.range(2000, 4000),
                    _ => dice.range(600, 1300),
                };
                let target = (open / left * factor / 1000).clamp(1500, 60_000);
                shop.compose(dice, &day, i >= square_at, target, f)
                    .ok_or_else(|| format!("{date}: no variant to sell"))?
            };
            total += order.total;
            day_orders.push(order);
        }
        let closing = shop.closing(&day);
        // When no pair closes the day, the last orders are composed again, going back further
        // as tries fail (never past the orders that squared the cents).
        let mut tries = 0;
        loop {
            if let Some((a, b)) = shop.close(dice, &day, &closing, revenue - total) {
                total += a.total + b.total;
                day_orders.push(a);
                day_orders.push(b);
                break;
            }
            tries += 1;
            // The (at most two) orders from `square_at` on squared the cents; keep them.
            let keep = (square_at + 2).min(n - 3);
            let depth = (1 + tries / 10).min(n - 2 - keep);
            if tries > 60 {
                return Err(format!(
                    "{date}: cannot close the day at {revenue} cents (open {})",
                    revenue - total
                ));
            }
            for _ in 0..depth {
                if let Some(last) = day_orders.pop() {
                    shop.apply(&day, &last.lines, -1);
                    total -= last.total;
                }
            }
            while day_orders.len() < n - 2 {
                let i = day_orders.len();
                let target = ((revenue - total) / (n - i) as i64).clamp(1500, 60_000);
                let order = shop
                    .compose(dice, &day, true, target, &forced[i])
                    .ok_or_else(|| format!("{date}: no variant to sell"))?;
                total += order.total;
                day_orders.push(order);
            }
        }
        debug_assert_eq!(total, revenue);

        // Times: sorted, so the order that closed the day is the last one.
        let mut times: Vec<NaiveTime> = (0..n)
            .map(|_| {
                let h = dice.weighted(&HOURS) as u32;
                let m = dice.below(60) as u32;
                NaiveTime::from_hms_opt(h, m, 0).unwrap_or(NaiveTime::MIN)
            })
            .collect();
        times.sort_unstable();
        let status = o
            .status_by_date
            .iter()
            .find(|(from, through, _)| from.is_none_or(|f| f <= date) && date <= *through)
            .map(|(_, _, s)| s.clone())
            .ok_or_else(|| format!("{date}: no order status in status_by_date"))?;
        for (order, time) in day_orders.iter_mut().zip(times) {
            order.at = super::util::local(tz, date.and_time(time));
            let mut customer = None;
            for _ in 0..1000 {
                let x = dice.below(weight_total as usize) as u64;
                let c = cumulative.partition_point(|&w| w <= x);
                if members[c].registered < order.at {
                    customer = Some(c);
                    break;
                }
            }
            order.customer = customer.ok_or_else(|| format!("{date}: no member registered"))?;
            order.status = status.clone();
            let next_day = date + TimeDelta::days(1);
            match status.as_str() {
                "delivered" => {
                    let days = dice.range(prose::DELIVERY_DAYS.0, prose::DELIVERY_DAYS.1);
                    let arrive = date + TimeDelta::days(days);
                    order.shipped_at = Some(at(
                        tz,
                        next_day,
                        dice.range(12, 17) as u32,
                        dice.below(60) as u32,
                    ));
                    order.delivered_at = Some(at(
                        tz,
                        arrive,
                        dice.range(9, 19) as u32,
                        dice.below(60) as u32,
                    ));
                    order.expected = Some((at(tz, arrive, 8, 0), at(tz, arrive, 20, 0)));
                }
                "in_transit" => {
                    let arrive = (date + TimeDelta::days(3)).max(seed_day);
                    order.shipped_at = Some(at(
                        tz,
                        next_day,
                        dice.range(12, 17) as u32,
                        dice.below(60) as u32,
                    ));
                    order.expected = Some((at(tz, arrive, 8, 0), at(tz, arrive, 20, 0)));
                }
                _ => {}
            }
        }
        orders.extend(day_orders);
        date += TimeDelta::days(1);
    }

    // Numbers follow order time across the window, skipping every named number.
    let mut next = o.order_number_start;
    for order in &mut orders {
        while named.order_numbers.contains(&next) {
            next += 1;
        }
        order.number = next;
        next += 1;
    }
    Ok(orders)
}

/// Rows in load order: orders, items, payments, events, deliveries.
pub(crate) fn rows(
    spec: &Spec,
    named: &Named,
    skus: &[Sku],
    members: &[Member],
    orders: &[Order],
    out: &mut crate::SeedRows,
) {
    let tenant = &spec.store;
    let cur = &named.currency;
    for o in orders {
        out.push((
            "sales.sales_order".to_owned(),
            json!({
                "tenant_id": tenant,
                "id": format!("ord_{}", o.number),
                "order_number": o.number.to_string(),
                "order_date": rfc3339(&o.at),
                "order_status": o.status,
                "customer_id": members[o.customer].id,
                "sales_channel": "web",
                "is_gift": false,
                "subtotal_value": money(o.subtotal),
                "subtotal_currency": cur,
                "discount_value": money(o.discount),
                "discount_currency": cur,
                "shipping_value": money(o.shipping),
                "shipping_currency": cur,
                "total_payment_due_value": money(o.total),
                "total_payment_due_currency": cur,
            }),
        ));
    }
    for o in orders {
        for (i, l) in o.lines.iter().enumerate() {
            let mut row = json!({
                "tenant_id": tenant,
                "id": format!("oln_{}_{}", o.number, i + 1),
                "order_id": format!("ord_{}", o.number),
                "position": i + 1,
                "product_id": skus[l.sku].id,
                "name": skus[l.sku].product_name,
                "order_quantity": l.qty,
                "order_item_status": o.status,
                "price": money(l.price),
                "price_currency": cur,
                "discount_value": money(l.discount),
                "discount_currency": cur,
            });
            if let (Some(p), Some(map)) = (l.promotion, row.as_object_mut()) {
                map.insert("promotion_id".to_owned(), json!(named.promotions[p].id));
            }
            out.push(("sales.order_item".to_owned(), row));
        }
    }
    for o in orders {
        out.push((
            "sales.payment".to_owned(),
            json!({
                "tenant_id": tenant,
                "id": format!("pay_{}", o.number),
                "order_id": format!("ord_{}", o.number),
                "payment_method": "credit_card",
                "payment_status": "payment_complete",
                "amount_value": money(o.total),
                "amount_currency": cur,
                "card_last4": members[o.customer].card,
                "installment_months": 0,
                "external_id": format!("ch_demo_{}", o.number),
                "paid_at": rfc3339(&(o.at + TimeDelta::minutes(1))),
            }),
        ));
    }
    for o in orders {
        let id = format!("ord_{}", o.number);
        let placed = rfc3339(&o.at);
        out.push((
            "sales.order_event".to_owned(),
            json!({
                "tenant_id": tenant,
                "order_id": id,
                "position": 1,
                "action_type": "OrderAction",
                "action_status": "completed",
                "agent_id": members[o.customer].id,
                "instrument": "POST /v1/sales/orders",
                "to_state": "processing",
                "start_time": placed,
                "end_time": placed,
            }),
        ));
        if let Some(t) = o.shipped_at {
            let t = rfc3339(&t);
            out.push((
                "sales.order_event".to_owned(),
                json!({
                    "tenant_id": tenant,
                    "order_id": id,
                    "position": 2,
                    "action_type": "SendAction",
                    "action_status": "completed",
                    "agent_id": "sys_carrier",
                    "instrument": "receive_carrier_event",
                    "from_state": "processing",
                    "to_state": "in_transit",
                    "start_time": t,
                    "end_time": t,
                }),
            ));
        }
        if let Some(t) = o.delivered_at {
            let t = rfc3339(&t);
            out.push((
                "sales.order_event".to_owned(),
                json!({
                    "tenant_id": tenant,
                    "order_id": id,
                    "position": 3,
                    "action_type": "ReceiveAction",
                    "action_status": "completed",
                    "agent_id": "sys_carrier",
                    "instrument": "receive_carrier_event",
                    "from_state": "in_transit",
                    "to_state": "delivered",
                    "start_time": t,
                    "end_time": t,
                }),
            ));
        }
    }
    for o in orders {
        let Some(shipped) = o.shipped_at else {
            continue;
        };
        let m = &members[o.customer];
        let tracking = format!("1ZNW{}0001", o.number);
        let mut row = json!({
            "tenant_id": tenant,
            "id": format!("dlv_{}", o.number),
            "order_id": format!("ord_{}", o.number),
            "delivery_state": if o.delivered_at.is_some() { "delivered" } else { "in_transit" },
            "delivery_address_id": m.address.id,
            "delivery_address": {
                "name": m.name(),
                "street_address": m.address.street,
                "address_locality": m.address.locality,
                "address_region": m.address.region,
                "postal_code": m.address.postal_code,
                "address_country": "US",
            },
            "has_delivery_method": "standard",
            "carrier": prose::CARRIER,
            "tracking_url": format!("https://track.example/{tracking}"),
            "tracking_number": tracking,
            "shipped_at": rfc3339(&shipped),
        });
        if let Some(map) = row.as_object_mut() {
            if let Some((from, until)) = o.expected {
                map.insert("expected_arrival_from".to_owned(), json!(rfc3339(&from)));
                map.insert("expected_arrival_until".to_owned(), json!(rfc3339(&until)));
            }
            if let Some(t) = o.delivered_at {
                map.insert("delivered_at".to_owned(), json!(rfc3339(&t)));
            }
        }
        out.push(("fulfillment.parcel_delivery".to_owned(), row));
    }
}
