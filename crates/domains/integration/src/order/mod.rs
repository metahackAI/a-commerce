//! The `order` resource (docs/api/integration.md `## order`): orders as outside channels hold
//! them. The store's own orders are `sales`; this resource reads the channels' copies and keeps
//! an order's status in step with its channel, moving it only the way sales allows.

pub(crate) mod lookup_external_order;
pub(crate) mod sync_order_status;

pub use lookup_external_order::LookupExternalOrder;
pub use sync_order_status::SyncOrderStatus;

use acom_contract::ToolError;
use serde_json::json;

use crate::ORDER_NOT_MAPPED;

/// The channel has no such order, or the store has no order for it yet.
pub(crate) fn not_mapped(channel_id: &str, reference: &str, why: &str) -> ToolError {
    ToolError::new(
        ORDER_NOT_MAPPED,
        format!("{reference} on {channel_id} {why}."),
    )
    .detail(json!({"entity": "order", "channel_id": channel_id, "external_order_ref": reference}))
    .next("lookup_external_order")
}
