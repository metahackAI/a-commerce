//! SQL over this domain's own read models, the `analytics` views (migrations 0011 and 0016).
//! Nothing here reads another domain's tables; the views are the read model, and what they
//! cannot hold without naming people comes from `acom_sales::service::for_analytics`.

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// Revenue and orders of one store day (`analytics.daily_sales`).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct DayTotal {
    pub observation_date: NaiveDate,
    pub revenue_value: Decimal,
    pub order_count: i64,
}

/// Sales of the given days in the store's currency; days without sales have no row.
pub async fn daily_totals(tx: &mut TxCx, days: &[NaiveDate]) -> Result<Vec<DayTotal>, ToolError> {
    sqlx::query_as(
        "SELECT observation_date, revenue_value, order_count FROM analytics.daily_sales
         WHERE tenant_id = $1 AND observation_date = ANY($2) AND revenue_currency = $3
         ORDER BY observation_date",
    )
    .bind(tx.tenant.as_str())
    .bind(days)
    .bind(tx.currency.as_str())
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A variant out of stock now (`analytics.variant_stock_out`).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StockOut {
    pub product_id: String,
    pub product_group_id: String,
    pub product_name: String,
    pub size: Option<String>,
    pub is_bestseller: bool,
    pub sold_out_at: DateTime<Utc>,
    pub availability_starts: Option<DateTime<Utc>>,
}

/// Variants that ran out before `before` and are still out.
pub async fn stock_outs(tx: &mut TxCx, before: DateTime<Utc>) -> Result<Vec<StockOut>, ToolError> {
    sqlx::query_as(
        "SELECT product_id, product_group_id, product_name, size, is_bestseller, sold_out_at,
                availability_starts
         FROM analytics.variant_stock_out
         WHERE tenant_id = $1 AND sold_out_at < $2
         ORDER BY product_group_id, product_id",
    )
    .bind(tx.tenant.as_str())
    .bind(before)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// What some variants sold over some days, per product and day.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct LineSales {
    pub product_group_id: String,
    pub observation_date: NaiveDate,
    pub revenue_value: Decimal,
    pub order_count: i64,
}

/// Net line revenue and orders of `variants` on `days`, leaving out orders that came from
/// `except_campaigns` (their effect is counted with the campaign).
pub async fn variant_sales(
    tx: &mut TxCx,
    variants: &[String],
    days: &[NaiveDate],
    except_campaigns: &[String],
) -> Result<Vec<LineSales>, ToolError> {
    sqlx::query_as(
        "SELECT product_group_id, observation_date, sum(revenue_value) AS revenue_value,
                count(DISTINCT order_id) AS order_count
         FROM analytics.order_line_summary
         WHERE tenant_id = $1 AND product_id = ANY($2) AND observation_date = ANY($3)
           AND revenue_currency = $4
           AND (campaign_id IS NULL OR NOT campaign_id = ANY($5))
         GROUP BY product_group_id, observation_date
         ORDER BY product_group_id, observation_date",
    )
    .bind(tx.tenant.as_str())
    .bind(variants)
    .bind(days)
    .bind(tx.currency.as_str())
    .bind(except_campaigns)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Distinct orders holding any of `variants`, per day (an order with two of them counts once).
pub async fn variant_orders(
    tx: &mut TxCx,
    variants: &[String],
    days: &[NaiveDate],
    except_campaigns: &[String],
) -> Result<Vec<(NaiveDate, i64)>, ToolError> {
    sqlx::query_as(
        "SELECT observation_date, count(DISTINCT order_id)
         FROM analytics.order_line_summary
         WHERE tenant_id = $1 AND product_id = ANY($2) AND observation_date = ANY($3)
           AND (campaign_id IS NULL OR NOT campaign_id = ANY($4))
         GROUP BY observation_date",
    )
    .bind(tx.tenant.as_str())
    .bind(variants)
    .bind(days)
    .bind(except_campaigns)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A campaign as analytics reads it (`analytics.campaign_send`).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct CampaignSend {
    pub campaign_id: String,
    pub name: String,
    pub template_id: Option<String>,
    pub message_channel: String,
    pub campaign_status: String,
    pub scheduled_time: Option<DateTime<Utc>>,
    pub date_sent: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
}

const CAMPAIGN_COLUMNS: &str = "campaign_id, name, template_id, message_channel, campaign_status,
    scheduled_time, date_sent, error_message";

/// Campaigns due in `[from, until)` that went out to nobody: the send failed or was cancelled.
pub async fn missed_campaigns(
    tx: &mut TxCx,
    from: DateTime<Utc>,
    until: DateTime<Utc>,
) -> Result<Vec<CampaignSend>, ToolError> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {CAMPAIGN_COLUMNS} FROM analytics.campaign_send
         WHERE tenant_id = $1 AND date_sent IS NULL AND campaign_status IN ('failed', 'cancelled')
           AND scheduled_time >= $2 AND scheduled_time < $3
         ORDER BY scheduled_time, campaign_id"
    )))
    .bind(tx.tenant.as_str())
    .bind(from)
    .bind(until)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Sends of one template that went out, oldest first.
