//! SQL over the fulfillment tables (`migrations/0008_fulfillment.sql`). Every query filters
//! `tenant_id`; values are always bind parameters.

use acom_contract::{ErrorCode, ToolError};
use acom_core::{Currency, Money};
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

pub fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// An amount read from a `*_value` + `*_currency` pair.
pub fn money(value: Decimal, currency: &str) -> Result<Money, ToolError> {
    let c = Currency::parse(currency.trim())
        .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?;
    Ok(Money::new(value, c))
}

/// Claim statuses after which nothing more happens to a claim.
pub const CLOSED_STATUSES: &[&str] = &["completed", "declined", "withdrawn", "converted"];

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ClaimRow {
    pub id: String,
    pub order_id: String,
    pub customer_id: String,
    pub claim_type: String,
    pub claim_status: String,
    pub claim_path: String,
    pub claim_reason: Option<String>,
    pub cancel_reason: Option<String>,
    pub reason_text: Option<String>,
    pub liability: String,
    pub return_fees: Option<String>,
    pub send_back_method: Option<String>,
    pub pickup_window: Option<Value>,
    pub evidence: Value,
    pub amount_value: Decimal,
    pub amount_currency: String,
    pub refund_estimate_value: Option<Decimal>,
    pub refund_estimate_currency: Option<String>,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub version: i32,
}

impl ClaimRow {
    pub fn amount(&self) -> Result<Money, ToolError> {
        money(self.amount_value, &self.amount_currency)
    }

    /// The return or exchange reason, or the cancel reason.
    pub fn reason(&self) -> Option<&str> {
        self.claim_reason
            .as_deref()
            .or(self.cancel_reason.as_deref())
    }

    pub fn is_open(&self) -> bool {
        !CLOSED_STATUSES.contains(&self.claim_status.as_str())
    }
}

const CLAIM_COLUMNS: &str = "id, order_id, customer_id, claim_type, claim_status, claim_path,
    claim_reason, cancel_reason, reason_text, liability, return_fees, send_back_method,
    pickup_window, evidence, amount_value, amount_currency::text AS amount_currency,
    refund_estimate_value, refund_estimate_currency::text AS refund_estimate_currency,
    start_time, end_time, version";

pub async fn claim(tx: &mut TxCx, claim_id: &str) -> Result<Option<ClaimRow>, ToolError> {
    sqlx::query_as::<_, ClaimRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {CLAIM_COLUMNS} FROM fulfillment.claim WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(claim_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The claim, locked for the rest of the transaction.
pub async fn claim_for_update(
    tx: &mut TxCx,
    claim_id: &str,
) -> Result<Option<ClaimRow>, ToolError> {
    sqlx::query_as::<_, ClaimRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {CLAIM_COLUMNS} FROM fulfillment.claim WHERE tenant_id = $1 AND id = $2
         FOR UPDATE"
    )))
    .bind(tx.tenant.as_str())
    .bind(claim_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Every claim on an order, oldest first.
pub async fn claims_on_order(tx: &mut TxCx, order_id: &str) -> Result<Vec<ClaimRow>, ToolError> {
    sqlx::query_as::<_, ClaimRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {CLAIM_COLUMNS} FROM fulfillment.claim WHERE tenant_id = $1 AND order_id = $2
         ORDER BY start_time, id"
    )))
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Filters for the return review queue and the claim search. Lists left empty match all.
#[derive(Debug, Default, Clone)]
pub struct ClaimFilter {
    pub claim_type: Option<String>,
    pub statuses: Vec<String>,
    pub reasons: Vec<String>,
    pub liability: Option<String>,
    pub order_id: Option<String>,
    pub customer_id: Option<String>,
    pub min_amount: Option<Decimal>,
    pub from: Option<DateTime<Utc>>,
    pub through: Option<DateTime<Utc>>,
}

