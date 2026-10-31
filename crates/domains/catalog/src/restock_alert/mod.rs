//! The `restock_alert` resource (docs/api/catalog.md#restock_alert): a shopper asks to hear when a
//! sold-out variant is back. When a stock change brings the variant back in stock by itself
//! (`inventory.auto_restore`, with `notify_restock_alerts: true`), the alerts waiting for it are
//! marked sent and handed to messaging through the outbox (`restock_alert.notify`), as a
//! transactional message (`restock_alert.message_kind`). Catalog sends nothing itself: delivery is
//! the messaging domain's.

pub mod list_restock_subscriptions;
pub mod subscribe_restock;

use acom_contract::{Change, ChangeOp, ToolError};
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::db;
use crate::rules::RestockRules;

/// The outbox topic a restock alert goes out under.
pub(crate) const NOTIFY_TOPIC: &str = "restock_alert.notify";

/// An alert as its shopper's list shows it.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct AlertRow {
    pub id: String,
    pub product_group_id: String,
    pub product_name: String,
    pub product_id: String,
    pub size: Option<String>,
    pub color: Option<String>,
    pub message_channel: String,
    pub availability: Option<String>,
    pub start_time: DateTime<Utc>,
    pub expires: DateTime<Utc>,
    pub notified_at: Option<DateTime<Utc>>,
}

/// A shopper's alerts, newest first, with the total.
pub(crate) async fn alerts_of(
    tx: &mut TxCx,
    customer_id: &str,
    limit: i64,
    offset: i64,
) -> Result<(Vec<AlertRow>, i64), ToolError> {
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM catalog.restock_alert WHERE tenant_id = $1 AND customer_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    let rows = sqlx::query_as::<_, AlertRow>(
        "SELECT a.id, p.product_group_id, g.name AS product_name, a.product_id, p.size, p.color,
                a.message_channel, o.availability, a.start_time, a.expires, a.notified_at
         FROM catalog.restock_alert a
         JOIN catalog.product p ON p.tenant_id = a.tenant_id AND p.id = a.product_id
         JOIN catalog.product_group g ON g.tenant_id = p.tenant_id AND g.id = p.product_group_id
         LEFT JOIN catalog.offer o ON o.tenant_id = p.tenant_id AND o.product_id = p.id
         WHERE a.tenant_id = $1 AND a.customer_id = $2
         ORDER BY a.start_time DESC, a.id DESC LIMIT $3 OFFSET $4",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((rows, total))
}

/// The open alert (not sent, not lapsed at `at`) of this shopper for this variant and channel.
pub(crate) async fn open_alert(
    tx: &mut TxCx,
    customer_id: &str,
    variant_id: &str,
    channel: &str,
    at: DateTime<Utc>,
) -> Result<Option<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM catalog.restock_alert
         WHERE tenant_id = $1 AND customer_id = $2 AND product_id = $3 AND message_channel = $4
           AND notified_at IS NULL AND expires > $5
         ORDER BY start_time LIMIT 1",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(variant_id)
    .bind(channel)
    .bind(at)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// How many shoppers wait for a variant at `at`, by channel.
pub(crate) async fn waiting(
    tx: &mut TxCx,
    variant_id: &str,
    at: DateTime<Utc>,
) -> Result<Vec<(String, i64)>, ToolError> {
    sqlx::query_as(
        "SELECT message_channel, count(*) FROM catalog.restock_alert
         WHERE tenant_id = $1 AND product_id = $2 AND notified_at IS NULL AND expires > $3
         GROUP BY message_channel ORDER BY message_channel",
    )
    .bind(tx.tenant.as_str())
    .bind(variant_id)
    .bind(at)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The `send` change a stock preview lists when the variant comes back and shoppers wait for it.
pub(crate) fn notify_change(
    variant_id: &str,
    waiting: &[(String, i64)],
    message_kind: &str,
) -> Option<Change> {
    let total: i64 = waiting.iter().map(|(_, n)| n).sum();
    (total > 0).then(|| {
        let by_channel: serde_json::Map<String, Value> =
            waiting.iter().map(|(c, n)| (c.clone(), json!(n))).collect();
        Change {
            op: ChangeOp::Send,
            entity: "restock_alert".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"variant_id": variant_id, "shoppers": total, "by_channel": by_channel,
                       "message_kind": message_kind}),
            note: Some(format!(
                "{total} shopper{} asked to hear when it is back; the alert cannot be taken back",
                if total == 1 { "" } else { "s" }
            )),
        }
    })
}

/// What a stock preview adds when a variant comes back in stock by itself: the alerts that will
/// go out, if the store sends them.
pub(crate) async fn notify_preview(
    tx: &mut TxCx,
    variant_id: &str,
    restored: bool,
    at: DateTime<Utc>,
) -> Result<Option<Change>, ToolError> {
    let rules = RestockRules::from_policy(&tx.policy)?;
    if !restored || !rules.notify {
        return Ok(None);
    }
    let waiting = waiting(tx, variant_id, at).await?;
    Ok(notify_change(variant_id, &waiting, &rules.message_kind))
}

/// The variant is back in stock: marks every alert waiting for it as sent and hands each to
/// messaging through the outbox. Does nothing when the store does not send restock alerts or the
/// variant is not sellable now. Returns how many alerts went out.
pub(crate) async fn notify_back_in_stock(
    tx: &mut TxCx,
    variant_id: &str,
) -> Result<usize, ToolError> {
    let rules = RestockRules::from_policy(&tx.policy)?;
    if !rules.notify {
        return Ok(0);
    }
    let now = tx.clock.now();
    let back: Option<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT p.product_group_id, g.name, p.size, p.color
         FROM catalog.product p
         JOIN catalog.product_group g ON g.tenant_id = p.tenant_id AND g.id = p.product_group_id
         JOIN catalog.offer o ON o.tenant_id = p.tenant_id AND o.product_id = p.id
         WHERE p.tenant_id = $1 AND p.id = $2 AND o.availability = 'in_stock'
           AND o.inventory_level > 0",
    )
    .bind(tx.tenant.as_str())
    .bind(variant_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)?;
    let Some((product_id, name, size, color)) = back else {
        return Ok(0);
    };
    let sent: Vec<(String, String, String)> = sqlx::query_as(
        "UPDATE catalog.restock_alert SET notified_at = $3, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND product_id = $2 AND notified_at IS NULL AND expires > $3
         RETURNING id, customer_id, message_channel",
    )
    .bind(tx.tenant.as_str())
    .bind(variant_id)
    .bind(now)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    for (id, customer_id, channel) in &sent {
        sqlx::query(
            "INSERT INTO platform.outbox (tenant_id, topic, object_id, payload)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(tx.tenant.as_str())
        .bind(NOTIFY_TOPIC)
        .bind(id)
        .bind(json!({"restock_alert_id": id, "customer_id": customer_id,
                     "product_id": product_id, "product_name": name, "variant_id": variant_id,
                     "size": size, "color": color, "channel": channel,
                     "message_kind": rules.message_kind}))
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    }
    Ok(sent.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restock_lists_one_send_for_the_shoppers_waiting() {
        assert!(notify_change("var_x", &[], "transactional").is_none());
        let c = notify_change(
            "var_chore_coat_m_olive",
            &[("email".into(), 2), ("push".into(), 1)],
            "transactional",
        )
        .unwrap_or_else(|| panic!("a change"));
        assert_eq!(c.op, ChangeOp::Send);
        assert_eq!(c.to["shoppers"], 3);
        assert_eq!(c.to["by_channel"]["email"], 2);
    }
}
