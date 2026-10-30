//! Search SQL: full-text ranking in PostgreSQL, from the store's search documents
//! (`search.search_document`) when they have been built, else from the catalog text itself.

use acom_contract::ToolError;
use acom_store::TxCx;

use crate::INDEX_UNAVAILABLE;
use crate::model::Doc;

/// A failed text search means the index cannot answer; the caller can retry or check it.
fn unavailable(e: sqlx::Error) -> ToolError {
    tracing::error!(error = %e, "text search failed");
    ToolError::new(
        INDEX_UNAVAILABLE,
        "Product text search is not available right now.",
    )
    .detail(serde_json::json!({"upstream": "search_index", "retryable": true}))
    .next("get_index_status")
}

/// Whether search documents have been built for this store.
pub async fn has_documents(tx: &mut TxCx) -> Result<bool, ToolError> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM search.search_document WHERE tenant_id = $1)")
        .bind(tx.tenant.as_str())
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(unavailable)
}

/// Products whose search document matches the query, with `ts_rank` relevance. Shoppers only
/// match documents of active products.
pub async fn rank_documents(
    tx: &mut TxCx,
    tsquery: &str,
    active_only: bool,
) -> Result<Vec<(String, f64)>, ToolError> {
    sqlx::query_as::<_, (String, f64)>(
        "SELECT d.product_group_id, ts_rank(d.search_vector, q)::float8
         FROM search.search_document d, to_tsquery('english', $2) AS q
         WHERE d.tenant_id = $1 AND d.search_vector @@ q
           AND (NOT $3 OR d.approval_status = 'active')",
    )
    .bind(tx.tenant.as_str())
    .bind(tsquery)
    .bind(active_only)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(unavailable)
}

/// The same ranking over documents built on the fly from catalog text, with the weights the
/// search document uses (name A; brand, categories, tags B; material C; description D). This
/// statement reads no table, only the arrays it is given.
pub async fn rank_texts(
    tx: &mut TxCx,
    docs: &[Doc],
    tsquery: &str,
) -> Result<Vec<(String, f64)>, ToolError> {
    let ids: Vec<&str> = docs.iter().map(|d| d.id.as_str()).collect();
    let names: Vec<&str> = docs.iter().map(|d| d.name.as_str()).collect();
    let labels: Vec<&str> = docs.iter().map(|d| d.labels.as_str()).collect();
    let materials: Vec<&str> = docs.iter().map(|d| d.material.as_str()).collect();
    let descriptions: Vec<&str> = docs.iter().map(|d| d.description.as_str()).collect();
    sqlx::query_as::<_, (String, f64)>(
        "SELECT t.id, ts_rank(t.v, q)::float8
         FROM (SELECT d.id,
                      setweight(to_tsvector('english', d.name), 'A')
                      || setweight(to_tsvector('english', d.labels), 'B')
                      || setweight(to_tsvector('english', d.material), 'C')
                      || setweight(to_tsvector('english', d.description), 'D') AS v
               FROM unnest($1::text[], $2::text[], $3::text[], $4::text[], $5::text[])
                    AS d(id, name, labels, material, description)) AS t,
              to_tsquery('english', $6) AS q
         WHERE t.v @@ q",
    )
    .bind(&ids)
    .bind(&names)
    .bind(&labels)
    .bind(&materials)
    .bind(&descriptions)
    .bind(tsquery)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(unavailable)
}
