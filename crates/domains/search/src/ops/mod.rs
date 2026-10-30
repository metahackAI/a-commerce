//! One type per operation of `docs/api/search.md#product`.

pub mod autocomplete;
pub mod compare;
pub mod facets;
pub mod products;

use acom_catalog::service::for_search;
use acom_contract::{ErrorCode, ToolError};
use acom_store::{Cx, TxCx};
use serde_json::{Value, json};

use crate::{model, repo};

/// The products a text query matches, with relevance, or `None` when there is no query. Uses
/// the search documents when the store has them, else the same weighted text built from the
/// catalog (`query.expansion_fallback_keeps_ranking`: either way the ranking is `ts_rank` over
/// the same weights).
pub(crate) async fn text_ranks(
    cx: &Cx,
    tx: &mut TxCx,
    query: Option<&str>,
    active_only: bool,
) -> Result<Option<Vec<(String, f64)>>, ToolError> {
    let Some(query) = query.map(str::trim).filter(|q| !q.is_empty()) else {
        return Ok(None);
    };
    let groups = model::synonyms(&cx.policy, "en");
    let Some(q) = model::tsquery(&model::expand(query, &groups)) else {
        // Only punctuation: nothing can match.
        return Ok(Some(Vec::new()));
    };
    if repo::has_documents(tx).await? {
        return repo::rank_documents(tx, &q, active_only).await.map(Some);
    }
    let texts = for_search::product_texts(tx, active_only).await?;
    let links = for_search::categories(tx, None).await?;
    let docs = model::docs(&texts, &links);
    repo::rank_texts(tx, &docs, &q).await.map(Some)
}

/// The products of a category and the ones under it, or `not_found`.
pub(crate) async fn category_filter(
    tx: &mut TxCx,
    category_id: Option<&str>,
    code: ErrorCode,
    next: &'static str,
) -> Result<Option<Vec<String>>, ToolError> {
    let Some(cat) = category_id else {
        return Ok(None);
    };
    for_search::category_products(tx, cat)
        .await?
        .map(Some)
        .ok_or_else(|| {
            ToolError::new(code, format!("There is no category {cat}."))
                .detail(json!({"entity": "category", "id": cat, "field": "category_id"}))
                .next(next)
        })
}

/// A list envelope `{items, total, next_cursor?}`.
pub(crate) fn list(items: Vec<Value>, total: i64, page: &acom_store::Page) -> Value {
    let returned = items.len();
    let mut out = json!({"items": items, "total": total});
    if let Some(c) = page.next_cursor(returned, total) {
        out["next_cursor"] = json!(c);
    }
    out
}

/// The availability shoppers see by default (`query.shopper_default_availability`).
pub(crate) fn shopper_availability(cx: &Cx) -> Vec<String> {
    cx.policy
        .lookup("search.query.shopper_default_availability")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod testing {
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
}
