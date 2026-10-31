//! Checks the generated demo seed against `generate.yml` and `docs/scenarios.md` §6-7,
//! without a database.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chrono::{DateTime, FixedOffset, NaiveDate, TimeDelta};
use chrono_tz::America::New_York;
use serde_json::Value;

use super::util::{decimal, money};
use super::{generate, schema_check};
use crate::SeedRows;

struct Seed {
    named: SeedRows,
    generated: SeedRows,
    spec: String,
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn seed() -> &'static Seed {
    static SEED: OnceLock<Seed> = OnceLock::new();
    SEED.get_or_init(|| {
        let dir = repo().join("seed/demo");
        let named = crate::named::load(&dir).unwrap();
        let spec = std::fs::read_to_string(dir.join("generate.yml")).unwrap();
        let generated = generate(&named, &spec).unwrap();
        Seed {
            named,
            generated,
            spec,
        }
    })
}

fn rows<'a>(rows: &'a SeedRows, table: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
    rows.iter().filter(move |(t, _)| t == table).map(|(_, r)| r)
}

/// Named and generated rows of a table, in load order.
fn all<'a>(table: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
    rows(&seed().named, table).chain(rows(&seed().generated, table))
}

fn s<'a>(row: &'a Value, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

fn cents(row: &Value, key: &str) -> i64 {
    (row[key].as_f64().unwrap() * 100.0).round() as i64
}

fn int(row: &Value, key: &str) -> i64 {
    row[key].as_i64().unwrap()
}

fn time(text: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(text).unwrap()
}

/// Store day of an RFC 3339 time written with the store offset.
fn day(text: &str) -> NaiveDate {
    time(text).date_naive()
}

fn d(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).unwrap()
}

fn spec() -> Value {
    serde_norway::from_str(&seed().spec).unwrap()
}

/// Sales per store day (`analytics.daily_sales`): orders not awaiting payment or cancelled.
fn daily_sales() -> BTreeMap<NaiveDate, (i64, i64)> {
    let mut by_day: BTreeMap<NaiveDate, (i64, i64)> = BTreeMap::new();
    for o in all("sales.sales_order") {
        if ["payment_due", "cancelled"].contains(&s(o, "order_status")) {
            continue;
        }
        let e = by_day.entry(day(s(o, "order_date"))).or_default();
        e.0 += cents(o, "total_payment_due_value");
        e.1 += 1;
    }
    by_day
}

/// Variant id -> (product id, size).
fn variants() -> BTreeMap<String, (String, String)> {
    all("catalog.product")
        .map(|v| {
            (
                s(v, "id").to_owned(),
                (s(v, "product_group_id").to_owned(), s(v, "size").to_owned()),
            )
        })
        .collect()
}

/// Order id -> order row.
fn orders() -> BTreeMap<String, &'static Value> {
    all("sales.sales_order")
        .map(|o| (s(o, "id").to_owned(), o))
        .collect()
}

#[test]
fn money_has_two_decimals_and_weights_one() {
    assert_eq!(money(8900).to_string(), "89.00");
    assert_eq!(money(5).to_string(), "0.05");
    assert_eq!(decimal(3400, 1).to_string(), "340.0");
    let order = rows(&seed().generated, "sales.sales_order").next().unwrap();
    let text = order["subtotal_value"].to_string();
    assert_eq!(text.split('.').nth(1).map(str::len), Some(2), "{text}");
}

#[test]
fn named_rows_load_in_file_table_and_row_order() {
    let named = &seed().named;
    assert_eq!(named[0].0, "platform.online_store");
    assert_eq!(s(&named[0].1, "id"), "store_northwind");
    let first = |t: &str| named.iter().position(|(x, _)| x == t).unwrap();
    assert!(first("platform.admin_user") < first("customer.person"));
    assert!(first("customer.person") < first("catalog.brand"));
    assert!(first("catalog.offer") < first("promotion.promotion"));
    assert!(first("sales.sales_order") < first("fulfillment.parcel_delivery"));
    // The 21 orders of the scenes, then the two waiting for their money (#29001, #29002).
    assert_eq!(rows(named, "sales.sales_order").count(), 23);
    let ids: Vec<&str> = rows(named, "sales.sales_order")
        .map(|o| s(o, "id"))
        .collect();
    assert_eq!(ids.first(), Some(&"ord_20105"));
    assert_eq!(ids[20], "ord_20431");
    assert_eq!(ids.last(), Some(&"ord_29002"));
}

#[test]
fn same_input_gives_the_same_rows() {
    let again = generate(&seed().named, &seed().spec).unwrap();
    assert_eq!(again.len(), seed().generated.len());
    assert!(again == seed().generated, "two runs differ");
}

