//! What the generator needs to know about the named rows.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDate};
use chrono_tz::Tz;
use serde_json::Value;

use super::util::{basis_points, cents, parse_time};
use crate::SeedRows;

#[derive(Debug)]
pub(crate) struct NamedVariant {
    pub id: String,
    pub product_id: String,
    pub size: Option<String>,
    pub price: i64,
    pub inventory: i64,
    pub sold_out_at: Option<DateTime<Tz>>,
    /// The list price before a committed price change, and when it changed.
    pub old_price: Option<(i64, DateTime<Tz>)>,
}

/// An `item_price` promotion with a percentage: it discounts order lines while it runs.
#[derive(Debug)]
pub(crate) struct ItemPromotion {
    pub id: String,
    pub percent_bp: i64,
    pub from: DateTime<Tz>,
    pub through: Option<DateTime<Tz>>,
    pub products: BTreeSet<String>,
    pub categories: BTreeSet<String>,
}

#[derive(Debug)]
pub(crate) struct Named {
    pub tz: Tz,
    pub currency: String,
    /// Product id -> (name, primary category id).
    pub products: BTreeMap<String, (String, String)>,
    pub variants: Vec<NamedVariant>,
    pub skus: BTreeSet<String>,
    /// term_code -> category id, for every category.
    pub categories: BTreeMap<String, String>,
    pub standard_categories: BTreeSet<String>,
    /// Display category id -> highest position used by named products.
    pub last_position: BTreeMap<String, i64>,
    pub promotions: Vec<ItemPromotion>,
    pub program_id: String,
    pub customers: BTreeSet<String>,
    pub order_numbers: BTreeSet<i64>,
    /// Store day -> (orders, revenue in cents) of named orders that count as sales.
    pub sales_by_day: BTreeMap<NaiveDate, (i64, i64)>,
    /// Campaign id -> when its messages went out (`date_sent`), None when they did not.
    pub campaigns: BTreeMap<String, Option<DateTime<Tz>>>,
}

fn s(row: &Value, key: &str) -> Option<String> {
    row.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn rows<'a>(named: &'a SeedRows, table: &'a str) -> impl Iterator<Item = &'a Value> + 'a {
    named
        .iter()
        .filter(move |(t, _)| t == table)
        .map(|(_, r)| r)
}

/// Order statuses that are not sales (`analytics.daily_sales`).
pub(crate) const NOT_SALES: &[&str] = &["payment_due", "cancelled"];

impl Named {
    pub(crate) fn read(named: &SeedRows, store: &str) -> Result<Named, String> {
        let store_row = rows(named, "platform.online_store")
            .find(|r| s(r, "id").as_deref() == Some(store))
            .ok_or_else(|| format!("named rows: no platform.online_store `{store}`"))?;
        let tz: Tz = s(store_row, "time_zone")
            .ok_or("named rows: the store has no time_zone")?
            .parse()
            .map_err(|_| "named rows: the store time_zone is not an IANA zone".to_owned())?;
        let currency = s(store_row, "currency").ok_or("named rows: the store has no currency")?;

        let mut categories = BTreeMap::new();
        let mut standard_categories = BTreeSet::new();
        for r in rows(named, "merchandising.defined_term") {
            if let (Some(id), Some(code)) = (s(r, "id"), s(r, "term_code")) {
                if s(r, "in_defined_term_set").as_deref() == Some("standard") {
                    standard_categories.insert(id.clone());
                }
                categories.insert(code, id);
            }
        }
        let mut primary = BTreeMap::new();
        let mut last_position: BTreeMap<String, i64> = BTreeMap::new();
        for r in rows(named, "merchandising.product_group_category") {
            let (Some(product), Some(category)) = (s(r, "product_group_id"), s(r, "category_id"))
            else {
                continue;
            };
            if r.get("is_primary").and_then(Value::as_bool) == Some(true) {
                primary.insert(product, category.clone());
            }
            if let Some(p) = r.get("position").and_then(Value::as_i64) {
                let last = last_position.entry(category).or_default();
                *last = (*last).max(p);
            }
        }
        let mut products = BTreeMap::new();
        for r in rows(named, "catalog.product_group") {
            let id = s(r, "id").ok_or("named rows: a product_group has no id")?;
            let name = s(r, "name").unwrap_or_default();
            let category = primary.get(&id).cloned().unwrap_or_default();
            products.insert(id, (name, category));
        }

        // Committed list price changes: the offer sold at `from` until `committed_at`.
        let mut old_prices: BTreeMap<String, (i64, DateTime<Tz>)> = BTreeMap::new();
        for r in rows(named, "platform.preview") {
            if s(r, "preview_state").as_deref() != Some("committed") {
                continue;
            }
            let Some(when) = s(r, "committed_at") else {
                continue;
            };
            let when = parse_time(tz, &when)?;
            for c in r
                .get("changes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if s(c, "entity").as_deref() != Some("offer") {
                    continue;
                }
                if let (Some(offer), Some(from)) = (
                    s(c, "id"),
                    c.get("from").and_then(|f| f.get("price")).and_then(cents),
                ) {
                    old_prices.insert(offer, (from, when));
                }
            }
        }

