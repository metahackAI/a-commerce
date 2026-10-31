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
    pub charge_value: Option<Decimal>,
    pub charge_currency: Option<String>,
    pub charge_paid_at: Option<DateTime<Utc>>,
}

impl ClaimRow {
    /// The extra cost the shopper pays before the claim goes ahead, if any.
    pub fn charge(&self) -> Result<Option<Money>, ToolError> {
        match (self.charge_value, &self.charge_currency) {
            (Some(v), Some(c)) => money(v, c).map(Some),
            _ => Ok(None),
        }
    }

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
    start_time, end_time, version, charge_value, charge_currency::text AS charge_currency,
    charge_paid_at";

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
    pub result: Option<Value>,
}

/// A claim's state changes in order.
pub async fn claim_events(tx: &mut TxCx, claim_id: &str) -> Result<Vec<ClaimEventRow>, ToolError> {
    sqlx::query_as::<_, ClaimEventRow>(
        "SELECT to_state, start_time, result FROM fulfillment.claim_event
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
    /// The exchange claim this parcel reships, if it is a reshipment.
    pub claim_id: Option<String>,
}

impl DeliveryRow {
    /// The order lines in the parcel and their units; `None` means every line of the order.
    pub fn lines(&self) -> Option<Vec<(String, i32)>> {
        let Some(Value::Array(l)) = &self.item_shipped else {
            return None;
        };
        Some(
            l.iter()
                .filter_map(|x| {
                    let id = x.get("order_item_id")?.as_str()?.to_owned();
                    let q = x
                        .get("order_quantity")
                        .and_then(Value::as_i64)
                        .and_then(|q| i32::try_from(q).ok())
                        .unwrap_or(1);
                    Some((id, q))
                })
                .collect(),
        )
    }
}

const DELIVERY_COLUMNS: &str = "id, order_id, item_shipped, delivery_state, delivery_address,
    has_delivery_method, carrier, tracking_number, tracking_url, expected_arrival_until, shipped_at, delivered_at, updated_at, version, claim_id";

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
    /// The extra cost due before the claim goes ahead (`charge_due`), if any.
    pub charge: Option<Money>,
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
           refund_estimate_value, refund_estimate_currency, start_time, end_time, charge_value,
           charge_currency)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                 $18, $19, $20, $21, $22, $23)",
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
    .bind(c.charge.map(|m| m.value))
    .bind(c.charge.map(|m| m.currency.as_str().to_owned()))
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

