//! Readers that domains depending on `sales` install at start-up, so order reads can show their
//! data without a dependency cycle. Fulfillment installs the delivery reader (`get_order` shows
//! the parcel), the claim reader (`get_order` shows cancel, return and exchange requests) and the
//! area reader (`change_shipping_address` checks the new address's surcharge). A read whose
//! reader is missing falls back to what the order recorded, or to the policy's default zone.

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use acom_contract::ToolError;
use acom_core::Money;
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

/// A cancel, return or exchange request on an order, as `get_order` shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimFacts {
    /// `clm_…`
    pub id: String,
    /// `cancel` | `return` | `exchange`.
    pub claim_type: String,
    /// `acom:claim_status` word.
    pub claim_status: String,
    /// The order lines it covers (`oln_…`).
    pub order_item_ids: Vec<String>,
    pub requested_at: DateTime<Utc>,
}

/// The claims of order `order_id`, oldest first.
pub type ClaimReader = for<'a> fn(&'a mut TxCx, &'a str) -> Reading<'a, Vec<ClaimFacts>>;

static CLAIMS: OnceLock<ClaimReader> = OnceLock::new();

/// Fulfillment installs this when it registers.
pub fn provide_claims(f: ClaimReader) {
    let _ = CLAIMS.set(f);
}

pub(crate) fn claims() -> Option<ClaimReader> {
    CLAIMS.get().copied()
}

/// The delivery area (shipping zone) an address is in, as `change_shipping_address` checks it.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaFacts {
    /// The zone's name ("Alaska and Hawaii").
    pub name: String,
    /// What the area adds to every parcel sent there.
    pub surcharge: Money,
}

/// The area of an address in `country` (ISO 3166-1 alpha-2) and, when known, `region` (a state
/// code); `None` when the store has saved no zone that covers it.
pub type AreaReader =
    for<'a> fn(&'a mut TxCx, &'a str, Option<&'a str>) -> Reading<'a, Option<AreaFacts>>;

static AREAS: OnceLock<AreaReader> = OnceLock::new();

/// Fulfillment installs this when it registers.
pub fn provide_areas(f: AreaReader) {
    let _ = AREAS.set(f);
}

pub(crate) fn areas() -> Option<AreaReader> {
    AREAS.get().copied()
}
