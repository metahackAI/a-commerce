//! Test contexts shared by the contract tests of every resource and by scene B4.

use std::sync::Arc;

use acom_auth::{Principal, Surface, scopes};
use acom_core::enums::ActorKind;
use acom_core::{FixedClock, StoreTime, TenantId};
use acom_policy::{FILES, PolicySet};
use acom_store::{Cx, Db};

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
        .unwrap_or_else(|e| panic!("{e}"))
}

/// A context whose database is never reached, at Sat Nov 7 10:00.
pub fn cx(kind: ActorKind, subject: &str) -> Cx {
    let tenant = TenantId::parse("store_northwind").unwrap_or_else(|e| panic!("{e}"));
    let time = StoreTime::new(chrono_tz::America::New_York);
    let now = time
        .parse("2026-11-07T10:00")
        .unwrap_or_else(|e| panic!("{e}"));
    let operator = kind == ActorKind::Operator;
    Cx {
        principal: Arc::new(Principal {
            subject: subject.into(),
            kind,
            tenant: tenant.clone(),
            surface: if operator { Surface::Bo } else { Surface::Fo },
            scopes: if operator {
                scopes::admin()
            } else {
                scopes::shopper()
            },
            name: None,
            session_id: None,
        }),
        tenant,
        request_id: "req_test".into(),
        clock: Arc::new(FixedClock::at(now)),
        time,
        currency: acom_core::Currency::parse("USD").unwrap_or_else(|e| panic!("{e}")),
        policy: Arc::new(policies()),
        db: Db::connect_lazy("postgres://nobody@127.0.0.1:1/none", 1)
            .unwrap_or_else(|e| panic!("{e}")),
    }
}

pub fn jamie() -> Cx {
    cx(ActorKind::Shopper, "cus_jamie")
}

/// A real database (`DATABASE_URL`) with the demo seed, with catalog's lookups wired.
pub async fn db(kind: ActorKind, subject: &str) -> Cx {
    acom_catalog::neighbors::install(crate::wiring::Wiring);
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| panic!("DATABASE_URL"));
    let mut cx = cx(kind, subject);
    cx.db = Db::connect(&url, 2).await.unwrap_or_else(|e| panic!("{e}"));
    cx
}