/// Appends evidence entries (`[{id, evidence_type, …}]`) to a claim; nothing is ever removed.
pub async fn append_evidence(
    tx: &mut TxCx,
    claim_id: &str,
    entries: &Value,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE fulfillment.claim SET evidence = evidence || $3, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(claim_id)
    .bind(entries)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Records when the shopper paid the claim's extra charge.
pub async fn set_charge_paid(
    tx: &mut TxCx,
    claim_id: &str,
    at: DateTime<Utc>,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE fulfillment.claim SET charge_paid_at = $3, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(claim_id)
    .bind(at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// A new inspection record.
#[derive(Debug, Clone)]
pub struct NewInspection<'a> {
    pub id: &'a str,
    pub claim_id: &'a str,
    pub condition: &'a str,
    pub liability: Option<&'a str>,
    pub description: Option<&'a str>,
    pub agent_id: &'a str,
    pub at: DateTime<Utc>,
}

pub async fn insert_inspection(tx: &mut TxCx, i: &NewInspection<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO fulfillment.inspection (tenant_id, id, claim_id, inspection_condition,
           liability, description, agent_id, end_time)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(i.id)
    .bind(i.claim_id)
    .bind(i.condition)
    .bind(i.liability)
    .bind(i.description)
    .bind(i.agent_id)
    .bind(i.at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Removes an inspection recorded by mistake (undo); the claim's events keep the history.
pub async fn delete_inspection(tx: &mut TxCx, inspection_id: &str) -> Result<(), ToolError> {
    sqlx::query("DELETE FROM fulfillment.inspection WHERE tenant_id = $1 AND id = $2")
        .bind(tx.tenant.as_str())
        .bind(inspection_id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    Ok(())
}

/// The refund, locked for the rest of the transaction.
pub async fn refund_for_update(
    tx: &mut TxCx,
    refund_id: &str,
) -> Result<Option<RefundRow>, ToolError> {
    sqlx::query_as::<_, RefundRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {REFUND_COLUMNS} FROM fulfillment.refund WHERE tenant_id = $1 AND id = $2
         FOR UPDATE"
    )))
    .bind(tx.tenant.as_str())
    .bind(refund_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Moves a refund from `from` to `to`. A refund sent again is open (no end time); a confirmed
/// or failed one ends now. Returns false when the refund was no longer in `from`.
pub async fn move_refund(
    tx: &mut TxCx,
    refund_id: &str,
    from: &str,
    to: &str,
) -> Result<bool, ToolError> {
    let now = tx.clock.now();
    let moved = sqlx::query(
        "UPDATE fulfillment.refund SET refund_status = $4,
           end_time = CASE WHEN $4 = 'requested' THEN NULL ELSE $5 END,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND refund_status = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(refund_id)
    .bind(from)
    .bind(to)
    .bind(now)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected();
    Ok(moved == 1)
}

/// One recorded change of a refund or a parcel.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventRow {
    pub instrument: Option<String>,
    pub to_state: Option<String>,
    pub result: Option<Value>,
    pub start_time: DateTime<Utc>,
}

/// A refund's recorded changes in order.
pub async fn refund_events(tx: &mut TxCx, refund_id: &str) -> Result<Vec<EventRow>, ToolError> {
    sqlx::query_as::<_, EventRow>(
        "SELECT instrument, to_state, result, start_time
         FROM fulfillment.refund_event WHERE tenant_id = $1 AND refund_id = $2
         ORDER BY position",
    )
    .bind(tx.tenant.as_str())
    .bind(refund_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A parcel's recorded changes in order.
pub async fn delivery_events(tx: &mut TxCx, delivery_id: &str) -> Result<Vec<EventRow>, ToolError> {
    sqlx::query_as::<_, EventRow>(
        "SELECT instrument, to_state, result, start_time
         FROM fulfillment.parcel_delivery_event WHERE tenant_id = $1 AND parcel_delivery_id = $2
         ORDER BY position",
    )
    .bind(tx.tenant.as_str())
    .bind(delivery_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Appends one change to the event table of a refund (`refund_event`) or a parcel
/// (`parcel_delivery_event`), with the same columns as a claim's events.
async fn append_to(
    tx: &mut TxCx,
    table: &str,
    key: &str,
    id: &str,
    e: &ClaimEvent<'_>,
) -> Result<(), ToolError> {
    let now = tx.clock.now();
    let agent = tx.principal.subject.clone();
    let agent_name = tx.principal.name.clone();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "INSERT INTO fulfillment.{table} (tenant_id, {key}, position, action_type,
           action_status, agent_id, agent_name, instrument, from_state, to_state, result,
           start_time, end_time, request_id)
         SELECT $1, $2, coalesce(max(position), 0) + 1, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                $11, $12
         FROM fulfillment.{table} WHERE tenant_id = $1 AND {key} = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
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

pub async fn append_refund_event(
    tx: &mut TxCx,
    refund_id: &str,
    e: &ClaimEvent<'_>,
) -> Result<(), ToolError> {
    append_to(tx, "refund_event", "refund_id", refund_id, e).await
}

pub async fn append_delivery_event(
    tx: &mut TxCx,
    delivery_id: &str,
    e: &ClaimEvent<'_>,
) -> Result<(), ToolError> {
    append_to(
        tx,
        "parcel_delivery_event",
        "parcel_delivery_id",
        delivery_id,
        e,
    )
    .await
}

/// The parcel, locked for the rest of the transaction.
pub async fn delivery_for_update(
    tx: &mut TxCx,
    delivery_id: &str,
) -> Result<Option<DeliveryRow>, ToolError> {
    sqlx::query_as::<_, DeliveryRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {DELIVERY_COLUMNS} FROM fulfillment.parcel_delivery
         WHERE tenant_id = $1 AND id = $2 FOR UPDATE"
    )))
    .bind(tx.tenant.as_str())
    .bind(delivery_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A new parcel, as `schedule_shipment` creates it.
#[derive(Debug, Clone)]
pub struct NewDelivery<'a> {
    pub id: &'a str,
    pub order_id: &'a str,
    /// `[{order_item_id, order_quantity}]`, plus `variant_id` for a reshipped replacement.
    pub item_shipped: Value,
    pub delivery_state: &'a str,
    pub delivery_address_id: Option<&'a str>,
    pub delivery_address: Value,
    pub method: &'a str,
    pub carrier: Option<&'a str>,
    pub expected_arrival_until: Option<DateTime<Utc>>,
    pub claim_id: Option<&'a str>,
}

pub async fn insert_delivery(tx: &mut TxCx, d: &NewDelivery<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO fulfillment.parcel_delivery (tenant_id, id, order_id, item_shipped,
           delivery_state, delivery_address_id, delivery_address, has_delivery_method, carrier,
           expected_arrival_until, claim_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(tx.tenant.as_str())
    .bind(d.id)
    .bind(d.order_id)
    .bind(&d.item_shipped)
    .bind(d.delivery_state)
    .bind(d.delivery_address_id)
    .bind(&d.delivery_address)
    .bind(d.method)
    .bind(d.carrier)
    .bind(d.expected_arrival_until)
    .bind(d.claim_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Writes (or, with `None`, clears) the label of a parcel: carrier and tracking number.
pub async fn set_delivery_label(
    tx: &mut TxCx,
    delivery_id: &str,
    carrier: Option<&str>,
    tracking_number: Option<&str>,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE fulfillment.parcel_delivery SET carrier = $3, tracking_number = $4,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(delivery_id)
    .bind(carrier)
    .bind(tracking_number)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// The parcel that already carries this carrier's tracking number, other than `except`.
pub async fn tracking_taken(
    tx: &mut TxCx,
    carrier: &str,
    tracking_number: &str,
    except: &str,
) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM fulfillment.parcel_delivery
         WHERE tenant_id = $1 AND lower(carrier) = lower($2) AND tracking_number = $3
           AND id <> $4
         LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(carrier)
    .bind(tracking_number)
    .bind(except)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// One stored shipping zone (`fulfillment.shipping_zone`).
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct ZoneRow {
    pub id: String,
    pub name: String,
    pub zone_type: String,
    pub shipping_destination: Value,
    pub fee_type: String,
    pub shipping_rate_value: Decimal,
    pub shipping_rate_currency: String,
    pub free_shipping_threshold_value: Option<Decimal>,
    pub free_shipping_threshold_currency: Option<String>,
    pub surcharge_value: Decimal,
    pub surcharge_currency: String,
    pub transit_business_days: i32,
    pub version: i32,
}

const ZONE_COLUMNS: &str = "id, name, zone_type, shipping_destination, fee_type,
    shipping_rate_value, shipping_rate_currency::text AS shipping_rate_currency,
    free_shipping_threshold_value,
    free_shipping_threshold_currency::text AS free_shipping_threshold_currency,
    surcharge_value, surcharge_currency::text AS surcharge_currency, transit_business_days,
    version";

/// Every stored shipping zone of the store, by name.
pub async fn zones(tx: &mut TxCx) -> Result<Vec<ZoneRow>, ToolError> {
    sqlx::query_as::<_, ZoneRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ZONE_COLUMNS} FROM fulfillment.shipping_zone WHERE tenant_id = $1
         ORDER BY zone_type DESC, name, id"
    )))
    .bind(tx.tenant.as_str())
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

pub async fn zone(tx: &mut TxCx, zone_id: &str) -> Result<Option<ZoneRow>, ToolError> {
    sqlx::query_as::<_, ZoneRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {ZONE_COLUMNS} FROM fulfillment.shipping_zone WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(zone_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A shipping zone as written: every column but the bookkeeping ones.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ZoneValues {
    pub name: String,
    pub zone_type: String,
    pub shipping_destination: Value,
    pub fee_type: String,
    pub shipping_rate: Money,
    pub free_over: Option<Money>,
    pub surcharge: Money,
    pub transit_business_days: i32,
}

pub async fn insert_zone(tx: &mut TxCx, id: &str, z: &ZoneValues) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO fulfillment.shipping_zone (tenant_id, id, name, zone_type,
           shipping_destination, fee_type, shipping_rate_value, shipping_rate_currency,
           free_shipping_threshold_value, free_shipping_threshold_currency, surcharge_value,
           surcharge_currency, transit_business_days)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(&z.name)
    .bind(&z.zone_type)
    .bind(&z.shipping_destination)
    .bind(&z.fee_type)
    .bind(z.shipping_rate.value)
    .bind(z.shipping_rate.currency.as_str())
    .bind(z.free_over.map(|m| m.value))
    .bind(z.free_over.map(|m| m.currency.as_str().to_owned()))
    .bind(z.surcharge.value)
    .bind(z.surcharge.currency.as_str())
    .bind(z.transit_business_days)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

pub async fn update_zone(tx: &mut TxCx, id: &str, z: &ZoneValues) -> Result<bool, ToolError> {
    let n = sqlx::query(
        "UPDATE fulfillment.shipping_zone SET name = $3, zone_type = $4,
           shipping_destination = $5, fee_type = $6, shipping_rate_value = $7,
           shipping_rate_currency = $8, free_shipping_threshold_value = $9,
           free_shipping_threshold_currency = $10, surcharge_value = $11,
           surcharge_currency = $12, transit_business_days = $13, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(&z.name)
    .bind(&z.zone_type)
    .bind(&z.shipping_destination)
    .bind(&z.fee_type)
    .bind(z.shipping_rate.value)
    .bind(z.shipping_rate.currency.as_str())
    .bind(z.free_over.map(|m| m.value))
    .bind(z.free_over.map(|m| m.currency.as_str().to_owned()))
    .bind(z.surcharge.value)
    .bind(z.surcharge.currency.as_str())
    .bind(z.transit_business_days)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected();
    Ok(n == 1)
}

pub async fn delete_zone(tx: &mut TxCx, id: &str) -> Result<(), ToolError> {
    sqlx::query("DELETE FROM fulfillment.shipping_zone WHERE tenant_id = $1 AND id = $2")
        .bind(tx.tenant.as_str())
        .bind(id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    Ok(())
}

impl ZoneRow {
    /// The zone's values, with money read from the value + currency pairs.
    pub fn values(&self) -> Result<ZoneValues, ToolError> {
        Ok(ZoneValues {
            name: self.name.clone(),
            zone_type: self.zone_type.clone(),
            shipping_destination: self.shipping_destination.clone(),
            fee_type: self.fee_type.clone(),
            shipping_rate: money(self.shipping_rate_value, &self.shipping_rate_currency)?,
            free_over: match (
                self.free_shipping_threshold_value,
                &self.free_shipping_threshold_currency,
            ) {
                (Some(v), Some(c)) => Some(money(v, c)?),
                _ => None,
            },
            surcharge: money(self.surcharge_value, &self.surcharge_currency)?,
            transit_business_days: self.transit_business_days,
        })
    }
}