#[test]
fn rows_fit_the_migrations_like_check_seed() {
    let tables = schema_check::parse(&repo().join("migrations"));
    assert!(tables.contains_key("sales.sales_order"));
    // The checker itself finds problems.
    let bad = vec![(
        "catalog.brand".to_owned(),
        serde_json::json!({"tenant_id": "store_nowhere", "id": "brn_x", "colour": "red"}),
    )];
    let found = schema_check::check(&tables, &bad).join("\n");
    assert!(found.contains("unknown column `colour`"), "{found}");
    assert!(found.contains("required column `name`"), "{found}");
    assert!(found.contains("`tenant_id` = `store_nowhere`"), "{found}");

    let mut everything = seed().named.clone();
    everything.extend(seed().generated.iter().cloned());
    let problems = schema_check::check(&tables, &everything);
    assert!(
        problems.is_empty(),
        "{} problems, first: {:#?}",
        problems.len(),
        &problems[..problems.len().min(20)]
    );
}

#[test]
fn every_generated_row_belongs_to_the_store() {
    for (table, row) in &seed().generated {
        assert_eq!(s(row, "tenant_id"), "store_northwind", "{table}: {row}");
    }
}

#[test]
fn times_carry_the_store_offset() {
    let mut checked = 0;
    for (table, row) in &seed().generated {
        for (key, v) in row.as_object().unwrap() {
            let Some(text) = v.as_str() else { continue };
            let Ok(t) = DateTime::parse_from_rfc3339(text) else {
                continue;
            };
            let store = t.with_timezone(&New_York);
            assert_eq!(
                store.format("%:z").to_string(),
                t.format("%:z").to_string(),
                "{table}.{key} = {text}"
            );
            checked += 1;
        }
    }
    assert!(checked > 100_000);
    let o = rows(&seed().generated, "sales.sales_order")
        .find(|o| day(s(o, "order_date")) == d(10, 31))
        .unwrap();
    assert!(s(o, "order_date").ends_with("-04:00"));
    let o = rows(&seed().generated, "sales.sales_order")
        .find(|o| day(s(o, "order_date")) == d(11, 2))
        .unwrap();
    assert!(s(o, "order_date").ends_with("-05:00"));
}

/// Product id -> total stock.
fn stock() -> BTreeMap<String, i64> {
    let variants = variants();
    let mut stock: BTreeMap<String, i64> = BTreeMap::new();
    for o in all("catalog.offer") {
        let (product, _) = &variants[s(o, "product_id")];
        *stock.entry(product.clone()).or_default() += int(o, "inventory_level");
    }
    stock
}

/// Product id -> primary category id.
fn primary_category() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for r in all("merchandising.product_group_category") {
        if r["is_primary"].as_bool() == Some(true) {
            let old = out.insert(
                s(r, "product_group_id").to_owned(),
                s(r, "category_id").to_owned(),
            );
            assert!(old.is_none(), "two primary categories: {r}");
        }
    }
    out
}

#[test]
fn summer_outerwear_has_42_products_and_40_with_ten_units() {
    let primary = primary_category();
    let stock = stock();
    let summer: Vec<&String> = primary
        .iter()
        .filter(|(_, c)| *c == "cat_summer_outerwear")
        .map(|(p, _)| p)
        .collect();
    assert_eq!(summer.len(), 42);
    let generated = summer
        .iter()
        .filter(|p| p.starts_with("prd_summer_outerwear_"))
        .count();
    assert_eq!(generated, 35);
    let enough: Vec<_> = summer.iter().filter(|p| stock[**p] >= 10).collect();
    assert_eq!(enough.len(), 40);
    let short: BTreeSet<&str> = summer
        .iter()
        .filter(|p| stock[**p] < 10)
        .map(|p| p.as_str())
        .collect();
    assert_eq!(short, BTreeSet::from(["prd_chore_coat", "prd_rain_parka"]));
    for p in summer
        .iter()
        .filter(|p| p.starts_with("prd_summer_outerwear_"))
    {
        assert!((12..=120).contains(&stock[*p]), "{p}: {}", stock[*p]);
    }
}

#[test]
fn about_400_products_in_the_24_standard_categories() {
    let products: Vec<&Value> = all("catalog.product_group").collect();
    assert_eq!(products.len(), 400);
    let standard: BTreeSet<&str> = all("merchandising.defined_term")
        .filter(|t| s(t, "in_defined_term_set") == "standard")
        .map(|t| s(t, "id"))
        .collect();
    assert_eq!(standard.len(), 24);
    let primary = primary_category();
    let mut used = BTreeSet::new();
    for p in &products {
        let c = primary.get(s(p, "id")).unwrap();
        assert!(standard.contains(c.as_str()), "{c}");
        used.insert(c.as_str());
    }
    assert_eq!(used, standard);
    // 20% of generated summer outerwear and outerwear are also on a display shelf.
    let display: Vec<&Value> = rows(&seed().generated, "merchandising.product_group_category")
        .filter(|r| r["is_primary"].as_bool() == Some(false))
        .collect();
    assert_eq!(display.len(), 7 + 3);
    let travel: Vec<i64> = all("merchandising.product_group_category")
        .filter(|r| s(r, "category_id") == "cat_travel")
        .map(|r| int(r, "position"))
        .collect();
    assert_eq!(travel, (1..=14).collect::<Vec<_>>());
}

