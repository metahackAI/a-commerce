//! SQL over the messaging tables (`migrations/0009_messaging.sql`). Every query filters
//! `tenant_id`; values are always bind parameters.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};

pub fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MessageRow {
    pub id: String,
    pub recipient_id: String,
    pub about_id: Option<String>,
    pub message_kind: String,
    pub message_channel: String,
    pub headline: Option<String>,
    pub text: String,
    pub date_sent: Option<DateTime<Utc>>,
    pub date_read: Option<DateTime<Utc>>,
}

const MESSAGE_COLUMNS: &str = "id, recipient_id, about_id, message_kind, message_channel,
    headline, text, date_sent, date_read";

pub async fn message(tx: &mut TxCx, id: &str) -> Result<Option<MessageRow>, ToolError> {
    sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {MESSAGE_COLUMNS} FROM messaging.message WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A message already sent with the same recipient, subject matter and kind (and, for `custom`,
/// the same text): the same notice never goes out twice.
pub async fn same_message(
    tx: &mut TxCx,
    recipient_id: &str,
    about_id: Option<&str>,
    kind: &str,
    text: &str,
) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM messaging.message
         WHERE tenant_id = $1 AND recipient_id = $2 AND about_id IS NOT DISTINCT FROM $3
           AND message_kind = $4 AND ($4 <> 'custom' OR text = $5)
         ORDER BY created_at LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(recipient_id)
    .bind(about_id)
    .bind(kind)
    .bind(text)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A message as it is sent.
#[derive(Debug, Clone)]
pub struct NewMessage<'a> {
    pub id: &'a str,
    pub recipient_id: &'a str,
    pub about_id: Option<&'a str>,
    pub kind: &'a str,
    pub channel: &'a str,
    pub headline: Option<&'a str>,
    pub text: &'a str,
}

pub async fn insert_message(tx: &mut TxCx, m: &NewMessage<'_>) -> Result<(), ToolError> {
    let now = tx.clock.now();
    let sender = tx.principal.subject.clone();
    sqlx::query(
        "INSERT INTO messaging.message (tenant_id, id, recipient_id, about_id, message_kind,
           message_channel, headline, text, sender_id, date_sent)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(tx.tenant.as_str())
    .bind(m.id)
    .bind(m.recipient_id)
    .bind(m.about_id)
    .bind(m.kind)
    .bind(m.channel)
    .bind(m.headline)
    .bind(m.text)
    .bind(&sender)
    .bind(now)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Filters for the notifications a shopper received.
#[derive(Debug, Clone)]
pub struct NotificationFilter<'a> {
    pub recipient_id: &'a str,
    pub channel: Option<&'a str>,
    pub read: Option<bool>,
    pub from: Option<DateTime<Utc>>,
    pub through: Option<DateTime<Utc>>,
}

const NOTIFICATION_FILTER: &str = "tenant_id = $1 AND recipient_id = $2 AND date_sent IS NOT NULL
    AND ($3::text IS NULL OR message_channel = $3)
    AND ($4::boolean IS NULL OR (date_read IS NOT NULL) = $4)
    AND ($5::timestamptz IS NULL OR date_sent >= $5)
    AND ($6::timestamptz IS NULL OR date_sent <= $6)";

/// One page of sent messages, newest first, and the total that match.
pub async fn notifications(
    tx: &mut TxCx,
    f: &NotificationFilter<'_>,
    limit: i64,
    offset: i64,
) -> Result<(i64, Vec<MessageRow>), ToolError> {
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM messaging.message WHERE {NOTIFICATION_FILTER}"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.recipient_id)
    .bind(f.channel)
    .bind(f.read)
    .bind(f.from)
    .bind(f.through)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, MessageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {MESSAGE_COLUMNS} FROM messaging.message WHERE {NOTIFICATION_FILTER}
         ORDER BY date_sent DESC, id DESC LIMIT $7 OFFSET $8"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.recipient_id)
    .bind(f.channel)
    .bind(f.read)
    .bind(f.from)
    .bind(f.through)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TemplateRow {
    pub id: String,
    pub name: String,
    pub message_channel: String,
    pub message_purpose: String,
    pub headline: Option<String>,
    pub text: String,
    pub variables: Vec<String>,
    pub in_language: String,
    pub template_status: String,
    pub created_at: DateTime<Utc>,
}

const TEMPLATE_COLUMNS: &str = "id, name, message_channel, message_purpose, headline, text,
    variables, in_language, template_status, created_at";

pub async fn template(tx: &mut TxCx, id: &str) -> Result<Option<TemplateRow>, ToolError> {
    sqlx::query_as::<_, TemplateRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {TEMPLATE_COLUMNS} FROM messaging.message_template WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

const TEMPLATE_FILTER: &str = "tenant_id = $1 AND template_status <> 'archived'
    AND ($2::text IS NULL OR message_channel = $2)
    AND ($3::text IS NULL OR message_purpose = $3)
    AND ($4::text IS NULL OR in_language = $4)";

/// One page of templates, newest first, and the total that match.
pub async fn templates(
    tx: &mut TxCx,
    channel: Option<&str>,
    purpose: Option<&str>,
    in_language: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(i64, Vec<TemplateRow>), ToolError> {
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM messaging.message_template WHERE {TEMPLATE_FILTER}"
    )))
    .bind(tx.tenant.as_str())
    .bind(channel)
    .bind(purpose)
    .bind(in_language)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, TemplateRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {TEMPLATE_COLUMNS} FROM messaging.message_template WHERE {TEMPLATE_FILTER}
         ORDER BY created_at DESC, id DESC LIMIT $5 OFFSET $6"
    )))
    .bind(tx.tenant.as_str())
    .bind(channel)
    .bind(purpose)
    .bind(in_language)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

/// A template as it is created.
#[derive(Debug, Clone)]
pub struct NewTemplate<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub channel: &'a str,
    pub purpose: &'a str,
    pub headline: Option<&'a str>,
    pub text: &'a str,
    pub variables: &'a [String],
    pub in_language: &'a str,
}

/// Creates the template; running it again with the same id changes nothing.
pub async fn insert_template(tx: &mut TxCx, t: &NewTemplate<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO messaging.message_template (tenant_id, id, name, message_channel,
           message_purpose, headline, text, variables, in_language, template_status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'draft')
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(t.id)
    .bind(t.name)
    .bind(t.channel)
    .bind(t.purpose)
    .bind(t.headline)
    .bind(t.text)
    .bind(t.variables)
    .bind(t.in_language)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Sets a template's status (`archived` takes a new draft back out of the list).
pub async fn set_template_status(tx: &mut TxCx, id: &str, status: &str) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE messaging.message_template SET template_status = $3, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND template_status <> $3",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(status)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}