        let mut offers: BTreeMap<String, &Value> = BTreeMap::new();
        for r in rows(named, "catalog.offer") {
            if let Some(p) = s(r, "product_id") {
                offers.insert(p, r);
            }
        }
        let mut variants = Vec::new();
        let mut skus = BTreeSet::new();
        for r in rows(named, "catalog.product") {
            let id = s(r, "id").ok_or("named rows: a product has no id")?;
            if let Some(sku) = s(r, "sku") {
                skus.insert(sku);
            }
            let offer = offers
                .get(&id)
                .ok_or_else(|| format!("named rows: variant `{id}` has no offer"))?;
            let offer_id = s(offer, "id").unwrap_or_default();
            variants.push(NamedVariant {
                product_id: s(r, "product_group_id").unwrap_or_default(),
                size: s(r, "size"),
                price: offer
                    .get("price")
                    .and_then(cents)
                    .ok_or_else(|| format!("named rows: offer of `{id}` has no price"))?,
                inventory: offer
                    .get("inventory_level")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
                sold_out_at: s(offer, "sold_out_at")
                    .map(|t| parse_time(tz, &t))
                    .transpose()?,
                old_price: old_prices.get(&offer_id).copied(),
                id,
            });
        }

        let mut promotions = Vec::new();
        for r in rows(named, "promotion.promotion") {
            let percent = r.get("percent_off").and_then(Value::as_f64);
            if s(r, "discount_layer").as_deref() != Some("item_price") {
                continue;
            }
            let (Some(id), Some(percent), Some(from)) = (s(r, "id"), percent, s(r, "valid_from"))
            else {
                continue;
            };
            let applies = r.get("applies_to").cloned().unwrap_or(Value::Null);
            let list = |key: &str| -> BTreeSet<String> {
                applies
                    .get(key)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            };
            promotions.push(ItemPromotion {
                percent_bp: basis_points(percent / 100.0),
                from: parse_time(tz, &from)?,
                through: s(r, "valid_through")
                    .map(|t| parse_time(tz, &t))
                    .transpose()?,
                products: list("product_ids"),
                categories: list("category_ids"),
                id,
            });
        }

        let program_id = rows(named, "customer.member_program")
            .find_map(|r| s(r, "id"))
            .ok_or("named rows: no customer.member_program")?;
        let customers = rows(named, "customer.person")
            .filter_map(|r| s(r, "id"))
            .collect();

        let mut order_numbers = BTreeSet::new();
        let mut sales_by_day: BTreeMap<NaiveDate, (i64, i64)> = BTreeMap::new();
        for r in rows(named, "sales.sales_order") {
            if let Some(n) = s(r, "order_number").and_then(|n| n.parse().ok()) {
                order_numbers.insert(n);
            }
            let status = s(r, "order_status").unwrap_or_default();
            if NOT_SALES.contains(&status.as_str()) {
                continue;
            }
            let Some(date) = s(r, "order_date") else {
                continue;
            };
            let day = parse_time(tz, &date)?.date_naive();
            let total = r
                .get("total_payment_due_value")
                .and_then(cents)
                .unwrap_or(0);
            let e = sales_by_day.entry(day).or_default();
            e.0 += 1;
            e.1 += total;
        }

        let mut campaigns = BTreeMap::new();
        for r in rows(named, "messaging.campaign") {
            let id = s(r, "id").ok_or("named rows: a campaign has no id")?;
            let sent = s(r, "date_sent").map(|t| parse_time(tz, &t)).transpose()?;
            campaigns.insert(id, sent);
        }

        Ok(Named {
            tz,
            currency,
            products,
            variants,
            skus,
            categories,
            standard_categories,
            last_position,
            promotions,
            program_id,
            customers,
            order_numbers,
            sales_by_day,
            campaigns,
        })
    }
}
