//! Test support: the policy files, the demo seed rows (named plus generated, no database), and
//! a request context against `DATABASE_URL` for the integration tests.

use std::path::PathBuf;
use std::sync::OnceLock;

use acom_auth::{Principal, Surface};
use acom_core::enums::ActorKind;
use acom_core::{StoreTime, TenantId};
use acom_policy::PolicySet;
use acom_store::{Cx, Db, Runtime};
use acom_testkit::SeedRows;
use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::repo::BookingRow;

fn repo_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."))
}

/// The default policies; with `demo`, plus the demo store's override.
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

pub(crate) fn rows(table: &str) -> impl Iterator<Item = &'static Value> + '_ {
    seed()
        .iter()
        .filter(move |(t, _)| t == table)
        .map(|(_, r)| r)
}

fn text(v: &Value, key: &str) -> Option<String> {
    v[key].as_str().map(str::to_owned)
}

pub(crate) fn store_time() -> StoreTime {
    StoreTime::new(chrono_tz::America::New_York)
}

/// A store-time moment.
pub(crate) fn at(s: &str) -> DateTime<Utc> {
    store_time().parse(s).unwrap()
}

/// The seeded bookings of a slot with their banners, as `repo::bookings_*` reads them.
pub(crate) fn bookings_of(slot_id: &str) -> Vec<BookingRow> {
    rows("merchandising.display_booking")
        .filter(|d| d["display_slot_id"] == slot_id)
        .map(|d| {
            let banner = rows("merchandising.banner").find(|b| b["id"] == d["banner_id"]);
            let from_banner = |k: &str| banner.and_then(|b| text(b, k));
            BookingRow {
                id: text(d, "id").unwrap_or_default(),
                display_slot_id: slot_id.to_owned(),
                banner_id: text(d, "banner_id"),
                valid_for_member_tier: text(d, "valid_for_member_tier"),
                valid_from: at(d["valid_from"].as_str().unwrap()),
                valid_through: d["valid_through"].as_str().map(at),
                display_status: text(d, "display_status").unwrap_or_default(),
                promotion_id: text(d, "promotion_id"),
                change_set_id: text(d, "change_set_id"),
                version: 1,
                eyebrow: from_banner("eyebrow"),
                headline: from_banner("headline"),
                alternative_headline: from_banner("alternative_headline"),
                cta_label: from_banner("cta_label"),
                url: from_banner("url"),
                image: from_banner("image"),
                image_product_group_id: from_banner("image_product_group_id"),
                image_color: from_banner("image_color"),
                item_list_element: None,
                sales_channel: None,
            }
        })
        .collect()
}

/// Members of a tier that `list_tiers` counts (not withdrawn or blocked).
pub(crate) fn member_count(tier: &str) -> i64 {
    let gone: Vec<&str> = rows("customer.person")
        .filter(|p| matches!(p["member_state"].as_str(), Some("withdrawn" | "blocked")))
        .filter_map(|p| p["id"].as_str())
        .collect();
    rows("customer.program_membership")
        .filter(|m| m["tier_id"] == tier)
        .filter(|m| !gone.contains(&m["customer_id"].as_str().unwrap_or_default()))
        .count() as i64
}

/// A request context for the demo store as `subject`, against `DATABASE_URL` (migrated and
/// seeded, demo clock on). Integration tests only.
pub(crate) async fn operator_cx(subject: &str) -> Cx {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let db = Db::connect(&url, 4).await.expect("connect");
    let runtime = Runtime::new(db, policies(false), true);
    let principal = Principal {
        subject: subject.into(),
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

/// A signed-in shopper (`cus_…`) or, for anything else, a guest, on the storefront.
pub(crate) async fn shopper_cx(subject: &str) -> Cx {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let db = Db::connect(&url, 2).await.expect("connect");
    let runtime = Runtime::new(db, policies(false), true);
    let signed_in = subject.starts_with("cus_");
    let principal = Principal {
        subject: subject.into(),
        kind: ActorKind::Shopper,
        tenant: TenantId::parse("store_northwind").unwrap(),
        surface: Surface::Fo,
        scopes: if signed_in {
            acom_auth::scopes::shopper()
        } else {
            acom_auth::scopes::guest()
        },
        name: None,
        session_id: Some("ses_test".into()),
    };
    runtime
        .context(principal, "req_test".into())
        .await
        .expect("context")
}
