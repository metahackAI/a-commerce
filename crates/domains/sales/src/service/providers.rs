//! Readers that domains depending on `sales` install at start-up, so order reads can show their
//! data without a dependency cycle. Fulfillment installs the delivery reader (`get_order` shows
//! the parcel). A read whose reader is missing falls back to what the order recorded.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use acom_contract::ToolError;
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use serde_json::Value;

/// What a reader returns: a boxed future borrowing the transaction.
pub type Reading<'a, T> = Pin<Box<dyn Future<Output = Result<T, ToolError>> + Send + 'a>>;

/// An order's parcel delivery, as `get_order` shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryFacts {
    /// `dlv_…`
    pub id: String,
    /// `{name, street_address, address_locality, address_region, postal_code, address_country}`.
    pub delivery_address: Value,
    pub expected_arrival_until: Option<DateTime<Utc>>,
    /// `delivery_state` word.
    pub delivery_state: String,
    pub delivered_at: Option<DateTime<Utc>>,
}

/// The delivery of order `order_id` (the latest, should there be several).
pub type DeliveryReader = for<'a> fn(&'a mut TxCx, &'a str) -> Reading<'a, Option<DeliveryFacts>>;

static DELIVERIES: OnceLock<DeliveryReader> = OnceLock::new();

/// Fulfillment installs this when it registers.
pub fn provide_deliveries(f: DeliveryReader) {
    let _ = DELIVERIES.set(f);
}

pub(crate) fn deliveries() -> Option<DeliveryReader> {
    DELIVERIES.get().copied()
}
