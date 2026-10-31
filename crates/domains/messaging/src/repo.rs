//! SQL over the messaging tables (`migrations/0009_messaging.sql`, `0016`, `0028`). Every query
//! filters `tenant_id`; values are always bind parameters.

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
    /// The campaign a marketing message is one shopper's copy of.
    pub campaign_id: Option<String>,
}

const MESSAGE_COLUMNS: &str = "id, recipient_id, about_id, message_kind, message_channel,
    headline, text, date_sent, date_read, campaign_id";

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

// ── campaigns (`messaging.campaign`) ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct CampaignRow {
    pub id: String,
    pub name: String,
    pub message_channel: String,
    pub template_id: Option<String>,
    pub valid_for_member_tier: Option<String>,
    pub segment_id: Option<String>,
    pub recipient_ids: Option<Vec<String>>,
    pub variable_values: Option<serde_json::Value>,
    pub campaign_status: String,
    pub scheduled_time: Option<DateTime<Utc>>,
    pub date_sent: Option<DateTime<Utc>>,
    pub recipient_count: Option<i32>,
    pub error: Option<serde_json::Value>,
    pub change_set_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub version: i32,
}

const CAMPAIGN_COLUMNS: &str = "id, name, message_channel, template_id, valid_for_member_tier,
    segment_id, recipient_ids, variable_values, campaign_status, scheduled_time, date_sent,
    recipient_count, error, change_set_id, created_at, version";

