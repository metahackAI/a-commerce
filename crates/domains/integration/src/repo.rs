//! SQL on the integration schema. Every query names `tenant_id` as well, although row-level
//! security already keeps stores apart. Other domains' tables are never read here.

use std::collections::BTreeMap;

use acom_contract::ToolError;
use acom_core::{Currency, Money};
use acom_store::{StoreError, TxCx};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};

pub(crate) fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

// ---------------------------------------------------------------- channels

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ChannelRow {
    pub id: String,
    pub name: String,
    pub channel_type: String,
    pub url: String,
    pub connection_status: String,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub seller_id: Option<String>,
    pub signing_secret_hash: Option<String>,
    pub status_mapping: Value,
}

impl ChannelRow {
    /// The internal word a channel's status word maps to in one part (`order`, `claim`).
    pub fn maps(&self, part: &str, external: &str) -> Option<String> {
        self.status_mapping
            .get(part)?
            .get(external.trim())?
            .as_str()
            .map(str::to_owned)
    }
}

const CHANNEL_COLUMNS: &str = "id, name, channel_type, url, connection_status, last_synced_at,
    seller_id, signing_secret_hash, status_mapping";

pub(crate) async fn channel(tx: &mut TxCx, id: &str) -> Result<Option<ChannelRow>, ToolError> {
    sqlx::query_as::<_, ChannelRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {CHANNEL_COLUMNS} FROM integration.channel WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Channels of a type and connection status (`None` = any), by name, and how many there are.
pub(crate) async fn channels(
    tx: &mut TxCx,
    channel_type: Option<&str>,
    connection_status: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<(Vec<ChannelRow>, i64), ToolError> {
    let rows = sqlx::query_as::<_, ChannelRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {CHANNEL_COLUMNS} FROM integration.channel
         WHERE tenant_id = $1 AND ($2::text IS NULL OR channel_type = $2)
           AND ($3::text IS NULL OR connection_status = $3)
         ORDER BY channel_type, name, id LIMIT $4 OFFSET $5"
    )))
    .bind(tx.tenant.as_str())
    .bind(channel_type)
    .bind(connection_status)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM integration.channel
         WHERE tenant_id = $1 AND ($2::text IS NULL OR channel_type = $2)
           AND ($3::text IS NULL OR connection_status = $3)",
    )
    .bind(tx.tenant.as_str())
    .bind(channel_type)
    .bind(connection_status)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((rows, total))
}

/// The channel with this name (case ignored) and address, if any.
pub(crate) async fn channel_named(
    tx: &mut TxCx,
    name: &str,
    url: &str,
) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM integration.channel
         WHERE tenant_id = $1 AND lower(name) = lower($2) AND url = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(name)
    .bind(url)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A channel to insert. Secrets come as hashes only.
#[derive(Debug, Clone)]
pub(crate) struct NewChannel<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub channel_type: &'a str,
    pub url: &'a str,
    pub seller_id: Option<&'a str>,
    pub credential_hash: &'a str,
    pub signing_secret_hash: Option<&'a str>,
    pub status_mapping: &'a Value,
}

pub(crate) async fn insert_channel(tx: &mut TxCx, c: &NewChannel<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO integration.channel (tenant_id, id, name, channel_type, url,
           connection_status, seller_id, credential_hash, signing_secret_hash, status_mapping)
         VALUES ($1, $2, $3, $4, $5, 'connected', $6, $7, $8, $9)",
    )
    .bind(tx.tenant.as_str())
    .bind(c.id)
    .bind(c.name)
    .bind(c.channel_type)
    .bind(c.url)
    .bind(c.seller_id)
    .bind(c.credential_hash)
    .bind(c.signing_secret_hash)
    .bind(c.status_mapping)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Whether anything refers to the channel (orders, claims or events it sent).
