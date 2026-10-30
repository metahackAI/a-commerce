//! Test helpers for the integration tests (`#[ignore]`d): a request context on the database at
//! `DATABASE_URL`, seeded with the demo store, at the demo clock of Sat Nov 7, 10:00.

use std::collections::BTreeSet;
use std::sync::Arc;

use acom_auth::{Principal, Surface, scopes};
use acom_contract::{Registry, RegistryBuilder};
use acom_core::enums::ActorKind;
use acom_core::{FixedClock, StoreTime, TenantId};
use acom_store::{Cx, Db};

/// A context for `subject`: `cus_…` is a signed-in shopper, `ses_…` a guest, anything else an
/// operator.
pub(crate) async fn cx(subject: &str) -> Cx {
    cx_at(subject, "2026-11-07T10:00").await
}

/// The same at another store time (`2026-11-06T11:00`).
pub(crate) async fn cx_at(subject: &str, at: &str) -> Cx {
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
        clock: Arc::new(FixedClock::at(time.parse(at).unwrap())),
        time,
        currency: acom_core::Currency::parse("USD").unwrap(),
        policy: Arc::new(crate::rules::tests::defaults()),
        db,
    }
}

/// The sales operations alone, for running previews through the preview store.
pub(crate) fn registry() -> Registry<Cx> {
    let mut b = RegistryBuilder::default();
    crate::register(&mut b);
    b.build().unwrap()
}