pub async fn campaign(tx: &mut TxCx, id: &str) -> Result<Option<CampaignRow>, ToolError> {
    sqlx::query_as::<_, CampaignRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {CAMPAIGN_COLUMNS} FROM messaging.campaign WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The campaign, locked for a send (two sends of one campaign wait for each other).
pub async fn campaign_for_send(tx: &mut TxCx, id: &str) -> Result<Option<CampaignRow>, ToolError> {
    sqlx::query_as::<_, CampaignRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {CAMPAIGN_COLUMNS} FROM messaging.campaign WHERE tenant_id = $1 AND id = $2
         FOR UPDATE"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Filters of `search_campaigns`.
#[derive(Debug, Clone, Default)]
pub struct CampaignFilter<'a> {
    pub status: Option<&'a str>,
    pub channel: Option<&'a str>,
    pub template_id: Option<&'a str>,
    pub from: Option<DateTime<Utc>>,
    pub through: Option<DateTime<Utc>>,
}

const CAMPAIGN_FILTER: &str = "tenant_id = $1
    AND ($2::text IS NULL OR campaign_status = $2)
    AND ($3::text IS NULL OR message_channel = $3)
    AND ($4::text IS NULL OR template_id = $4)
    AND ($5::timestamptz IS NULL OR scheduled_time >= $5)
    AND ($6::timestamptz IS NULL OR scheduled_time <= $6)";

/// One page of campaigns, latest send time first (drafts without a time first of all), and the
/// total that match.
pub async fn campaigns(
    tx: &mut TxCx,
    f: &CampaignFilter<'_>,
    limit: i64,
    offset: i64,
) -> Result<(i64, Vec<CampaignRow>), ToolError> {
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM messaging.campaign WHERE {CAMPAIGN_FILTER}"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.status)
    .bind(f.channel)
    .bind(f.template_id)
    .bind(f.from)
    .bind(f.through)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, CampaignRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {CAMPAIGN_COLUMNS} FROM messaging.campaign WHERE {CAMPAIGN_FILTER}
         ORDER BY scheduled_time DESC NULLS FIRST, created_at DESC, id DESC LIMIT $7 OFFSET $8"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.status)
    .bind(f.channel)
    .bind(f.template_id)
    .bind(f.from)
    .bind(f.through)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

/// A campaign as it is set up.
#[derive(Debug, Clone)]
pub struct NewCampaign<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub channel: &'a str,
    pub template_id: &'a str,
    pub tier: Option<&'a str>,
    pub segment_id: Option<&'a str>,
    pub recipient_ids: Option<&'a [String]>,
    pub variable_values: &'a serde_json::Value,
    pub status: &'a str,
    pub scheduled_time: Option<DateTime<Utc>>,
    pub recipient_count: i32,
}

/// A campaign already set up with the same name, channel, audience and time that is not
/// cancelled: the same send is never booked twice.
pub async fn same_campaign(
    tx: &mut TxCx,
    c: &NewCampaign<'_>,
) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM messaging.campaign
         WHERE tenant_id = $1 AND name = $2 AND message_channel = $3
           AND valid_for_member_tier IS NOT DISTINCT FROM $4
           AND segment_id IS NOT DISTINCT FROM $5
           AND recipient_ids IS NOT DISTINCT FROM $6
           AND scheduled_time IS NOT DISTINCT FROM $7
           AND campaign_status <> 'cancelled'
         ORDER BY created_at LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(c.name)
    .bind(c.channel)
    .bind(c.tier)
    .bind(c.segment_id)
    .bind(c.recipient_ids)
    .bind(c.scheduled_time)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Creates the campaign; running it again with the same id changes nothing.
pub async fn insert_campaign(tx: &mut TxCx, c: &NewCampaign<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO messaging.campaign (tenant_id, id, name, message_channel, template_id,
           valid_for_member_tier, segment_id, recipient_ids, variable_values, campaign_status,
           scheduled_time, recipient_count, change_set_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(c.id)
    .bind(c.name)
    .bind(c.channel)
    .bind(c.template_id)
    .bind(c.tier)
    .bind(c.segment_id)
    .bind(c.recipient_ids)
    .bind(c.variable_values)
    .bind(c.status)
    .bind(c.scheduled_time)
    .bind(c.recipient_count)
    .bind(tx.change_set_id.clone())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    sqlx::query(
        "UPDATE messaging.message_template SET template_status = 'in_use', updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND template_status = 'draft'",
    )
    .bind(tx.tenant.as_str())
    .bind(c.template_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Moves a campaign to a status, with when it went out, how many it reached and why it failed.
pub async fn set_campaign_status(
    tx: &mut TxCx,
    id: &str,
    status: &str,
    date_sent: Option<DateTime<Utc>>,
    recipient_count: Option<i32>,
    error: Option<&serde_json::Value>,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE messaging.campaign SET campaign_status = $3, date_sent = COALESCE($4, date_sent),
           recipient_count = COALESCE($5, recipient_count), error = $6, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(status)
    .bind(date_sent)
    .bind(recipient_count)
    .bind(error)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// What a campaign's own messages say: how many went out and how many were opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SendStats {
    pub messages: i64,
    pub opened: i64,
}

pub async fn send_stats(tx: &mut TxCx, campaign_id: &str) -> Result<SendStats, ToolError> {
    let (messages, opened): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(date_read) FROM messaging.message
         WHERE tenant_id = $1 AND campaign_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(campaign_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(SendStats { messages, opened })
}

/// Marketing messages each of these shoppers was sent in `[from, to)`, for the daily cap.
pub async fn marketing_sent(
    tx: &mut TxCx,
    recipients: &[String],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<(String, i64)>, ToolError> {
    sqlx::query_as(
        "SELECT recipient_id, count(*) FROM messaging.message
         WHERE tenant_id = $1 AND recipient_id = ANY($2) AND message_kind = 'campaign'
           AND date_sent >= $3 AND date_sent < $4
         GROUP BY recipient_id",
    )
    .bind(tx.tenant.as_str())
    .bind(recipients)
    .bind(from)
    .bind(to)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// One shopper's copy of a campaign.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignMessage {
    pub id: String,
    pub recipient_id: String,
    pub headline: Option<String>,
    pub text: String,
}

/// Records a campaign's messages as sent now (a message already recorded stays as it is).
pub async fn insert_campaign_messages(
    tx: &mut TxCx,
    campaign_id: &str,
    channel: &str,
    messages: &[CampaignMessage],
) -> Result<u64, ToolError> {
    let ids: Vec<&str> = messages.iter().map(|m| m.id.as_str()).collect();
    let recipients: Vec<&str> = messages.iter().map(|m| m.recipient_id.as_str()).collect();
    let headlines: Vec<Option<&str>> = messages.iter().map(|m| m.headline.as_deref()).collect();
    let texts: Vec<&str> = messages.iter().map(|m| m.text.as_str()).collect();
    let now = tx.clock.now();
    let sender = tx.principal.subject.clone();
    let done = sqlx::query(
        "INSERT INTO messaging.message (tenant_id, id, recipient_id, message_kind,
           message_channel, headline, text, sender_id, date_sent, campaign_id, change_set_id)
         SELECT $1, m.id, m.recipient_id, 'campaign', $6, m.headline, m.text, $7, $8, $9, $10
         FROM unnest($2::text[], $3::text[], $4::text[], $5::text[])
              AS m(id, recipient_id, headline, text)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(&ids)
    .bind(&recipients)
    .bind(&headlines)
    .bind(&texts)
    .bind(channel)
    .bind(&sender)
    .bind(now)
    .bind(campaign_id)
    .bind(tx.change_set_id.clone())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(done.rows_affected())
}