const CLAIM_FILTER: &str = "tenant_id = $1
    AND ($2::text IS NULL OR claim_type = $2)
    AND (cardinality($3::text[]) = 0 OR claim_status = ANY($3))
    AND (cardinality($4::text[]) = 0 OR claim_reason = ANY($4) OR cancel_reason = ANY($4))
    AND ($5::text IS NULL OR liability = $5)
    AND ($6::text IS NULL OR order_id = $6)
    AND ($7::text IS NULL OR customer_id = $7)
    AND ($8::numeric IS NULL OR amount_value >= $8)
    AND ($9::timestamptz IS NULL OR start_time >= $9)
    AND ($10::timestamptz IS NULL OR start_time <= $10)";

/// How a claim list is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimOrder {
    /// Waiting longest first.
    Oldest,
    Newest,
    Largest,
}

/// One page of claims and the total that match.
pub async fn search_claims(
    tx: &mut TxCx,
    f: &ClaimFilter,
    order: ClaimOrder,
    limit: i64,
    offset: i64,
) -> Result<(i64, Vec<ClaimRow>), ToolError> {
    let sort = match order {
        ClaimOrder::Oldest => "start_time ASC, id ASC",
        ClaimOrder::Newest => "start_time DESC, id DESC",
        ClaimOrder::Largest => "amount_value DESC, start_time ASC, id ASC",
    };
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM fulfillment.claim WHERE {CLAIM_FILTER}"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.claim_type.as_deref())
    .bind(&f.statuses)
    .bind(&f.reasons)
    .bind(f.liability.as_deref())
    .bind(f.order_id.as_deref())
    .bind(f.customer_id.as_deref())
    .bind(f.min_amount)
    .bind(f.from)
    .bind(f.through)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, ClaimRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {CLAIM_COLUMNS} FROM fulfillment.claim WHERE {CLAIM_FILTER}
         ORDER BY {sort} LIMIT $11 OFFSET $12"
    )))
    .bind(tx.tenant.as_str())
    .bind(f.claim_type.as_deref())
    .bind(&f.statuses)
    .bind(&f.reasons)
    .bind(f.liability.as_deref())
    .bind(f.order_id.as_deref())
    .bind(f.customer_id.as_deref())
    .bind(f.min_amount)
    .bind(f.from)
    .bind(f.through)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((total, rows))
}

/// Whether any claim of the store is in another currency than `currency` (an amount filter
/// in that currency cannot be compared then).
pub async fn other_currency_exists(tx: &mut TxCx, currency: &str) -> Result<bool, ToolError> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM fulfillment.claim
                        WHERE tenant_id = $1 AND amount_currency <> $2)",
    )
    .bind(tx.tenant.as_str())
    .bind(currency)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Return requests a customer sent since `since` (withdrawn ones do not count).
