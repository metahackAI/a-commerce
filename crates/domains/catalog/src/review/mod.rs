//! The `review` resource (docs/api/catalog.md#review): what its operations share. Reviews show at
//! once and are moderated afterwards; the shopper who wrote one may change or delete it, and any
//! signed-in shopper may report it into the moderation queue.

pub mod create_review;
pub mod delete_review;
pub mod get_review_summary;
pub mod list_reviews;
pub mod moderate_review;
pub mod report_review;
pub mod update_review;
pub mod vote_review_helpful;

use acom_contract::ToolError;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use serde_json::json;

use crate::{REVIEW_NOT_FOUND, db};

/// The signed-in shopper writing or voting; guests must sign in first.
fn shopper_id(cx: &Cx, what: &str) -> Result<String, ToolError> {
    crate::signed_in_shopper(cx, what)
}

/// `review_not_found`, pointing at `list_reviews`.
fn review_not_found(id: &str) -> ToolError {
    ToolError::new(REVIEW_NOT_FOUND, format!("There is no review {id}."))
        .detail(json!({"entity": "review", "id": id}))
        .next("list_reviews")
}

/// A review as its writer's changes see it.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct OwnedReview {
    pub id: String,
    pub product_group_id: String,
    pub author_id: Option<String>,
    pub rating_value: i16,
    pub review_body: String,
    pub image: Vec<String>,
    pub creative_work_status: String,
    pub version: i32,
}

pub(crate) async fn owned_review(
    tx: &mut TxCx,
    id: &str,
) -> Result<Option<OwnedReview>, ToolError> {
    sqlx::query_as::<_, OwnedReview>(
        "SELECT id, product_group_id, author_id, rating_value, review_body, image,
                creative_work_status, version
         FROM catalog.review WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The writer's review, or `review_not_found` / `not_author`.
async fn writers_review(tx: &mut TxCx, id: &str, shopper: &str) -> Result<OwnedReview, ToolError> {
    let r = owned_review(tx, id)
        .await?
        .ok_or_else(|| review_not_found(id))?;
    if r.author_id.as_deref() != Some(shopper) {
        return Err(crate::not_author("review", id, "list_reviews"));
    }
    Ok(r)
}

/// What `update_review` writes.
pub(crate) struct ReviewEdit<'a> {
    pub id: &'a str,
    pub author_id: &'a str,
    pub rating: i16,
    pub body: &'a str,
    pub image: &'a [String],
    pub keywords: &'a [String],
    pub status: &'a str,
}

/// Rewrites a review, only for its writer. Returns the rows changed.
pub(crate) async fn edit_review(tx: &mut TxCx, e: &ReviewEdit<'_>) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE catalog.review SET rating_value = $4, review_body = $5, image = $6, keywords = $7,
           creative_work_status = $8, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND author_id = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(e.id)
    .bind(e.author_id)
    .bind(e.rating)
    .bind(e.body)
    .bind(e.image)
    .bind(e.keywords)
    .bind(e.status)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

/// Removes a review with its votes and reports, only for its writer. Removing one that is gone
/// already changes nothing.
pub(crate) async fn remove_review(
    tx: &mut TxCx,
    id: &str,
    author_id: &str,
) -> Result<u64, ToolError> {
    for sql in [
        "DELETE FROM catalog.review_vote v USING catalog.review r
         WHERE v.tenant_id = $1 AND r.tenant_id = $1 AND v.review_id = r.id
           AND r.id = $2 AND r.author_id = $3",
        "DELETE FROM catalog.review_report p USING catalog.review r
         WHERE p.tenant_id = $1 AND r.tenant_id = $1 AND p.review_id = r.id
           AND r.id = $2 AND r.author_id = $3",
    ] {
        sqlx::query(sql)
            .bind(tx.tenant.as_str())
            .bind(id)
            .bind(author_id)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
    }
    let r = sqlx::query(
        "DELETE FROM catalog.review WHERE tenant_id = $1 AND id = $2 AND author_id = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(author_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}

/// The report a shopper already made on a review, if any.
pub(crate) async fn report_by(
    tx: &mut TxCx,
    review_id: &str,
    customer_id: &str,
) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM catalog.review_report
         WHERE tenant_id = $1 AND review_id = $2 AND customer_id = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(review_id)
    .bind(customer_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A new report (`report_review`).
pub(crate) struct NewReport<'a> {
    pub id: &'a str,
    pub review_id: &'a str,
    pub customer_id: &'a str,
    pub reason: &'a str,
    pub note: Option<&'a str>,
    pub at: DateTime<Utc>,
}

/// Inserts the report unless this shopper already reported the review; `true` when written.
pub(crate) async fn insert_report(tx: &mut TxCx, r: &NewReport<'_>) -> Result<bool, ToolError> {
    let done = sqlx::query(
        "INSERT INTO catalog.review_report (tenant_id, id, review_id, customer_id, report_reason,
           description, start_time)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(r.id)
    .bind(r.review_id)
    .bind(r.customer_id)
    .bind(r.reason)
    .bind(r.note)
    .bind(r.at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(done.rows_affected() == 1)
}

/// Moves a review from one moderation state to another; `0` when it was not in `from`.
pub(crate) async fn move_status(
    tx: &mut TxCx,
    id: &str,
    from: &str,
    to: &str,
) -> Result<u64, ToolError> {
    let r = sqlx::query(
        "UPDATE catalog.review SET creative_work_status = $4, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND creative_work_status = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(from)
    .bind(to)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(r.rows_affected())
}
