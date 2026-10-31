//! Test helpers for the integration tests (`#[ignore]`d): a request context on the database at
//! `DATABASE_URL`, seeded with the demo store, at the demo clock of Fri Nov 6, 11:00.

use std::collections::BTreeSet;
use std::sync::Arc;

use acom_auth::{Principal, Surface, scopes};
use acom_contract::{Registry, RegistryBuilder};
use acom_core::enums::ActorKind;
use acom_core::{FixedClock, StoreTime, TenantId};
use acom_store::{Cx, Db};

/// A context for an operator (`adm_…`, platform administrator scopes unless `scopes` is given),
/// an API client (`key_…`), or another subject.
pub(crate) async fn cx_with(subject: &str, kind: ActorKind, scopes: BTreeSet<String>) -> Cx {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let db = Db::connect(&url, 2).await.unwrap();
    let time = StoreTime::new(chrono_tz::America::New_York);
    let surface = match kind {
        ActorKind::ExternalAgent => Surface::Mcp,
        ActorKind::Shopper => Surface::Fo,
        ActorKind::System => Surface::System,
        _ => Surface::Bo,
    };
    Cx {
        tenant: TenantId::parse("store_northwind").unwrap(),
        principal: Arc::new(Principal {
            subject: subject.to_owned(),
            kind,
            tenant: TenantId::parse("store_northwind").unwrap(),
            surface,
            scopes,
            name: None,
            session_id: None,
        }),
        request_id: "req_test".to_owned(),
        clock: Arc::new(FixedClock::at(time.parse("2026-11-06T11:00").unwrap())),
        time,
        currency: acom_core::Currency::parse("USD").unwrap(),
        policy: Arc::new(crate::rules::tests::defaults()),
        db,
    }
}

/// A context whose pool never connects: for what is decided before any query.
pub(crate) fn offline(subject: &str, scopes: BTreeSet<String>) -> Cx {
    let time = StoreTime::new(chrono_tz::America::New_York);
    Cx {
        tenant: TenantId::parse("store_northwind").unwrap(),
        principal: Arc::new(Principal {
            subject: subject.to_owned(),
            kind: ActorKind::Operator,
            tenant: TenantId::parse("store_northwind").unwrap(),
            surface: Surface::Bo,
            scopes,
            name: None,
            session_id: None,
        }),
        request_id: "req_test".to_owned(),
        clock: Arc::new(FixedClock::at(time.parse("2026-11-06T11:00").unwrap())),
        time,
        currency: acom_core::Currency::parse("USD").unwrap(),
        policy: Arc::new(crate::rules::tests::defaults()),
        db: Db::connect_lazy("postgres://offline.invalid/acom", 1).unwrap(),
    }
}

/// The demo platform administrator.
pub(crate) async fn admin() -> Cx {
    cx_with("adm_admin", ActorKind::Operator, scopes::admin()).await
}

/// The platform operations alone, for running previews through the preview store.
pub(crate) fn registry() -> Registry<Cx> {
    let mut b = RegistryBuilder::default();
    crate::register(&mut b);
    b.build().unwrap()
}

/// Runs SQL in the demo store as the owner role (`MIGRATION_DATABASE_URL`): for setting up rows
/// and cleaning up the ones the service role may not delete (previews, the action log, keys).
pub(crate) async fn owner_exec(sql: &str, binds: &[&str]) {
    let url = std::env::var("MIGRATION_DATABASE_URL").expect("MIGRATION_DATABASE_URL");
    let db = Db::connect(&url, 1).await.unwrap();
    let mut tx = db.pool().begin().await.unwrap();
    sqlx::query("SELECT set_config('app.tenant_id', 'store_northwind', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()));
    for b in binds {
        q = q.bind(*b);
    }
    q.execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
}

/// Runs SQL as the store, for setting up and cleaning after a test.
pub(crate) async fn exec(cx: &Cx, sql: &str, binds: &[&str]) {
    let mut tx = cx.begin().await.unwrap();
    let mut q = sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()));
    for b in binds {
        q = q.bind(*b);
    }
    q.execute(&mut *tx.tx).await.unwrap();
    tx.commit().await.unwrap();
}