#[test]
fn generated_prices_costs_skus_and_emails() {
    let variants = variants();
    let spec = spec();
    let mut skus = BTreeSet::new();
    for v in all("catalog.product") {
        assert!(
            skus.insert(s(v, "sku").to_owned()),
            "duplicate sku {}",
            s(v, "sku")
        );
    }
    for o in rows(&seed().generated, "catalog.offer") {
        let price = cents(o, "price");
        let cost = cents(o, "cost_value");
        assert_eq!(price % 1000, 900, "x9.00 price: {o}");
        assert!(cost * 115 <= price * 100, "below cost + 15%: {o}");
        assert_eq!(int(o, "safety_stock"), 2);
        let (product, _) = &variants[s(o, "product_id")];
        let term = product
            .trim_start_matches("prd_")
            .rsplit_once('_')
            .unwrap()
            .0;
        let c = if term == "summer_outerwear" {
            &spec["catalog"]["summer_outerwear"]
        } else {
            &spec["catalog"]["other_categories"][term]
        };
        let (lo, hi) = if term == "summer_outerwear" {
            (
                c["list_price"]["min"].as_i64(),
                c["list_price"]["max"].as_i64(),
            )
        } else {
            (c["price"][0].as_i64(), c["price"][1].as_i64())
        };
        assert!(
            (lo.unwrap() * 100..=hi.unwrap() * 100).contains(&price),
            "{o}"
        );
        if term == "summer_outerwear" {
            assert_eq!(cost, (price * 45 + 50) / 100, "cost = price x 0.45: {o}");
        }
        let available = if int(o, "inventory_level") > 0 {
            "in_stock"
        } else {
            "out_of_stock"
        };
        assert_eq!(s(o, "availability"), available);
    }
    let mut emails = BTreeSet::new();
    for p in all("customer.person") {
        assert!(
            emails.insert(s(p, "email")),
            "duplicate email {}",
            s(p, "email")
        );
    }
    let mut numbers = BTreeSet::new();
    for m in all("customer.program_membership") {
        assert!(numbers.insert(s(m, "membership_number")));
    }
    // Every generated knitwear offer has its autumn knit price at 15% off.
    let ups: BTreeMap<&str, &Value> = rows(&seed().generated, "promotion.unit_price_specification")
        .map(|u| (s(u, "offer_id"), u))
        .collect();
    let knit_offers: Vec<&Value> = rows(&seed().generated, "catalog.offer")
        .filter(|o| s(o, "id").starts_with("off_knitwear_"))
        .collect();
    assert_eq!(ups.len(), knit_offers.len());
    for o in knit_offers {
        let u = ups[s(o, "id")];
        assert_eq!(cents(u, "price"), (cents(o, "price") * 85 + 50) / 100);
        assert_eq!(s(u, "promotion_id"), "promo_autumn_knit");
        assert_eq!(s(u, "valid_from"), "2026-11-01T00:00:00-04:00");
        assert_eq!(s(u, "valid_through"), "2026-11-15T23:59:00-05:00");
        assert!(u["valid_for_member_tier"].is_null());
    }
}

#[test]
fn tier_counts_include_the_named_people() {
    let mut tiers: BTreeMap<&str, usize> = BTreeMap::new();
    for m in all("customer.program_membership") {
        *tiers.entry(s(m, "tier_id")).or_default() += 1;
    }
    assert_eq!(tiers["vip"], 1284);
    assert_eq!(tiers["gold"], 2900);
    assert_eq!(tiers["member"], 9816);
    assert_eq!(all("customer.person").count(), 1284 + 2900 + 9816);
    let jamie = all("customer.program_membership")
        .find(|m| s(m, "customer_id") == "cus_jamie")
        .unwrap();
    assert_eq!(s(jamie, "tier_id"), "vip");
    for p in rows(&seed().generated, "customer.person") {
        assert_eq!(s(p, "member_state"), "active");
    }
    // One home address each, and a November Gold coupon for every generated Gold member.
    let people = rows(&seed().generated, "customer.person").count();
    assert_eq!(
        rows(&seed().generated, "customer.postal_address").count(),
        people
    );
    let gold: BTreeSet<&str> = rows(&seed().generated, "customer.program_membership")
        .filter(|m| s(m, "tier_id") == "gold")
        .map(|m| s(m, "customer_id"))
        .collect();
    let coupons: BTreeSet<&str> = rows(&seed().generated, "promotion.member_coupon")
        .map(|c| {
            assert_eq!(s(c, "coupon_id"), "cpn_gold_10");
            assert_eq!(s(c, "coupon_status"), "available");
            assert_eq!(s(c, "owned_from"), "2026-11-01T00:00:00-04:00");
            assert_eq!(s(c, "owned_through"), "2026-11-30T23:59:00-05:00");
            s(c, "customer_id")
        })
        .collect();
    assert_eq!(coupons, gold);
    assert_eq!(gold.len(), 2897);
    // tier_since: registration for members, the first of a month at 03:00 for gold and vip.
    let registered: BTreeMap<&str, &str> = rows(&seed().generated, "customer.person")
        .map(|p| (s(p, "id"), s(p, "registered_at")))
        .collect();
    for m in rows(&seed().generated, "customer.program_membership") {
        let since = s(m, "tier_since");
        let reg = registered[s(m, "customer_id")];
        if s(m, "tier_id") == "member" {
            assert_eq!(since, reg);
        } else {
            assert!(time(since) > time(reg), "{m}");
            assert!(since[8..16] == *"01T03:00", "{since}");
            assert!(day(since) <= d(11, 1));
        }
    }
}