pub async fn sent_campaigns_of(
    tx: &mut TxCx,
    template_id: &str,
) -> Result<Vec<CampaignSend>, ToolError> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {CAMPAIGN_COLUMNS} FROM analytics.campaign_send
         WHERE tenant_id = $1 AND template_id = $2 AND date_sent IS NOT NULL
         ORDER BY date_sent, campaign_id"
    )))
    .bind(tx.tenant.as_str())
    .bind(template_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Revenue and orders that came from `campaigns`, per day of `days`.
pub async fn campaign_sales(
    tx: &mut TxCx,
    campaigns: &[String],
    days: &[NaiveDate],
) -> Result<Vec<DayTotal>, ToolError> {
    sqlx::query_as(
        "SELECT observation_date, sum(revenue_value) AS revenue_value, count(*) AS order_count
         FROM analytics.order_summary
         WHERE tenant_id = $1 AND order_status <> 'cancelled' AND campaign_id = ANY($2)
           AND observation_date = ANY($3) AND revenue_currency = $4
         GROUP BY observation_date
         ORDER BY observation_date",
    )
    .bind(tx.tenant.as_str())
    .bind(campaigns)
    .bind(days)
    .bind(tx.currency.as_str())
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// One bucket of a trend or overview: a day, week or month start (or the whole period), an
/// optional dimension value, and the order figures in it.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Bucket {
    pub observation_date: Option<NaiveDate>,
    pub dimension: Option<String>,
    /// Paid orders, cancelled ones included.
    pub placed: i64,
    pub cancelled: i64,
    /// Paid, not cancelled orders.
    pub order_count: i64,
    pub revenue_value: Decimal,
    /// Orders with a return asked for (and not declined or withdrawn).
    pub returned: i64,
}

/// How buckets are cut: by day, week (starting Monday) or month, or the whole period.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    Day,
    Week,
    Month,
    Whole,
}

impl Cut {
    fn date_expr(self) -> &'static str {
        match self {
            Cut::Day => "observation_date",
            Cut::Week => "date_trunc('week', observation_date::timestamp)::date",
            Cut::Month => "date_trunc('month', observation_date::timestamp)::date",
            Cut::Whole => "NULL::date",
        }
    }
}

/// Which dimension buckets are split by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    None,
    MemberTier,
    Category,
}

/// Order figures of `[from, through]` per bucket.
pub async fn buckets(
    tx: &mut TxCx,
    from: NaiveDate,
    through: NaiveDate,
    cut: Cut,
    split: Split,
) -> Result<Vec<Bucket>, ToolError> {
    let date = cut.date_expr();
    let sql = match split {
        Split::None | Split::MemberTier => {
            let dim = if split == Split::MemberTier {
                "member_tier"
            } else {
                "NULL::text"
            };
            format!(
                "SELECT {date} AS observation_date, {dim} AS dimension,
                        count(*) AS placed,
                        count(*) FILTER (WHERE order_status = 'cancelled') AS cancelled,
                        count(*) FILTER (WHERE order_status <> 'cancelled') AS order_count,
                        coalesce(sum(revenue_value) FILTER (WHERE order_status <> 'cancelled'), 0)
                          AS revenue_value,
                        count(*) FILTER (WHERE order_status <> 'cancelled' AND has_return_request)
                          AS returned
                 FROM analytics.order_summary
                 WHERE tenant_id = $1 AND observation_date BETWEEN $2 AND $3
                   AND revenue_currency = $4
                 GROUP BY 1, 2 ORDER BY 1, 2"
            )
        }
        Split::Category => format!(
            "SELECT {date} AS observation_date, category_id AS dimension,
                    count(DISTINCT order_id) AS placed, 0::bigint AS cancelled,
                    count(DISTINCT order_id) AS order_count,
                    coalesce(sum(revenue_value), 0) AS revenue_value, 0::bigint AS returned
             FROM analytics.order_line_summary
             WHERE tenant_id = $1 AND observation_date BETWEEN $2 AND $3
               AND revenue_currency = $4
             GROUP BY 1, 2 ORDER BY 1, 2"
        ),
    };
    sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(tx.tenant.as_str())
        .bind(from)
        .bind(through)
        .bind(tx.currency.as_str())
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)
}

/// Accounts created in `[from, through]`, per tier the members are in now.
pub async fn new_members(
    tx: &mut TxCx,
    from: NaiveDate,
    through: NaiveDate,
) -> Result<Vec<(Option<String>, i64)>, ToolError> {
    sqlx::query_as(
        "SELECT member_tier, sum(member_count)::bigint FROM analytics.daily_new_members
         WHERE tenant_id = $1 AND observation_date BETWEEN $2 AND $3
         GROUP BY member_tier ORDER BY member_tier",
    )
    .bind(tx.tenant.as_str())
    .bind(from)
    .bind(through)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Members per tier now.
pub async fn tier_counts(tx: &mut TxCx) -> Result<Vec<(String, i64)>, ToolError> {
    sqlx::query_as(
        "SELECT member_tier, member_count FROM analytics.member_tier_count
         WHERE tenant_id = $1 ORDER BY member_tier",
    )
    .bind(tx.tenant.as_str())
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}