pub(crate) async fn channel_in_use(tx: &mut TxCx, id: &str) -> Result<bool, ToolError> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM integration.external_order WHERE tenant_id = $1 AND channel_id = $2)
             OR EXISTS (SELECT 1 FROM integration.external_claim WHERE tenant_id = $1 AND channel_id = $2)
             OR EXISTS (SELECT 1 FROM integration.webhook_event WHERE tenant_id = $1 AND channel_id = $2)",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

pub(crate) async fn delete_channel(tx: &mut TxCx, id: &str) -> Result<(), ToolError> {
    sqlx::query("DELETE FROM integration.channel WHERE tenant_id = $1 AND id = $2")
        .bind(tx.tenant.as_str())
        .bind(id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    Ok(())
}

/// Records that the channel was heard from (or written to) now.
pub(crate) async fn touch_channel(
    tx: &mut TxCx,
    id: &str,
    at: DateTime<Utc>,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE integration.channel SET last_synced_at = $3, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

// ---------------------------------------------------------------- channel orders and claims

/// An order or a claim as a channel holds it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ExternalRow {
    pub channel_id: String,
    /// `external_order_ref` or `external_claim_ref`.
    pub reference: String,
    /// The channel's order a claim is about; the order itself for an order.
    pub order_reference: String,
    /// Claims only: `cancel`, `return` or `exchange`.
    pub claim_type: Option<String>,
    /// `order_id` or `claim_id`.
    pub mapped_id: Option<String>,
    pub external_status: String,
    pub ordered_item: Value,
    /// `order_date` or `start_time`.
    pub at: DateTime<Utc>,
    pub last_synced_at: DateTime<Utc>,
}

/// Which copies: orders or claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyKind {
    Order,
    Claim,
}

impl CopyKind {
    fn select(self) -> &'static str {
        match self {
            CopyKind::Order => {
                "SELECT channel_id, external_order_ref AS reference,
                        external_order_ref AS order_reference, NULL::text AS claim_type,
                        order_id AS mapped_id, external_status, ordered_item, order_date AS at,
                        last_synced_at
                 FROM integration.external_order"
            }
            CopyKind::Claim => {
                "SELECT channel_id, external_claim_ref AS reference,
                        external_order_ref AS order_reference, claim_type,
                        claim_id AS mapped_id, external_status, ordered_item, start_time AS at,
                        last_synced_at
                 FROM integration.external_claim"
            }
        }
    }

    fn table(self) -> (&'static str, &'static str, &'static str) {
        match self {
            CopyKind::Order => (
                "integration.external_order",
                "external_order_ref",
                "order_date",
            ),
            CopyKind::Claim => (
                "integration.external_claim",
                "external_claim_ref",
                "start_time",
            ),
        }
    }
}

/// What to look for among a channel's copies.
#[derive(Debug, Clone, Default)]
pub(crate) struct CopyFilter {
    pub reference: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub through: Option<DateTime<Utc>>,
    pub external_status: Option<String>,
    pub claim_type: Option<String>,
}

pub(crate) async fn copies(
    tx: &mut TxCx,
    kind: CopyKind,
    channel_id: &str,
    f: &CopyFilter,
    limit: i64,
    offset: i64,
) -> Result<(Vec<ExternalRow>, i64), ToolError> {
    let (table, reference, at) = kind.table();
    let claim_type = if kind == CopyKind::Claim {
        "AND ($7::text IS NULL OR claim_type = $7)"
    } else {
        "AND ($7::text IS NULL OR TRUE)"
    };
    let filter = format!(
        "WHERE tenant_id = $1 AND channel_id = $2 AND ($3::text IS NULL OR {reference} = $3)
           AND ($4::timestamptz IS NULL OR {at} >= $4) AND ($5::timestamptz IS NULL OR {at} <= $5)
           AND ($6::text IS NULL OR external_status = $6) {claim_type}"
    );
    let rows = sqlx::query_as::<_, ExternalRow>(sqlx::AssertSqlSafe(format!(
        "{} {filter} ORDER BY {at} DESC, {reference} LIMIT $8 OFFSET $9",
        kind.select()
    )))
    .bind(tx.tenant.as_str())
    .bind(channel_id)
    .bind(f.reference.as_deref())
    .bind(f.from)
    .bind(f.through)
    .bind(f.external_status.as_deref())
    .bind(f.claim_type.as_deref())
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    let total: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {table} {filter}"
    )))
    .bind(tx.tenant.as_str())
    .bind(channel_id)
    .bind(f.reference.as_deref())
    .bind(f.from)
    .bind(f.through)
    .bind(f.external_status.as_deref())
    .bind(f.claim_type.as_deref())
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((rows, total))
}

