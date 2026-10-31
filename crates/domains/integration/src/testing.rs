//! Test helpers: the default policies, a context that never reaches a database, and one on the
//! database at `DATABASE_URL` (demo store, already migrated and seeded) for the `#[ignore]`d
//! integration tests.

use std::sync::Arc;

use acom_auth::{Principal, Surface, scopes};
use acom_core::enums::ActorKind;
use acom_core::{FixedClock, StoreTime, TenantId};
use acom_policy::{FILES, PolicySet};
use acom_store::{Cx, Db};

/// The default policies in `policies/`.
pub fn policies() -> PolicySet {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../policies");
    let texts: Vec<(&str, String)> = FILES
        .iter()
        .map(|f| {
            (
                *f,
                std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap_or_default(),
            )
        })
        .collect();
    PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str())))
        .unwrap_or_else(|e| panic!("policies: {e}"))
}

fn context(subject: &str, db: Db) -> Cx {
    let tenant = TenantId::parse("store_northwind").unwrap_or_else(|e| panic!("{e}"));
    let time = StoreTime::new(chrono_tz::America::New_York);
    let now = time
        .parse("2026-11-06T11:00")
        .unwrap_or_else(|e| panic!("{e}"));
    let principal = if subject.starts_with("sys_") {
        Principal::system(tenant.clone(), subject)
    } else {
        Principal {
            subject: subject.to_owned(),
            kind: ActorKind::Operator,
            tenant: tenant.clone(),
            surface: Surface::Bo,
            scopes: scopes::admin(),
            name: Some("Son SeongJun".into()),
            session_id: None,
        }
    };
    Cx {
        tenant,
        principal: Arc::new(principal),
        request_id: "req_test".into(),
        clock: Arc::new(FixedClock::at(now)),
        time,
        currency: acom_core::Currency::USD,
        policy: Arc::new(policies()),
        db,
    }
}

/// A context whose database is never reached, at Fri Nov 6 11:00: `sys_…` is the system (the
/// sender of carrier and payment events), anything else an operator.
pub fn offline(subject: &str) -> Cx {
    context(
        subject,
        Db::connect_lazy("postgres://offline.invalid/acom", 1).unwrap_or_else(|e| panic!("{e}")),
    )
}

/// The same on the database at `DATABASE_URL`. Catalog's category lookups are installed the way
/// the registry installs them.
pub async fn db(subject: &str) -> Cx {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| panic!("DATABASE_URL"));
    let mut b = acom_contract::RegistryBuilder::default();
    acom_search::register(&mut b);
    context(
        subject,
        Db::connect(&url, 2).await.unwrap_or_else(|e| panic!("{e}")),
    )
}
