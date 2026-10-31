//! The `product` resource (docs/api/search.md#product): `search_products`, `compare_products`,
//! `autocomplete` and `get_facets`, one file each, and what they share (the text ranking, the
//! category filter, the list envelope, the shopper's default availability).

mod autocomplete;
mod compare_products;
mod get_facets;
mod search_products;

pub use autocomplete::*;
pub use compare_products::*;
pub use get_facets::*;
pub use search_products::*;

use acom_catalog::service::for_search;
use acom_contract::{ErrorCode, ToolError};
use acom_store::{Cx, TxCx};
use serde_json::{Value, json};

use crate::{model, repo};

/// The products a text query matches, with relevance, or `None` when there is no query. Synonyms
/// widen what matches; the ranking counts how many of the shopper's words (each with its
/// synonyms) a product covers, then the text rank, with the words they typed counting twice. Uses the search documents when the store has them, else the same
/// weighted text built from the catalog (`query.expansion_fallback_keeps_ranking`: either way
/// the ranking is `ts_rank` over the same weights).
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
    let (Some(q), Some(own)) = (
        model::tsquery(&model::expand(query, &groups)),
        model::tsquery(&model::expand(query, &[])),
    ) else {
        // Only punctuation: nothing can match.
        return Ok(Some(Vec::new()));
    };
    let concepts = model::concepts(query, &groups);
    let rank = repo::Rank {
        query: &q,
        own: &own,
        concepts: &concepts,
    };
    if repo::has_documents(tx).await? {
        return repo::rank_documents(tx, &rank, active_only).await.map(Some);
    }
    let texts = for_search::product_texts(tx, active_only).await?;
    let links = for_search::categories(tx, None).await?;
    let docs = model::docs(&texts, &links);
    repo::rank_texts(tx, &docs, &rank).await.map(Some)
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