/// One copy by the channel's reference.
pub(crate) async fn copy(
    tx: &mut TxCx,
    kind: CopyKind,
    channel_id: &str,
    reference: &str,
) -> Result<Option<ExternalRow>, ToolError> {
    let (_, column, _) = kind.table();
    sqlx::query_as::<_, ExternalRow>(sqlx::AssertSqlSafe(format!(
        "{} WHERE tenant_id = $1 AND channel_id = $2 AND {column} = $3",
        kind.select()
    )))
    .bind(tx.tenant.as_str())
    .bind(channel_id)
    .bind(reference)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The store's order a channel order reference maps to, on any marketplace channel.
pub(crate) async fn mapped_order(
    tx: &mut TxCx,
    external_order_ref: &str,
) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT order_id FROM integration.external_order
         WHERE tenant_id = $1 AND external_order_ref = $2 AND order_id IS NOT NULL
         ORDER BY last_synced_at DESC LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(external_order_ref)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Keeps the channel's status on our copy, and when it was synced.
pub(crate) async fn set_copy_status(
    tx: &mut TxCx,
    kind: CopyKind,
    channel_id: &str,
    reference: &str,
    external_status: &str,
    at: DateTime<Utc>,
) -> Result<(), ToolError> {
    let (table, column, _) = kind.table();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE {table} SET external_status = $4, last_synced_at = $5, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND channel_id = $2 AND {column} = $3"
    )))
    .bind(tx.tenant.as_str())
    .bind(channel_id)
    .bind(reference)
    .bind(external_status)
    .bind(at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

// ---------------------------------------------------------------- imports

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ImportRow {
    pub id: String,
    pub seller_id: Option<String>,
    pub import_mode: String,
    pub action_status: String,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
}

pub(crate) async fn import(tx: &mut TxCx, id: &str) -> Result<Option<ImportRow>, ToolError> {
    sqlx::query_as::<_, ImportRow>(
        "SELECT id, seller_id, import_mode, action_status, start_time, end_time
         FROM integration.product_import WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct ImportItemRow {
    pub position: i32,
    pub external_id: String,
    pub product_group_id: Option<String>,
    pub item_status: String,
    pub error: Option<Value>,
}

pub(crate) async fn import_items(
    tx: &mut TxCx,
    import_id: &str,
) -> Result<Vec<ImportItemRow>, ToolError> {
    sqlx::query_as::<_, ImportItemRow>(
        "SELECT position, external_id, product_group_id, item_status, error
         FROM integration.product_import_item WHERE tenant_id = $1 AND product_import_id = $2
         ORDER BY position",
    )
    .bind(tx.tenant.as_str())
    .bind(import_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// What the earlier completed imports of the same feed (a seller's, or the store's own) left
/// behind: their category words, and the products and variants their codes became.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Remembered {
    /// Outside category word → `cat_…`.
    pub categories: BTreeMap<String, String>,
    /// Outside product code → `prd_…`.
    pub products: BTreeMap<String, String>,
    /// Outside variant code → `var_…`.
    pub variants: BTreeMap<String, String>,
}

pub(crate) async fn remembered(
    tx: &mut TxCx,
    seller_id: Option<&str>,
) -> Result<Remembered, ToolError> {
    let maps: Vec<Value> = sqlx::query_scalar(
        "SELECT category_mapping FROM integration.product_import
         WHERE tenant_id = $1 AND seller_id IS NOT DISTINCT FROM $2 AND action_status = 'completed'
         ORDER BY start_time, id",
    )
    .bind(tx.tenant.as_str())
    .bind(seller_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    let mut out = Remembered::default();
    for m in maps {
        if let Some(m) = m.as_object() {
            for (word, id) in m {
                if let Some(id) = id.as_str() {
                    out.categories.insert(word.to_lowercase(), id.to_owned());
                }
            }
        }
    }
    let items: Vec<(String, Option<String>, Value)> = sqlx::query_as(
        "SELECT i.external_id, i.product_group_id, i.variant_mapping
         FROM integration.product_import_item i
         JOIN integration.product_import p
           ON p.tenant_id = i.tenant_id AND p.id = i.product_import_id
         WHERE i.tenant_id = $1 AND p.seller_id IS NOT DISTINCT FROM $2
           AND p.action_status = 'completed' AND i.item_status = 'accepted'
         ORDER BY p.start_time, p.id, i.position",
    )
    .bind(tx.tenant.as_str())
    .bind(seller_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    for (code, product, variants) in items {
        if let Some(p) = product {
            out.products.insert(code.to_lowercase(), p);
        }
        for v in variants.as_array().into_iter().flatten() {
            if let (Some(code), Some(id)) = (
                v.get("external_id").and_then(Value::as_str),
                v.get("variant_id").and_then(Value::as_str),
            ) {
                out.variants.insert(code.to_lowercase(), id.to_owned());
            }
        }
    }
    Ok(out)
}

/// An import to record.
#[derive(Debug, Clone)]
pub(crate) struct NewImport<'a> {
    pub id: &'a str,
    pub seller_id: Option<&'a str>,
    pub import_mode: &'a str,
    pub category_mapping: &'a BTreeMap<String, String>,
    pub at: DateTime<Utc>,
}

pub(crate) async fn insert_import(tx: &mut TxCx, i: &NewImport<'_>) -> Result<(), ToolError> {
    let agent = tx.principal.subject.clone();
    let change_set = tx.change_set_id.clone();
    sqlx::query(
        "INSERT INTO integration.product_import (tenant_id, id, seller_id, import_mode,
           action_status, category_mapping, agent_id, change_set_id, start_time, end_time)
         VALUES ($1, $2, $3, $4, 'completed', $5, $6, $7, $8, $8)",
    )
    .bind(tx.tenant.as_str())
    .bind(i.id)
    .bind(i.seller_id)
    .bind(i.import_mode)
    .bind(json!(i.category_mapping))
    .bind(&agent)
    .bind(change_set.as_deref())
    .bind(i.at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// One row's result to record.
#[derive(Debug, Clone)]
pub(crate) struct NewImportItem<'a> {
    pub position: i32,
    pub external_id: &'a str,
    pub product_group_id: Option<&'a str>,
    pub item_status: &'a str,
    pub variant_mapping: Value,
    pub error: Option<Value>,
}

pub(crate) async fn insert_import_item(
    tx: &mut TxCx,
    import_id: &str,
    i: &NewImportItem<'_>,
) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO integration.product_import_item (tenant_id, product_import_id, position,
           external_id, product_group_id, item_status, variant_mapping, error)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(tx.tenant.as_str())
    .bind(import_id)
    .bind(i.position)
    .bind(i.external_id)
    .bind(i.product_group_id)
    .bind(i.item_status)
    .bind(&i.variant_mapping)
    .bind(&i.error)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

pub(crate) async fn delete_import(tx: &mut TxCx, id: &str) -> Result<(), ToolError> {
    sqlx::query(
        "DELETE FROM integration.product_import_item WHERE tenant_id = $1 AND product_import_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    sqlx::query("DELETE FROM integration.product_import WHERE tenant_id = $1 AND id = $2")
        .bind(tx.tenant.as_str())
        .bind(id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    Ok(())
}

// ---------------------------------------------------------------- received events

#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct EventRow {
    pub id: String,
    pub carrier_event_type: Option<String>,
    pub payment_event_type: Option<String>,
    pub order_id: String,
    pub tracking_number: Option<String>,
    pub payment_ref: Option<String>,
    pub amount_value: Option<Decimal>,
    pub amount_currency: Option<String>,
    pub start_time: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub action_status: String,
}

impl EventRow {
    pub fn amount(&self) -> Option<Money> {
        let currency = Currency::parse(self.amount_currency.as_deref()?.trim()).ok()?;
        Some(Money::new(self.amount_value?, currency))
    }
}

const EVENT_COLUMNS: &str = "id, carrier_event_type, payment_event_type, order_id,
    tracking_number, payment_ref, amount_value, amount_currency::text AS amount_currency,
    start_time, received_at, action_status";

pub(crate) async fn event(tx: &mut TxCx, id: &str) -> Result<Option<EventRow>, ToolError> {
    sqlx::query_as::<_, EventRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {EVENT_COLUMNS} FROM integration.webhook_event WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The event recorded under this key, if the same event came before.
pub(crate) async fn event_by_key(tx: &mut TxCx, key: &str) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM integration.webhook_event WHERE tenant_id = $1 AND event_key = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(key)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A received event to record.
#[derive(Debug, Clone)]
pub(crate) struct NewEvent<'a> {
    pub id: &'a str,
    pub channel_id: &'a str,
    pub carrier_event_type: Option<&'a str>,
    pub payment_event_type: Option<&'a str>,
    pub event_key: &'a str,
    pub external_order_ref: &'a str,
    pub order_id: &'a str,
    pub tracking_number: Option<&'a str>,
    pub payment_ref: Option<&'a str>,
    pub amount: Option<Money>,
    pub start_time: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
}

pub(crate) async fn insert_event(tx: &mut TxCx, e: &NewEvent<'_>) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO integration.webhook_event (tenant_id, id, channel_id, carrier_event_type,
           payment_event_type, event_key, external_order_ref, order_id, tracking_number,
           payment_ref, amount_value, amount_currency, start_time, received_at, action_status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, 'potential')",
    )
    .bind(tx.tenant.as_str())
    .bind(e.id)
    .bind(e.channel_id)
    .bind(e.carrier_event_type)
    .bind(e.payment_event_type)
    .bind(e.event_key)
    .bind(e.external_order_ref)
    .bind(e.order_id)
    .bind(e.tracking_number)
    .bind(e.payment_ref)
    .bind(e.amount.map(|m| m.value))
    .bind(e.amount.map(|m| m.currency.to_string()))
    .bind(e.start_time)
    .bind(e.received_at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// How a recorded event ended.
#[derive(Debug, Clone, Default)]
pub(crate) struct EventEnd {
    /// `completed` or `failed`.
    pub action_status: &'static str,
    pub delivery_id: Option<String>,
    pub payment_id: Option<String>,
    pub refund_id: Option<String>,
    pub result: Option<Value>,
    pub error: Option<Value>,
    pub at: Option<DateTime<Utc>>,
}

pub(crate) async fn end_event(tx: &mut TxCx, id: &str, e: &EventEnd) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE integration.webhook_event SET action_status = $3, delivery_id = $4,
           payment_id = $5, refund_id = $6, result = $7, error = $8, end_time = $9,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(e.action_status)
    .bind(e.delivery_id.as_deref())
    .bind(e.payment_id.as_deref())
    .bind(e.refund_id.as_deref())
    .bind(&e.result)
    .bind(&e.error)
    .bind(e.at)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}
