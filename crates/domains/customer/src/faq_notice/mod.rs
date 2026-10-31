//! `faq_notice`: the store's FAQ (`schema:Question`, table `customer.question`) and notices
//! (`schema:NewsArticle`, table `customer.news_article`). Everyone may read the FAQ and the
//! published notices; operators also see drafts and publish them.

use acom_contract::ToolError;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::access::{Caller, caller, day, db};
use crate::{FAQ_NOT_FOUND, NOTICE_NOT_FOUND};

mod get_faq;
mod get_notice;
mod list_faqs;
mod list_notices;
mod publish_notice;

pub use get_faq::*;
pub use get_notice::*;
pub use list_faqs::*;
pub use list_notices::*;
pub use publish_notice::*;

fn faq_not_found(id: &str) -> ToolError {
    ToolError::new(FAQ_NOT_FOUND, format!("There is no FAQ {id}."))
        .detail(json!({"entity": "faq", "id": id}))
        .next("list_faqs")
}

fn notice_not_found(id: &str) -> ToolError {
    ToolError::new(NOTICE_NOT_FOUND, format!("There is no notice {id}."))
        .detail(json!({"entity": "notice", "id": id}))
        .next("list_notices")
}

/// Shoppers and guests see published notices only; operators see drafts too.
fn sees_drafts(cx: &Cx) -> bool {
    caller(&cx.principal) == Caller::Staff
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct QuestionRow {
    pub id: String,
    pub name: String,
    pub accepted_answer: String,
    pub category: String,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct NoticeRow {
    pub id: String,
    pub headline: String,
    pub article_body: String,
    pub category: String,
    pub notice_status: String,
    pub date_published: Option<DateTime<Utc>>,
    pub is_pinned: bool,
    pub version: i32,
}

const NOTICE_COLUMNS: &str =
    "id, headline, article_body, category, notice_status, date_published, is_pinned, version";

async fn question(tx: &mut TxCx, id: &str) -> Result<Option<QuestionRow>, ToolError> {
    sqlx::query_as::<_, QuestionRow>(
        "SELECT id, name, accepted_answer, category FROM customer.question
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

async fn notice(tx: &mut TxCx, id: &str) -> Result<Option<NoticeRow>, ToolError> {
    sqlx::query_as::<_, NoticeRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {NOTICE_COLUMNS} FROM customer.news_article WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Whether a shopper may see the notice now: published, and its publication date has come.
fn visible_to_shoppers(n: &NoticeRow, now: DateTime<Utc>) -> bool {
    n.notice_status == "published" && n.date_published.is_none_or(|d| d <= now)
}

/// A notice as lists show it.
fn notice_item(cx: &Cx, n: &NoticeRow) -> Value {
    json!({"id": n.id, "headline": n.headline, "category": n.category,
           "date_published": n.date_published.map(|d| day(cx.time, d)),
           "status": n.notice_status, "pinned": n.is_pinned})
}

#[cfg(test)]
mod tests {
    use acom_contract::{ReadOp, WriteOp};

    use super::*;

    /// Integration: needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn reads_the_faq_and_publishes_a_notice() {
        let guest = crate::testdb::cx("ses_dev_guest").await;
        let faqs = ListFaqs
            .read(
                &guest,
                ListFaqsInput {
                    category: Some("returns".into()),
                    query: None,
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(faqs["items"][0]["id"], "faq_returns");
        let notices = ListNotices
            .read(
                &guest,
                ListNoticesInput {
                    category: None,
                    published_from: None,
                    published_through: None,
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap();
        // The holiday notice is still a draft: guests do not see it.
        assert!(
            notices["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|n| n["id"] != "ntc_holiday")
        );
        let cx = crate::testdb::cx("adm_sj").await;
        let planned = PublishNotice
            .preview(
                &cx,
                PublishNoticeInput {
                    notice_id: "ntc_holiday".into(),
                    date_published: None,
                    pinned: Some(true),
                },
            )
            .await
            .unwrap();
        assert_eq!(planned.changes[0].to, json!("published"));
        let mut tx = cx.begin().await.unwrap();
        let done = PublishNotice.commit(&mut tx, &planned.plan).await.unwrap();
        let after = notice(&mut tx, "ntc_holiday").await.unwrap().unwrap();
        assert_eq!(after.notice_status, "published");
        PublishNotice
            .undo(&mut tx, &planned.plan, &done)
            .await
            .unwrap();
        let undone = notice(&mut tx, "ntc_holiday").await.unwrap().unwrap();
        assert_eq!(undone.notice_status, "draft");
        // Rolled back on drop.
    }
}
