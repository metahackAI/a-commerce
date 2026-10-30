//! One type per operation of `docs/api/catalog.md`, grouped by resource.

pub mod brand;
pub mod history;
pub mod inventory;
pub mod offer;
pub mod product;
pub mod product_group;
pub mod review;

use acom_contract::{ErrorCode, ToolError};
use serde_json::json;

use crate::PRODUCT_NOT_FOUND;

/// `product_not_found` for a product (`prd_`) or a variant (`var_`).
pub(crate) fn product_not_found(entity: &str, id: &str, next: &'static str) -> ToolError {
    let what = if entity == "variant" {
        "variant"
    } else {
        "product"
    };
    ToolError::new(
        PRODUCT_NOT_FOUND,
        format!("There is no {what} {id} in this store."),
    )
    .detail(json!({"entity": entity, "id": id}))
    .next(next)
}

/// The spec's plain `not_found` with the entity named in `detail`.
pub(crate) fn not_found(entity: &str, id: &str, next: &'static str) -> ToolError {
    ToolError::new(
        ErrorCode::NOT_FOUND,
        format!(
            "There is no {} {id} in this store.",
            entity.replace('_', " ")
        ),
    )
    .detail(json!({"entity": entity, "id": id}))
    .next(next)
}

#[cfg(test)]
pub(crate) mod testing {
    use std::sync::Arc;

    use acom_auth::{Principal, Surface, scopes};
    use acom_core::enums::ActorKind;
    use acom_core::{FixedClock, StoreTime, TenantId};
    use acom_store::{Cx, Db};

    /// A context whose database is never reached: for checks that fail before any query.
    pub fn cx(kind: ActorKind, subject: &str) -> Cx {
        let tenant = TenantId::parse("store_northwind").unwrap_or_else(|e| panic!("{e}"));
        let now = StoreTime::new(chrono_tz::America::New_York)
            .parse("2026-11-07T10:00")
            .unwrap_or_else(|e| panic!("{e}"));
        Cx {
            principal: Arc::new(Principal {
                subject: subject.into(),
                kind,
                tenant: tenant.clone(),
                surface: Surface::Fo,
                scopes: scopes::shopper(),
                name: Some("Jamie".into()),
                session_id: None,
            }),
            tenant,
            request_id: "req_test".into(),
            clock: Arc::new(FixedClock::at(now)),
            time: StoreTime::new(chrono_tz::America::New_York),
            currency: acom_core::Currency::parse("USD").unwrap_or_else(|e| panic!("{e}")),
            policy: Arc::new(crate::testing::policies()),
            db: Db::connect_lazy("postgres://nobody@127.0.0.1:1/none", 1)
                .unwrap_or_else(|e| panic!("{e}")),
        }
    }

    /// Jamie, a signed-in VIP shopper.
    pub fn shopper() -> Cx {
        cx(ActorKind::Shopper, "cus_jamie")
    }

    /// A guest without an account.
    pub fn guest() -> Cx {
        cx(ActorKind::Shopper, "ses_guest")
    }

    /// A context on a real database (`DATABASE_URL`) as the demo store's operator SJ, for the
    /// integration tests (`cargo test -- --ignored`).
    pub async fn operator_db() -> Cx {
        let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| panic!("DATABASE_URL"));
        let mut cx = cx(ActorKind::Operator, "adm_sj");
        cx.db = Db::connect(&url, 2).await.unwrap_or_else(|e| panic!("{e}"));
        let mut p = (*cx.principal).clone();
        p.surface = Surface::Bo;
        p.scopes = scopes::admin();
        cx.principal = Arc::new(p);
        let now = cx
            .time
            .parse("2026-11-06T11:00")
            .unwrap_or_else(|e| panic!("{e}"));
        cx.clock = Arc::new(FixedClock::at(now));
        cx
    }

    /// The same, as Jamie on Saturday morning.
    pub async fn shopper_db() -> Cx {
        let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| panic!("DATABASE_URL"));
        let mut cx = shopper();
        cx.db = Db::connect(&url, 2).await.unwrap_or_else(|e| panic!("{e}"));
        cx
    }
}
