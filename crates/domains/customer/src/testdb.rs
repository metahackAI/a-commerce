//! Test helpers for the integration tests (`#[ignore]`d): a request context on the database at
//! `DATABASE_URL`, seeded with the demo store, at the demo clock of Sat Nov 7, 10:00.

use std::collections::BTreeSet;
use std::sync::Arc;

use acom_auth::{Principal, Surface, scopes};
use acom_core::enums::ActorKind;
use acom_core::{FixedClock, StoreTime, TenantId};
use acom_policy::{FILES, PolicySet};
use acom_store::{Cx, Db, TxCx};

pub(crate) fn policies() -> PolicySet {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../policies");
    let texts: Vec<(&str, String)> = FILES
        .iter()
        .map(|f| {
            (
                *f,
                std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap(),
            )
        })
        .collect();
    PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str()))).unwrap()
}

/// A context for `subject`: `cus_…` is a signed-in shopper, `ses_…` a guest, anything else an
/// operator.
pub(crate) async fn cx(subject: &str) -> Cx {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let db = Db::connect(&url, 2).await.unwrap();
    let time = StoreTime::new(chrono_tz::America::New_York);
    let (kind, set): (ActorKind, BTreeSet<String>) = if subject.starts_with("cus_") {
        (ActorKind::Shopper, scopes::shopper())
    } else if subject.starts_with("ses_") {
        (ActorKind::Shopper, scopes::guest())
    } else {
        (ActorKind::Operator, scopes::admin())
    };
    Cx {
        tenant: TenantId::parse("store_northwind").unwrap(),
        principal: Arc::new(Principal {
            subject: subject.to_owned(),
            kind,
            tenant: TenantId::parse("store_northwind").unwrap(),
            surface: Surface::Fo,
            scopes: set,
            name: None,
            session_id: Some("ses_test".to_owned()),
        }),
        request_id: "req_test".to_owned(),
        clock: Arc::new(FixedClock::at(time.parse("2026-11-07T10:00").unwrap())),
        time,
        currency: acom_core::Currency::parse("USD").unwrap(),
        policy: Arc::new(policies()),
        db,
    }
}

/// A transaction for `subject`, rolled back when dropped.
pub(crate) async fn tx(subject: &str) -> TxCx {
    cx(subject).await.begin().await.unwrap()
}
