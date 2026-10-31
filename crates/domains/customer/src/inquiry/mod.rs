//! `inquiry`: customer service inquiries (`customer.customer_inquiry`) and their threads
//! (`customer.comment`). An inquiry is `received`, then `in_progress` with an operator, then
//! `resolved` (`customer_service.states`). Replies to the customer are recorded in the thread;
//! delivery is not connected here, so a reply says it was recorded and not delivered.

use acom_contract::ToolError;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::INQUIRY_NOT_FOUND;
use crate::access::{Caller, caller, db};

mod assign_inquiry;
mod close_inquiry;
mod create_inquiry;
mod get_inquiry;
mod respond_inquiry;
mod search_inquiries;

pub use assign_inquiry::*;
pub use close_inquiry::*;
pub use create_inquiry::*;
pub use get_inquiry::*;
pub use respond_inquiry::*;
pub use search_inquiries::*;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct InquiryRow {
    pub id: String,
    pub customer_id: String,
    pub inquiry_type: String,
    pub inquiry_status: String,
    pub order_id: Option<String>,
    pub admin_user_id: Option<String>,
    pub text: String,
    pub resolution: Option<String>,
    pub date_received: DateTime<Utc>,
    pub assigned_at: Option<DateTime<Utc>>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub version: i32,
}

const INQUIRY_COLUMNS: &str = "id, customer_id, inquiry_type, inquiry_status, order_id,
    admin_user_id, text, resolution, date_received, assigned_at, resolved_at, version";

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct CommentRow {
    pub id: String,
    pub author_type: String,
    pub agent_id: String,
    pub text: String,
    pub message_channel: Option<String>,
    pub date_created: DateTime<Utc>,
    pub date_sent: Option<DateTime<Utc>>,
}

pub(crate) fn inquiry_not_found(id: &str) -> ToolError {
    ToolError::new(INQUIRY_NOT_FOUND, format!("There is no inquiry {id}."))
        .detail(json!({"entity": "inquiry", "id": id}))
        .next("search_inquiries")
}