#[test]
fn daily_sales_match_the_spec_targets() {
    let sales = daily_sales();
    let spec = spec();
    let targets = spec["orders"]["daily_sales"]["targets"]
        .as_object()
        .unwrap();
    for (date, pair) in targets {
        let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
        let want = (
            (pair[0].as_f64().unwrap() * 100.0).round() as i64,
            pair[1].as_i64().unwrap(),
        );
        assert_eq!(sales[&date], want, "{date}");
    }
    // B3: Thursday Nov 5 against the Thursday average.
    assert_eq!(sales[&d(11, 5)], (1_824_000, 201));
    let thursdays = [d(10, 8), d(10, 15), d(10, 22)];
    let sum: i64 = thursdays.iter().map(|t| sales[t].0).sum();
    assert_eq!(sum, 6_900_000);
    let four = [d(10, 8), d(10, 15), d(10, 22), d(10, 29)];
    assert_eq!(four.iter().map(|t| sales[t].0).sum::<i64>() / 4, 2_290_000);
    assert_eq!(four.iter().map(|t| sales[t].1).sum::<i64>() / 4, 247);
    // Other days of the window: 150-260 orders at 88-96 USD each.
    let mut date = d(10, 1);
    while date <= d(11, 5) {
        if !targets.contains_key(&date.format("%Y-%m-%d").to_string()) {
            let (rev, n) = sales[&date];
            assert!((150..=260).contains(&n), "{date}: {n} orders");
            assert!(
                rev >= 8800 * n - 100 && rev <= 9600 * n + 100,
                "{date}: {rev} / {n}"
            );
        }
        date += TimeDelta::days(1);
    }
    // Before Oct 1 only named orders exist.
    for o in rows(&seed().generated, "sales.sales_order") {
        assert!(day(s(o, "order_date")) >= d(10, 1));
        assert!(day(s(o, "order_date")) <= d(11, 5));
    }
}

