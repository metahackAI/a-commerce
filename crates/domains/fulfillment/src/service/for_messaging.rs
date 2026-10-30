//! What `messaging` reads and changes in fulfillment: the open return requests of an order
//! (the policy grounds of a photo request or a decline), its refunds (a refund notice), and the
//! claim moving to `photos_requested` once the photo request is sent.

use acom_contract::ToolError;
use acom_core::Money;
use acom_sales::service::for_fulfillment as sales;
use acom_store::TxCx;
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::json;

use crate::model::delivery::{days_since, local_date};
use crate::model::money_text;
use crate::model::returns::photo_count;
use crate::repo::{self, ClaimEvent};
use crate::{delivered_at, payment_label};

/// An open return or exchange request on an order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRequest {
    pub claim_id: String,
    pub claim_type: String,
    pub status: String,
    pub reason: Option<String>,
    /// Photos attached so far.
    pub photos: i64,
    /// Store-local delivery day and the days since, when the order was delivered.
    pub delivered_on: Option<NaiveDate>,
    pub days_since_delivery: Option<i64>,
    pub version: i32,
}

/// The order's return and exchange requests that are still open, oldest first.
pub async fn open_requests(
    tx: &mut TxCx,
    order_id: &str,
    now: DateTime<Utc>,
) -> Result<Vec<OpenRequest>, ToolError> {
    let zone = tx.time.zone;
    let claims = repo::claims_on_order(tx, order_id).await?;
    let deliveries = repo::deliveries_of_orders(tx, &[order_id.to_owned()]).await?;
    let delivered = delivered_at(&deliveries, order_id);
    Ok(claims
        .into_iter()
        .filter(|c| c.claim_type != "cancel" && c.is_open())
        .map(|c| OpenRequest {
            photos: photo_count(&c.evidence),
            claim_id: c.id,
            claim_type: c.claim_type,
            status: c.claim_status,
            reason: c.claim_reason,
            delivered_on: delivered.map(|d| local_date(d, zone)),
            days_since_delivery: delivered.map(|d| days_since(d, now, zone)),
            version: c.version,
        })
        .collect())
}

/// A refund on an order, as a refund notice tells it.
#[derive(Debug, Clone, PartialEq)]
pub struct RefundNotice {
    pub refund_id: String,
    pub claim_id: String,
    pub amount: Money,
    /// `card ending 0937`.
    pub to: String,
    pub refund_status: String,
}

impl RefundNotice {
    /// `$312.00 back to card ending 0937`.
    pub fn sentence(&self) -> String {
        format!("{} back to {}", money_text(self.amount), self.to)
    }
}

/// The refunds on an order that did not fail.
pub async fn refunds(tx: &mut TxCx, order_id: &str) -> Result<Vec<RefundNotice>, ToolError> {
    let rows = repo::refunds_on_order(tx, order_id).await?;
    let payments = sales::payments(tx, order_id).await?;
    rows.into_iter()
        .map(|r| {
            let to = match r.refund_method.as_str() {
                "original_payment" => r
                    .payment_id
                    .as_ref()
                    .and_then(|id| payments.iter().find(|p| &p.id == id))
                    .map(payment_label)
                    .unwrap_or_else(|| "the original payment".to_owned()),
                other => other.replace('_', " "),
            };
            Ok(RefundNotice {
                amount: r.amount()?,
                refund_id: r.id,
                claim_id: r.claim_id,
                to,
                refund_status: r.refund_status,
            })
        })
        .collect()
}

/// After a photo request is sent: each waiting request moves `requested` → `photos_requested`
/// (claims already past that are left alone). Returns the claims that moved.
pub async fn photos_requested(
    tx: &mut TxCx,
    claim_ids: &[String],
    message_id: &str,
) -> Result<Vec<String>, ToolError> {
    let mut moved = Vec::new();
    for id in claim_ids {
        if repo::move_claim(tx, id, "requested", "photos_requested").await? {
            repo::append_event(
                tx,
                id,
                &ClaimEvent {
                    action_type: "AskAction",
                    action_status: "completed",
                    instrument: "send_customer_message",
                    from_state: Some("requested"),
                    to_state: Some("photos_requested"),
                    result: Some(json!({"message_id": message_id})),
                },
            )
            .await?;
            moved.push(id.clone());
        }
    }
    Ok(moved)
}
