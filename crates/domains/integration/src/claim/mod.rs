//! The `claim` resource (docs/api/integration.md `## claim`): cancel, return and exchange
//! requests as outside channels hold them. The store's own claims are `fulfillment`; this
//! resource reads the channels' copies and keeps a claim's status in step with its channel,
//! moving it only the way fulfillment allows. Refunds never go through here (`refund_return`).

pub(crate) mod lookup_external_claim;
pub(crate) mod sync_claim_status;

pub use lookup_external_claim::LookupExternalClaim;
pub use sync_claim_status::SyncClaimStatus;

use acom_contract::ToolError;
use serde_json::json;

use crate::CLAIM_NOT_MAPPED;

/// The channel has no such request, or the store has no claim for it yet.
pub(crate) fn not_mapped(channel_id: &str, reference: &str, why: &str) -> ToolError {
    ToolError::new(
        CLAIM_NOT_MAPPED,
        format!("{reference} on {channel_id} {why}."),
    )
    .detail(json!({"entity": "claim", "channel_id": channel_id, "external_claim_ref": reference}))
    .next("lookup_external_claim")
}