#[test]
fn orders_add_up_and_follow_the_order_rules() {
    let g = &seed().generated;
    let variants = variants();
    let primary = primary_category();
    let old_price: BTreeMap<&str, i64> = BTreeMap::from([
        ("prd_breton_tee", 1900),
        ("prd_canvas_tote", 2400),
        ("prd_ribbed_beanie", 1400),
        ("prd_hiking_socks", 1200),
    ]);
    let list: BTreeMap<&str, i64> = all("catalog.offer")
        .map(|o| (s(o, "product_id"), cents(o, "price")))
        .collect();
    let mut items: BTreeMap<&str, Vec<&Value>> = BTreeMap::new();
    for i in rows(g, "sales.order_item") {
        items.entry(s(i, "order_id")).or_default().push(i);
    }
    let payments: BTreeMap<&str, &Value> = rows(g, "sales.payment")
        .map(|p| (s(p, "order_id"), p))
        .collect();
    let deliveries: BTreeMap<&str, &Value> = rows(g, "fulfillment.parcel_delivery")
        .map(|p| (s(p, "order_id"), p))
        .collect();
    let mut events: BTreeMap<&str, Vec<&Value>> = BTreeMap::new();
    for e in rows(g, "sales.order_event") {
        events.entry(s(e, "order_id")).or_default().push(e);
    }
    let mut lines_hist: BTreeMap<usize, usize> = BTreeMap::new();
    for o in rows(g, "sales.sales_order") {
        let id = s(o, "id");
        let date = day(s(o, "order_date"));
        let its = &items[id];
        *lines_hist.entry(its.len()).or_default() += 1;
        let mut subtotal = 0;
        let mut discount = 0;
        for (k, i) in its.iter().enumerate() {
            assert_eq!(int(i, "position"), k as i64 + 1);
            let qty = int(i, "order_quantity");
            assert!((1..=2).contains(&qty));
            let price = cents(i, "price");
            let (product, _) = &variants[s(i, "product_id")];
            let expected = old_price
                .get(product.as_str())
                .copied()
                .unwrap_or(list[s(i, "product_id")]);
            assert_eq!(price, expected, "{i}");
            let disc = cents(i, "discount_value");
            let linen = ["prd_linen_shirt", "prd_camp_overshirt"].contains(&product.as_str())
                && date <= d(10, 20);
            let knit = primary[product] == "cat_knitwear" && date >= d(11, 1);
            match s(i, "promotion_id") {
                "promo_linen_clearance" => {
                    assert!(linen);
                    assert_eq!(disc, (price * qty * 20 + 50) / 100);
                }
                "promo_autumn_knit" => {
                    assert!(knit);
                    assert_eq!(disc, (price * qty * 15 + 50) / 100);
                }
                "" => {
                    assert!(!linen && !knit, "{i}");
                    assert_eq!(disc, 0);
                }
                other => panic!("unexpected promotion {other}"),
            }
            assert_eq!(s(i, "order_item_status"), s(o, "order_status"));
            subtotal += price * qty;
            discount += disc;
        }
        assert_eq!(cents(o, "subtotal_value"), subtotal);
        assert_eq!(cents(o, "discount_value"), discount);
        let shipping = if subtotal < 8000 { 600 } else { 0 };
        assert_eq!(cents(o, "shipping_value"), shipping);
        assert_eq!(
            cents(o, "total_payment_due_value"),
            subtotal - discount + shipping
        );
        assert!(o.get("coupon_id").is_none() && o.get("discount_code").is_none());

        let p = payments[id];
        assert_eq!(
            cents(p, "amount_value"),
            cents(o, "total_payment_due_value")
        );
        assert_eq!(s(p, "payment_method"), "credit_card");
        assert_eq!(s(p, "payment_status"), "payment_complete");
        assert_eq!(
            time(s(p, "paid_at")) - time(s(o, "order_date")),
            TimeDelta::minutes(1)
        );

        let status = s(o, "order_status");
        let want = if date < d(11, 1) {
            "delivered"
        } else if date <= d(11, 3) {
            "in_transit"
        } else {
            "processing"
        };
        assert_eq!(status, want, "{id}");
        let ev = &events[id];
        match status {
            "delivered" => {
                let dl = deliveries[id];
                assert_eq!(s(dl, "delivery_state"), "delivered");
                assert_eq!(s(dl, "carrier"), "UPS");
                let days = (day(s(dl, "delivered_at")) - date).num_days();
                assert!((2..=4).contains(&days), "{dl}");
                assert!(time(s(dl, "shipped_at")) < time(s(dl, "delivered_at")));
                assert_eq!(ev.len(), 3);
                assert_eq!(s(ev[2], "to_state"), "delivered");
            }
            "in_transit" => {
                let dl = deliveries[id];
                assert_eq!(s(dl, "delivery_state"), "in_transit");
                assert!(dl.get("delivered_at").is_none());
                assert_eq!(ev.len(), 2);
            }
            _ => {
                assert!(!deliveries.contains_key(id));
                assert_eq!(ev.len(), 1);
            }
        }
        if let Some(dl) = deliveries.get(id) {
            let address = format!("adr_m{}", &s(o, "customer_id")["cus_m".len()..]);
            assert_eq!(s(dl, "delivery_address_id"), address);
        }
    }
    // Roughly the spec's line mix (62% / 28% / 10%).
    let n: usize = lines_hist.values().sum();
    let one = lines_hist[&1] * 100 / n;
    assert!((50..=75).contains(&one), "{lines_hist:?}");
}

#[test]
fn orders_are_numbered_in_date_order_skipping_named_numbers() {
    let named: BTreeSet<i64> = rows(&seed().named, "sales.sales_order")
        .map(|o| s(o, "order_number").parse().unwrap())
        .collect();
    let mut prev: Option<(i64, DateTime<FixedOffset>)> = None;
    let mut first = None;
    for o in rows(&seed().generated, "sales.sales_order") {
        let n: i64 = s(o, "order_number").parse().unwrap();
        assert_eq!(s(o, "id"), format!("ord_{n}"));
        assert!(!named.contains(&n));
        let t = time(s(o, "order_date"));
        if let Some((pn, pt)) = prev {
            assert!(n > pn);
            assert!(t >= pt, "#{n} is placed before #{pn}");
        }
        first.get_or_insert(n);
        prev = Some((n, t));
    }
    assert_eq!(first, Some(20001));
}