pub async fn recent_returns(
    tx: &mut TxCx,
    customer_id: &str,
    since: DateTime<Utc>,
) -> Result<i64, ToolError> {
    sqlx::query_scalar(
        "SELECT count(*) FROM fulfillment.claim
         WHERE tenant_id = $1 AND customer_id = $2 AND claim_type = 'return'
           AND claim_status <> 'withdrawn' AND start_time >= $3",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(since)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ClaimItemRow {
    pub claim_id: String,
    pub order_item_id: String,
    pub order_quantity: i32,
    pub replacement_product_id: Option<String>,
}

/// The lines of several claims.
pub async fn claim_items(
    tx: &mut TxCx,
    claim_ids: &[String],
) -> Result<Vec<ClaimItemRow>, ToolError> {
    sqlx::query_as::<_, ClaimItemRow>(
        "SELECT claim_id, order_item_id, order_quantity, replacement_product_id
         FROM fulfillment.claim_item WHERE tenant_id = $1 AND claim_id = ANY($2)
         ORDER BY claim_id, order_item_id",
    )
    .bind(tx.tenant.as_str())
    .bind(claim_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ClaimEventRow {
    pub to_state: Option<String>,
    pub start_time: DateTime<Utc>,
}

/// A claim's state changes in order.
pub async fn claim_events(tx: &mut TxCx, claim_id: &str) -> Result<Vec<ClaimEventRow>, ToolError> {
    sqlx::query_as::<_, ClaimEventRow>(
        "SELECT to_state, start_time FROM fulfillment.claim_event
         WHERE tenant_id = $1 AND claim_id = $2 ORDER BY position",
    )
    .bind(tx.tenant.as_str())
    .bind(claim_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct InspectionRow {
    pub id: String,
    pub inspection_condition: String,
    pub liability: Option<String>,
    pub description: Option<String>,
    pub end_time: DateTime<Utc>,
}

pub async fn inspection(tx: &mut TxCx, claim_id: &str) -> Result<Option<InspectionRow>, ToolError> {
    sqlx::query_as::<_, InspectionRow>(
        "SELECT id, inspection_condition, liability, description, end_time
         FROM fulfillment.inspection WHERE tenant_id = $1 AND claim_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(claim_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RefundRow {
    pub id: String,
    pub claim_id: String,
    pub order_id: String,
    pub payment_id: Option<String>,
    pub amount_value: Decimal,
    pub amount_currency: String,
    pub refund_method: String,
    pub refund_type: String,
    pub refund_status: String,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
}

impl RefundRow {
    pub fn amount(&self) -> Result<Money, ToolError> {
        money(self.amount_value, &self.amount_currency)
    }
}

const REFUND_COLUMNS: &str = "id, claim_id, order_id, payment_id, amount_value,
    amount_currency::text AS amount_currency, refund_method, refund_type, refund_status,
    start_time, end_time";

pub async fn refund(tx: &mut TxCx, refund_id: &str) -> Result<Option<RefundRow>, ToolError> {
    sqlx::query_as::<_, RefundRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {REFUND_COLUMNS} FROM fulfillment.refund WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(refund_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The one refund a claim can have.
pub async fn refund_of_claim(
    tx: &mut TxCx,
    claim_id: &str,
) -> Result<Option<RefundRow>, ToolError> {
    sqlx::query_as::<_, RefundRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {REFUND_COLUMNS} FROM fulfillment.refund WHERE tenant_id = $1 AND claim_id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(claim_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Every refund on an order that did not fail.
pub async fn refunds_on_order(tx: &mut TxCx, order_id: &str) -> Result<Vec<RefundRow>, ToolError> {
    sqlx::query_as::<_, RefundRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {REFUND_COLUMNS} FROM fulfillment.refund
         WHERE tenant_id = $1 AND order_id = $2 AND refund_status <> 'failed'
         ORDER BY start_time, id"
    )))
    .bind(tx.tenant.as_str())
    .bind(order_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A new refund, as `refund_return` or an approved cancel sends it.
#[derive(Debug, Clone)]
pub struct NewRefund<'a> {
    pub id: &'a str,
    pub claim_id: &'a str,
    pub order_id: &'a str,
    pub payment_id: Option<&'a str>,
    pub amount: Money,
    pub refund_method: &'a str,
    pub refund_type: &'a str,
}

pub async fn insert_refund(tx: &mut TxCx, r: &NewRefund<'_>) -> Result<(), ToolError> {
    let now = tx.clock.now();
    sqlx::query(
        "INSERT INTO fulfillment.refund (tenant_id, id, claim_id, order_id, payment_id,
           amount_value, amount_currency, refund_method, refund_type, refund_status, start_time)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'requested', $10)",
    )
    .bind(tx.tenant.as_str())
    .bind(r.id)
    .bind(r.claim_id)
    .bind(r.order_id)
    .bind(r.payment_id)
    .bind(r.amount.value)
    .bind(r.amount.currency.as_str())
    .bind(r.refund_method)
    .bind(r.refund_type)
    .bind(now)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DeliveryRow {
    pub id: String,
    pub order_id: String,
    pub item_shipped: Option<Value>,
    pub delivery_state: String,
    pub delivery_address: Value,
    pub has_delivery_method: String,
    pub carrier: Option<String>,
    pub tracking_number: Option<String>,
    pub tracking_url: Option<String>,
    pub expected_arrival_until: Option<DateTime<Utc>>,
    pub shipped_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub version: i32,
}

const DELIVERY_COLUMNS: &str = "id, order_id, item_shipped, delivery_state, delivery_address,
    has_delivery_method, carrier, tracking_number, tracking_url, expected_arrival_until, shipped_at, delivered_at, updated_at, version";

pub async fn delivery(tx: &mut TxCx, delivery_id: &str) -> Result<Option<DeliveryRow>, ToolError> {
    sqlx::query_as::<_, DeliveryRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {DELIVERY_COLUMNS} FROM fulfillment.parcel_delivery
         WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(delivery_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

pub async fn delivery_by_tracking(
    tx: &mut TxCx,
    tracking_number: &str,
) -> Result<Option<DeliveryRow>, ToolError> {
    sqlx::query_as::<_, DeliveryRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {DELIVERY_COLUMNS} FROM fulfillment.parcel_delivery
         WHERE tenant_id = $1 AND tracking_number = $2 ORDER BY created_at DESC LIMIT 1"
    )))
    .bind(tx.tenant.as_str())
    .bind(tracking_number)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The deliveries of several orders.
pub async fn deliveries_of_orders(
    tx: &mut TxCx,
    order_ids: &[String],
) -> Result<Vec<DeliveryRow>, ToolError> {
    sqlx::query_as::<_, DeliveryRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {DELIVERY_COLUMNS} FROM fulfillment.parcel_delivery
         WHERE tenant_id = $1 AND order_id = ANY($2) ORDER BY order_id, created_at, id"
    )))
    .bind(tx.tenant.as_str())
    .bind(order_ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Moves a parcel to `state` and returns the state it had, or `None` when it does not exist.
pub async fn set_delivery_state(
    tx: &mut TxCx,
    delivery_id: &str,
    state: &str,
) -> Result<Option<String>, ToolError> {
    let before: Option<String> = sqlx::query_scalar(
        "SELECT delivery_state FROM fulfillment.parcel_delivery
         WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(tx.tenant.as_str())
    .bind(delivery_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    if before.as_deref().is_some_and(|b| b != state) {
        sqlx::query(
            "UPDATE fulfillment.parcel_delivery SET delivery_state = $3, updated_at = now(),
               version = version + 1
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(delivery_id)
        .bind(state)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    Ok(before)
}

/// A new claim and its lines.
#[derive(Debug, Clone)]
pub struct NewClaim<'a> {
    pub id: &'a str,
    pub order_id: &'a str,
    pub customer_id: &'a str,
    pub claim_type: &'a str,
    pub claim_status: &'a str,
    pub claim_path: &'a str,
    pub claim_reason: Option<&'a str>,
    pub cancel_reason: Option<&'a str>,
    pub reason_text: Option<&'a str>,
    pub liability: &'a str,
    pub return_fees: Option<&'a str>,
    pub send_back_method: Option<&'a str>,
    pub pickup_window: Option<Value>,
    pub evidence: Value,
    pub amount: Money,
    pub refund_estimate: Option<Money>,
    /// `(order_item_id, quantity, replacement variant)`.
    pub items: Vec<(String, i32, Option<String>)>,
}

pub async fn insert_claim(tx: &mut TxCx, c: &NewClaim<'_>) -> Result<(), ToolError> {
    let now = tx.clock.now();
    let closed = CLOSED_STATUSES.contains(&c.claim_status);
    sqlx::query(
        "INSERT INTO fulfillment.claim (tenant_id, id, order_id, customer_id, claim_type,
           claim_status, claim_path, claim_reason, cancel_reason, reason_text, liability,
           return_fees, send_back_method, pickup_window, evidence, amount_value, amount_currency,
           refund_estimate_value, refund_estimate_currency, start_time, end_time)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                 $18, $19, $20, $21)",
    )
    .bind(tx.tenant.as_str())
    .bind(c.id)
    .bind(c.order_id)
    .bind(c.customer_id)
    .bind(c.claim_type)
    .bind(c.claim_status)
    .bind(c.claim_path)
    .bind(c.claim_reason)
    .bind(c.cancel_reason)
    .bind(c.reason_text)
    .bind(c.liability)
    .bind(c.return_fees)
    .bind(c.send_back_method)
    .bind(&c.pickup_window)
    .bind(&c.evidence)
    .bind(c.amount.value)
    .bind(c.amount.currency.as_str())
    .bind(c.refund_estimate.map(|m| m.value))
    .bind(c.refund_estimate.map(|m| m.currency.as_str().to_owned()))
    .bind(now)
    .bind(closed.then_some(now))
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    for (item, quantity, replacement) in &c.items {
        sqlx::query(
            "INSERT INTO fulfillment.claim_item (tenant_id, claim_id, order_item_id,
               order_quantity, replacement_product_id)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(tx.tenant.as_str())
        .bind(c.id)
        .bind(item)
        .bind(quantity)
        .bind(replacement.as_deref())
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    Ok(())
}

/// Moves a claim from `from` to `to` (setting `end_time` when `to` closes it). Returns false
/// when the claim was no longer in `from`.
pub async fn move_claim(
    tx: &mut TxCx,
    claim_id: &str,
    from: &str,
    to: &str,
) -> Result<bool, ToolError> {
    let now = tx.clock.now();
    let closes = CLOSED_STATUSES.contains(&to);
    let moved = sqlx::query(
        "UPDATE fulfillment.claim SET claim_status = $4,
           end_time = CASE WHEN $5 THEN $6 ELSE NULL END,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND claim_status = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(claim_id)
    .bind(from)
    .bind(to)
    .bind(closes)
    .bind(now)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected();
    Ok(moved == 1)
}

/// Sets who is at fault on a claim (an operator's decision when approving).
pub async fn set_liability(
    tx: &mut TxCx,
    claim_id: &str,
    liability: &str,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE fulfillment.claim SET liability = $3, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND liability <> $3",
    )
    .bind(tx.tenant.as_str())
    .bind(claim_id)
    .bind(liability)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// One state change of a claim, appended (`fulfillment.claim_event`).
#[derive(Debug, Clone)]
pub struct ClaimEvent<'a> {
    pub action_type: &'a str,
    pub action_status: &'a str,
    pub instrument: &'a str,
    pub from_state: Option<&'a str>,
    pub to_state: Option<&'a str>,
    pub result: Option<Value>,
}

pub async fn append_event(
    tx: &mut TxCx,
    claim_id: &str,
    e: &ClaimEvent<'_>,
) -> Result<(), ToolError> {
    let now = tx.clock.now();
    let agent = tx.principal.subject.clone();
    let agent_name = tx.principal.name.clone();
    sqlx::query(
        "INSERT INTO fulfillment.claim_event (tenant_id, claim_id, position, action_type,
           action_status, agent_id, agent_name, instrument, from_state, to_state, result,
           start_time, end_time, request_id)
         SELECT $1, $2, coalesce(max(position), 0) + 1, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                $11, $12
         FROM fulfillment.claim_event WHERE tenant_id = $1 AND claim_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(claim_id)
    .bind(e.action_type)
    .bind(e.action_status)
    .bind(&agent)
    .bind(agent_name.as_deref())
    .bind(e.instrument)
    .bind(e.from_state)
    .bind(e.to_state)
    .bind(&e.result)
    .bind(now)
    .bind(&tx.request_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}