pub(crate) async fn inquiry(tx: &mut TxCx, id: &str) -> Result<Option<InquiryRow>, ToolError> {
    sqlx::query_as::<_, InquiryRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {INQUIRY_COLUMNS} FROM customer.customer_inquiry WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The inquiry, as the caller may see it: operators see every inquiry, a shopper only their own.
async fn visible_inquiry(cx: &Cx, tx: &mut TxCx, id: &str) -> Result<InquiryRow, ToolError> {
    let row = inquiry(tx, id).await?;
    match (row, caller(&cx.principal)) {
        (Some(r), Caller::Staff) => Ok(r),
        (Some(r), Caller::Shopper(me)) if r.customer_id == me => Ok(r),
        _ => Err(inquiry_not_found(id)),
    }
}

async fn thread(tx: &mut TxCx, inquiry_id: &str) -> Result<Vec<CommentRow>, ToolError> {
    sqlx::query_as::<_, CommentRow>(
        "SELECT id, author_type, agent_id, text, message_channel, date_created, date_sent
         FROM customer.comment WHERE tenant_id = $1 AND inquiry_id = $2
         ORDER BY date_created, id",
    )
    .bind(tx.tenant.as_str())
    .bind(inquiry_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// One message of a thread to add.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NewComment {
    pub id: String,
    pub author_type: String,
    /// Who wrote it (`cus_…`, `adm_…`, `sys_…`): the person who drafted it, not the runner.
    pub agent_id: String,
    pub text: String,
    /// How a reply goes to the customer; `None` for messages that are not sent.
    pub message_channel: Option<String>,
}

/// Adds a message to a thread (a repeated run adds nothing). Delivery is not connected here, so
/// `date_sent` stays empty.
async fn add_comment(tx: &mut TxCx, inquiry_id: &str, c: &NewComment) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO customer.comment (tenant_id, id, inquiry_id, author_type, agent_id, text,
           message_channel, date_created)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(&c.id)
    .bind(inquiry_id)
    .bind(&c.author_type)
    .bind(&c.agent_id)
    .bind(&c.text)
    .bind(c.message_channel.as_deref())
    .bind(tx.clock.now())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// The state, assignee and resolution of an inquiry.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InquiryState {
    pub status: String,
    pub admin_user_id: Option<String>,
    pub assigned_at: Option<DateTime<Utc>>,
    pub resolution: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
}

impl From<&InquiryRow> for InquiryState {
    fn from(r: &InquiryRow) -> Self {
        Self {
            status: r.inquiry_status.clone(),
            admin_user_id: r.admin_user_id.clone(),
            assigned_at: r.assigned_at,
            resolution: r.resolution.clone(),
            resolved_at: r.resolved_at,
        }
    }
}

/// Writes the state (a no-op when it already is that state).
async fn write_state(tx: &mut TxCx, id: &str, s: &InquiryState) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE customer.customer_inquiry SET inquiry_status = $3, admin_user_id = $4,
           assigned_at = $5, resolution = $6, resolved_at = $7, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2
           AND (inquiry_status, admin_user_id, assigned_at, resolution, resolved_at)
               IS DISTINCT FROM ($3, $4, $5, $6, $7)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(&s.status)
    .bind(s.admin_user_id.as_deref())
    .bind(s.assigned_at)
    .bind(s.resolution.as_deref())
    .bind(s.resolved_at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Open (`in_progress`) inquiries per operator, for `customer_service.max_open_per_agent`, and
/// every operator who has handled an inquiry (the round robin's pool), least loaded first.
async fn operator_load(tx: &mut TxCx) -> Result<Vec<(String, i64)>, ToolError> {
    sqlx::query_as(
        "SELECT admin_user_id, count(*) FILTER (WHERE inquiry_status = 'in_progress')
         FROM customer.customer_inquiry
         WHERE tenant_id = $1 AND admin_user_id IS NOT NULL
         GROUP BY admin_user_id ORDER BY 2, max(assigned_at) NULLS FIRST, admin_user_id",
    )
    .bind(tx.tenant.as_str())
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A reply or note as the thread shows it.
fn comment_json(cx: &Cx, c: &CommentRow) -> Value {
    let mut v = json!({"id": c.id, "author_type": c.author_type, "text": c.text,
                       "at": cx.time.format(c.date_created)});
    if let Some(ch) = &c.message_channel {
        v["channel"] = json!(ch);
        v["delivered"] = json!(c.date_sent.is_some());
    }
    v
}

fn new_comment_id() -> String {
    format!("cmt_{}", acom_core::id::new_suffix())
}

#[cfg(test)]
mod tests {
    use acom_contract::{ReadOp, WriteOp};
    use acom_core::enums::InquiryType;

    use super::*;

    /// Integration: needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn an_inquiry_goes_from_received_to_resolved() {
        let shopper = crate::testdb::cx("cus_alex").await;
        let planned = CreateInquiry
            .preview(
                &shopper,
                CreateInquiryInput {
                    inquiry_type: InquiryType::Account,
                    text: "I cannot change my email.".into(),
                    order_id: None,
                    customer_id: "me".into(),
                },
            )
            .await
            .unwrap();
        let cx = crate::testdb::cx("adm_sj").await;
        let mut tx = shopper.begin().await.unwrap();
        let made = CreateInquiry.commit(&mut tx, &planned.plan).await.unwrap();
        let row = inquiry(&mut tx, &made.id).await.unwrap().unwrap();
        assert_eq!(row.inquiry_status, "received");
        // Assign by hand, reply, close; then undo the close and the assignment.
        let assign = AssignPlan {
            inquiry_id: made.id.clone(),
            before: InquiryState::from(&row),
            after: InquiryState {
                status: "in_progress".into(),
                admin_user_id: Some("adm_sj".into()),
                assigned_at: Some(cx.clock.now()),
                resolution: None,
                resolved_at: None,
            },
        };
        AssignInquiry.commit(&mut tx, &assign).await.unwrap();
        let reply = RespondPlan {
            inquiry_id: made.id.clone(),
            comment: NewComment {
                id: new_comment_id(),
                author_type: "staff".into(),
                agent_id: "adm_sj".into(),
                text: "Thanks, we are on it.".into(),
                message_channel: Some("email".into()),
            },
        };
        RespondInquiry.commit(&mut tx, &reply).await.unwrap();
        let comments = thread(&mut tx, &made.id).await.unwrap();
        assert_eq!(comments.len(), 2);
        assert!(comments[1].date_sent.is_none());
        let row = inquiry(&mut tx, &made.id).await.unwrap().unwrap();
        let close = ClosePlan {
            inquiry_id: made.id.clone(),
            before: InquiryState::from(&row),
            after: InquiryState {
                status: "resolved".into(),
                resolution: Some("Email changed.".into()),
                resolved_at: Some(cx.clock.now()),
                ..InquiryState::from(&row)
            },
            notice: None,
        };
        let closed = CloseInquiry.commit(&mut tx, &close).await.unwrap();
        assert_eq!(
            inquiry(&mut tx, &made.id)
                .await
                .unwrap()
                .unwrap()
                .inquiry_status,
            "resolved"
        );
        CloseInquiry.undo(&mut tx, &close, &closed).await.unwrap();
        AssignInquiry.undo(&mut tx, &assign, &closed).await.unwrap();
        assert_eq!(
            inquiry(&mut tx, &made.id)
                .await
                .unwrap()
                .unwrap()
                .inquiry_status,
            "received"
        );
        drop(tx); // Rolled back.
        let got = GetInquiry
            .read(
                &cx,
                GetInquiryInput {
                    inquiry_id: "inq_771".into(),
                    fields: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(got["thread"][0]["text"], "The sole is coming apart.");
        assert_eq!(got["order_id"], "ord_20402");
    }
}
