//! Test support: the policy files (with the demo store's override), the demo seed rows (named
//! plus generated, no database), and a request context against `DATABASE_URL` for the
//! integration tests.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::OnceLock;

use acom_auth::{Principal, Surface};
use acom_catalog::service::for_promotion::{ProductOffers, VariantOffer};
use acom_core::TenantId;
use acom_core::enums::ActorKind;
use acom_policy::PolicySet;
use acom_store::{Cx, Db, Runtime};
use acom_testkit::SeedRows;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

use crate::model::decimal;
use crate::repo::PromotionRow;

fn repo_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."))
}

/// The default policies; with `demo`, plus the demo store's override (`max_discount.by_tier.vip`).
pub(crate) fn policies(demo: bool) -> PolicySet {
    let dir = repo_dir().join("policies");
    let texts: Vec<(&str, String)> = acom_policy::FILES
        .iter()
        .map(|f| {
            (
                *f,
                std::fs::read_to_string(dir.join(format!("{f}.yml"))).unwrap(),
            )
        })
        .collect();
    let p = PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str()))).unwrap();
    if demo {
        p.with_overrides([("promotion", "max_discount:\n  by_tier:\n    vip: 0.30\n")])
            .unwrap()
    } else {
        p
    }
}

/// Every demo seed row: the named files and what the generator adds, built once.
pub(crate) fn seed() -> &'static SeedRows {
    static SEED: OnceLock<SeedRows> = OnceLock::new();
    SEED.get_or_init(|| {
        let dir = repo_dir().join("seed/demo");
        let mut rows = acom_testkit::named::load(&dir).unwrap();
        let spec = std::fs::read_to_string(dir.join("generate.yml")).unwrap();
        let generated = acom_testkit::generate::generate(&rows, &spec).unwrap();
        rows.extend(generated);
        rows
    })
}

/// The rows of one table.
pub(crate) fn rows(table: &str) -> impl Iterator<Item = &'static Value> + '_ {
    seed()
        .iter()
        .filter(move |(t, _)| t == table)
        .map(|(_, r)| r)
}

pub(crate) fn text(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_owned()
}

pub(crate) fn time(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str()
        .map(|s| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc))
}

pub(crate) fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

/// Products placed in a category in the seed (no subcategories), in seed order.
pub(crate) fn placed_in(category_id: &str) -> Vec<String> {
    rows("merchandising.product_group_category")
        .filter(|r| r["category_id"] == category_id)
        .map(|r| text(r, "product_group_id"))
        .collect()
}

/// What `catalog::service::for_promotion::products_with_offers` reads, from the seed.
pub(crate) fn products_with_offers(ids: &[String]) -> Vec<ProductOffers> {
    ids.iter()
        .filter_map(|id| {
            let g = rows("catalog.product_group").find(|r| r["id"] == id.as_str())?;
            let variants: Vec<&Value> = rows("catalog.product")
                .filter(|r| r["product_group_id"] == id.as_str())
                .collect();
            let mut offers: Vec<VariantOffer> = variants
                .iter()
                .filter_map(|v| {
                    let o = rows("catalog.offer").find(|o| o["product_id"] == v["id"])?;
                    Some(VariantOffer {
                        offer_id: text(o, "id"),
                        variant_id: text(v, "id"),
                        color: v["color"].as_str().map(str::to_owned),
                        size: v["size"].as_str().map(str::to_owned),
                        price: decimal(&o["price"])?,
                        price_currency: text(o, "price_currency"),
                        cost_value: decimal(&o["cost_value"]),
                        cost_currency: o["cost_currency"].as_str().map(str::to_owned),
                        inventory_level: o["inventory_level"]
                            .as_i64()
                            .and_then(|n| i32::try_from(n).ok())
                            .unwrap_or(0),
                        availability: o["availability"].as_str().unwrap_or("in_stock").to_owned(),
                        version: 1,
                    })
                })
                .collect();
            offers.sort_by(|a, b| a.variant_id.cmp(&b.variant_id));
            Some(ProductOffers {
                product_id: id.clone(),
                name: text(g, "name"),
                approval_status: g["approval_status"].as_str().unwrap_or("draft").to_owned(),
                offers,
            })
        })
        .collect()
}

/// The seeded promotions as rows.
pub(crate) fn promotions() -> Vec<PromotionRow> {
    rows("promotion.promotion")
        .map(|r| PromotionRow {
            id: text(r, "id"),
            name: text(r, "name"),
            description: r["description"].as_str().map(str::to_owned),
            promotion_status: text(r, "promotion_status"),
            discount_layer: text(r, "discount_layer"),
            valid_for_member_tier: r["valid_for_member_tier"].as_str().map(str::to_owned),
            valid_from: time(&r["valid_from"]).unwrap(),
            valid_through: time(&r["valid_through"]),
            percent_off: decimal(&r["percent_off"]),
            amount_off_value: decimal(&r["amount_off_value"]),
            amount_off_currency: r["amount_off_currency"].as_str().map(str::to_owned),
            applies_to: r["applies_to"].clone(),
            change_set_id: None,
            version: 1,
        })
        .collect()
}

/// Members of a tier that `list_tiers` counts (not withdrawn or blocked).
pub(crate) fn member_count(tier: &str) -> i64 {
    let gone: Vec<String> = rows("customer.person")
        .filter(|p| matches!(p["member_state"].as_str(), Some("withdrawn" | "blocked")))
        .map(|p| text(p, "id"))
        .collect();
    rows("customer.program_membership")
        .filter(|m| m["tier_id"] == tier && !gone.contains(&text(m, "customer_id")))
        .count() as i64
}

/// A request context for the demo store as operator SJ, against `DATABASE_URL` (migrated and
/// seeded, demo clock on). Integration tests only.
pub(crate) async fn operator_cx() -> Cx {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let db = Db::connect(&url, 2).await.expect("connect");
    let runtime = Runtime::new(db, policies(false), true);
    let principal = Principal {
        subject: "adm_sj".into(),
        kind: ActorKind::Operator,
        tenant: TenantId::parse("store_northwind").unwrap(),
        surface: Surface::Bo,
        scopes: acom_auth::scopes::admin(),
        name: Some("Son SeongJun".into()),
        session_id: None,
    };
    runtime
        .context(principal, "req_test".into())
        .await
        .expect("context")
}