#[test]
fn generated_ids_follow_their_patterns() {
    let g = &seed().generated;
    let digits = |x: &str, n: usize| x.len() == n && x.bytes().all(|b| b.is_ascii_digit());
    let word = |x: &str| {
        !x.is_empty()
            && x.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    let terms: BTreeSet<String> = all("merchandising.defined_term")
        .map(|t| s(t, "term_code").to_owned())
        .collect();
    let products: BTreeMap<&str, &Value> = rows(g, "catalog.product_group")
        .map(|p| (s(p, "id"), p))
        .collect();
    for id in products.keys() {
        let (term, nn) = id.trim_start_matches("prd_").rsplit_once('_').unwrap();
        assert!(
            id.starts_with("prd_") && terms.contains(term) && digits(nn, 2),
            "{id}"
        );
    }
    for v in rows(g, "catalog.product") {
        let id = s(v, "id");
        let product = s(v, "product_group_id");
        let suffix = product.trim_start_matches("prd_");
        let mut want = format!("var_{suffix}");
        if let Some(size) = v.get("size").and_then(Value::as_str) {
            want.push_str(&format!("_{}", size.to_lowercase()));
        }
        want.push_str(&format!(
            "_{}",
            s(v, "color").to_lowercase().replace(' ', "_")
        ));
        assert_eq!(id, want);
        let (term, nn) = suffix.rsplit_once('_').unwrap();
        let initials: String = if term.contains('_') {
            term.split('_').map(|w| &w[..1]).collect()
        } else {
            term.to_owned()
        };
        let mut sku = format!("{initials}-{nn}-{}", s(v, "color").to_lowercase());
        if let Some(size) = v.get("size").and_then(Value::as_str) {
            sku.push_str(&format!("-{}", size.to_lowercase()));
        }
        assert_eq!(s(v, "sku"), sku);
    }
    assert!(rows(g, "catalog.product").any(|v| s(v, "sku").starts_with("so-07-")));
    for o in rows(g, "catalog.offer") {
        assert_eq!(
            s(o, "id").trim_start_matches("off_"),
            s(o, "product_id").trim_start_matches("var_")
        );
    }
    for u in rows(g, "promotion.unit_price_specification") {
        let suffix = s(u, "offer_id").trim_start_matches("off_");
        assert_eq!(s(u, "id"), format!("ups_{suffix}_autumn_knit"));
    }
    for p in rows(g, "customer.person") {
        let n = s(p, "id").trim_start_matches("cus_m");
        assert!(s(p, "id").starts_with("cus_m") && digits(n, 6));
        assert_eq!(s(p, "email"), format!("m{n}@example.com"));
    }
    for a in rows(g, "customer.postal_address") {
        assert_eq!(
            s(a, "id").trim_start_matches("adr_m"),
            s(a, "customer_id").trim_start_matches("cus_m")
        );
        assert!(["NY", "NJ", "CT", "MA", "RI", "VT", "ME", "PA"].contains(&s(a, "address_region")));
    }
    for m in rows(g, "customer.program_membership") {
        let n = s(m, "membership_number");
        assert!(n.starts_with("NW-1") && digits(&n[4..], 5), "{n}");
        assert_eq!(&n[4..], &s(m, "customer_id")["cus_m0".len()..]);
    }
    for c in rows(g, "promotion.member_coupon") {
        let n = s(c, "customer_id").trim_start_matches("cus_m");
        assert_eq!(s(c, "id"), format!("mcp_m{n}_gold_10_202611"));
    }
    let orders = orders();
    for i in rows(g, "sales.order_item") {
        let n = s(i, "order_id").trim_start_matches("ord_");
        assert_eq!(s(i, "id"), format!("oln_{n}_{}", int(i, "position")));
        assert!(orders.contains_key(s(i, "order_id")));
    }
    for p in rows(g, "sales.payment") {
        let n = s(p, "order_id").trim_start_matches("ord_");
        assert_eq!(s(p, "id"), format!("pay_{n}"));
    }
    for p in rows(g, "fulfillment.parcel_delivery") {
        let n = s(p, "order_id").trim_start_matches("ord_");
        assert_eq!(s(p, "id"), format!("dlv_{n}"));
    }
    for (table, row) in g {
        if let Some(id) = row.get("id").and_then(Value::as_str) {
            let (prefix, rest) = id.split_once('_').unwrap();
            assert!(word(rest), "{table}: {id}");
            assert!(
                [
                    "prd", "var", "off", "ups", "cus", "adr", "mcp", "ord", "oln", "pay", "dlv"
                ]
                .contains(&prefix),
                "{table}: {id}"
            );
        }
    }
}

#[test]
fn never_touch_customers_get_nothing_generated() {
    let spec = spec();
    let never: BTreeSet<&str> = spec["never_touch"]["customers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(never.len(), 13);
    fn scan<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
        match v {
            Value::String(s) => out.push(s),
            Value::Array(a) => a.iter().for_each(|x| scan(x, out)),
            Value::Object(m) => m.values().for_each(|x| scan(x, out)),
            _ => {}
        }
    }
    for (table, row) in &seed().generated {
        let mut texts = Vec::new();
        scan(row, &mut texts);
        for t in texts {
            assert!(!never.contains(t), "{table} references {t}: {row}");
        }
        if let Some(c) = row.get("customer_id").and_then(Value::as_str) {
            assert!(c.starts_with("cus_m"), "{table}: {c}");
        }
    }
    // Members buy only after they register.
    let registered: BTreeMap<&str, DateTime<FixedOffset>> =
        rows(&seed().generated, "customer.person")
            .map(|p| (s(p, "id"), time(s(p, "registered_at"))))
            .collect();
    for o in rows(&seed().generated, "sales.sales_order") {
        assert!(
            registered[s(o, "customer_id")] < time(s(o, "order_date")),
            "{o}"
        );
    }
}

/// Units of each variant sold on the given store days (named and generated orders).
fn units_sold(days: &[NaiveDate]) -> BTreeMap<String, i64> {
    let orders = orders();
    let mut out: BTreeMap<String, i64> = BTreeMap::new();
    for i in all("sales.order_item") {
        let o = orders[s(i, "order_id")];
        if days.contains(&day(s(o, "order_date"))) {
            *out.entry(s(i, "product_id").to_owned()).or_default() += int(i, "order_quantity");
        }
    }
    out
}

fn last_seven_days() -> Vec<NaiveDate> {
    (0..7).map(|k| d(10, 30) + TimeDelta::days(k)).collect()
}

#[test]
fn linen_shirt_sells_61_units_in_the_last_seven_days() {
    let sold = units_sold(&last_seven_days());
    let linen: BTreeMap<&str, i64> = sold
        .iter()
        .filter_map(|(v, n)| {
            v.strip_prefix("var_linen_shirt_")
                .and_then(|r| r.strip_suffix("_white"))
                .map(|size| (size, *n))
        })
        .collect();
    assert_eq!(
        linen,
        BTreeMap::from([("l", 14), ("m", 19), ("s", 16), ("xl", 6), ("xs", 6)])
    );
    let stock: i64 = all("catalog.offer")
        .filter(|o| s(o, "id").starts_with("off_linen_shirt_"))
        .map(|o| int(o, "inventory_level"))
        .sum();
    assert_eq!(stock, 38);
}

#[test]
fn only_the_linen_shirt_trips_the_stock_out_forecast() {
    let sold = units_sold(&last_seven_days());
    let inventory: BTreeMap<&str, i64> = all("catalog.offer")
        .map(|o| (s(o, "product_id"), int(o, "inventory_level")))
        .collect();
    let mut tripped = BTreeSet::new();
    for (v, n) in &sold {
        // Days of stock = inventory / (sold / 7); the watcher trips at 5 days or less.
        if *n > 0 && inventory[v.as_str()] * 7 <= 5 * n {
            tripped.insert(v.as_str());
        }
    }
    assert!(!tripped.is_empty());
    for v in &tripped {
        assert!(
            v.starts_with("var_linen_shirt_"),
            "{v} trips stock_out_forecast"
        );
    }
    let linen_sold: i64 = sold
        .iter()
        .filter(|(v, _)| v.starts_with("var_linen_shirt_"))
        .map(|(_, n)| n)
        .sum();
    assert_eq!(
        38 * 7 * 10 / linen_sold,
        43,
        "about 4.4 days of linen shirt stock"
    );
}

#[test]
fn b3_sold_out_sizes_sell_on_compare_thursdays_and_not_on_nov_5() {
    let sold_out: BTreeSet<String> = all("catalog.offer")
        .filter(|o| o.get("sold_out_at").is_some_and(|t| !t.is_null()))
        .map(|o| s(o, "product_id").to_owned())
        .collect();
    assert_eq!(sold_out.len(), 6);
    let orders = orders();
    let mut chore = 0;
    let mut parka_denim = 0;
    let mut revenue = 0;
    let thursdays = [d(10, 8), d(10, 15), d(10, 22), d(10, 29)];
    for i in rows(&seed().generated, "sales.order_item") {
        if !sold_out.contains(s(i, "product_id")) {
            continue;
        }
        let o = orders[s(i, "order_id")];
        let date = day(s(o, "order_date"));
        assert!(date < d(10, 30), "sold-out size sold on {date}: {i}");
        if thursdays.contains(&date) {
            assert_eq!(cents(i, "discount_value"), 0);
            let qty = int(i, "order_quantity");
            revenue += cents(i, "price") * qty;
            if s(i, "product_id").starts_with("var_chore_coat_") {
                chore += qty;
            } else {
                parka_denim += qty;
            }
        }
    }
    assert_eq!((chore, parka_denim), (37, 47));
    assert_eq!(revenue, 1_080_000);
}

#[test]
fn b3_weekly_email_orders_average_1300_on_compare_thursdays_and_none_on_nov_5() {
    let campaigns: BTreeMap<&str, &Value> =
        all("messaging.campaign").map(|c| (s(c, "id"), c)).collect();
    let sold_out: BTreeSet<&str> = all("catalog.offer")
        .filter(|o| o.get("sold_out_at").is_some_and(|t| !t.is_null()))
        .map(|o| s(o, "product_id"))
        .collect();
    let mut with_sold_out: BTreeSet<&str> = BTreeSet::new();
    for i in rows(&seed().generated, "sales.order_item") {
        if sold_out.contains(s(i, "product_id")) {
            with_sold_out.insert(s(i, "order_id"));
        }
    }
    let mut revenue: BTreeMap<&str, i64> = BTreeMap::new();
    for o in all("sales.sales_order") {
        let Some(c) = o.get("campaign_id").and_then(Value::as_str) else {
            continue;
        };
        let sent = time(s(campaigns[c], "date_sent"));
        let at = time(s(o, "order_date"));
        assert!(at >= sent && at.date_naive() == sent.date_naive(), "{o}");
        assert!(
            !with_sold_out.contains(s(o, "id")),
            "{o} holds a sold-out size"
        );
        *revenue.entry(c).or_default() += cents(o, "total_payment_due_value");
    }
    assert_eq!(
        revenue.into_iter().collect::<Vec<_>>(),
        [
            ("cmp_weekly_1008", 124_000),
            ("cmp_weekly_1015", 141_000),
            ("cmp_weekly_1022", 118_000),
            ("cmp_weekly_1029", 137_000),
        ],
        "5,200.00 over the four compare Thursdays: 1,300.00 on an average Thursday"
    );
    let failed = campaigns["cmp_weekly_1105"];
    assert_eq!(s(failed, "campaign_status"), "failed");
    assert!(failed.get("date_sent").is_none_or(Value::is_null));
    assert_eq!(day(s(failed, "scheduled_time")), d(11, 5));
}

#[test]
fn subset_sums_use_each_value_once_and_prefer_early_ones() {
    use super::orders::subset_sum;
    assert_eq!(subset_sum(&[500, 300, 200, 700], 1000), Some(vec![0, 1, 2]));
    assert_eq!(subset_sum(&[400, 400, 300], 800), Some(vec![0, 1]));
    assert_eq!(subset_sum(&[400, 300], 800), None);
    assert_eq!(subset_sum(&[], 0), Some(vec![]));
}

#[test]
fn rows_reference_only_earlier_rows_in_table_order() {
    let order: Vec<&str> = {
        let mut seen = Vec::new();
        for (t, _) in &seed().generated {
            if seen.last() != Some(&t.as_str()) {
                assert!(!seen.contains(&t.as_str()), "{t} appears in two runs");
                seen.push(t.as_str());
            }
        }
        seen
    };
    assert_eq!(
        order,
        [
            "catalog.product_group",
            "catalog.product",
            "catalog.offer",
            "merchandising.product_group_category",
            "promotion.unit_price_specification",
            "customer.person",
            "customer.postal_address",
            "customer.program_membership",
            "promotion.member_coupon",
            "sales.sales_order",
            "sales.order_item",
            "sales.payment",
            "sales.order_event",
            "fulfillment.parcel_delivery",
        ]
    );
}

#[test]
fn renames_change_only_the_renamed_products_names() {
    let seed = seed();
    let mut spec: Value = serde_norway::from_str(&seed.spec).unwrap();
    let renamed: Vec<String> = spec["catalog"]["renames"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    assert_eq!(renamed.len(), 6);
    spec["catalog"].as_object_mut().unwrap().remove("renames");
    let drawn = generate(&seed.named, &serde_norway::to_string(&spec).unwrap()).unwrap();

    // Same rows in the same order; a row may differ only if it belongs to a renamed product,
    // and then only in the fields that follow the name.
    assert_eq!(seed.generated.len(), drawn.len());
    let follows_name = ["name", "description", "material", "image"];
    let of_renamed = |row: &Value| {
        renamed.iter().any(|id| {
            let variants = format!("var_{}_", id.trim_start_matches("prd_"));
            ["id", "product_group_id", "product_id"]
                .iter()
                .any(|k| s(row, k) == id || s(row, k).starts_with(&variants))
        })
    };
    let mut changed = 0;
    for ((table, now), (_, before)) in seed.generated.iter().zip(&drawn) {
        if now == before {
            continue;
        }
        changed += 1;
        assert!(
            of_renamed(now),
            "{table} row changed but is not a renamed product's: {now}"
        );
        for (k, v) in now.as_object().unwrap() {
            if before.get(k) != Some(v) {
                assert!(
                    follows_name.contains(&k.as_str()),
                    "{table}.{k} changed: {now}"
                );
            }
        }
    }
    assert!(changed > 0);
    let names: BTreeMap<&str, &str> = rows(&seed.generated, "catalog.product_group")
        .map(|r| (s(r, "id"), s(r, "name")))
        .collect();
    assert_eq!(names["prd_hats_03"], "Fleece bucket cap");
    assert_eq!(names["prd_underwear_09"], "Bamboo rayon bralette");
}
